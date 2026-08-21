//! The reading GUI. A thin main: argv in, `reader::ui::run` out.
//!
//! Everything this binary can do that `hww` cannot is behind the `gui` feature, so the CLI
//! keeps compile-time absence of the reader's one added network capability: loading an image
//! subresource on an explicit click.

use hww::reader::{settings, ui};

fn main() -> eframe::Result {
    let mut target = None;
    let mut rewrite = true;
    let mut end_of_flags = false;
    for a in std::env::args().skip(1) {
        match a.as_str() {
            "--" if !end_of_flags => end_of_flags = true,
            "--no-rewrite" if !end_of_flags => rewrite = false,
            "--show-rewrites" if !end_of_flags => {
                print!("{}", hww::rewrite::describe());
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

    // A parse failure costs the settings, not the launch, and the reason is shown in the
    // status strip rather than swallowed.
    let (settings, settings_note) = settings::load();
    ui::run(ui::Launch {
        start,
        settings,
        settings_note,
        rewrite,
    })
}
