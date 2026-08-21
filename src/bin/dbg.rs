//! Debug one cached page: what did content selection pick, and what came out?
//! usage: dbg <signals.jsonl> <cache-dir> <url-substring> [--text]
use anyhow::{Result, bail};
use hww::{html, render};
use serde::Deserialize;

const USAGE: &str = "usage: dbg <signals.jsonl> <cache-dir> <url-substring> [--text]";

/// Truncate to at most `max` *bytes*, on a character boundary.
///
/// `&s[..max]` panics when the boundary lands inside a multi-byte character, which for a
/// three-byte character is most of the time — so this tool used to die on any CJK page.
fn head(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[derive(Deserialize)]
struct Sig {
    chars: usize,
    enc: String,
}
#[derive(Deserialize)]
struct Row {
    url: String,
    key: String,
    status: Option<u16>,
    sig: Option<Sig>,
}

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 4 {
        bail!("{USAGE}");
    }
    let rows: Vec<Row> = std::fs::read_to_string(&a[1])?
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    for r in rows.iter().filter(|r| r.url.contains(&a[3])) {
        let (Some(200), Some(sig)) = (r.status, &r.sig) else {
            continue;
        };
        let Ok(bytes) = std::fs::read(format!("{}/{}.raw", a[2], r.key)) else {
            continue;
        };
        let enc =
            encoding_rs::Encoding::for_label(sig.enc.as_bytes()).unwrap_or(encoding_rs::UTF_8);
        let (text, _, _) = enc.decode(&bytes);
        let doc = html::extract(&text, &url::Url::parse(&r.url)?);
        println!(
            "== {}\n   visible={} blocks={} text_len={} title={:?}\n   ROOT: {}",
            r.url,
            sig.chars,
            doc.blocks.len(),
            doc.text_len(),
            doc.title,
            html::debug_root(&text)
        );
        if a.len() > 4 {
            let t = render::to_text(&doc, &render::TextOpts::default());
            println!("{}", head(&t, 1400));
        }
        break;
    }
    Ok(())
}
