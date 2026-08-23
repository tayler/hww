//! Reading style: the GUI's `render::TextOpts`.
//!
//! **This is the only legal home for width, colour, font, and spacing.** The IR carries no
//! styling and never will; if you find yourself wanting a `color`, `width`, `align`, or
//! `font` field in `src/ir.rs`, it belongs here instead. `render::TextOpts` is the same idea
//! for the text renderer, and the two are deliberately separate: a reading column measured in
//! characters and a terminal wrapped at 78 columns are different presentation decisions about
//! the same document.
//!
//! No egui types appear in this file, so the default CI job, which never compiles egui,
//! still type-checks and tests it.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FontChoice {
    /// IBM Plex Sans. Variable, so its bold is the same file at `wght` 700.
    Sans,
    /// IBM Plex Serif.
    ///
    /// This variant used to be absent on the grounds that a setting with no face behind it is
    /// worse than no setting, the same reason there is still no `justify` field. The face
    /// arrived, so the variant did: see `reader::ui::fonts`.
    Serif,
    /// IBM Plex Mono. A whole page in it is a real preference, not a joke.
    ///
    /// The one role with no italic face shipped, so emphasis here is epaint's skew. See
    /// [`crate::reader::face::Face::synthesises_italics`].
    Mono,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    Light,
    Dark,
    /// Warm paper. Not a tint of `Light`: the whole point is a lower-contrast ground.
    Sepia,
    /// Follow the desktop.
    System,
    /// Reached from `settings.json`, not from `d`. See [`Theme::next`].
    ContrastLight,
    ContrastDark,
}

impl Theme {
    /// Every theme with colours of its own.
    ///
    /// `System` is a redirect, not a palette. The contrast tests walk this rather than a
    /// hardcoded list, because a palette added without an entry here is a palette nothing
    /// checks, and the way that fails is a theme whose own warnings are unreadable.
    /// `real_lists_every_palette` is what makes a missing entry a compile error rather than a
    /// silence.
    pub const REAL: [Theme; 5] = [
        Theme::Light,
        Theme::Sepia,
        Theme::Dark,
        Theme::ContrastLight,
        Theme::ContrastDark,
    ];

    /// The contrast this theme's palette holds itself to: AA, or AAA for the contrast pair.
    ///
    /// Not a `Palette` field. It is not a colour, and a palette carries nothing else;
    /// `dark_mode` is already the one exception and a second would make it a pattern.
    pub fn floor(self) -> f32 {
        match self {
            Theme::ContrastLight | Theme::ContrastDark => 7.0,
            _ => 4.5,
        }
    }

    /// `d` cycles; `System` is deliberately in the cycle so it can be got back to without
    /// opening a settings file.
    ///
    /// The contrast palettes are **not** in the cycle. Someone who wants high contrast wants it
    /// permanently, and someone pressing `d` for reading comfort should not pass through two
    /// high-contrast screens to get back. They still need an arm, because this match is
    /// exhaustive and because a self-loop is a key that silently does nothing; both drop into
    /// the cycle at `System`.
    pub fn next(self) -> Self {
        match self {
            Theme::System => Theme::Light,
            Theme::Light => Theme::Sepia,
            Theme::Sepia => Theme::Dark,
            Theme::Dark => Theme::System,
            Theme::ContrastLight | Theme::ContrastDark => Theme::System,
        }
    }
}

/// A `theme` this binary does not know falls back to `System` instead of failing the whole file.
///
/// `ReadOpts` is `#[serde(default)]`, which covers a *missing* field and not an unparseable one.
/// Without this, an older binary reading a `settings.json` containing `"ContrastDark"` loses
/// every other setting in the file, and then writes the defaults back over it on the first `d`,
/// `[`, or `]`. That is a data-loss path with a keystroke as its trigger, for the price of one
/// type.
fn theme_or_system<'de, D>(d: D) -> Result<Theme, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Wire {
        Known(Theme),
        Unknown(serde::de::IgnoredAny),
    }
    Ok(match Wire::deserialize(d)? {
        Wire::Known(t) => t,
        Wire::Unknown(_) => Theme::System,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImagePolicy {
    /// Show a one-line strip naming the host. Loads only on an explicit click or `i`.
    Placeholder,
    /// Never offer to load. The strip still says an image was there; silently dropping
    /// content is how a reader lies about a page.
    Never,
}

/// Presentation settings for the reading column.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReadOpts {
    /// Reading measure in characters, the analogue of `render::TextOpts::width`. Typography's
    /// answer is 66; 66–78 is the usable band.
    pub measure_chars: f32,
    /// A typographic baseline, set once. **Not** a hotkey: live resizing is egui's
    /// `zoom_factor`, which already owns `Ctrl`+`+`/`-`/`0`, `Ctrl`+wheel, and pinch.
    pub base_size_pt: f32,
    /// Multiple of the font size.
    pub line_height: f32,
    /// Multiple of the font size, between blocks.
    pub paragraph_spacing: f32,
    pub family: FontChoice,
    #[serde(deserialize_with = "theme_or_system")]
    pub theme: Theme,
    /// Show a link's target inline, the way `render::TextOpts::show_links` does.
    pub show_link_urls: bool,
    pub images: ImagePolicy,
    /// Points of indent per reply level in a thread.
    pub indent_per_depth: f32,
    /// Levels of indent before nesting stops accumulating. Without a cap a 40-deep thread
    /// renders two pixels wide.
    pub max_thread_indent: u16,
}

impl Default for ReadOpts {
    fn default() -> Self {
        Self {
            measure_chars: 70.0,
            base_size_pt: 17.0,
            line_height: 1.35,
            paragraph_spacing: 0.85,
            family: FontChoice::Sans,
            theme: Theme::System,
            show_link_urls: false,
            images: ImagePolicy::Placeholder,
            indent_per_depth: 18.0,
            max_thread_indent: 8,
        }
    }
}

impl ReadOpts {
    pub const MEASURE_MIN: f32 = 45.0;
    pub const MEASURE_MAX: f32 = 110.0;

    /// `[` and `]`. hww's own reading knob, the one a browser does not have.
    pub fn widen(&mut self) {
        self.measure_chars = (self.measure_chars + 4.0).min(Self::MEASURE_MAX);
    }

    pub fn narrow(&mut self) {
        self.measure_chars = (self.measure_chars - 4.0).max(Self::MEASURE_MIN);
    }

    /// Indent for a reply at `depth`, in points.
    pub fn indent_for(&self, depth: u16) -> f32 {
        f32::from(depth.min(self.max_thread_indent)) * self.indent_per_depth
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measure_stays_inside_the_readable_band() {
        let mut o = ReadOpts::default();
        for _ in 0..50 {
            o.widen();
        }
        assert_eq!(o.measure_chars, ReadOpts::MEASURE_MAX);
        for _ in 0..50 {
            o.narrow();
        }
        assert_eq!(o.measure_chars, ReadOpts::MEASURE_MIN);
    }

    #[test]
    fn thread_indent_is_capped_so_deep_replies_stay_readable() {
        let o = ReadOpts::default();
        assert_eq!(o.indent_for(3), 54.0);
        // A 40-deep thread must not render two pixels wide.
        assert_eq!(o.indent_for(40), o.indent_for(o.max_thread_indent));
    }

    #[test]
    fn theme_cycle_returns_to_system() {
        let mut t = Theme::System;
        for _ in 0..4 {
            t = t.next();
        }
        assert_eq!(t, Theme::System);
    }

    /// `d` from a contrast palette lands back in the four-cycle rather than doing nothing.
    ///
    /// The failure this catches is a self-loop: an arm added only to satisfy the exhaustive
    /// match, which compiles, ships, and makes the key look broken.
    #[test]
    fn a_contrast_palette_is_not_a_dead_end() {
        let cycle = [Theme::System, Theme::Light, Theme::Sepia, Theme::Dark];
        for t in [Theme::ContrastLight, Theme::ContrastDark] {
            assert!(cycle.contains(&t.next()), "{t:?} does not rejoin the cycle");
        }
    }

    /// [`Theme::REAL`] lists every theme that is not the redirect.
    ///
    /// The `match` is exhaustive on purpose and is the whole point of the test: a variant added
    /// to `Theme` fails to compile *here*, which is the prompt to classify it and put it in
    /// `REAL`. Without that, a palette can be added to `theme::palette`'s own exhaustive match
    /// and to nothing else, and it escapes every contrast test in the crate silently. Asserting
    /// `REAL.len()` cannot catch it: the length is still whatever it was.
    #[test]
    fn real_lists_every_palette() {
        for t in [
            Theme::System,
            Theme::Light,
            Theme::Sepia,
            Theme::Dark,
            Theme::ContrastLight,
            Theme::ContrastDark,
        ] {
            let is_redirect = match t {
                Theme::System => true,
                Theme::Light
                | Theme::Sepia
                | Theme::Dark
                | Theme::ContrastLight
                | Theme::ContrastDark => false,
            };
            assert_eq!(Theme::REAL.contains(&t), !is_redirect, "{t:?}");
        }
    }

    /// Every real theme names a floor, and the contrast pair asks for more than AA.
    #[test]
    fn the_contrast_pair_holds_itself_higher() {
        for t in Theme::REAL {
            assert!(t.floor() >= 4.5);
        }
        assert!(Theme::ContrastLight.floor() > Theme::Light.floor());
        assert!(Theme::ContrastDark.floor() > Theme::Dark.floor());
    }

    /// A theme this binary has never heard of costs the theme, not the file.
    #[test]
    fn an_unknown_theme_does_not_take_the_rest_of_the_settings_with_it() {
        let o: ReadOpts =
            serde_json::from_str(r#"{"theme": "Aubergine", "measure_chars": 80.0}"#).unwrap();
        assert_eq!(o.theme, Theme::System);
        assert_eq!(o.measure_chars, 80.0);
    }

    /// And a theme it does know still round-trips, including the two new ones.
    #[test]
    fn every_real_theme_round_trips_through_json() {
        for t in Theme::REAL {
            let o = ReadOpts {
                theme: t,
                ..Default::default()
            };
            let back: ReadOpts = serde_json::from_str(&serde_json::to_string(&o).unwrap()).unwrap();
            assert_eq!(back.theme, t);
        }
    }

    /// Settings round-trip through the on-disk JSON, and an older file missing a field still
    /// loads: the reason for `#[serde(default)]`.
    #[test]
    fn partial_settings_json_fills_in_defaults() {
        let o: ReadOpts = serde_json::from_str(r#"{"measure_chars": 80.0}"#).unwrap();
        assert_eq!(o.measure_chars, 80.0);
        assert_eq!(o.base_size_pt, ReadOpts::default().base_size_pt);
    }
}
