//! What the desktop asks for, for `Theme::System`.
//!
//! egui learns the desktop's light/dark preference from winit, and on Linux winit does not
//! have it: `EventLoop::system_theme` is a hardcoded `None` for both backends, and the
//! Wayland `Window::theme` that looks like an answer is the client-side decoration theme the
//! application itself set. So `raw.system_theme` is always `None` here and `Theme::System`
//! resolved to Light on every Linux desktop, including a desktop in dark mode.
//!
//! The answer the desktop actually publishes is `color-scheme` in the `org.freedesktop.appearance`
//! namespace of the XDG settings portal: `0` no preference, `1` prefer dark, `2` prefer light.
//! It is the same value a browser reads for `prefers-color-scheme`, it is served on the session
//! bus by every desktop that ships a portal, and it changes live. GNOME's own
//! `org.gnome.desktop.interface color-scheme` is the fallback for a desktop with no portal.
//!
//! Both are read by running `gdbus` and `gsettings`, not by linking a D-Bus client. A session
//! bus call is the whole requirement, the two binaries ship with the same glib the desktop
//! publishing those settings is built on, and the alternative is a large async dependency tree
//! for one string. No request leaves the machine, nothing is written, and what is read is two
//! public desktop settings.
//!
//! There is no `cfg` for the platforms that do not work this way. On macOS and Windows winit
//! answers first and this is never consulted; if it were, the commands would simply not be
//! there and it would report knowing nothing, which is the same answer.
//!
//! Only [`watch`] starts any of it. `hww-shot` never calls it, so a screenshot of
//! `--theme system` stays what it always was rather than following the machine that took it.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU8, Ordering};

/// The portal namespace and key. Named once because the query and the change signal that
/// reports the same value must agree on it.
const NAMESPACE: &str = "org.freedesktop.appearance";
const KEY: &str = "color-scheme";

const UNKNOWN: u8 = 0;
const LIGHT: u8 = 1;
const DARK: u8 = 2;

/// The desktop's preference, as last read. Process-global because that is what it describes:
/// one desktop, one answer, consulted from wherever a palette is chosen.
static SCHEME: AtomicU8 = AtomicU8::new(UNKNOWN);

/// Whether the desktop asks for dark. `None` when nothing has been asked or nothing answered,
/// which is not "light": the caller decides what to do without an answer.
pub fn prefers_dark() -> Option<bool> {
    match SCHEME.load(Ordering::Relaxed) {
        DARK => Some(true),
        LIGHT => Some(false),
        _ => None,
    }
}

/// A running `gdbus monitor`. Dropping it kills the child, which ends the reader thread on EOF.
///
/// Without the kill, an exiting hww leaves an idle `gdbus` behind until the next appearance
/// change writes into a closed pipe, so a session of opening and closing the browser
/// accumulates them.
pub struct Watch(Child);

impl Drop for Watch {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Read the preference now, then follow it. `wake` is called on every change, after the new
/// value is stored, so a repaint sees it.
///
/// Returns `None` when the desktop published no preference, or when the change signal could not
/// be subscribed to. A `None` from the second case still leaves a correct value in [`SCHEME`]:
/// the theme is right at startup and simply stops tracking, which is the right degradation for
/// a desktop that answered a question once and cannot be asked again.
#[must_use = "dropping the watch stops it"]
pub fn watch(wake: impl Fn() + Send + 'static) -> Option<Watch> {
    let now = query()?;
    SCHEME.store(encode(now), Ordering::Relaxed);

    let mut child = Command::new("gdbus")
        .args([
            "monitor",
            "--session",
            "--dest",
            "org.freedesktop.portal.Desktop",
            "--object-path",
            "/org/freedesktop/portal/desktop",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let out = child.stdout.take()?;

    std::thread::Builder::new()
        .name("hww-desktop-theme".to_owned())
        .spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                if let Some(dark) = changed(&line) {
                    SCHEME.store(encode(dark), Ordering::Relaxed);
                    wake();
                }
            }
        })
        .ok()?;

    Some(Watch(child))
}

fn encode(dark: bool) -> u8 {
    if dark { DARK } else { LIGHT }
}

/// Ask the portal, then GNOME. The portal is the cross-desktop answer; `ReadOne` is newer than
/// `Read`, and a portal that has only the older method answers the same value wrapped one
/// variant deeper, which [`scheme_reply`] does not care about.
fn query() -> Option<bool> {
    let portal = |method: &str| {
        gdbus(&[
            "call",
            "--session",
            // A call on a local bus answers in under a millisecond, and this one is made before
            // the first frame. D-Bus's own default is 25 seconds, which is a browser that will
            // not start on a session whose portal is wedged.
            "--timeout",
            "1",
            "--dest",
            "org.freedesktop.portal.Desktop",
            "--object-path",
            "/org/freedesktop/portal/desktop",
            "--method",
            &format!("org.freedesktop.portal.Settings.{method}"),
            NAMESPACE,
            KEY,
        ])
    };
    portal("ReadOne")
        .or_else(|| portal("Read"))
        .or_else(gnome_scheme)
}

fn gdbus(args: &[&str]) -> Option<bool> {
    let out = Command::new("gdbus").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    scheme_reply(&String::from_utf8_lossy(&out.stdout))
}

fn gnome_scheme() -> Option<bool> {
    let out = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", KEY])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    named_scheme(&String::from_utf8_lossy(&out.stdout))
}

/// The `uint32` in a portal reply, whatever it is wrapped in.
///
/// `ReadOne` prints `(<uint32 1>,)` and `Read` prints `(<<uint32 1>>,)`; a desktop that has no
/// preference prints `0`, which is not a request for dark.
fn scheme_reply(reply: &str) -> Option<bool> {
    let digits = reply.split("uint32").nth(1)?.trim_start();
    let n: u32 = digits
        .split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()?;
    Some(n == 1)
}

/// GNOME's spelling of the same setting: `prefer-dark`, `prefer-light`, or `default`.
fn named_scheme(value: &str) -> Option<bool> {
    let v = value.trim().trim_matches('\'');
    match v {
        "prefer-dark" => Some(true),
        "prefer-light" | "default" => Some(false),
        _ => None,
    }
}

/// The appearance change out of a `gdbus monitor` line.
///
/// The portal emits `SettingChanged` for its own namespace and for every backend namespace it
/// proxies, so GNOME's `org.gnome.desktop.interface color-scheme` arrives alongside the
/// standard one carrying the same news in a different spelling. Only the standard namespace is
/// read; taking both would store the same value twice per change and wake the window for it.
fn changed(line: &str) -> Option<bool> {
    let (head, args) = line.split_once("SettingChanged")?;
    if !head.contains("org.freedesktop.portal.Settings") {
        return None;
    }
    let mut fields = args.split('\'');
    if fields.nth(1)? != NAMESPACE || fields.nth(1)? != KEY {
        return None;
    }
    scheme_reply(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_both_portal_reply_shapes() {
        assert_eq!(scheme_reply("(<uint32 1>,)\n"), Some(true));
        assert_eq!(scheme_reply("(<uint32 2>,)\n"), Some(false));
        // No preference is not a request for dark.
        assert_eq!(scheme_reply("(<uint32 0>,)\n"), Some(false));
        // The older `Read`, one variant deeper.
        assert_eq!(scheme_reply("(<<uint32 1>>,)\n"), Some(true));
        assert_eq!(scheme_reply("(<'prefer-dark'>,)\n"), None);
        assert_eq!(scheme_reply(""), None);
    }

    #[test]
    fn reads_gnome_spelling() {
        assert_eq!(named_scheme("'prefer-dark'\n"), Some(true));
        assert_eq!(named_scheme("'prefer-light'\n"), Some(false));
        assert_eq!(named_scheme("'default'\n"), Some(false));
        assert_eq!(named_scheme("'newthing'\n"), None);
    }

    /// Verbatim `gdbus monitor` output, captured from a GNOME session toggling dark mode.
    #[test]
    fn follows_only_the_standard_namespace() {
        let dark = "/org/freedesktop/portal/desktop: org.freedesktop.portal.Settings.SettingChanged ('org.freedesktop.appearance', 'color-scheme', <uint32 1>)";
        let light = "/org/freedesktop/portal/desktop: org.freedesktop.portal.Settings.SettingChanged ('org.freedesktop.appearance', 'color-scheme', <uint32 0>)";
        let gnome = "/org/freedesktop/portal/desktop: org.freedesktop.portal.Settings.SettingChanged ('org.gnome.desktop.interface', 'color-scheme', <'prefer-dark'>)";
        let other = "/org/freedesktop/portal/desktop: org.freedesktop.portal.Settings.SettingChanged ('org.freedesktop.appearance', 'accent-color', <(0.2, 0.4, 0.6)>)";
        assert_eq!(changed(dark), Some(true));
        assert_eq!(changed(light), Some(false));
        assert_eq!(changed(gnome), None);
        assert_eq!(changed(other), None);
        assert_eq!(
            changed("The name org.freedesktop.portal.Desktop is owned by :1.60"),
            None
        );
    }
}
