//! Calibration harness: run the extractor over a local byte cache and compare against a
//! baseline produced by another extractor. Offline — never refetches.
//!
//! The baseline JSON is **not** in this repo, and neither is the cache: both are derived from
//! somebody's real browsing history. Regenerate them locally with `corpus`, a fetcher, and
//! whatever reference extractor you want to measure against (Phase 0 used trafilatura 2.2.0).
//!
//! Day-to-day regression cover lives in `#[cfg(test)]` instead — synthetic fixtures that
//! encode the structural patterns that actually broke, with no corpus and no network.
//!
//! usage: bench <signals.jsonl> <baseline.json> <cache-dir>

use anyhow::Result;
use hww::{html, ir::Document};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize)]
struct Sig { chars: usize, enc: String }
#[derive(Deserialize)]
struct Row { url: String, host: String, key: String, status: Option<u16>, ctype: Option<String>, sig: Option<Sig> }
#[derive(Deserialize)]
struct Traf { url: String, tr: usize }

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let rows: Vec<Row> = std::fs::read_to_string(&a[1])?
        .lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    let traf: HashMap<String, usize> = serde_json::from_str::<Vec<Traf>>(&std::fs::read_to_string(&a[2])?)?
        .into_iter().map(|t| (t.url, t.tr)).collect();

    let (mut win, mut close, mut lose, mut n) = (0, 0, 0, 0);
    let mut rows_out: Vec<(f64, String, usize, usize, usize, String)> = Vec::new();

    for r in rows {
        let (Some(200), Some(sig)) = (r.status, &r.sig) else { continue };
        if !traf.contains_key(&r.url) { continue }
        let Ok(bytes) = std::fs::read(format!("{}/{}.raw", a[3], r.key)) else { continue };
        let enc = encoding_rs::Encoding::for_label(sig.enc.as_bytes()).unwrap_or(encoding_rs::UTF_8);
        let (text, _, _) = enc.decode(&bytes);
        let url = url::Url::parse(&r.url)?;
        let doc: Document = html::extract(&text, &url);
        let ours = doc.text_len();
        let theirs = traf[&r.url];
        n += 1;
        let ratio = ours as f64 / theirs.max(1) as f64;
        if theirs < 250 && ours < 250 { close += 1 }
        else if ratio >= 0.75 { win += 1 }
        else if ratio >= 0.45 { close += 1 }
        else { lose += 1 }
        rows_out.push((ratio, r.host.clone(), sig.chars, theirs, ours, r.url.clone()));
    }

    println!("compared {n} pages against trafilatura\n");
    println!("  >=75% of trafilatura's output : {win:3}  ({:.0}%)", 100.0 * win as f64 / n as f64);
    println!("  45-75% (usable, thinner)      : {close:3}  ({:.0}%)", 100.0 * close as f64 / n as f64);
    println!("  <45% (regression)             : {lose:3}  ({:.0}%)", 100.0 * lose as f64 / n as f64);

    rows_out.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    println!("\nworst 12 (ratio, visible, trafilatura, hww):");
    for (ratio, host, vis, theirs, ours, url) in rows_out.iter().take(12) {
        println!("  {ratio:5.2}  vis={vis:7} traf={theirs:6} hww={ours:6}  {host:24} {}", &url[..url.len().min(52)]);
    }
    println!("\nbest 6:");
    for (ratio, host, vis, theirs, ours, _) in rows_out.iter().rev().take(6) {
        println!("  {ratio:5.2}  vis={vis:7} traf={theirs:6} hww={ours:6}  {host}");
    }
    Ok(())
}
