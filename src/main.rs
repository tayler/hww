use anyhow::Result;
use hww::{fetch::{self, Fetcher}, html, render};

fn main() -> Result<()> {
    let Some(arg) = std::env::args().nth(1) else {
        eprintln!("usage: hww <url>");
        std::process::exit(2);
    };
    let url = url::Url::parse(&arg)?;
    let resp = Fetcher::new()?.get(&url)?;
    let (source, enc) = fetch::decode(&resp.body, resp.content_type.as_deref());
    let doc = html::extract(&source, &resp.final_url);
    eprintln!(
        "[{} {} | {} | {} bytes -> {} chars | {} redirect(s) | {} cookie attempt(s) discarded]",
        resp.status, enc.name(), resp.final_url, resp.body.len(), doc.text_len(),
        resp.hops.len(), resp.cookie_attempts
    );
    print!("{}", render::to_text(&doc, &render::TextOpts::default()));
    Ok(())
}
