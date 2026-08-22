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
    /// Decoration: a separator that says "these are two things" and nothing more.
    /// `ui.separator()`, the outline's divider, the code frame, the window stroke. Deliberately
    /// soft, and deliberately not load-bearing: see [`Palette::guide`].
    pub rule: Color32,
    /// A mark that **identifies structure** rather than decorating it: the blockquote bar, the
    /// thread indent guide, the edge that separates a docked panel from the page, the hairline
    /// on a band with no ground of its own, and the border that is the whole of what says a
    /// button or a text field is one.
    ///
    /// Held to 3:1 against `bg` and `chrome_bg`, the two grounds those land on, by
    /// `guide_is_visible_where_it_is_drawn`. `rule` stays soft because firming every divider in
    /// order to fix two bars is worse; this is the second role that split off instead. A control
    /// at rest is why the floor is not negotiable: with no fill, the border carries the
    /// affordance alone, and `rule` at 1.41 left Retry / Retry without rewrite / Copy URL, the
    /// only three controls a failed page has, reading as text.
    pub guide: Color32,
    /// A notice, where it must be seen: the rewrite report, a transient status, a caution
    /// strip, a failure heading, an image that would not load.
    ///
    /// Was `accent`, which had quietly become five different jobs, one of which was not a
    /// notice at all. See [`notice_ink`].
    pub notice_fg: Color32,
    /// The ground behind a `Caution` band, and behind nothing else.
    ///
    /// Was `notice_bg`, shared by all three severities. `Quiet` and `Failure` fill the viewport,
    /// so they have no page to be a different surface *from*; `Caution` is the one severity with
    /// prose under it. Held to 1.5:1 from `bg` by `a_caution_band_is_a_different_surface`, and it
    /// is a real surface rather than a tint because the alternative caps how dark it can go: an
    /// ink that has to stay legible on it is what sets the ceiling, and colouring the ground
    /// instead of the words is the only way past that.
    pub caution_bg: Color32,
    /// The current find match. Its own role because find is the reader searching the page,
    /// not the reader reporting on it, and the two must be free to move independently.
    pub find_current_bg: Color32,
    /// Surfaces that **float over** the page: the page-info panel, the help card, the hover-URL
    /// strip, and egui's tooltips via `window_fill`.
    ///
    /// Docked panels do not use it. They take `bg` and one `guide` edge on the side facing the
    /// page, because a docked panel is separated by position and a floating one is not. In the
    /// contrast pair this equals `bg` on purpose, which is the platform convention, and it works
    /// because every floating surface is drawn with a stroke around it.
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
            // Was #70706A, which measured 4.40 on `chrome_bg` and 3.80 on the old `notice_bg`.
            dim: Color32::from_rgb(0x63, 0x63, 0x5B),
            link: Color32::from_rgb(0x15, 0x4B, 0x9E),
            strong: Color32::from_rgb(0x00, 0x00, 0x00),
            code_bg: Color32::from_rgb(0xEE, 0xEE, 0xE9),
            rule: Color32::from_rgb(0xD8, 0xD8, 0xD2),
            guide: Color32::from_rgb(0x89, 0x89, 0x82),
            notice_fg: Color32::from_rgb(0x9A, 0x45, 0x0B),
            caution_bg: Color32::from_rgb(0xDF, 0xB2, 0x71),
            find_current_bg: Color32::from_rgb(0x9A, 0x45, 0x0B),
            chrome_bg: Color32::from_rgb(0xF1, 0xF1, 0xEC),
            selection: Color32::from_rgba_unmultiplied(0x15, 0x4B, 0x9E, 0x40),
            dark_mode: false,
        },
        Theme::Sepia => Palette {
            bg: Color32::from_rgb(0xF4, 0xEC, 0xD8),
            fg: Color32::from_rgb(0x39, 0x31, 0x26),
            // Was #7C6F5A: 3.71 on `chrome_bg`, 3.50 on the old `notice_bg`. The worst of the
            // three, and the one that made the audit worth running.
            dim: Color32::from_rgb(0x68, 0x5C, 0x4A),
            link: Color32::from_rgb(0x1D, 0x4E, 0x77),
            strong: Color32::from_rgb(0x1E, 0x18, 0x0F),
            code_bg: Color32::from_rgb(0xE7, 0xDD, 0xC4),
            rule: Color32::from_rgb(0xD5, 0xC8, 0xA9),
            guide: Color32::from_rgb(0x88, 0x7D, 0x62),
            notice_fg: Color32::from_rgb(0x8E, 0x3F, 0x12),
            caution_bg: Color32::from_rgb(0xD6, 0xB2, 0x79),
            find_current_bg: Color32::from_rgb(0x8E, 0x3F, 0x12),
            chrome_bg: Color32::from_rgb(0xE9, 0xDF, 0xC8),
            selection: Color32::from_rgba_unmultiplied(0x1D, 0x4E, 0x77, 0x40),
            dark_mode: false,
        },
        Theme::Dark => Palette {
            bg: Color32::from_rgb(0x16, 0x18, 0x1A),
            fg: Color32::from_rgb(0xD4, 0xD1, 0xCB),
            // Was #8B8882: 4.36 on `chrome_bg`, 3.76 on the old `notice_bg`.
            dim: Color32::from_rgb(0x9C, 0x99, 0x92),
            link: Color32::from_rgb(0x7F, 0xB4, 0xF5),
            strong: Color32::from_rgb(0xF3, 0xF1, 0xEC),
            code_bg: Color32::from_rgb(0x22, 0x25, 0x28),
            rule: Color32::from_rgb(0x33, 0x37, 0x3B),
            guide: Color32::from_rgb(0x6A, 0x71, 0x78),
            notice_fg: Color32::from_rgb(0xE0, 0x9B, 0x54),
            caution_bg: Color32::from_rgb(0x55, 0x3D, 0x24),
            find_current_bg: Color32::from_rgb(0xE0, 0x9B, 0x54),
            chrome_bg: Color32::from_rgb(0x1E, 0x21, 0x23),
            selection: Color32::from_rgba_unmultiplied(0x7F, 0xB4, 0xF5, 0x40),
            dark_mode: true,
        },
        // The contrast pair. `chrome_bg` equals `bg` on purpose: that is the platform convention
        // for high contrast, and it costs nothing here because every floating surface is already
        // drawn with a stroke around it. `guide` goes to the far end of the range rather than to
        // a mid grey, because at this floor a structural mark is not decoration to be softened.
        Theme::ContrastLight => Palette {
            bg: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            fg: Color32::from_rgb(0x00, 0x00, 0x00),
            dim: Color32::from_rgb(0x4D, 0x4D, 0x4D),
            link: Color32::from_rgb(0x00, 0x30, 0x8F),
            strong: Color32::from_rgb(0x00, 0x00, 0x00),
            code_bg: Color32::from_rgb(0xF2, 0xF2, 0xF2),
            rule: Color32::from_rgb(0x55, 0x55, 0x55),
            guide: Color32::from_rgb(0x00, 0x00, 0x00),
            notice_fg: Color32::from_rgb(0x7A, 0x26, 0x00),
            caution_bg: Color32::from_rgb(0xFF, 0xC2, 0x4D),
            find_current_bg: Color32::from_rgb(0x00, 0x00, 0x00),
            chrome_bg: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            selection: Color32::from_rgba_unmultiplied(0x00, 0x30, 0x8F, 0x40),
            dark_mode: false,
        },
        Theme::ContrastDark => Palette {
            bg: Color32::from_rgb(0x00, 0x00, 0x00),
            fg: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            dim: Color32::from_rgb(0xAB, 0xAB, 0xAB),
            link: Color32::from_rgb(0x7F, 0xC0, 0xFF),
            strong: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            code_bg: Color32::from_rgb(0x14, 0x14, 0x14),
            rule: Color32::from_rgb(0xAB, 0xAB, 0xAB),
            guide: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            notice_fg: Color32::from_rgb(0xFF, 0xB0, 0x00),
            caution_bg: Color32::from_rgb(0x4A, 0x33, 0x00),
            find_current_bg: Color32::from_rgb(0xFF, 0xB0, 0x00),
            chrome_bg: Color32::from_rgb(0x00, 0x00, 0x00),
            selection: Color32::from_rgba_unmultiplied(0x7F, 0xC0, 0xFF, 0x40),
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

/// Chrome: status strip, URL bar, outline, page info, and every word a band says. Fixed
/// relative to the body so a wide reading size does not turn the strip into a second column of
/// prose.
///
/// **Monospace, and that is the whole of the chrome's form.** The page keeps the proportional
/// face and gets no chrome fills; the chrome takes the monospace face and gets labelled. One
/// family tells a reader which of the two is speaking, and it survives a screenshot and a
/// monochrome eye the way a colour does not.
///
/// It is not free. `Proportional` resolves to `[Ubuntu-Light, NotoEmoji, emoji-icon-font]` and
/// Hack is in `Monospace` only, so this also moves the chrome onto the one embedded face that
/// has arrows and prime marks; see `chrome_is_monospace`. Hack's advance is wider at the same
/// size, so the status strip truncates sooner. And with [`FontChoice::Mono`] the page is in this
/// family too, which leaves size as the only type signal for that reader; the band's full bleed
/// and `notice::MARK` are what carry it there.
pub fn chrome_font(opts: &ReadOpts) -> FontId {
    FontId::new((opts.base_size_pt * 0.78).max(11.0), FontFamily::Monospace)
}

/// `TextStyle::Small`: the chrome size in the **page's** family.
///
/// This exists because `.small()` is page text at most of its call sites (the byline, a code
/// block's language chip, a comment timestamp, an image caption) and mapping `Small` straight to
/// [`chrome_font`] would set all of them in monospace. Chrome asks for [`chrome_font`] by name
/// instead. The rule: if it is the page's words, `Small`; if it is the reader's, `chrome_font`.
pub fn small_font(opts: &ReadOpts) -> FontId {
    FontId::new(chrome_font(opts).size, body_family(opts))
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
    /// `notice::MARK`, the severity word, and every `detail` line.
    ///
    /// Its own role because `dim` measures 3.10 on Light's `caution_bg` and cannot be used
    /// there. On the two severities that keep `bg` it still resolves to `dim`.
    pub aside: Color32,
}

/// Only `Caution` gets a ground of its own.
///
/// The old rule was that every severity shares one fill, argued from a band being unmistakable
/// because it is full-bleed. That argument is why this changed: `Severity::fills_the_viewport`
/// is true for `Quiet` and `Failure`, so those two are already as full-bleed as it gets and have
/// no page under them to be a different surface *from*. `Caution` is the only severity with
/// prose underneath.
///
/// A second constraint forces the same answer. How dark a fill can go is capped by whatever has
/// to stay legible on it, and on the old shared ground that was `dim` at 1.32:1 from the page,
/// or `notice_fg` at 1.42:1. Colouring the ground instead of the words is the only way past
/// that ceiling, and it is only affordable once one severity owns the ground.
///
/// Dropping the fill on the other two raises every ink on them, because they now land on `bg`:
/// heading to 6.38 / 6.21 / 7.62, body to 16.94 / 10.87 / 11.68, aside to 5.95 / 5.54 / 6.26.
/// The worst pair in the audit stops existing rather than being corrected.
///
/// There was a fourth field here, a `guide` hairline on the edges of a band with no ground. It
/// was built and removed, because it can never fire where it would mean anything: the only
/// severities without a ground are the two that claim the viewport, so the top edge lands
/// exactly on the bottom edge the URL or find bar already drew, or, with no bar open, on row
/// zero of the window, where it renders half clipped and reads as a stray window border. Every
/// palette was photographed to check. A band that fills the viewport has no page beside it to
/// be told apart from, which is the same argument that took the fill away.
pub fn notice_ink(pal: &Palette, severity: Severity) -> NoticeInk {
    match severity {
        Severity::Quiet => NoticeInk {
            fill: pal.bg,
            heading: pal.dim,
            body: pal.dim,
            aside: pal.dim,
        },
        Severity::Caution => NoticeInk {
            fill: pal.caution_bg,
            heading: pal.fg,
            body: pal.fg,
            aside: pal.fg,
        },
        // A failure's body is prose meant to be read, so it is set in `fg`; only the heading
        // carries the notice colour. That is what `error_screen` already did.
        Severity::Failure => NoticeInk {
            fill: pal.bg,
            heading: pal.notice_fg,
            body: pal.fg,
            aside: pal.dim,
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
    FontId::new(chrome_font(opts).size * 1.45, FontFamily::Monospace)
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
    // Floating, so it keeps the chrome ground. After the help card stopped being a `Window`
    // this dresses egui's tooltips, which `status_strip` raises three of.
    visuals.window_fill = pal.chrome_bg;
    // The `TextEdit` ground in the URL and find bars. `bg`, because those bars no longer have a
    // fill of their own and this would otherwise be the only one left in the docked chrome;
    // `inactive.bg_stroke` below is what gives the field its border instead.
    visuals.extreme_bg_color = pal.bg;
    visuals.faint_bg_color = pal.code_bg;
    visuals.code_bg_color = pal.code_bg;
    visuals.hyperlink_color = pal.link;
    visuals.selection.bg_fill = pal.selection;
    visuals.selection.stroke = Stroke::new(1.0, pal.link);
    visuals.window_stroke = Stroke::new(1.0, pal.rule);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, pal.rule);
    // No fill on a widget at rest. Taking the fill off the panels and leaving it on the buttons
    // inside them would be neither treatment: a row of filled buttons sitting on nothing. A
    // button still has to read as one, so it gets a border instead, which is also what the URL
    // and find fields get.
    //
    // `guide`, not `rule`. The border is the *entire* affordance once the fill is gone, which
    // is `guide`'s definition and the opposite of `rule`'s: a control's edge identifies it, and
    // `rule` is documented soft and not load-bearing precisely so that it cannot be asked to do
    // this. It measured 1.41 / 1.41 / 1.48 against `bg` on the failure screen, where the three
    // buttons are the only controls the reader has left. Text was never the shortfall: the
    // labels take `override_text_color`, which is `fg` at 16.94 / 10.87 / 11.68.
    visuals.widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, pal.guide);
    // Hover is transient feedback rather than chrome, so a momentary tint is the cheapest way
    // to show a hit target. The tint alone is not enough to see (`code_bg` is 1.14 from `bg`, and
    // it is a page role rather than a hit-target one), so the border firms to `fg` as well: rest
    // and hover have to differ by more than a fill nobody can find, now that rest is no longer
    // the near-invisible edge that made any change to it look like feedback.
    visuals.widgets.hovered.weak_bg_fill = pal.code_bg;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, pal.fg);
    // Square. The reader draws no rounded box anywhere else, and a rounded `TextEdit` in a panel
    // with no fill behind it is the one shape that still looks like somebody else's widget.
    for w in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
    ] {
        w.corner_radius = egui::CornerRadius::ZERO;
    }
    // The reader draws its own focus affordance on links; egui's default is a thick blue box
    // that fights the text.
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, pal.link);
    style.visuals = visuals;

    style.text_styles = [
        (egui::TextStyle::Body, body_font(opts)),
        (egui::TextStyle::Heading, heading_font(opts, 1)),
        (egui::TextStyle::Monospace, mono_font(opts)),
        // `Button` is chrome by definition. `Small` is not: see `small_font`.
        (egui::TextStyle::Button, chrome_font(opts)),
        (egui::TextStyle::Small, small_font(opts)),
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

    /// These tests are the exception to "no tests under `ui/`", and the exception is argued
    /// rather than assumed.
    ///
    /// The rule for this directory is that it holds what only a `Context` can run, so it is
    /// compiled and never tested. [`palette`] is not that: it is a pure
    /// `(Theme, bool) -> Palette`, and [`notice_ink`] is a pure
    /// `(&Palette, Severity) -> NoticeInk`. Neither takes a `Context`, unlike [`apply`],
    /// [`measure_px`], and [`system_is_dark`], which all do. The directory's real rule is
    /// "split by what a test can reach", and both of these are reachable. A further test under
    /// `ui/` needs the argument made again, not this paragraph cited.
    ///
    /// # Why luminance and not distance
    ///
    /// What stood here measured euclidean RGB against a floor of 24, and the band it was
    /// guarding passed at 47.6 / 35.9 / 48.0 while measuring 1.29 / 1.19 / 1.34 in luminance.
    /// The metric credited a hue swing the eye reads as almost nothing, which is exactly how a
    /// palette drifts under a passing test: `dim` was under AA on every ground it landed on
    /// except the page itself, for as long as that test existed.
    ///
    /// WCAG 2.2 relative luminance. sRGB, and `Color32` is not premultiplied here because every
    /// role compared is opaque; `selection` is the only one that is not, and it is not a role
    /// anything here is measured against.
    fn relative_luminance(c: Color32) -> f32 {
        let f = |v: u8| {
            let v = f32::from(v) / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * f(c.r()) + 0.7152 * f(c.g()) + 0.0722 * f(c.b())
    }

    /// `(lighter + 0.05) / (darker + 0.05)`. 1.0 is identical, 21.0 is black on white.
    fn contrast(a: Color32, b: Color32) -> f32 {
        let (x, y) = (relative_luminance(a), relative_luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    /// Every ink clears its theme's floor on every ground it can land on.
    ///
    /// Walks [`Theme::REAL`] rather than a list written here, so a palette added without an
    /// entry there is caught by `opts::tests::real_lists_every_palette`, whose exhaustive match
    /// stops compiling on a new variant, instead of quietly escaping this.
    #[test]
    fn every_palette_meets_its_floor() {
        for theme in Theme::REAL {
            let p = palette(theme, false);
            let floor = theme.floor();
            let grounds = [
                ("bg", p.bg),
                ("chrome_bg", p.chrome_bg),
                ("code_bg", p.code_bg),
            ];
            let inks = [
                ("fg", p.fg),
                ("dim", p.dim),
                ("link", p.link),
                ("strong", p.strong),
                ("notice_fg", p.notice_fg),
            ];
            for (gn, g) in grounds {
                for (inkn, ink) in inks {
                    let c = contrast(ink, g);
                    assert!(
                        c >= floor,
                        "{theme:?}: {inkn} on {gn} is {c:.2}, floor {floor}"
                    );
                }
            }
            // The current find match inverts: the page ground becomes the ink.
            let c = contrast(p.bg, p.find_current_bg);
            assert!(c >= floor, "{theme:?}: bg on find_current_bg is {c:.2}");
        }
    }

    /// Step 4's table, as a test rather than as a comment about one.
    ///
    /// Walking [`notice_ink`] is the point. Asserting `fg` against `caution_bg` by hand would
    /// leave the half that matters unchecked: `dim` measures 3.10 on Light's `caution_bg`, which
    /// is the whole reason `aside` exists, and a hand-written pair does not notice when someone
    /// puts it back.
    #[test]
    fn every_notice_ink_is_legible_on_its_own_ground() {
        for theme in Theme::REAL {
            let p = palette(theme, false);
            let floor = theme.floor();
            for sev in [Severity::Quiet, Severity::Caution, Severity::Failure] {
                let ink = notice_ink(&p, sev);
                for (name, c) in [
                    ("heading", ink.heading),
                    ("body", ink.body),
                    ("aside", ink.aside),
                ] {
                    let m = contrast(c, ink.fill);
                    assert!(
                        m >= floor,
                        "{theme:?}/{sev:?}: {name} is {m:.2}, floor {floor}"
                    );
                }
            }
        }
    }

    /// A caution has a page under it, so its ground has to read as a different surface.
    ///
    /// Not a WCAG rule, a project one. Below about 1.5 a strip reads as a tint of the page
    /// rather than as something laid over it, which is the failure the old distance test was
    /// meant to catch and did not.
    #[test]
    fn a_caution_band_is_a_different_surface() {
        for theme in Theme::REAL {
            let p = palette(theme, false);
            let c = contrast(p.caution_bg, p.bg);
            assert!(c >= 1.5, "{theme:?}: caution_bg is {c:.2} from bg");
        }
    }

    /// `guide` is visible wherever it is drawn, which after this change is a lot of places.
    ///
    /// The blockquote bar, the thread indent guide, the edge of every docked panel, the hairline
    /// on a band with no ground, and the border of every button and text field at rest. `rule`
    /// measures 1.41 / 1.41 / 1.48 against `bg` and cannot carry any of them; that is why the
    /// role split rather than `rule` being firmed.
    ///
    /// Both grounds, not just `bg`. Controls land on `chrome_bg` too (the page-info panel's
    /// close, the link context menu), and Light and Sepia cleared `bg` at 3.24 / 3.15 while
    /// measuring 2.91 / 2.80 there. That is the gap a one-ground test does not see, and it is
    /// what the two `guide` values moved for.
    #[test]
    fn guide_is_visible_where_it_is_drawn() {
        for theme in Theme::REAL {
            let p = palette(theme, false);
            for (name, ground) in [("bg", p.bg), ("chrome_bg", p.chrome_bg)] {
                let c = contrast(p.guide, ground);
                assert!(c >= 3.0, "{theme:?}: guide is {c:.2} from {name}");
            }
        }
    }

    /// The chrome is monospace and the page is not, which is the whole of the form split.
    ///
    /// Reachable for the same reason [`palette`] is: [`chrome_font`] and [`small_font`] take
    /// only a `&ReadOpts`, never a `Context`.
    ///
    /// Tied to `notice::no_notice_contains_an_arrow`, which lives where the wording does. The
    /// two are one fact: `Proportional` resolves to `[Ubuntu-Light, NotoEmoji,
    /// emoji-icon-font]` and none of those carries an arrow or a prime mark, while Hack, which
    /// is `Monospace` only, carries all of them. Moving the chrome here means the ban on arrows
    /// is policy rather than necessity, and the ban stays: no string wants one today, and
    /// lifting it is a separate decision. If this test ever fails, that decision has been taken
    /// by accident.
    #[test]
    fn chrome_is_monospace() {
        let mut opts = ReadOpts::default();
        for family in [FontChoice::Sans, FontChoice::Mono] {
            opts.family = family;
            assert_eq!(chrome_font(&opts).family, FontFamily::Monospace);
            assert_eq!(notice_heading_font(&opts).family, FontFamily::Monospace);
            // `Small` follows the page, or the byline and the code chip go monospace with it.
            assert_eq!(small_font(&opts).family, body_font(&opts).family);
            assert_eq!(small_font(&opts).size, chrome_font(&opts).size);
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
        // And every theme that is not the redirect is in `REAL`, so nothing escapes the checks
        // above by being added to the `match` and nowhere else.
        for theme in Theme::REAL {
            assert_ne!(theme, Theme::System);
        }
        assert_eq!(Theme::REAL.len(), 5);
    }
}
