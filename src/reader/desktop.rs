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
//! publishing those settings is built on, and a client of its own means an async runtime in the
//! default build: the `zbus` already in the tree arrives under `accesskit`, so it is reachable
//! only from `gui`, and this module is compiled and tested without it. No request leaves the
//! machine, nothing is written, and what is read is two public desktop settings.
//!
//! Windows and macOS are excluded by [`ASKS_THE_DESKTOP`] rather than left to fail on their
//! own. winit answers first on both, so nothing here is ever consulted for a palette, and the
//! commands are not reliably absent either: `gdbus` and `gsettings` ship with the GTK runtimes
//! and MSYS2 installs that plenty of Windows machines have on PATH. `hww.exe` is a
//! GUI-subsystem binary and Windows hands a console child of one its own console window, so
//! asking anyway costs a flash of consoles at startup and one that the `gdbus monitor` behind
//! [`watch`] would hold open for the rest of the session.
//!
//! Only [`watch`] starts any of it. `hww-shot` never calls it, so a screenshot of
//! `--theme system` stays what it always was rather than following the machine that took it.

use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// The portal namespace and key. Named once because the query and the change signal that
/// reports the same value must agree on it.
const NAMESPACE: &str = "org.freedesktop.appearance";
const KEY: &str = "color-scheme";

/// The portal's own numbering, reused rather than invented, so the number [`scheme_reply`]
/// reads and the number [`SCHEME`] holds cannot mean opposite things in one file.
const UNKNOWN: u8 = 0;
const DARK: u8 = 1;
const LIGHT: u8 = 2;

/// The desktop's preference, as last read. Process-global because that is what it describes:
/// one desktop, one answer, consulted from wherever a palette is chosen.
static SCHEME: AtomicU8 = AtomicU8::new(UNKNOWN);

/// Whether this is a desktop winit cannot answer for, which is every one but Windows and
/// macOS. See the module comment for what asking costs on the two that can answer.
const ASKS_THE_DESKTOP: bool = !cfg!(any(target_os = "windows", target_os = "macos"));

/// Whether the desktop asks for dark. `None` when nothing has been asked or nothing answered,
/// which is not "light": the caller decides what to do without an answer.
pub fn prefers_dark() -> Option<bool> {
    match SCHEME.load(Ordering::Relaxed) {
        DARK => Some(true),
        LIGHT => Some(false),
        _ => None,
    }
}

/// Whether to read dark, given winit's answer and the desktop's.
///
/// The precedence, in the one place it is written: winit wins wherever it has an answer, this
/// module answers where winit reports nothing at all, and neither knowing is Light, because a
/// palette has to be picked. It takes both answers rather than reading either, so the rule is
/// decidable without a window; `theme::system_is_dark` is the adapter that supplies them.
pub fn resolve(winit_dark: Option<bool>, desktop_dark: Option<bool>) -> bool {
    winit_dark.or(desktop_dark).unwrap_or(false)
}

/// A running child, killed when this is dropped, which ends any thread reading its output on
/// EOF. Long-lived for the `gdbus monitor` behind [`watch`], momentary for a [`query`] command.
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

/// A child spawned with its stdout piped and nothing else attached, already inside the guard
/// that kills it.
///
/// Every `?` after a call to this is why it hands back the guard rather than a `Child`: an
/// early return still reaps the child, where a bare `Child` would leave one running for the
/// rest of the login session.
fn piped(cmd: &mut Command) -> Option<Watch> {
    Some(Watch(
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?,
    ))
}

/// Read the preference now, then follow it. `wake` is called on every change, after the new
/// value is stored, so a repaint sees it.
///
/// Returns `None` on a platform winit answers for, when the desktop published no preference, or
/// when the change signal could not be subscribed to. A `None` from the last case still leaves a
/// correct value in [`SCHEME`]: the theme is right at startup and simply stops tracking, which
/// is the right degradation for a desktop that answered a question once and cannot be asked
/// again.
#[must_use = "dropping the watch stops it"]
pub fn watch(wake: impl Fn() + Send + 'static) -> Option<Watch> {
    if !ASKS_THE_DESKTOP {
        return None;
    }

    let now = query()?;
    SCHEME.store(encode(now), Ordering::Relaxed);

    let mut watch = piped(Command::new("gdbus").args([
        "monitor",
        "--session",
        "--dest",
        "org.freedesktop.portal.Desktop",
        "--object-path",
        "/org/freedesktop/portal/desktop",
    ]))?;
    let out = watch.0.stdout.take()?;

    std::thread::Builder::new()
        .name("hww-desktop-theme".to_owned())
        .spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                if let Some(dark) = changed(&line) {
                    // Only a value that moved is worth a frame. The portal re-announces the
                    // same one when a desktop re-applies its appearance or a backend restarts,
                    // and a repaint for news the palette already has is the same waste this
                    // parser refuses the GNOME namespace to avoid.
                    let scheme = encode(dark);
                    if SCHEME.swap(scheme, Ordering::Relaxed) != scheme {
                        wake();
                    }
                }
            }
        })
        .ok()?;

    Some(watch)
}

fn encode(dark: bool) -> u8 {
    if dark { DARK } else { LIGHT }
}

/// How long the whole startup query may take before it is abandoned.
///
/// One budget for the fall-through rather than one per command, and applied from out here
/// rather than left to the commands: `gdbus` is asked for a timeout of its own, `gsettings` has
/// no flag for one and reads through dconf, and a wedged dconf backend is otherwise a browser
/// that shows no window. Every call on this path runs on the UI thread before the first frame,
/// so an unbounded one is a hang rather than a slow start, and three commands each given a
/// second of their own would be three seconds of a window that has not appeared yet.
///
/// A desktop that answers at all answers the first command in under a millisecond.
const QUERY_TIMEOUT: Duration = Duration::from_secs(1);

/// Ask the portal, then GNOME. The portal is the cross-desktop answer; `ReadOne` is newer than
/// `Read`, and a portal that has only the older method answers the same value wrapped one
/// variant deeper, which [`scheme_reply`] does not care about.
fn query() -> Option<bool> {
    let deadline = Instant::now() + QUERY_TIMEOUT;
    let portal = |method: &str| {
        let reply = output_within(
            Command::new("gdbus").args([
                "call",
                "--session",
                // A call on a local bus answers in under a millisecond, and this one is made
                // before the first frame. D-Bus's own default is 25 seconds, which is a browser
                // that will not start on a session whose portal is wedged.
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
            ]),
            deadline,
        )?;
        scheme_reply(&reply)
    };
    let gnome = || {
        let value = output_within(
            Command::new("gsettings").args(["get", "org.gnome.desktop.interface", KEY]),
            deadline,
        )?;
        named_scheme(&value)
    };
    portal("ReadOne").or_else(|| portal("Read")).or_else(gnome)
}

/// A command's stdout, or `None` if it could not be started, could not be read, or was still
/// running at `deadline`.
///
/// Read on a thread rather than after a `wait`, so a child that fills its pipe and one that
/// never writes at all are both bounded by the same deadline. [`Watch`] kills the child on the
/// way out of every path here, including the successful one: a child that has already exited
/// refuses the kill and that is fine, while one that closed its pipe without exiting is exactly
/// what the deadline is for. Killing it also ends the read on EOF, so the thread does not
/// outlive the call either.
///
/// The exit status is not consulted. It cannot be: the child is killed at the deadline, so
/// there is nothing to read a status from in the case this exists to handle. Both callers pass
/// what came back to a parser that answers `None` for anything that is not the value it wants,
/// which is the same refusal a step later.
fn output_within(cmd: &mut Command, deadline: Instant) -> Option<String> {
    let mut child = piped(cmd)?;
    let out = child.0.stdout.take()?;
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("hww-desktop-query".to_owned())
        .spawn(move || {
            let mut buf = String::new();
            let _ = BufReader::new(out).read_to_string(&mut buf);
            let _ = tx.send(buf);
        })
        .ok()?;
    rx.recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .ok()
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
    Some(n == u32::from(DARK))
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
///
/// The interface is checked as well as the namespace, so what this answers depends on the line
/// alone rather than on how narrowly the monitor happened to be scoped: the `--dest` and
/// `--object-path` in [`watch`] are an argument list, and a parser that reads its own guarantee
/// out of them is one flag away from reading a backend's copy of the value as the portal's.
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

    /// winit first, this module second, Light when neither knows.
    #[test]
    fn winit_outranks_the_portal() {
        assert!(resolve(Some(true), Some(false)));
        assert!(!resolve(Some(false), Some(true)));
        assert!(resolve(None, Some(true)));
        assert!(!resolve(None, Some(false)));
        assert!(!resolve(None, None));
    }

    /// A query that never answers is abandoned rather than waited on.
    ///
    /// The whole of [`query`] runs on the UI thread before the first frame, so this is the
    /// difference between a slow start and a window that never appears. `gdbus` is asked for a
    /// timeout of its own and `gsettings` has no flag for one, which is why the bound lives out
    /// here and covers both. A machine with no `sleep` fails the spawn and answers `None` by the
    /// other route, which is the same answer.
    #[test]
    fn a_query_that_hangs_is_abandoned() {
        let started = Instant::now();
        assert_eq!(
            output_within(Command::new("sleep").arg("30"), started + QUERY_TIMEOUT),
            None,
            "a command still running at the deadline has nothing to report"
        );
        assert!(
            started.elapsed() < QUERY_TIMEOUT * 5,
            "the deadline is what bounds this, not the command"
        );
    }

    /// A deadline already past is not a fresh second for the command that reaches it.
    ///
    /// [`query`] falls through up to three commands on one budget, so this is what keeps a
    /// desktop with no portal and a wedged dconf to the same startup delay as any other.
    #[test]
    fn a_spent_budget_is_not_renewed() {
        let started = Instant::now();
        assert_eq!(
            output_within(Command::new("sleep").arg("30"), started),
            None,
            "a deadline in the past leaves no time to wait"
        );
        assert!(
            started.elapsed() < QUERY_TIMEOUT,
            "the command is abandoned immediately, not after another full budget"
        );
    }

    /// Verbatim `gdbus monitor` output, captured from a GNOME session toggling dark mode.
    #[test]
    fn follows_only_the_standard_namespace() {
        let dark = "/org/freedesktop/portal/desktop: org.freedesktop.portal.Settings.SettingChanged ('org.freedesktop.appearance', 'color-scheme', <uint32 1>)";
        let light = "/org/freedesktop/portal/desktop: org.freedesktop.portal.Settings.SettingChanged ('org.freedesktop.appearance', 'color-scheme', <uint32 0>)";
        let gnome = "/org/freedesktop/portal/desktop: org.freedesktop.portal.Settings.SettingChanged ('org.gnome.desktop.interface', 'color-scheme', <'prefer-dark'>)";
        let other = "/org/freedesktop/portal/desktop: org.freedesktop.portal.Settings.SettingChanged ('org.freedesktop.appearance', 'accent-color', <(0.2, 0.4, 0.6)>)";
        // The backend the portal proxies, announcing the value the portal is about to. Rejected
        // by the interface rather than by the monitor's `--dest`, which is what lets that
        // argument list change without changing what this answers.
        let backend = "/org/freedesktop/portal/desktop: org.freedesktop.impl.portal.Settings.SettingChanged ('org.freedesktop.appearance', 'color-scheme', <uint32 1>)";
        assert_eq!(changed(dark), Some(true));
        assert_eq!(changed(light), Some(false));
        assert_eq!(changed(gnome), None);
        assert_eq!(changed(other), None);
        assert_eq!(changed(backend), None);
        assert_eq!(
            changed("The name org.freedesktop.portal.Desktop is owned by :1.60"),
            None
        );
    }
}
