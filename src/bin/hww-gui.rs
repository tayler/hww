//! The reading GUI. A thin main: argv in, `reader::ui::run` out.
//!
//! Everything this binary can do that `hww` cannot is behind the `gui` feature, so the CLI
//! keeps compile-time absence of the image subresource path (article pictures on click, and
//! the page favicon).

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
            // The extractor's account of a page in place of the page: the triage tool that
            // used to be `hww --why`. Prints and exits without opening a window.
            "--why" if !end_of_flags => why = true,
            "--show-rewrites" if !end_of_flags => {
                print!("{}", hww::sites::describe_rewrites());
                return Ok(());
            }
            "--show-profiles" if !end_of_flags => {
                print!("{}", hww::sites::describe_profiles());
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

    let start = match target {
        None => None,
        Some(arg) => {
            // A bare host is what people type at a shell too.
            let candidate = if arg.contains("://") {
                arg.clone()
            } else {
                format!("https://{arg}")
            };
            match url::Url::parse(&candidate) {
                Ok(u) => Some(u),
                Err(e) => {
                    eprintln!("{arg} is not a usable address: {e}");
                    std::process::exit(2);
                }
            }
        }
    };

    // `--why` is the same pipeline as a load, reporting on itself instead of rendering the
    // page. It runs headless: no settings, no window. Every printed string is sanitized
    // because the Explanation carries the page's own `class`/`id` attributes, and a terminal
    // reads an escape sequence in one of those as a command.
    if why {
        let Some(url) = start else {
            eprintln!("--why needs a URL");
            std::process::exit(2);
        };
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

    // A parse failure costs the settings, not the launch, and the reason is shown in the
    // status strip rather than swallowed.
    let (settings, settings_note) = settings::load();
    ui::run(ui::Launch {
        start,
        settings,
        settings_note,
        opts,
    })
}
