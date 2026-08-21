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
    /// egui's embedded proportional face.
    Sans,
    /// egui's embedded monospace face. A whole page in it is a real preference, not a joke.
    Mono,
    // No `Serif`. egui's `default_fonts` ship no serif face, so the variant would be a setting
    // with nothing behind it, the same reason there is no `justify` field. It arrives with the
    // embedded face, and the two are one decision.
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    Light,
    Dark,
    /// Warm paper. Not a tint of `Light`: the whole point is a lower-contrast ground.
    Sepia,
    /// Follow the desktop.
    System,
}

impl Theme {
    /// `d` cycles; `System` is deliberately in the cycle so it can be got back to without
    /// opening a settings file.
    pub fn next(self) -> Self {
        match self {
            Theme::System => Theme::Light,
            Theme::Light => Theme::Sepia,
            Theme::Sepia => Theme::Dark,
            Theme::Dark => Theme::System,
        }
    }
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
            line_height: 1.55,
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

    /// Settings round-trip through the on-disk JSON, and an older file missing a field still
    /// loads: the reason for `#[serde(default)]`.
    #[test]
    fn partial_settings_json_fills_in_defaults() {
        let o: ReadOpts = serde_json::from_str(r#"{"measure_chars": 80.0}"#).unwrap();
        assert_eq!(o.measure_chars, 80.0);
        assert_eq!(o.base_size_pt, ReadOpts::default().base_size_pt);
    }
}
