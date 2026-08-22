//! `ReadOpts` -> `egui::Style`. **The only file in the crate where a colour appears.**
//!
//! Everything here is a presentation decision, which is exactly why it is confined to one
//! module that the IR cannot reach and that no extractor imports.

use crate::reader::notice::Severity;
use crate::reader::opts::{FontChoice, ReadOpts, Theme};
use eframe::egui::{self, Color32, FontFamily, FontId, Stroke};

/// The complete set of colours the reader draws with. Adding a role here is cheap; reaching
/// for `Color32` anywhere else is the thing to catch in review.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    /// Body text.
    pub fg: Color32,
    /// Metadata, provenance, and placeholders: present but not competing with the prose.
    pub dim: Color32,
    pub link: Color32,
    /// `Strong` is colour until an embedded bold face lands: egui's default fonts ship no
    /// bold and there is no synthetic bold, so weight is not available to ask for.
    pub strong: Color32,
    pub code_bg: Color32,
    /// Rules, thread indent guides, and block borders.
    pub rule: Color32,
    /// A notice, where it must be seen: the rewrite report, a transient status, a caution
    /// strip, a failure heading, an image that would not load.
    ///
    /// Was `accent`, which had quietly become five different jobs, one of which was not a
    /// notice at all. See [`notice_ink`].
    pub notice_fg: Color32,
    /// The ground behind a band. **Not** `chrome_bg`: that sits about five units from
    /// `code_bg` in every theme, so a band wearing it would be indistinguishable from a code
    /// block. `notice_fill_clears_every_page_colour` is what holds this apart.
    pub notice_bg: Color32,
    /// The current find match. Its own role because find is the reader searching the page,
    /// not the reader reporting on it, and the two must be free to move independently.
    pub find_current_bg: Color32,
    /// The other find matches, visible at a glance without competing with the current one.
    pub find_bg: Color32,
    /// Panel chrome: the status strip, the URL bar, the find bar, the outline. Bands do not
    /// use it; a panel is separated from the page by position, which a band is not.
    pub chrome_bg: Color32,
    pub selection: Color32,
    pub dark_mode: bool,
}

pub fn palette(theme: Theme, system_dark: bool) -> Palette {
    match theme {
        Theme::System => {
            if system_dark {
                palette(Theme::Dark, system_dark)
            } else {
                palette(Theme::Light, system_dark)
            }
        }
        Theme::Light => Palette {
            bg: Color32::from_rgb(0xFD, 0xFD, 0xFB),
            fg: Color32::from_rgb(0x1B, 0x1B, 0x19),
            dim: Color32::from_rgb(0x70, 0x70, 0x6A),
            link: Color32::from_rgb(0x15, 0x4B, 0x9E),
            strong: Color32::from_rgb(0x00, 0x00, 0x00),
            code_bg: Color32::from_rgb(0xEE, 0xEE, 0xE9),
            rule: Color32::from_rgb(0xD8, 0xD8, 0xD2),
            notice_fg: Color32::from_rgb(0x9A, 0x45, 0x0B),
            notice_bg: Color32::from_rgb(0xD8, 0xE2, 0xEE),
            find_current_bg: Color32::from_rgb(0x9A, 0x45, 0x0B),
            find_bg: Color32::from_rgba_unmultiplied(0x9A, 0x45, 0x0B, 0x38),
            chrome_bg: Color32::from_rgb(0xF1, 0xF1, 0xEC),
            selection: Color32::from_rgba_unmultiplied(0x15, 0x4B, 0x9E, 0x40),
            dark_mode: false,
        },
        Theme::Sepia => Palette {
            bg: Color32::from_rgb(0xF4, 0xEC, 0xD8),
            fg: Color32::from_rgb(0x39, 0x31, 0x26),
            dim: Color32::from_rgb(0x7C, 0x6F, 0x5A),
            link: Color32::from_rgb(0x1D, 0x4E, 0x77),
            strong: Color32::from_rgb(0x1E, 0x18, 0x0F),
            code_bg: Color32::from_rgb(0xE7, 0xDD, 0xC4),
            rule: Color32::from_rgb(0xD5, 0xC8, 0xA9),
            notice_fg: Color32::from_rgb(0x8E, 0x3F, 0x12),
            notice_bg: Color32::from_rgb(0xD6, 0xDA, 0xE0),
            find_current_bg: Color32::from_rgb(0x8E, 0x3F, 0x12),
            find_bg: Color32::from_rgba_unmultiplied(0x8E, 0x3F, 0x12, 0x40),
            chrome_bg: Color32::from_rgb(0xE9, 0xDF, 0xC8),
            selection: Color32::from_rgba_unmultiplied(0x1D, 0x4E, 0x77, 0x40),
            dark_mode: false,
        },
        Theme::Dark => Palette {
            bg: Color32::from_rgb(0x16, 0x18, 0x1A),
            fg: Color32::from_rgb(0xD4, 0xD1, 0xCB),
            dim: Color32::from_rgb(0x8B, 0x88, 0x82),
            link: Color32::from_rgb(0x7F, 0xB4, 0xF5),
            strong: Color32::from_rgb(0xF3, 0xF1, 0xEC),
            code_bg: Color32::from_rgb(0x22, 0x25, 0x28),
            rule: Color32::from_rgb(0x33, 0x37, 0x3B),
            notice_fg: Color32::from_rgb(0xE0, 0x9B, 0x54),
            notice_bg: Color32::from_rgb(0x1B, 0x32, 0x42),
            find_current_bg: Color32::from_rgb(0xE0, 0x9B, 0x54),
            find_bg: Color32::from_rgba_unmultiplied(0xE0, 0x9B, 0x54, 0x45),
            chrome_bg: Color32::from_rgb(0x1E, 0x21, 0x23),
            selection: Color32::from_rgba_unmultiplied(0x7F, 0xB4, 0xF5, 0x40),
            dark_mode: true,
        },
    }
}

/// The family body text is set in. Headings, quotes, and captions follow it; code never does.
fn body_family(opts: &ReadOpts) -> FontFamily {
    match opts.family {
        FontChoice::Sans => FontFamily::Proportional,
        FontChoice::Mono => FontFamily::Monospace,
    }
}

pub fn body_font(opts: &ReadOpts) -> FontId {
    FontId::new(opts.base_size_pt, body_family(opts))
}

/// Headings step down from the body size. Level 1 is the article title, so it gets the jump;
/// levels past 4 stop shrinking, because a heading smaller than its paragraph is not a heading.
pub fn heading_font(opts: &ReadOpts, level: u8) -> FontId {
    let scale = match level {
        0 | 1 => 1.65,
        2 => 1.35,
        3 => 1.18,
        4 => 1.07,
        _ => 1.0,
    };
    FontId::new(opts.base_size_pt * scale, body_family(opts))
}

/// Code is 0.92x because a monospace face at the body's optical size reads larger than it is.
pub fn mono_font(opts: &ReadOpts) -> FontId {
    FontId::new(opts.base_size_pt * 0.92, FontFamily::Monospace)
}

/// Chrome: status strip, URL bar, and outline. Fixed relative to the body so a wide reading
/// size does not turn the strip into a second column of prose.
pub fn chrome_font(opts: &ReadOpts) -> FontId {
    FontId::new(
        (opts.base_size_pt * 0.78).max(11.0),
        FontFamily::Proportional,
    )
}

/// Snap a length to egui's layout grid.
///
/// Every vertical measurement the reader derives from the font size is fractional, and a
/// fractional one accumulates down a long page into a drifting baseline. egui checks the same
/// grid and paints an "Unaligned" warning across the column in debug builds when a `Ui` misses
/// it, which is how this was found.
pub fn snap(v: f32) -> f32 {
    use eframe::egui::emath::GuiRounding as _;
    v.round_ui()
}

/// Line height in points, shared by every section of every galley.
///
/// This is what keeps baselines aligned across the several labels one paragraph is split into.
/// A segment that forgets it sits a pixel off its neighbours, and the eye reads that as a
/// broken line rather than as a subtle bug.
pub fn line_height_px(opts: &ReadOpts) -> f32 {
    // Rounded, because epaint advises an even number of pixels per row for even text and
    // because a fractional row height accumulates into a visibly drifting baseline down a long
    // page.
    snap(opts.base_size_pt * opts.line_height)
}

/// The reading column's width in points.
///
/// `measure_chars` is a count of characters, so it has to be turned into pixels by asking the
/// font how wide its characters actually are. The average lowercase advance is the right proxy
/// for a typographic measure; em width would overshoot badly on a proportional face.
pub fn measure_px(ctx: &egui::Context, opts: &ReadOpts) -> f32 {
    const SAMPLE: &str = "abcdefghijklmnopqrstuvwxyz";
    let font = body_font(opts);
    let width = ctx.fonts_mut(|f| {
        f.layout_no_wrap(SAMPLE.to_owned(), font, Color32::PLACEHOLDER)
            .rect
            .width()
    });
    snap((width / SAMPLE.len() as f32) * opts.measure_chars)
}

/// Whether the desktop is asking for a dark reader. Only consulted for `Theme::System`.
pub fn system_is_dark(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.raw.system_theme) == Some(egui::Theme::Dark)
}

/// How one severity of notice is painted.
///
/// [`Severity`] lives outside `ui/`, where a test can reach it; this is the paint, and it lives
/// here because this is the only file in the crate where a colour appears. Splitting them is
/// what lets the wording be tested without a window and the palette be retuned without touching
/// a call site.
#[derive(Debug, Clone, Copy)]
pub struct NoticeInk {
    pub fill: Color32,
    pub heading: Color32,
    pub body: Color32,
}

/// Every severity shares `notice_bg`. They differ by what they carry (a heading, actions, a
/// whole viewport) rather than by a fourth wash nobody could name, and a band is already
/// unmistakable from being full-bleed: no `ir::Block` can escape the reading measure.
pub fn notice_ink(pal: &Palette, severity: Severity) -> NoticeInk {
    match severity {
        Severity::Quiet => NoticeInk {
            fill: pal.notice_bg,
            heading: pal.dim,
            body: pal.dim,
        },
        Severity::Caution => NoticeInk {
            fill: pal.notice_bg,
            heading: pal.notice_fg,
            body: pal.notice_fg,
        },
        // A failure's body is prose meant to be read, so it is set in `fg`; only the heading
        // carries the notice colour. That is what `error_screen` already did.
        Severity::Failure => NoticeInk {
            fill: pal.notice_bg,
            heading: pal.notice_fg,
            body: pal.fg,
        },
    }
}

/// A failure heading.
///
/// Deliberately **not** [`heading_font`]: hww never borrows the page's type scale, because a
/// heading set exactly like an article's `<h2>` is the thing this whole column exists to stop.
/// It is not a guarantee of a distinct size, because `chrome_font`'s 11 pt floor breaks the
/// proportion at small reading sizes; size is a supporting signal and the full-bleed band is
/// the load-bearing one.
pub fn notice_heading_font(opts: &ReadOpts) -> FontId {
    FontId::new(chrome_font(opts).size * 1.45, FontFamily::Proportional)
}

/// Vertical padding inside a band, in points. Integer because `egui::Margin` is `i8`, which is
/// also why the band's horizontal gutter cannot be a margin and is an `add_space` instead.
pub fn band_pad(opts: &ReadOpts) -> i8 {
    (opts.base_size_pt * 0.55).round().clamp(6.0, 24.0) as i8
}

/// Install the palette and metrics. Called every frame; cheap, and it means a theme or
/// measure change takes effect on the frame it happens rather than on the next navigation.
pub fn apply(ctx: &egui::Context, opts: &ReadOpts) -> Palette {
    let pal = palette(opts.theme, system_is_dark(ctx));
    let mut style = egui::Style::default();

    let mut visuals = if pal.dark_mode {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    visuals.override_text_color = Some(pal.fg);
    visuals.panel_fill = pal.bg;
    visuals.window_fill = pal.chrome_bg;
    visuals.extreme_bg_color = pal.chrome_bg;
    visuals.faint_bg_color = pal.code_bg;
    visuals.code_bg_color = pal.code_bg;
    visuals.hyperlink_color = pal.link;
    visuals.selection.bg_fill = pal.selection;
    visuals.selection.stroke = Stroke::new(1.0, pal.link);
    visuals.window_stroke = Stroke::new(1.0, pal.rule);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, pal.rule);
    visuals.widgets.inactive.weak_bg_fill = pal.chrome_bg;
    visuals.widgets.hovered.weak_bg_fill = pal.code_bg;
    // The reader draws its own focus affordance on links; egui's default is a thick blue box
    // that fights the text.
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, pal.link);
    style.visuals = visuals;

    style.text_styles = [
        (egui::TextStyle::Body, body_font(opts)),
        (egui::TextStyle::Heading, heading_font(opts, 1)),
        (egui::TextStyle::Monospace, mono_font(opts)),
        (egui::TextStyle::Button, chrome_font(opts)),
        (egui::TextStyle::Small, chrome_font(opts)),
    ]
    .into();

    // Vertical spacing is the gap between blocks. Horizontal spacing is for *chrome*; prose
    // sets it to zero locally in `inline_ui`, because word spacing there comes from the text
    // itself and any widget spacing would double it.
    style.spacing.item_spacing = egui::vec2(6.0, snap(opts.base_size_pt * opts.paragraph_spacing));
    style.spacing.interact_size.y = snap(opts.base_size_pt * 1.2);
    // The reader owns every colour it draws, so both of egui's theme slots get the same style:
    // which one egui would otherwise pick is not a decision that should reach the page.
    ctx.all_styles_mut(|s| *s = style.clone());
    pal
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one test under `ui/`, and it needs its exception argued rather than assumed.
    ///
    /// The rule for this directory is that it holds what only a `Context` can run, so it is
    /// compiled and never tested. [`palette`] is not that: it is a pure
    /// `(Theme, bool) -> Palette`, unlike [`apply`], [`measure_px`], and [`system_is_dark`],
    /// which all take one. The directory's real rule is "split by what a test can reach", and
    /// this is reachable.
    ///
    /// What it pins is the governing rule of `reader::notice`: a notice shares no colour with
    /// the page. Exact inequality would say nothing, because `chrome_bg` and `code_bg`
    /// already differ and sit about five units apart, which no eye can see. So this asks for a
    /// visible margin instead, and it is the reason a band is not filled with `chrome_bg`.
    #[test]
    fn notice_fill_clears_every_page_colour() {
        /// Euclidean RGB. Crude next to a real perceptual metric, and sufficient: the failure
        /// it has to catch is two roles a few units apart, not a subtle mismatch.
        fn distance(a: Color32, b: Color32) -> f32 {
            let d = |x: u8, y: u8| f32::from(x) - f32::from(y);
            (d(a.r(), b.r()).powi(2) + d(a.g(), b.g()).powi(2) + d(a.b(), b.b()).powi(2)).sqrt()
        }
        // Chosen so that a band is plainly a different surface while still being calm. The
        // `chrome_bg`/`code_bg` pair measures 5.2 in Light, which is what "too close" means.
        const FLOOR: f32 = 24.0;

        for theme in [Theme::Light, Theme::Sepia, Theme::Dark] {
            let p = palette(theme, false);
            // Every colour the reading column can put on screen.
            let page = [
                ("bg", p.bg),
                ("fg", p.fg),
                ("dim", p.dim),
                ("link", p.link),
                ("strong", p.strong),
                ("code_bg", p.code_bg),
                ("rule", p.rule),
                ("find_current_bg", p.find_current_bg),
            ];
            for (name, c) in page {
                let d = distance(p.notice_bg, c);
                assert!(
                    d >= FLOOR,
                    "{theme:?}: notice_bg is {d:.1} from {name}; a band would read as page",
                );
            }
            // A notice must not look like the page's own words. Compared against text roles
            // only: `notice_fg` is a foreground, and asking it to clear `bg` would be asking it
            // to be illegible on its own band.
            for (name, c) in [
                ("fg", p.fg),
                ("dim", p.dim),
                ("link", p.link),
                ("strong", p.strong),
            ] {
                let d = distance(p.notice_fg, c);
                assert!(
                    d >= FLOOR,
                    "{theme:?}: notice_fg is {d:.1} from {name}; a notice would read as prose",
                );
            }
            // And it has to be legible where it is actually painted.
            assert!(distance(p.notice_fg, p.notice_bg) >= FLOOR);
        }
    }

    /// `Theme::System` is a redirect, not a palette. If it ever grew colours of its own they
    /// would escape the check above, which walks the three real themes.
    #[test]
    fn system_theme_resolves_to_a_real_palette() {
        assert_eq!(
            palette(Theme::System, true).bg,
            palette(Theme::Dark, true).bg
        );
        assert_eq!(
            palette(Theme::System, false).bg,
            palette(Theme::Light, false).bg
        );
    }
}
