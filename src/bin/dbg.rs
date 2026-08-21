//! Debug one cached page: what did content selection pick, and what came out?
//! usage: dbg <signals.jsonl> <cache-dir> <url-substring> [--text]
use anyhow::Result;
use hww::{html, render};
use serde::Deserialize;

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
            println!("{}", &t[..t.len().min(1400)]);
        }
        break;
    }
    Ok(())
}
