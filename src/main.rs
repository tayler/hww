use anyhow::Result;
use hww::{
    render, rewrite, session,
    session::{LoadOptions, Session},
};

fn main() -> Result<()> {
    let mut target = None;
    let mut opts = LoadOptions::default();
    let mut text = render::TextOpts::default();
    let mut why = false;
    let mut end_of_flags = false;
    for a in std::env::args().skip(1) {
        match a.as_str() {
            // The conventional end-of-options separator: what follows is the URL, even if it
            // starts with a dash.
            "--" if !end_of_flags => end_of_flags = true,
            "--no-rewrite" if !end_of_flags => opts.rewrite = false,
            // `show_links` had no caller until this flag: the GUI marks a link by drawing
            // it, and a text renderer that never prints an href leaves the CLI unable to
            // show, or check, what the extractor recovered.
            "--links" if !end_of_flags => text.show_links = true,
            // The extractor's account of itself in place of the page: which root candidates
            // it weighed and chose, what the thread detector saw, where each metadata
            // field came from. What a page is triaged with.
            "--why" if !end_of_flags => why = true,
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
        eprintln!("usage: hww [--links] [--why] [--no-rewrite] [--show-rewrites] <url>");
        std::process::exit(2);
    };

    let requested = url::Url::parse(&arg)?;
    let session = Session::new()?;
    // The notice travels before the request does (see `session::Rewrite`).
    let mut notify = |n: &session::Rewrite| eprintln!("{n}");
    if why {
        let (why, prov) = session.explain(&requested, &opts, &mut notify)?;
        eprintln!("{prov}");
        print!("{why}");
        return Ok(());
    }
    let loaded = session.load(&requested, &opts, &mut notify)?;

    eprintln!("{}", loaded.prov);
    print!("{}", render::to_text(&loaded.doc, &text));
    Ok(())
}
