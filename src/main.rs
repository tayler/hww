use anyhow::Result;
use hww::{
    fetch::{self, Fetcher},
    html, render, rewrite,
};

fn main() -> Result<()> {
    let mut target = None;
    let mut rewriting = true;
    let mut end_of_flags = false;
    for a in std::env::args().skip(1) {
        match a.as_str() {
            // The conventional end-of-options separator: what follows is the URL, even if it
            // starts with a dash.
            "--" if !end_of_flags => end_of_flags = true,
            "--no-rewrite" if !end_of_flags => rewriting = false,
            "--show-rewrites" if !end_of_flags => {
                print!("{}", rewrite::describe());
                return Ok(());
            }
            // Without this arm a mistyped flag is silently fetched as if it were the URL.
            s if !end_of_flags && s.starts_with("--") => {
                eprintln!("unknown flag: {s}");
                std::process::exit(2);
            }
            // The same silent-wrong-target one step over: last-wins would fetch the second URL
            // and never mention the first.
            s if target.is_some() => {
                eprintln!("unexpected extra argument: {s}");
                std::process::exit(2);
            }
            s => target = Some(s.to_owned()),
        }
    }
    let Some(arg) = target else {
        eprintln!("usage: hww [--no-rewrite] [--show-rewrites] <url>");
        std::process::exit(2);
    };

    let requested = url::Url::parse(&arg)?;
    let url = if rewriting {
        rewrite::apply(&requested)
    } else {
        requested.clone()
    };
    let rewritten = url != requested;

    // A rewrite changes which host is contacted, so it is reported *before* the request is
    // made. Reporting it alongside the response would let any fetch failure — a bot-block hang
    // on the alternate host being the likeliest — swallow the fact that hww went somewhere the
    // user did not type. Charter point 4 is unconditional.
    if rewritten {
        eprintln!(
            "[rewrote {} -> {}]",
            requested.host_str().unwrap_or("?"),
            url.host_str().unwrap_or("?")
        );
    }

    let resp = Fetcher::new()?.get(&url)?;
    let (source, enc) = fetch::decode(&resp.body, resp.content_type.as_deref());
    let doc = html::extract(&source, &resp.final_url);

    // Not landing on the host we rewrote *to* means the alternate did not hold. The bounce is
    // usually to a sibling of the requested host rather than to that host itself, so comparing
    // against the request would miss the common case. Not an error, but the rule is dead, and
    // saying so is the whole early-warning system: a failed rewrite is not retried.
    let provenance = if rewritten && resp.final_url.host_str() != url.host_str() {
        format!(
            " | {} redirected to {} — rule appears dead",
            url.host_str().unwrap_or("?"),
            resp.final_url.host_str().unwrap_or("?")
        )
    } else {
        String::new()
    };

    eprintln!(
        "[{} {} | {}{} | {} bytes -> {} chars | {} redirect(s) | {} cookie attempt(s) discarded]",
        resp.status,
        enc.name(),
        resp.final_url,
        provenance,
        resp.body.len(),
        doc.text_len(),
        resp.hops.len(),
        resp.cookie_attempts
    );
    print!("{}", render::to_text(&doc, &render::TextOpts::default()));
    Ok(())
}
