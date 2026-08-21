//! Settings on disk: one small JSON file, written atomically, never fatal.
//!
//! eframe's `persistence` feature is off — it pulls `ron` in to remember a window size — so
//! this is hand-rolled over the `serde_json` already in the tree, and it also carries the
//! zoom factor, which is egui's state rather than ours.
//!
//! Nothing here is page content. The archive — reading history at rest — is a whole second
//! doctrine and is deliberately not started; this file holds preferences and nothing else.

use crate::reader::opts::ReadOpts;
use std::io::Write;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Settings {
    pub read: ReadOpts,
    /// egui's `zoom_factor` — full-page zoom, bound to `Ctrl`+`+`/`-`/`0`, `Ctrl`+wheel, and
    /// pinch by egui itself. The reader implements no zoom of its own; it only remembers this.
    pub zoom_factor: f32,
    /// Send an origin-only `Referer` with image subresource requests.
    ///
    /// Lives here rather than in [`ReadOpts`], which is styling-only and stays that way. It is
    /// applied through `fetch::Limits`. Turning it off is a real choice with a real cost:
    /// hotlink protection then substitutes or refuses images, and the substitution case is
    /// undetectable — see `fetch::Referer::PageOrigin`.
    pub send_image_referer: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            read: ReadOpts::default(),
            zoom_factor: 1.0,
            send_image_referer: true,
        }
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
                    "{} is not valid settings ({e}) — using defaults",
                    path.display()
                )),
            ),
        },
    }
}

/// Write via a temporary file and a rename, so a crash mid-write costs the settings rather
/// than the next launch.
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
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(json.as_bytes())?;
        f.write_all(b"\n")?;
        // The rename is only atomic with respect to a crash if the bytes are actually down.
        f.sync_all()?;
    }
    std::fs::rename(&tmp, &path)
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

        unsafe { std::env::remove_var("HWW_CONFIG_DIR") };
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_older_settings_file_missing_fields_still_loads() {
        let s: Settings = serde_json::from_str(r#"{"zoom_factor": 1.5}"#).unwrap();
        assert_eq!(s.zoom_factor, 1.5);
        assert!(s.send_image_referer);
        assert_eq!(s.read, ReadOpts::default());
    }
}
