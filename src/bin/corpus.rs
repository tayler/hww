//! Build a test corpus from a local browser history.
//!
//! Replaces the Phase 0 Python sampler. Carries forward its one durable finding: **sample by
//! recency from the visits table, never by `visit_count`.** Frequency ranking surfaces search
//! engines, webmail, and dashboards (the pages a reader never needs), while the articles
//! actually under test are read once and sit at `visit_count = 1`.
//!
//! Classification here is purely structural. No host lists: what a given person reads is not
//! a property of the software, and baking their providers into a classifier is how a test
//! harness ends up encoding somebody's bank.
//!
//! usage: corpus [profile.sqlite] [--per-host N] [--limit N] [--out FILE]

use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Serialize)]
struct Sample {
    url: String,
    title: String,
    host: String,
    last_visit: i64,
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str, default: usize| -> usize {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    };
    let per_host = flag("--per-host", 3);
    let limit = flag("--limit", 150);
    let out = args
        .iter()
        .position(|a| a == "--out")
        .and_then(|i| args.get(i + 1).cloned())
        .unwrap_or_else(|| "corpus.jsonl".into());
    let explicit = args
        .first()
        .filter(|a| !a.starts_with("--"))
        .map(PathBuf::from);

    let profile = match explicit {
        Some(p) => p,
        None => find_profile().context("no Firefox profile found; pass a places.sqlite path")?,
    };
    eprintln!("profile: {}", profile.display());

    // The DB is locked while the browser runs, and recent visits live in the -wal sidecar.
    let tmp = std::env::temp_dir().join(format!("hww-corpus-{}", std::process::id()));
    std::fs::create_dir_all(&tmp)?;
    let staged = tmp.join("places.sqlite");
    for suffix in ["", "-wal", "-shm"] {
        let src = PathBuf::from(format!("{}{suffix}", profile.display()));
        if src.exists() {
            std::fs::copy(&src, format!("{}{suffix}", staged.display()))?;
        }
    }

    let db = rusqlite::Connection::open(&staged)?;
    let mut stmt = db.prepare(
        "SELECT p.url, IFNULL(p.title,''), MAX(v.visit_date) d
         FROM moz_historyvisits v JOIN moz_places p ON p.id = v.place_id
         WHERE v.visit_type IN (1,2,3)   -- link / typed / bookmark; no embeds or redirects
         GROUP BY p.url ORDER BY d DESC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })?;

    let mut counts: HashMap<&'static str, usize> = HashMap::new();
    let mut per: HashMap<String, usize> = HashMap::new();
    let mut picked: Vec<Sample> = Vec::new();
    let mut total = 0usize;

    for row in rows {
        let (url, title, date) = row?;
        total += 1;
        let Ok(parsed) = url::Url::parse(&url) else {
            continue;
        };
        let class = classify(&parsed);
        *counts.entry(class).or_default() += 1;
        if class != "CANDIDATE" || picked.len() >= limit {
            continue;
        }
        let Some(host) = parsed.host_str().map(str::to_owned) else {
            continue;
        };
        let n = per.entry(host.clone()).or_default();
        if *n >= per_host {
            continue;
        }
        *n += 1;
        picked.push(Sample {
            url,
            title,
            host,
            last_visit: date,
        });
    }

    if picked.is_empty() {
        bail!("no document-shaped URLs found in {}", profile.display());
    }

    let mut classes: Vec<_> = counts.iter().collect();
    classes.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
    eprintln!("\n{total} distinct URLs (link/typed/bookmark visits)");
    for (k, n) in classes {
        eprintln!("   {k:10} {n:6}  {:5.1}%", 100.0 * *n as f64 / total as f64);
    }

    let body: Vec<String> = picked
        .iter()
        .map(|s| serde_json::to_string(s).unwrap())
        .collect();
    std::fs::write(&out, body.join("\n") + "\n")?;
    eprintln!(
        "\nsampled {} URLs across {} hosts (max {per_host}/host) -> {out}",
        picked.len(),
        per.len()
    );
    std::fs::remove_dir_all(&tmp).ok();
    Ok(())
}

/// Structural classification only.
fn classify(u: &url::Url) -> &'static str {
    if !matches!(u.scheme(), "http" | "https") {
        return "NONWEB";
    }
    // Query check comes first: many search engines put the query on the site root
    // (`example.com/?q=...`), which the path check below would misfile as ROOT.
    if u.query_pairs()
        .any(|(k, _)| matches!(&*k, "q" | "query" | "search" | "keyword"))
    {
        return "SEARCH";
    }
    let segs: Vec<&str> = u.path().split('/').filter(|s| !s.is_empty()).collect();
    let Some(first) = segs.first() else {
        return "ROOT"; // a bare homepage is a destination, not a document
    };
    if matches!(
        first.to_ascii_lowercase().as_str(),
        "search" | "s" | "find" | "results" | "suche"
    ) {
        return "SEARCH";
    }
    "CANDIDATE"
}

/// Standard install, Snap, and Flatpak locations.
fn find_profile() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let roots = [
        format!("{home}/.mozilla/firefox"),
        format!("{home}/snap/firefox/common/.mozilla/firefox"),
        format!("{home}/.var/app/org.mozilla.firefox/.mozilla/firefox"),
    ];
    roots
        .iter()
        .filter_map(|r| largest_places(Path::new(r)))
        .max_by_key(|(size, _)| *size)
        .map(|(_, p)| p)
}

fn largest_places(root: &Path) -> Option<(u64, PathBuf)> {
    std::fs::read_dir(root)
        .ok()?
        .filter_map(|e| {
            let p = e.ok()?.path().join("places.sqlite");
            let size = std::fs::metadata(&p).ok()?.len();
            Some((size, p))
        })
        .max_by_key(|(size, _)| *size)
}
