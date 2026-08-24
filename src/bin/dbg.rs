//! Debug one cached page: the extractor's own account of it (`html::explain`), and the text.
//! usage: dbg <signals.jsonl> <cache-dir> <url-substring> [--text]
use anyhow::{Result, bail};
use hww::{html, render};
use serde::Deserialize;

const USAGE: &str = "usage: dbg <signals.jsonl> <cache-dir> <url-substring> [--text]";

/// Truncate to at most `max` *bytes*, on a character boundary.
///
/// `&s[..max]` panics when the boundary lands inside a multi-byte character, which for a
/// three-byte character is most of the time, so this tool used to die on any CJK page.
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
        let url = url::Url::parse(&r.url)?;
        let doc = html::extract(&text, &url);
        // `r.url` and the Explanation carry page-controlled text (a URL, `class`/`id`
        // attributes) to a terminal; sanitize so an escape sequence in one is not a command.
        // `to_text` sanitizes itself.
        println!(
            "== {}\n   visible={} (from the corpus signal)",
            render::sanitize_for_terminal(&r.url),
            sig.chars
        );
        print!(
            "{}",
            render::sanitize_for_terminal(&html::explain(&text, &url).to_string())
        );
        if a.len() > 4 {
            let t = render::to_text(&doc, &render::TextOpts::default());
            println!("{}", head(&t, 1400));
        }
        break;
    }
    Ok(())
}
