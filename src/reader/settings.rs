//! Settings on disk: one small JSON file, written atomically, and never fatal.
//!
//! eframe's `persistence` feature is off (it pulls `ron` in to remember a window size), so
//! this is hand-rolled over the `serde_json` already in the tree, and it also carries the
//! zoom factor, which is egui's state rather than ours.
//!
//! Nothing here is page content. What the reader asked hww to *keep* is a second file in this
//! same directory and a second doctrine: see [`crate::reader::archive`], which is the module to
//! read before adding anything else to this program that writes to disk. This file holds
//! preferences, including the two switches that govern that one.

use crate::reader::opts::ReadOpts;
use crate::sites::EngineId;
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
    /// Which engine the URL bar searches when what was typed is not an address.
    ///
    /// A hostname never reaches this struct: the id names a row in `sites::ENGINES`, which
    /// stays the only place in the crate that spells a host out.
    #[serde(deserialize_with = "engine_or_default")]
    pub search_engine: crate::sites::EngineId,
    /// Whether hww may write the library at all.
    ///
    /// The archive doctrine's off switch. Off means hww stops keeping pages; it does not mean
    /// what is already kept disappears, and the panel puts "forget everything" in the same group
    /// for exactly that reason — a switch that silently discarded the library would be as much a
    /// surprise as one that silently retained it. See [`crate::reader::archive`].
    pub keep_library: bool,
    /// Whether hww may write the reading list at all.
    ///
    /// The archive doctrine's switch for its third tenant, and it means what
    /// [`Settings::keep_library`] means by the same word: off stops the marking and leaves what is
    /// already listed, with "forget the reading list" in the same group as the other half. On by
    /// default like the library's, and for the library's reason rather than the history's —
    /// nothing is written until the reader marks a link, so a default that permits it promises
    /// nothing about what hww does on its own. See [`crate::reader::archive`].
    pub keep_reading_list: bool,
    /// Whether hww may write down the pages it draws.
    ///
    /// The archive doctrine's second switch, and the one that governs a record nobody asked for
    /// page by page. On by default, which is why the group it sits in prints the file's path and
    /// carries the button that empties it: see [`crate::reader::archive`], where the cost of that
    /// default is argued rather than assumed. Off stops the writing and leaves what is already
    /// there, exactly as [`Settings::keep_library`] does, because the two switches must not mean
    /// different things by the same word.
    pub keep_history: bool,
    /// What hww opens on when the command line names no page.
    ///
    /// Not a styling choice, so it is here rather than in [`ReadOpts`]. A reader who wants the
    /// bare splash back should not have to argue with the application about it.
    #[serde(deserialize_with = "opens_on_or_default")]
    pub open_on: OpenOn,
    /// Whether the menu bar is drawn.
    ///
    /// The one setting here that exists because of a chrome decision rather than a reading one.
    /// Every other surface in this reader hides (the URL bar, the find bar, the outline, the
    /// panels), and a menu bar that cannot costs a row of every page forever. `F10` reveals it
    /// again, so turning it off is never a way to lose the reader's own controls.
    pub show_menu_bar: bool,
}

/// A `search_engine` this binary does not know falls back to the default, for exactly the
/// reason `opts::theme_or_system` and `opts::images_or_default` exist.
///
/// `Settings` is `#[serde(default)]`, which covers a *missing* field and not an unparseable
/// one. Engines are the field here most likely to gain a value: the table grows whenever an
/// engine is measured to answer a plain client. Without this, a binary predating that row
/// reading a file that names it loses every other setting in the file, and writes the defaults
/// back over it on the next change.
fn engine_or_default<'de, D>(d: D) -> Result<EngineId, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum Wire {
        Known(EngineId),
        Unknown(serde::de::IgnoredAny),
    }
    use serde::Deserialize as _;
    Ok(match Wire::deserialize(d)? {
        Wire::Known(e) => e,
        Wire::Unknown(_) => EngineId::default(),
    })
}

/// What is on screen when hww starts with no page named.
///
/// The home screen is not a new surface: `ReaderApp::new` takes the same builtin-view path the
/// library key takes, so opening on the library is literally "hww opened with the library" and
/// inherits find,
/// the outline, scrolling, Tab focus, and correct menu gating with no special casing. Drawing
/// library entries into the idle screen instead would have forked the page model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OpenOn {
    /// The library, when there is anything in it. An empty one still opens on the splash: first
    /// run is unchanged, because nothing has happened yet and there is nothing to report.
    #[default]
    Library,
    /// The splash and an open URL bar, which is what hww has always done.
    Nothing,
}

/// An `open_on` this binary does not know falls back to the default, for the same reason
/// [`engine_or_default`] exists: `#[serde(default)]` covers a *missing* field and not an
/// unparseable one, and without this a binary predating a later value would lose every other
/// setting in the file and write the defaults back over it.
fn opens_on_or_default<'de, D>(d: D) -> Result<OpenOn, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum Wire {
        Known(OpenOn),
        Unknown(serde::de::IgnoredAny),
    }
    use serde::Deserialize as _;
    Ok(match Wire::deserialize(d)? {
        Wire::Known(v) => v,
        Wire::Unknown(_) => OpenOn::default(),
    })
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            read: ReadOpts::default(),
            zoom_factor: 1.0,
            send_image_referer: true,
            scroll_lines: 3.0,
            search_engine: crate::sites::EngineId::default(),
            keep_library: true,
            keep_reading_list: true,
            keep_history: true,
            open_on: OpenOn::default(),
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

/// Where the library file lives, beside the settings.
///
/// One directory, deliberately. Saved items are arguably state rather than configuration, which
/// on Linux is `XDG_STATE_HOME`, but the archive doctrine (`reader::archive`) is about *what* is
/// written and *who asked for it*, not about which spec-blessed folder it lands in. A second
/// directory would double the uninstall instructions in README for no privacy gain, and
/// `HWW_CONFIG_DIR` would then override half of what hww writes.
pub fn archive_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("library.json"))
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
    let json = serde_json::to_string_pretty(settings)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    write_atomically(&path, json.as_bytes())
}

/// Put `bytes` at `path` through a temporary file and a rename, creating the directory first.
///
/// Shared with [`crate::reader::archive`] rather than copied there. Both files live in the same
/// directory, both are hand-editable JSON the reader would rather not lose to a crash, and the
/// three durability steps below are the sort of thing that gets copied once correctly and then
/// diverges: a second copy that forgot `sync_dir` would be indistinguishable from this one until
/// a machine lost power.
pub fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let Some(dir) = path.parent() else {
        return Ok(());
    };
    std::fs::create_dir_all(dir)?;
    let tmp = path.with_extension("json.tmp");
    write_then_rename(&tmp, path, dir, bytes).inspect_err(|_| {
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
        want.keep_library = false;
        want.keep_reading_list = false;
        want.keep_history = false;
        want.open_on = OpenOn::Nothing;
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

    /// The whole point of the unknown-value fallback: an engine this binary has never heard of
    /// costs that one field, not the file. Without it, `serde` fails the whole struct, `load`
    /// returns the defaults, and the next write flattens the reader's zoom, theme, and menu
    /// preference too.
    #[test]
    fn an_unknown_engine_costs_only_that_field() {
        let json = r#"{
            "search_engine": "an-engine-from-a-later-build",
            "zoom_factor": 1.4,
            "show_menu_bar": false,
            "send_image_referer": false
        }"#;
        let s: Settings = serde_json::from_str(json).expect("the file still parses");
        assert_eq!(s.search_engine, crate::sites::EngineId::default());
        assert_eq!(s.zoom_factor, 1.4);
        assert!(!s.show_menu_bar);
        assert!(!s.send_image_referer);
    }

    /// The archive's four switches survive the file, and an existing installation's file —
    /// which has none of them — keeps the library on rather than silently having it off.
    ///
    /// The history's default is asserted here too, and it is the assertion worth having: it is
    /// the one switch that writes without being asked, so a build that shipped it defaulting the
    /// other way, or silently flipping under an older file, would be a different program from the
    /// one README describes.
    #[test]
    fn the_library_switches_round_trip_and_default_on() {
        let older: Settings = serde_json::from_str(r#"{"zoom_factor": 1.5}"#).unwrap();
        assert!(
            older.keep_library,
            "an existing file must not turn keeping off"
        );
        assert!(older.keep_history, "nor history");
        assert!(older.keep_reading_list, "nor the reading list");
        assert_eq!(older.open_on, OpenOn::Library);
        assert!(Settings::default().keep_history);

        let want = Settings {
            keep_library: false,
            keep_reading_list: false,
            keep_history: false,
            open_on: OpenOn::Nothing,
            ..Settings::default()
        };
        let text = serde_json::to_string(&want).unwrap();
        assert_eq!(serde_json::from_str::<Settings>(&text).unwrap(), want);
    }

    /// The unknown-value fallback, on the newest enum in the file. Without it a binary predating
    /// a later choice loses every other setting in the file and writes the defaults back.
    #[test]
    fn an_unknown_open_on_costs_only_that_field() {
        let json = r#"{"open_on": "the-weather", "zoom_factor": 1.4, "keep_library": false}"#;
        let s: Settings = serde_json::from_str(json).expect("the file still parses");
        assert_eq!(s.open_on, OpenOn::default());
        assert_eq!(s.zoom_factor, 1.4);
        assert!(!s.keep_library);
    }

    /// A value of the wrong shape entirely, not just an unknown name.
    #[test]
    fn a_malformed_engine_costs_only_that_field() {
        let s: Settings =
            serde_json::from_str(r#"{"search_engine": 42, "zoom_factor": 1.25}"#).unwrap();
        assert_eq!(s.search_engine, crate::sites::EngineId::default());
        assert_eq!(s.zoom_factor, 1.25);
    }

    /// Every engine id round-trips through the file, so a preference set in the panel is the
    /// one read back at the next launch.
    #[test]
    fn every_engine_round_trips_through_the_file() {
        for id in crate::sites::EngineId::ALL {
            let before = Settings {
                search_engine: id,
                ..Settings::default()
            };
            let text = serde_json::to_string(&before).unwrap();
            let after: Settings = serde_json::from_str(&text).unwrap();
            assert_eq!(after.search_engine, id);
            // The serialized form names the engine, never its address.
            assert!(
                !text.contains(crate::sites::engine_by_id(id).host),
                "the settings file spelled a hostname: {text}"
            );
        }
    }
}
