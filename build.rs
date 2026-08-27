//! The Windows icon resource, and nothing else.
//!
//! The window and taskbar icon is not this: `reader::ui::run` hands egui the bytes of
//! `assets/logo/hww-256.png` through `eframe::icon_data::from_png_bytes`, and that is what a
//! running hww shows. This resource is the icon Explorer, the taskbar's pinned entry, and the
//! Alt+Tab list read off the *file*, before anything has run. Nothing but Windows has that
//! notion, so on every other target this script does nothing at all.
//!
//! The check is `CARGO_CFG_TARGET_OS` rather than `cfg!(windows)`. A build script is compiled
//! for the host, so `cfg!` here would answer for the machine doing the building rather than
//! the machine the binary is for, and would silently skip the resource on any cross build.
//!
//! A missing resource compiler warns and continues. The icon is cosmetic, and refusing to
//! build hww at all because a partial MSVC install lacks `rc.exe` would trade a working
//! reader for a picture.

fn main() {
    println!("cargo::rerun-if-changed=assets/logo/hww.ico");
    println!("cargo::rerun-if-changed=build.rs");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/logo/hww.ico");
    if let Err(e) = res.compile() {
        println!(
            "cargo::warning=hww.ico was not embedded ({e}); the binary keeps the default icon"
        );
    }
}
