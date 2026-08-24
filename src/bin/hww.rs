//! The hww application. A thin main: argv in, `reader::ui::run` out.
//!
//! The binary requires the `gui` feature; keeping that feature off by default leaves the core
//! library, tests, and local tools free of the image subresource path and GUI dependency graph.

use hww::reader::{settings, ui};

fn main() -> eframe::Result {
    let mut target = None;
    let mut opts = hww::session::LoadOptions::default();
    let mut why = false;
    let mut end_of_flags = false;
    for a in std::env::args().skip(1) {
        match a.as_str() {
            "--" if !end_of_flags => end_of_flags = true,
            "--no-rewrite" if !end_of_flags => opts.rewrite = false,
            "--no-profile" if !end_of_flags => opts.profile = false,
            "--no-search" if !end_of_flags => opts.search = false,
            // The extractor's account of a page in place of the page: triage, not reading.
            // Prints and exits without a window.
            "--why" if !end_of_flags => why = true,
            "--show-rewrites" if !end_of_flags => {
                print!("{}", hww::sites::describe_rewrites());
                return Ok(());
            }
            "--show-profiles" if !end_of_flags => {
                print!("{}", hww::sites::describe_profiles());
                return Ok(());
            }
            "--show-engines" if !end_of_flags => {
                print!("{}", hww::sites::describe_engines());
                return Ok(());
            }
            s if !end_of_flags && s.starts_with("--") => {
                eprintln!("unknown flag: {s}");
                std::process::exit(2);
            }
            s if target.is_some() => {
                eprintln!("unexpected extra argument: {s}");
                std::process::exit(2);
            }
            s => target = Some(s.to_owned()),
        }
    }

    // Loaded before the argument is resolved, because which engine a search goes to is part of
    // resolving it. `--why` still renders nothing and opens no window; it reads this file for
    // the one routing decision it cannot make without it, so `hww --why "some words"` and the
    // same words typed into the window reach the same host.
    //
    // A parse failure costs the settings, not the launch, and the reason is shown in the
    // status strip rather than swallowed.
    let (settings, settings_note) = settings::load();
    let engine = hww::sites::engine_by_id(settings.search_engine);

    let start = match target {
        // `hww ""` is an empty argument, which is the shell's version of Enter on a blank URL
        // bar: it asks for nothing, so it opens on nothing rather than searching for it.
        None => None,
        Some(arg) if arg.trim().is_empty() => None,
        // Words rather than an address are a search, at a shell as much as in the URL bar.
        // Quote them: argv splits on spaces and an unquoted second word is still an error.
        Some(arg) => match hww::search::resolve_input(&arg, engine) {
            Ok(u) => Some(u),
            Err(e) => {
                eprintln!("{arg} is not a usable address: {e}");
                std::process::exit(2);
            }
        },
    };

    // `--why` is the same pipeline as a load, reporting on itself instead of rendering the
    // page. It opens no window and renders nothing, but it is not settings-free: the engine a
    // search resolves to came out of that file above, so a file that failed to parse changed
    // where these words are about to go and says so here rather than in a status strip nobody
    // is looking at. Every printed string is sanitized because the Explanation carries the
    // page's own `class`/`id` attributes, and a terminal reads an escape sequence in one of
    // those as a command.
    if why {
        let Some(url) = start else {
            eprintln!("--why needs a URL");
            std::process::exit(2);
        };
        if let Some(note) = &settings_note {
            eprintln!("{}", hww::render::sanitize_for_terminal(note));
        }
        let session = match hww::session::Session::new() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        };
        let mut notify = |n: &hww::session::Rewrite| {
            eprintln!("{}", hww::render::sanitize_for_terminal(&n.to_string()));
        };
        match session.explain(&url, &opts, &mut notify) {
            Ok((explanation, prov)) => {
                eprintln!("{}", hww::render::sanitize_for_terminal(&prov.to_string()));
                if let Some(r) = &prov.profile {
                    eprintln!("{}", hww::render::sanitize_for_terminal(&r.to_string()));
                }
                print!(
                    "{}",
                    hww::render::sanitize_for_terminal(&explanation.to_string())
                );
            }
            Err(e) => {
                eprintln!("{}", hww::render::sanitize_for_terminal(&e.to_string()));
                std::process::exit(1);
            }
        }
        return Ok(());
    }

    ui::run(ui::Launch {
        start,
        settings,
        settings_note,
        opts,
    })
}
