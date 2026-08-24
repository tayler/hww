//! Settings on disk: one small JSON file, written atomically, and never fatal.
//!
//! eframe's `persistence` feature is off (it pulls `ron` in to remember a window size), so
//! this is hand-rolled over the `serde_json` already in the tree, and it also carries the
//! zoom factor, which is egui's state rather than ours.
//!
//! Nothing here is page content. The archive (reading history at rest) is a whole second
//! doctrine and is deliberately not started; this file holds preferences and nothing else.

use crate::reader::opts::ReadOpts;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Settings {
    pub read: ReadOpts,
    /// egui's `zoom_factor`: full-page zoom, bound to `Ctrl`+`+`/`-`/`0`, `Ctrl`+wheel, and
    /// pinch by egui itself. The reader implements no zoom of its own; it only remembers this.
    pub zoom_factor: f32,
    /// Send an origin-only `Referer` with image subresource requests.
    ///
    /// Lives here rather than in [`ReadOpts`], which is styling-only and stays that way. It is
    /// applied through `fetch::Limits`. Turning it off is a real choice with a real cost:
    /// hotlink protection then substitutes or refuses images, and the substitution case is
    /// undetectable (see `fetch::Referer::PageOrigin`).
    pub send_image_referer: bool,
    /// Lines of text per scroll step: one mouse-wheel notch, and one press of `j`/`k`.
    ///
    /// Lines rather than points, because the distance that reads as "one step" is a property
    /// of the text being read, not of the display. egui's own default is 40 points a notch,
    /// which is 1.5 lines of the default body and lands short of a step; three is the browser
    /// convention and is what `j`/`k` already moved.
    pub scroll_lines: f32,
    /// Whether the menu bar is drawn.
    ///
    /// The one setting here that exists because of a chrome decision rather than a reading one.
    /// Every other surface in this reader hides (the URL bar, the find bar, the outline, the
    /// panels), and a menu bar that cannot costs a row of every page forever. `F10` reveals it
    /// again, so turning it off is never a way to lose the reader's own controls.
    pub show_menu_bar: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            read: ReadOpts::default(),
            zoom_factor: 1.0,
            send_image_referer: true,
            scroll_lines: 3.0,
            show_menu_bar: true,
        }
    }
}

impl Settings {
    pub const SCROLL_LINES_MIN: f32 = 0.5;
    pub const SCROLL_LINES_MAX: f32 = 20.0;

    /// egui's `zoom_factor`, bounded so a slider has two ends.
    ///
    /// Wider than the reading-size band on purpose: this is the accessibility lever, and it is
    /// the one that scales strokes, radii and fixed margins along with the type. A hand-edited
    /// `0` would render a window of nothing.
    ///
    /// The two numbers are egui's own `MIN_ZOOM_FACTOR` / `MAX_ZOOM_FACTOR` (`gui_zoom.rs`),
    /// and they have to be, because the reader implements no zoom of its own: `Ctrl`+`+`/`-`,
    /// `Ctrl`+wheel and pinch are handled inside egui and reach this whole range whatever hww
    /// says. A narrower band here would not stop them, it would only disagree with them, and
    /// the disagreement is not inert — `Slider` defaults to `SliderClamping::Always`, so
    /// merely *opening* the settings panel at a zoom outside the band would pull the value to
    /// the nearest end and rezoom the window without anyone touching the control.
    pub const ZOOM_MIN: f32 = 0.2;
    pub const ZOOM_MAX: f32 = 5.0;

    /// [`Self::scroll_lines`] as something safe to multiply a line height by.
    ///
    /// The file is hand-edited, and `0` (a wheel that does nothing) or a negative (a wheel that
    /// scrolls the wrong way) are the sort of typo that reads as a broken reader rather than as
    /// a bad setting. `NaN` fails every comparison, so it falls out of the clamp and is caught
    /// on its own.
    pub fn scroll_lines(&self) -> f32 {
        if self.scroll_lines.is_nan() {
            return Self::default().scroll_lines;
        }
        self.scroll_lines
            .clamp(Self::SCROLL_LINES_MIN, Self::SCROLL_LINES_MAX)
    }

    /// [`Self::zoom_factor`] as something safe to hand to egui.
    ///
    /// The same argument as [`Self::scroll_lines`]: the file is hand-edited, and `0` is a blank
    /// window while a negative is a window drawn inside out. Neither reads as a bad setting;
    /// both read as a broken reader.
    pub fn zoom_factor(&self) -> f32 {
        if self.zoom_factor.is_nan() {
            return Self::default().zoom_factor;
        }
        self.zoom_factor.clamp(Self::ZOOM_MIN, Self::ZOOM_MAX)
    }
}

/// Where the settings file lives, hand-rolled rather than pulling in `dirs`.
///
/// `HWW_CONFIG_DIR` overrides everything, which is also what makes this testable.
pub fn config_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("HWW_CONFIG_DIR") {
        return Some(PathBuf::from(dir));
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support/hww"))
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("hww"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .map(|c| c.join("hww"))
    }
}

pub fn settings_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("settings.json"))
}

/// Load, or fall back to defaults with a reason.
///
/// A parse failure returns the defaults *and* the complaint rather than refusing to start.
/// Nothing in this file is worth failing a reader over, and a reader that will not open
/// because of a stray comma is a worse outcome than one that opens with default settings and
/// says so in the status strip.
pub fn load() -> (Settings, Option<String>) {
    let Some(path) = settings_path() else {
        return (Settings::default(), None);
    };
    match std::fs::read_to_string(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Settings::default(), None),
        Err(e) => (
            Settings::default(),
            Some(format!("could not read {}: {e}", path.display())),
        ),
        Ok(text) => match serde_json::from_str(&text) {
            Ok(s) => (s, None),
            Err(e) => (
                Settings::default(),
                Some(format!(
                    "{} is not valid settings ({e}); using defaults",
                    path.display()
                )),
            ),
        },
    }
}

/// Write via a temporary file and a rename, so a crash mid-write costs the settings rather
/// than the next launch.
///
/// Any failure takes the temporary with it. A stray `settings.json.tmp` next to a perfectly
/// good `settings.json` is the sort of litter that gets read as corruption later.
pub fn save(settings: &Settings) -> std::io::Result<()> {
    let Some(path) = settings_path() else {
        return Ok(());
    };
    let Some(dir) = path.parent() else {
        return Ok(());
    };
    std::fs::create_dir_all(dir)?;
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(settings)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    write_then_rename(&tmp, &path, dir, json.as_bytes()).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

fn write_then_rename(tmp: &Path, path: &Path, dir: &Path, json: &[u8]) -> std::io::Result<()> {
    {
        let mut f = std::fs::File::create(tmp)?;
        f.write_all(json)?;
        f.write_all(b"\n")?;
        // The rename is only atomic with respect to a crash if the bytes are actually down.
        f.sync_all()?;
    }
    std::fs::rename(tmp, path)?;
    sync_dir(dir)
}

/// `sync_all` on the file durably lands its *bytes*; the rename that gives those bytes their
/// name is a change to the *directory*, and is not durable until the directory is synced too.
/// Crash in between and neither name need point at the new settings: the one outcome the
/// temp-and-rename dance exists to rule out.
#[cfg(not(target_os = "windows"))]
fn sync_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::File::open(dir)?.sync_all()
}

/// Windows exposes no directory handle that can be opened and fsynced this way, and there is
/// no portable stand-in. The temp-and-rename is still worth having there for the torn-write
/// case; only the ordering guarantee is weaker.
#[cfg(target_os = "windows")]
fn sync_dir(_dir: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `HWW_CONFIG_DIR` is process-wide, so these run as one test rather than racing.
    #[test]
    fn settings_round_trip_and_survive_a_corrupt_file() {
        let dir = std::env::temp_dir().join(format!("hww-settings-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // SAFETY: single-threaded within this test, and the variable is read-only elsewhere.
        unsafe { std::env::set_var("HWW_CONFIG_DIR", &dir) };

        // Absent file: defaults, no complaint.
        let (s, err) = load();
        assert_eq!(s, Settings::default());
        assert!(err.is_none());

        let mut want = Settings::default();
        want.read.measure_chars = 82.0;
        want.zoom_factor = 1.25;
        want.send_image_referer = false;
        want.scroll_lines = 5.0;
        want.show_menu_bar = false;
        save(&want).unwrap();
        let (got, err) = load();
        assert_eq!(got, want);
        assert!(err.is_none());
        // The temporary must not survive a successful write.
        assert!(!dir.join("settings.json.tmp").exists());

        // Corrupt file: defaults, and a reason to show.
        std::fs::write(dir.join("settings.json"), "{ not json").unwrap();
        let (got, err) = load();
        assert_eq!(got, Settings::default());
        assert!(err.unwrap().contains("using defaults"));

        // A save that cannot land must not leave its temporary behind. A directory where the
        // settings file belongs fails the rename *after* the temporary has been written, which
        // is the only interesting half of the error path.
        std::fs::remove_file(dir.join("settings.json")).unwrap();
        std::fs::create_dir(dir.join("settings.json")).unwrap();
        assert!(save(&want).is_err());
        assert!(!dir.join("settings.json.tmp").exists());

        unsafe { std::env::remove_var("HWW_CONFIG_DIR") };
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_older_settings_file_missing_fields_still_loads() {
        let s: Settings = serde_json::from_str(r#"{"zoom_factor": 1.5}"#).unwrap();
        assert_eq!(s.zoom_factor, 1.5);
        assert!(s.send_image_referer);
        assert_eq!(s.scroll_lines, Settings::default().scroll_lines);
        assert_eq!(s.read, ReadOpts::default());
        // The newest field, and the one an existing installation will be missing. Defaulting it
        // to `false` would hide the menu bar for everybody who already has a settings file,
        // which is precisely the population that would not know `F10` brings it back.
        assert!(s.show_menu_bar);
    }

    /// A hand-edited file is the only way in, so the accessor and not the field is what the
    /// reader multiplies by: zero, negative, and `NaN` all have to come out of it as a scroll
    /// that still moves down the page.
    #[test]
    fn a_hand_edited_scroll_lines_cannot_stop_or_invert_the_wheel() {
        let with = |v: f32| Settings {
            scroll_lines: v,
            ..Settings::default()
        };
        assert_eq!(with(0.0).scroll_lines(), Settings::SCROLL_LINES_MIN);
        assert_eq!(with(-3.0).scroll_lines(), Settings::SCROLL_LINES_MIN);
        assert_eq!(with(1e9).scroll_lines(), Settings::SCROLL_LINES_MAX);
        assert_eq!(
            with(f32::NAN).scroll_lines(),
            Settings::default().scroll_lines
        );
        assert_eq!(with(4.5).scroll_lines(), 4.5);
    }

    /// The same guard for zoom, which egui does not bound itself.
    #[test]
    fn a_hand_edited_zoom_cannot_blank_or_invert_the_window() {
        let with = |v: f32| Settings {
            zoom_factor: v,
            ..Settings::default()
        };
        assert_eq!(with(0.0).zoom_factor(), Settings::ZOOM_MIN);
        assert_eq!(with(-2.0).zoom_factor(), Settings::ZOOM_MIN);
        assert_eq!(with(1e9).zoom_factor(), Settings::ZOOM_MAX);
        assert_eq!(
            with(f32::NAN).zoom_factor(),
            Settings::default().zoom_factor
        );
        assert_eq!(with(1.25).zoom_factor(), 1.25);
    }

    /// Both bands contain the value the reader opens on. Same argument as
    /// `every_range_contains_its_default` in `opts`: a slider whose range excludes its own
    /// starting value moves the setting the instant the panel is drawn.
    #[test]
    fn every_settings_range_contains_its_default() {
        let d = Settings::default();
        assert!((Settings::ZOOM_MIN..=Settings::ZOOM_MAX).contains(&d.zoom_factor));
        assert!(
            (Settings::SCROLL_LINES_MIN..=Settings::SCROLL_LINES_MAX).contains(&d.scroll_lines)
        );
    }

    /// The band has to cover everything egui's own zoom reaches, because hww implements no
    /// zoom: `Ctrl`+`+`/`-`, `Ctrl`+wheel and pinch are handled inside egui and clamp to
    /// `MIN_ZOOM_FACTOR` / `MAX_ZOOM_FACTOR` in its `gui_zoom.rs`, whatever this file says.
    ///
    /// Narrowing either end does not stop those keys, it only disagrees with them, and the
    /// disagreement rezooms the window on its own: `Slider` defaults to
    /// `SliderClamping::Always`, so opening the settings panel at a zoom outside the band pulls
    /// the value to the nearest end and `prefs_ui` then reports it as a change the reader made.
    /// The numbers are duplicated rather than imported because egui keeps them private.
    #[test]
    fn the_zoom_band_covers_everything_eguis_own_keys_reach() {
        // egui's `MIN_ZOOM_FACTOR` / `MAX_ZOOM_FACTOR`, duplicated because it keeps them
        // private. A zoom egui can produce has to survive the clamp untouched.
        for z in [0.2, 0.5, 1.0, 3.0, 5.0] {
            let s = Settings {
                zoom_factor: z,
                ..Settings::default()
            };
            assert_eq!(
                s.zoom_factor(),
                z,
                "egui's own zoom reaches {z}; the band moved it"
            );
        }
        // And the clamp still catches what egui never produces: the hand-edited file.
        let edited = |v| Settings {
            zoom_factor: v,
            ..Settings::default()
        };
        assert_eq!(edited(0.0).zoom_factor(), Settings::ZOOM_MIN);
        assert_eq!(edited(-1.0).zoom_factor(), Settings::ZOOM_MIN);
        assert_eq!(edited(1e9).zoom_factor(), Settings::ZOOM_MAX);
    }
}
