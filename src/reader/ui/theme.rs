//! `ReadOpts` -> `egui::Style`. **The only file in the crate where a colour appears.**
//!
//! Everything here is a presentation decision, which is exactly why it is confined to one
//! module that the IR cannot reach and that no extractor imports.

use crate::reader::desktop;
use crate::reader::face::Face;
use crate::reader::notice::Severity;
use crate::reader::opts::{FontChoice, ReadOpts, Theme};
use crate::reader::ui::fonts;
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
    /// Emphasised ink in the reader's own surfaces: a comment's author, a table's header row,
    /// the help card's key column.
    ///
    /// **Not** inline `Style::strong`, which is a real bold face now that one is embedded. This
    /// role is what is left after that moved out: places with no run structure to carry a face,
    /// where the emphasis is the reader's rather than the page's.
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
    /// The ground behind a `Caution` bar, and behind nothing else.
    ///
    /// A pale tint now, not a surface: the bar is docked under the top chrome with a painted
    /// icon, a hairline, and a close control, so the ground is no longer the only thing saying
    /// "this is a remark", and it can be quiet. Held to 1.1:1 from `bg` by
    /// `a_caution_bar_is_tinted_and_its_inks_clear_the_floor`, which also asks every ink the bar
    /// draws (`fg`, `dim`, `notice_fg`) to clear the theme's floor on it. The old 1.5 floor
    /// existed because the ground carried the whole signal; see `notice_ui`.
    pub caution_bg: Color32,
    /// The underline under a link in prose: `link` at 65% over the page.
    ///
    /// Decoration, like `rule`, so it is not one of the inks `every_palette_meets_its_floor`
    /// walks; but it is the one non-colour cue that says "this is a link", so
    /// `the_link_underline_is_visible` holds its blend over `bg` to the 3:1 non-text floor.
    /// Stored with alpha so one value serves every ground the link can sit on.
    pub link_underline: Color32,
    /// The current find match. Its own role because find is the reader searching the page,
    /// not the reader reporting on it, and the two must be free to move independently.
    pub find_current_bg: Color32,
    /// Top chrome (URL bar, find bar, notice bars, outline) and surfaces that **float over**
    /// the page: the page-info panel, the help card, the hover-URL strip, the toast's neighbours,
    /// and egui's tooltips via `window_fill`. The status strip stays on `bg`.
    ///
    /// In the contrast pair this equals `bg` on purpose, which is the platform convention, and
    /// it works because every floating surface is drawn with a stroke around it.
    pub chrome_bg: Color32,
    pub selection: Color32,
    /// The transient status toast: the page's negative. A dark pill on a light page, a lifted
    /// grey on a dark one, so a word that is on screen for six seconds is found at a glance and
    /// is unmistakably the reader's. `toast_fg` is held to the theme's floor on it.
    pub toast_bg: Color32,
    pub toast_fg: Color32,
    pub dark_mode: bool,
}

/// The one corner radius, in points. Five: three read as square from a normal distance and
/// seven turns an 18 pt keycap into a pill. It goes on what sits *on* the page or floats *over*
/// it (every control, the code ground, the image frame, a chip, a panel) and never on anything
/// that spans the window (a bar, the strip, a rule, a guide): those are edges, and an edge has
/// no corner to round. `theme::apply` installs it on every egui widget at once, and the frames
/// drawn by hand take it by name.
pub const RADIUS: u8 = 5;

/// `link_underline`'s alpha: 65% of `link` over `bg` blends to 3.3:1 or better on every theme,
/// which is the non-text floor; Sepia is the tight one, at 3.29. 45% was tried and blends to
/// 2.2 on Light, which is faint for the one cue that says "this is a link" to an eye that does
/// not see the blue, and 60% left Sepia at 2.95. Blended the way epaint blends, in gamma space.
const UNDERLINE_ALPHA: u8 = 166;

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
            caution_bg: Color32::from_rgb(0xF4, 0xED, 0xDB),
            link_underline: Color32::from_rgba_unmultiplied(0x15, 0x4B, 0x9E, UNDERLINE_ALPHA),
            find_current_bg: Color32::from_rgb(0x9A, 0x45, 0x0B),
            chrome_bg: Color32::from_rgb(0xF1, 0xF1, 0xEC),
            selection: Color32::from_rgba_unmultiplied(0x15, 0x4B, 0x9E, 0x40),
            toast_bg: Color32::from_rgb(0x2B, 0x2D, 0x30),
            toast_fg: Color32::from_rgb(0xEC, 0xEC, 0xE7),
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
            caution_bg: Color32::from_rgb(0xE8, 0xDD, 0xBF),
            link_underline: Color32::from_rgba_unmultiplied(0x1D, 0x4E, 0x77, UNDERLINE_ALPHA),
            find_current_bg: Color32::from_rgb(0x8E, 0x3F, 0x12),
            chrome_bg: Color32::from_rgb(0xE9, 0xDF, 0xC8),
            selection: Color32::from_rgba_unmultiplied(0x1D, 0x4E, 0x77, 0x40),
            toast_bg: Color32::from_rgb(0x3A, 0x33, 0x28),
            toast_fg: Color32::from_rgb(0xEF, 0xE8, 0xD6),
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
            caution_bg: Color32::from_rgb(0x2A, 0x25, 0x19),
            link_underline: Color32::from_rgba_unmultiplied(0x7F, 0xB4, 0xF5, UNDERLINE_ALPHA),
            find_current_bg: Color32::from_rgb(0xE0, 0x9B, 0x54),
            chrome_bg: Color32::from_rgb(0x1E, 0x21, 0x23),
            selection: Color32::from_rgba_unmultiplied(0x7F, 0xB4, 0xF5, 0x40),
            toast_bg: Color32::from_rgb(0x3B, 0x3F, 0x44),
            toast_fg: Color32::from_rgb(0xED, 0xEC, 0xE7),
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
            caution_bg: Color32::from_rgb(0xFF, 0xF0, 0xC2),
            link_underline: Color32::from_rgba_unmultiplied(0x00, 0x30, 0x8F, UNDERLINE_ALPHA),
            find_current_bg: Color32::from_rgb(0x00, 0x00, 0x00),
            chrome_bg: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            selection: Color32::from_rgba_unmultiplied(0x00, 0x30, 0x8F, 0x40),
            toast_bg: Color32::from_rgb(0x00, 0x00, 0x00),
            toast_fg: Color32::from_rgb(0xFF, 0xFF, 0xFF),
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
            caution_bg: Color32::from_rgb(0x1F, 0x1A, 0x00),
            link_underline: Color32::from_rgba_unmultiplied(0x7F, 0xC0, 0xFF, UNDERLINE_ALPHA),
            find_current_bg: Color32::from_rgb(0xFF, 0xB0, 0x00),
            chrome_bg: Color32::from_rgb(0x00, 0x00, 0x00),
            selection: Color32::from_rgba_unmultiplied(0x7F, 0xC0, 0xFF, 0x40),
            toast_bg: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            toast_fg: Color32::from_rgb(0x00, 0x00, 0x00),
            dark_mode: true,
        },
    }
}

/// The family body text is set in. Headings, quotes, and captions follow it; code never does.
///
/// Plain weight and upright: `Strong` and `Emph` are separate faces, resolved per run by
/// [`crate::reader::face::Face`] rather than here, because a `FontId` covers a whole galley
/// section and an inline style covers part of one.
fn body_family(opts: &ReadOpts) -> FontFamily {
    fonts::family(Face::new(opts.family, false, false))
}

pub fn body_font(opts: &ReadOpts) -> FontId {
    FontId::new(opts.base_size_pt, body_family(opts))
}

/// The role a heading at `level` is set in.
///
/// The title and the first two heading levels take the **serif** when the page is set in sans:
/// a display face over a text face is the oldest pairing there is, and Plex Serif is already
/// embedded for the serif reading choice, so it costs nothing. A serif page keeps the serif, and
/// a mono page keeps mono: a reader who chose one family for the whole page gets the whole page
/// in it. Levels past 3 follow the body, because by then the heading is a run-in, not a display.
///
/// A role rather than a `FontId`, so `inline_ui::Setting` can resolve `Strong` and `Emph`
/// inside the heading to the matching faces: an `<em>` in a serif heading is serif italic.
pub fn heading_role(opts: &ReadOpts, level: u8) -> FontChoice {
    match (opts.family, level) {
        (FontChoice::Sans, 0..=3) => FontChoice::Serif,
        (family, _) => family,
    }
}

/// Headings step down from the body size. Level 1 is the article title, so it gets the jump,
/// and it is the one level set in the display role's **bold** face; levels past 4 stop
/// shrinking, because a heading smaller than its paragraph is not a heading.
pub fn heading_font(opts: &ReadOpts, level: u8) -> FontId {
    let scale = match level {
        0 | 1 => 1.9,
        2 => 1.35,
        3 => 1.18,
        4 => 1.07,
        _ => 1.0,
    };
    let face = Face::new(heading_role(opts, level), level <= 1, false);
    FontId::new(opts.base_size_pt * scale, fonts::family(face))
}

/// Air above a heading, in points: more than below, so a heading belongs to what follows it.
/// Below is the ordinary block gap, which `ready_screen`'s placer adds and a skipped block
/// reproduces; the extra sits *inside* the block, where `measure::Heights` sees it.
pub fn heading_space_above(opts: &ReadOpts, level: u8) -> f32 {
    snap(opts.base_size_pt * if level <= 3 { 1.2 } else { 0.6 })
}

/// Code is 0.92x because a monospace face at the body's optical size reads larger than it is.
/// One number for a code block and for code inside a sentence, so the two cannot drift apart.
pub const CODE_SCALE: f32 = 0.92;

pub fn mono_font(opts: &ReadOpts) -> FontId {
    FontId::new(opts.base_size_pt * CODE_SCALE, FontFamily::Monospace)
}

/// Chrome: status strip, URL bar, outline, page info, and every word a bar says. Fixed
/// relative to the body so a wide reading size does not turn the strip into a second column of
/// prose.
///
/// **Monospace, and that is the whole of the chrome's form.** The page keeps the proportional
/// face and gets no chrome fills; the chrome takes the monospace face and gets labelled. One
/// family tells a reader which of the two is speaking, and it survives a screenshot and a
/// monochrome eye the way a colour does not.
///
/// It is not free. A monospace advance is wider at the same size, so the status strip truncates
/// sooner; see `chrome_is_monospace`. And with [`crate::reader::opts::FontChoice::Mono`] the page is in this family
/// too, which leaves size as the only type signal for that reader; the bar's dock under the
/// chrome and the painted icon are what carry it there.
pub fn chrome_font(opts: &ReadOpts) -> FontId {
    FontId::new((opts.base_size_pt * 0.78).max(11.0), FontFamily::Monospace)
}

/// A chrome *label*: the site eyebrow over a title, a panel's title, a group heading in the
/// page-info panel. Uppercase with a little tracking, in the chrome font and colour given, so it
/// reads as a label rather than as a line of the text it sits over. A `LayoutJob`, because
/// letter spacing is a `TextFormat` property and `RichText` has none.
pub fn label_job(text: &str, opts: &ReadOpts, color: Color32) -> egui::text::LayoutJob {
    let font = chrome_font(opts);
    let spacing = font.size * 0.08;
    let mut job = egui::text::LayoutJob::default();
    job.append(
        &text.to_uppercase(),
        0.0,
        egui::TextFormat {
            font_id: font,
            color,
            extra_letter_spacing: spacing,
            ..Default::default()
        },
    );
    job
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

/// Body line height in points: [`ReadOpts::line_height`] times the body size.
///
/// This is the row `j`/`k` move by, and the floor [`line_height_for`] will not go under, so a
/// caption keeps the page's rhythm. It is not the row a title wraps at; a title is 1.9× this
/// size and used to inherit these pixels as its only leading, which is how descenders sat on
/// the next line.
pub fn line_height_px(opts: &ReadOpts) -> f32 {
    // Rounded, because epaint advises an even number of pixels per row for even text and
    // because a fractional row height accumulates into a visibly drifting baseline down a long
    // page.
    snap(opts.base_size_pt * opts.line_height)
}

/// Display leading, as a multiple of the face's own size. Prose is 1.35; a title at that
/// ratio of 1.9× the body is loose in the way a wrapped headline is not. Display type is set
/// tighter than prose, not to the same pixels and not to the same multiple.
const HEADING_LINE_HEIGHT: f32 = 1.2;

/// Line height in points for text set at `size`.
///
/// Every section of one galley still shares one number, which is what keeps the several labels
/// of a paragraph on one baseline. The body ratio is a floor, so a caption keeps the page's
/// rhythm; a heading takes 1.2 of *its* size so a wrapped title has air without taking the
/// body's 1.35.
pub fn line_height_for(opts: &ReadOpts, size: f32) -> f32 {
    line_height_px(opts).max(snap(size * HEADING_LINE_HEIGHT))
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
///
/// winit answers on the platforms where it can, and `raw.system_theme` is that answer.
/// [`desktop`] is the Linux answer, where winit reports nothing at all; see its module doc.
/// Neither knowing is Light, because a palette has to be picked.
pub fn system_is_dark(ctx: &egui::Context) -> bool {
    match ctx.input(|i| i.raw.system_theme) {
        Some(theme) => theme == egui::Theme::Dark,
        None => desktop::prefers_dark().unwrap_or(false),
    }
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
    /// Every `detail` line: the URL under a failure, the address a dead rule was asked for.
    pub aside: Color32,
    /// The painted severity icon: the triangle, the circled `i`, the crossed circle. A graphical
    /// object, so it is held to 3:1 on `fill` rather than to the text floor.
    pub icon: Color32,
}

/// How each severity is painted, in the dress browsers already taught everyone.
///
/// A `Quiet` remark is an information bar: the top chrome's own ground, a dim icon. A
/// `Caution` is the same bar on a pale tint, with the icon in the notice colour; the tint is a
/// cue, not the signal, which is why it can be as quiet as it is (see `Palette::caution_bg`). A
/// `Failure` is an error page on the page ground: a muted glyph in `guide`, the heading in the
/// notice colour, and the explanation in `fg`, because it is prose meant to be read.
///
/// What the old band did with colour it did because colour was nearly all it had. Every ink
/// here still clears the theme's floor on its own fill, which
/// `every_notice_ink_is_legible_on_its_own_ground` walks.
pub fn notice_ink(pal: &Palette, severity: Severity) -> NoticeInk {
    match severity {
        Severity::Quiet => NoticeInk {
            fill: pal.chrome_bg,
            heading: pal.fg,
            body: pal.fg,
            aside: pal.dim,
            icon: pal.dim,
        },
        Severity::Caution => NoticeInk {
            fill: pal.caution_bg,
            heading: pal.fg,
            body: pal.fg,
            aside: pal.dim,
            icon: pal.notice_fg,
        },
        Severity::Failure => NoticeInk {
            fill: pal.bg,
            heading: pal.notice_fg,
            body: pal.fg,
            aside: pal.dim,
            icon: pal.guide,
        },
    }
}

/// The error page's heading.
///
/// The one line of the reader's own that is not monospace, and deliberately: an error page is
/// the convention every browser shares (a short heading in the UI face, a paragraph, the code,
/// Reload), and a reader meets it on the worst day a page can have. It is set in the **sans
/// bold** at 1.4x body whatever the page's family, so it is never the page's own `<h2>`: the
/// page is serif-headed or mono-headed by `heading_role`, and sans bold at this size belongs to
/// nothing an article can contain.
pub fn notice_heading_font(opts: &ReadOpts) -> FontId {
    FontId::new(
        snap(opts.base_size_pt * 1.4),
        fonts::family(Face::new(FontChoice::Sans, true, false)),
    )
}

/// The idle screen's wordmark: the serif bold, large. The one screen where the reader is the
/// page, so it takes the page's display face rather than the chrome's.
pub fn wordmark_font(opts: &ReadOpts) -> FontId {
    FontId::new(
        snap(opts.base_size_pt * 2.6),
        fonts::family(Face::new(FontChoice::Serif, true, false)),
    )
}

/// Vertical padding inside a notice bar, in points. Integer because `egui::Margin` is `i8`.
pub fn bar_pad(opts: &ReadOpts) -> i8 {
    (opts.base_size_pt * 0.5).round().clamp(6.0, 24.0) as i8
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
    // One radius on every control (see `RADIUS`): the URL and find fields, every button, the
    // help card's keycaps. A frame the reader draws by hand takes the same constant by name, and
    // the things that span the window (bars, the strip, rules) are not widgets and stay square.
    for w in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
    ] {
        w.corner_radius = egui::CornerRadius::same(RADIUS);
    }
    // The floating surfaces egui dresses itself: tooltips and the link context menu.
    visuals.window_corner_radius = egui::CornerRadius::same(RADIUS);
    visuals.menu_corner_radius = egui::CornerRadius::same(RADIUS);
    // The reader draws its own focus affordance on links; egui's default is a thick blue box
    // that fights the text.
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, pal.link);
    // Browser cursor on every Button. Native toolkits keep the arrow; a second browser should
    // not. `Visuals::dark`/`light` leave this `None`.
    visuals.interact_cursor = Some(egui::CursorIcon::PointingHand);
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
    use crate::reader::opts::FontChoice;

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
            // So does the toast, and its ink is its own role.
            let c = contrast(p.toast_fg, p.toast_bg);
            assert!(c >= floor, "{theme:?}: toast_fg on toast_bg is {c:.2}");
            // And it is a different surface from the page it floats over: the old band's 1.5,
            // which is the line below which a fill reads as a tint of the page rather than as
            // a thing laid on it. Dark's lift is the closest, at 1.72.
            let c = contrast(p.toast_bg, p.bg);
            assert!(c >= 1.5, "{theme:?}: toast_bg is {c:.2} from bg");
        }
    }

    /// `link_underline` over `bg`, composited, clears the 3:1 non-text floor.
    ///
    /// The underline is decoration in the sense that the word above it carries the text
    /// contrast, but it is the one cue that says "link" to an eye that does not see the hue,
    /// and WCAG 1.4.11 holds a graphical object that conveys information to 3:1. At 45% alpha
    /// it blended to 2.2 on Light and at 60% Sepia sat at 2.95; this is why the constant is 65%.
    /// Blended in gamma space, because that is how epaint composites a translucent stroke.
    #[test]
    fn the_link_underline_is_visible() {
        for theme in Theme::REAL {
            let p = palette(theme, false);
            // `Color32` stores premultiplied components; the blend wants the straight ones.
            let [r, g, b, a] = p.link_underline.to_srgba_unmultiplied();
            let ground = p.bg;
            let a = f32::from(a) / 255.0;
            let blend = |o: u8, g: u8| {
                (f32::from(o) * a + f32::from(g) * (1.0 - a))
                    .round()
                    .clamp(0.0, 255.0) as u8
            };
            let blended = Color32::from_rgb(
                blend(r, ground.r()),
                blend(g, ground.g()),
                blend(b, ground.b()),
            );
            let c = contrast(blended, p.bg);
            assert!(c >= 3.0, "{theme:?}: the underline blends to {c:.2} on bg");
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
                // The icon is a shape, not text: the non-text floor.
                let m = contrast(ink.icon, ink.fill);
                assert!(m >= 3.0, "{theme:?}/{sev:?}: icon is {m:.2}");
            }
        }
    }

    /// A caution bar is tinted, and every ink it draws clears the floor on the tint.
    ///
    /// The floor on the tint itself is 1.1, not the 1.5 the old band held, and the difference
    /// is what the bar has that the band did not: a dock under the chrome, a painted icon, a
    /// hairline, and a close. The ground used to be the whole signal, so it had to read as a
    /// separate surface; now it is one of five, and it can be a tint. What must not drift is
    /// the ink on it: `dim` measured 3.10 on the old Light ground, which is why `aside` had to
    /// be `fg` there, and a tint that fails `dim` would send that back. The icon is a graphical
    /// object, so it is held to 3:1 rather than to the text floor.
    #[test]
    fn a_caution_bar_is_tinted_and_its_inks_clear_the_floor() {
        for theme in Theme::REAL {
            let p = palette(theme, false);
            let floor = theme.floor();
            let tint = contrast(p.caution_bg, p.bg);
            assert!(tint >= 1.1, "{theme:?}: caution_bg is {tint:.2} from bg");
            for (name, ink) in [("fg", p.fg), ("dim", p.dim), ("notice_fg", p.notice_fg)] {
                let c = contrast(ink, p.caution_bg);
                assert!(c >= floor, "{theme:?}: {name} on caution_bg is {c:.2}");
            }
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
    /// The error page's heading is the one named exception (see [`notice_heading_font`]): it is
    /// the sans bold, and it must not be the family the page's headings are set in, which is
    /// what the second assertion pins for the sans reader, where `heading_role` hands the page
    /// the serif.
    #[test]
    fn chrome_is_monospace() {
        let mut opts = ReadOpts::default();
        for family in [FontChoice::Sans, FontChoice::Serif, FontChoice::Mono] {
            opts.family = family;
            assert_eq!(chrome_font(&opts).family, FontFamily::Monospace);
            // `Small` follows the page, or the byline and the code chip go monospace with it.
            assert_eq!(small_font(&opts).family, body_font(&opts).family);
            assert_eq!(small_font(&opts).size, chrome_font(&opts).size);
        }
        opts.family = FontChoice::Sans;
        assert_ne!(
            notice_heading_font(&opts).family,
            heading_font(&opts, 2).family,
            "the error page's heading must not be the page's"
        );
    }

    /// A sans page is serif-headed; a serif or mono page keeps its own family throughout, and
    /// the title is the display role's bold. Decidable without a `Context` for the same reason
    /// as the test above.
    #[test]
    fn headings_take_the_display_role_only_over_a_sans_page() {
        let mut opts = ReadOpts {
            family: FontChoice::Sans,
            ..ReadOpts::default()
        };
        for level in 1..=3 {
            assert_eq!(heading_role(&opts, level), FontChoice::Serif);
        }
        assert_eq!(heading_role(&opts, 4), FontChoice::Sans);
        for family in [FontChoice::Serif, FontChoice::Mono] {
            opts.family = family;
            for level in 1..=5 {
                assert_eq!(heading_role(&opts, level), family, "{family:?} h{level}");
            }
        }
        opts.family = FontChoice::Sans;
        assert_eq!(
            heading_font(&opts, 1).family,
            fonts::family(Face::new(FontChoice::Serif, true, false))
        );
        assert_eq!(
            heading_font(&opts, 2).family,
            fonts::family(Face::new(FontChoice::Serif, false, false))
        );
        assert!(heading_space_above(&opts, 2) > heading_space_above(&opts, 5));
    }

    /// A title is 1.9× the body. Wrapping it at the body's pixels is 0.82 of its own size,
    /// which is how descenders sat on the next line. The body row stays the floor so a caption
    /// keeps the page's rhythm.
    #[test]
    fn a_wrapped_title_takes_its_own_leading() {
        let opts = ReadOpts::default();
        let body = line_height_px(&opts);
        let title = heading_font(&opts, 1).size;
        let title_lh = line_height_for(&opts, title);
        assert!(title_lh > body);
        assert_eq!(title_lh, snap(title * HEADING_LINE_HEIGHT));
        assert!(title_lh < snap(title * opts.line_height));
        assert_eq!(line_height_for(&opts, opts.base_size_pt), body);
        assert!(line_height_for(&opts, opts.base_size_pt * 0.8) >= body);
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
