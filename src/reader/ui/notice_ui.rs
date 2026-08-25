//! `notice::Notice` -> the reader's own marks on screen. **The one place the reader draws its own
//! text.**
//!
//! Four shapes, and all four are ones a reader has met in every other browser, which is the
//! point: a reading client that is a second browser should report like one, not invent a
//! grammar of its own for "something about this page". They are:
//!
//! - [`bar`]: an **infobar**, for a page that loaded. Docked under the URL and find bars and
//!   above the page, so it does not scroll away; a severity icon at the left, one sentence, the
//!   actions as buttons at the right, and a close. One bar per remark, stacked. Firefox's
//!   notification bar, Chrome's infobar, GTK's `InfoBar`, and Material's banner share this
//!   anatomy exactly.
//! - [`error_page`]: for a navigation that produced no page. A muted glyph, a short heading in
//!   the UI face, a paragraph, the code in mono for whoever files the bug, Reload first.
//! - [`splash`]: the idle screen, the one screen where the reader *is* the page: the
//!   waveform, the wordmark, the two keys.
//! - [`loading_line`]: one dim line on an empty page while the first navigation is in flight.
//! - [`toast`]: a transient status ("Copied URL", "loading 2 images from …") as a dark rounded
//!   pill above the status strip for [`notice::TOAST_FOR`]: the snackbar every phone and most
//!   desktop apps use. It used to be a line of dim text in the strip, which a reader looking at
//!   the page did not see; a pill that is the page's negative is found in a glance.
//!
//! [`progress_rule`] is the fifth, and older: what the reader has to say about a fetch that has
//! not answered yet, said in the chrome because the reading column is busy holding the page the
//! reader is still on. Everything lives in this module so one property survives: the reader's
//! own marks have one home, and a new one is added here.
//!
//! # What makes a bar unmistakable
//!
//! **Position, then shape, then colour.** A bar is docked outside the scroll area, which is a
//! place page content cannot reach at all; that is a stronger property than the full-bleed
//! width the old band relied on, and it is what lets the bar be quiet. The icon's *shape* says
//! the severity (a triangle for a caution, a circled `i` for information, a crossed circle for
//! a failure), and it says it to a monochrome eye and in a screenshot; [`Severity::word`] is the
//! icon's accessible name and hover text, so it says it to a screen reader too. Colour is
//! support: `theme::notice_ink` gives a caution a pale tint and the icon the notice colour, and
//! `theme::every_notice_ink_is_legible_on_its_own_ground` is what keeps that honest.
//!
//! It follows that a bar borrows **no page idiom**: not `heading_font`, not `body_font`, not
//! the quote mark, not the code ground. `theme::notice_ink` and `theme::chrome_font` are where
//! its type and paint come from, and both belong to the reader rather than to any page. The
//! error page's heading is the named exception, and `theme::notice_heading_font` says why.
//!
//! # Dismissal
//!
//! A bar has a close, because every bar a reader has met has one. Closing it hides that remark
//! for the page it is about and no longer: `ReaderApp::commit` clears the set with every other
//! per-page reset. Nothing is lost by closing one: the page-info panel keeps the record.
//!
//! # Find does not search notices
//!
//! Bars are drawn outside the reading column, so they never reach `RenderCtx::begin_list` and
//! never count towards `find_total`. That is deliberate: find is the reader searching the
//! page, and a notice is not the page.

use crate::reader::notice::{self, Button, Notice, Severity};
use crate::reader::opts::ReadOpts;
use crate::reader::ui::theme::{self, Palette};
use eframe::egui::{self, RichText, Ui};

/// The reading column's geometry: the width the column spans, and the measure the page keeps to.
///
/// One `Column` is built per frame and feeds the article column and every full-screen state
/// (the error page, the splash), so the two cannot disagree about a left edge.
#[derive(Debug, Clone, Copy)]
pub struct Column {
    /// The reading measure, clamped to what the panel actually has. The outline panel takes
    /// width from the column, so this is a ceiling rather than a promise.
    pub measure: f32,
    /// Central-panel left edge to article left edge.
    pub gutter: f32,
}

impl Column {
    /// `wanted` is the unclamped [`theme::measure_px`].
    pub fn new(ui: &Ui, wanted: f32) -> Self {
        let full = ui.available_width();
        let measure = theme::snap(wanted.min(full));
        Self {
            measure,
            // Snapped: this becomes the left edge of a `Ui`, and egui paints an "Unaligned"
            // warning across the page in debug builds when one misses the pixel grid.
            gutter: theme::snap(((full - measure) * 0.5).max(0.0)),
        }
    }
}

/// The fetch-in-flight indicator: a hairline across the top edge of the status strip.
///
/// # Why the chrome and not a bar
///
/// Because there is a page under it. A navigation keeps the outgoing article on screen until the
/// new one commits, and nothing is the matter with a page that is loading; a bar is for something
/// the reader has to say about a page, and this is about the errand.
///
/// # Why a bar as well as the seconds the strip already counts
///
/// Two jobs. The strip's `{:.1}s` is the precise number, and precise on purpose: a bot-block hang
/// and a slow CDN are indistinguishable for ten seconds, and only a moving number separates them.
/// But the strip sets `TextWrapMode::Truncate`, so on a narrow window that text is the first thing
/// cut, the same failure that moved the page-info ribbon into a panel, and a number has to be
/// *read*. A rule spans the strip at any width and is legible at a glance.
///
/// # `guide`, and two pixels
///
/// `guide` is the palette's role for a mark that identifies structure rather than decorating it,
/// and the one held to 3:1 against `chrome_bg` by `guide_is_visible_where_it_is_drawn`, which is
/// the WCAG 1.4.11 floor a two-pixel graphical object needs. `notice_fg` would clear that too and
/// would be wrong for another reason: nothing is the matter with a page that is loading.
///
/// # Where it is painted
///
/// On a layer over `rect`, after the panel is laid out, and *downward into the strip*. Inside the
/// panel closure `ui.painter()` is clipped to the content rect, which the inner margin insets, so
/// a rule at the true top edge is clipped away; and the hover-URL overlay anchors its bottom edge
/// at exactly this line, so a rule drawn above it vanishes whenever the pointer is on a link. It
/// takes layout from nothing either way.
pub fn progress_rule(ctx: &egui::Context, pal: &Palette, rect: egui::Rect, fraction: f32) {
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("hww-progress"),
    ));
    // Two physical pixels, so the rule keeps its weight at every zoom.
    let h = (2.0 / ctx.pixels_per_point()).max(1.0);
    let top = theme::snap(rect.top());
    let track = egui::Rect::from_min_max(
        egui::pos2(rect.left(), top),
        egui::pos2(rect.right(), top + h),
    );
    // The track first: without it the bar is invisible for the first second of every fetch, which
    // is most of them, and an indicator nobody sees appear is not one.
    painter.rect_filled(track, 0.0, pal.rule);
    let width = theme::snap(track.width() * fraction.clamp(0.0, 1.0));
    if width > 0.0 {
        let mut filled = track;
        filled.set_right(track.left() + width);
        painter.rect_filled(filled, 0.0, pal.guide);
    }
}

/// What one bar reported this frame.
#[derive(Debug)]
pub struct BarEvent {
    pub pressed: Option<Button>,
    /// The close was clicked.
    pub dismissed: bool,
    /// The panel's rect, for the hairline `app` draws on its lower edge.
    pub rect: egui::Rect,
}

/// One notice as an infobar: a top panel under the URL and find bars.
///
/// `index` keeps the panel ids distinct when several bars stack. The caller draws the bottom
/// hairline from `BarEvent::rect`, because `panel_edge` is the app's and a `Frame` stroke would
/// box all four sides.
pub fn bar(ui: &mut Ui, pal: &Palette, opts: &ReadOpts, notice: &Notice, index: usize) -> BarEvent {
    let ink = theme::notice_ink(pal, notice.severity);
    let font = theme::chrome_font(opts);
    let mut event = BarEvent {
        pressed: None,
        dismissed: false,
        rect: egui::Rect::NOTHING,
    };
    let panel = egui::Panel::top(egui::Id::new(("hww-notice", index)))
        .show_separator_line(false)
        .frame(
            egui::Frame::new()
                .fill(ink.fill)
                .inner_margin(egui::Margin::symmetric(10, theme::bar_pad(opts))),
        )
        .show(ui, |ui| {
            // Every word in a bar is the reader's, so the family is set once here.
            ui.style_mut().override_font_id = Some(font.clone());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                // Centred on the first line of text, which is one chrome line tall.
                let d = theme::snap(font.size * 1.2);
                let line = ui
                    .text_style_height(&egui::TextStyle::Button)
                    .max(font.size * 1.3);
                let (rect, resp) =
                    ui.allocate_exact_size(egui::vec2(d, line), egui::Sense::hover());
                let icon_rect = egui::Rect::from_center_size(
                    egui::pos2(rect.center().x, rect.top() + line * 0.5),
                    egui::vec2(d, d),
                );
                if ui.is_rect_visible(rect) {
                    icon(ui.painter(), icon_rect, notice.severity, ink.icon);
                }
                // The severity reaches a screen reader through the icon's name, and the pointer
                // through its hover text; the shape is what a sighted reader gets.
                resp.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Label,
                        ui.is_enabled(),
                        notice.severity.word(),
                    )
                });
                resp.on_hover_text(notice.severity.word());

                // The close, then the actions, from the right; the text takes what is left.
                // Laid out first so the text column knows its width, and drawn right-to-left so
                // the close sits at the edge, where every other bar puts it.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                    let close = ui
                        .add(egui::Button::new(RichText::new("×").color(ink.aside)).frame(false))
                        .on_hover_text("Dismiss");
                    if close.clicked() {
                        event.dismissed = true;
                    }
                    for b in notice.actions.iter().rev() {
                        if ui.button(b.label()).clicked() {
                            event.pressed = Some(*b);
                        }
                    }
                    ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                        ui.spacing_mut().item_spacing.y = theme::snap(font.size * 0.2);
                        ui.label(RichText::new(notice.body.as_str()).color(ink.body));
                        for d in &notice.detail {
                            ui.label(RichText::new(d.as_str()).color(ink.aside));
                        }
                    });
                });
            });
        });
    event.rect = panel.response.rect;
    event
}

/// The severity, as a shape, painted into `rect`: a triangle with a bang, a circled `i`, a
/// crossed circle. Painted rather than typed, for the reason the page-info icon is: no face
/// needs to carry it, it takes the palette exactly, and it scales with the chrome.
pub fn icon(painter: &egui::Painter, rect: egui::Rect, severity: Severity, ink: egui::Color32) {
    let r = rect;
    let w = (r.height() * 0.09).max(1.2);
    let stroke = egui::Stroke::new(w, ink);
    let c = r.center();
    match severity {
        Severity::Caution => {
            let pts = vec![
                egui::pos2(c.x, r.top() + w),
                egui::pos2(r.right() - w, r.bottom() - w),
                egui::pos2(r.left() + w, r.bottom() - w),
            ];
            painter.add(egui::Shape::closed_line(pts, stroke));
            let bang = egui::Stroke::new(w * 1.15, ink);
            painter.line_segment(
                [
                    egui::pos2(c.x, r.top() + r.height() * 0.42),
                    egui::pos2(c.x, r.top() + r.height() * 0.66),
                ],
                bang,
            );
            painter.circle_filled(egui::pos2(c.x, r.top() + r.height() * 0.80), w * 0.65, ink);
        }
        Severity::Quiet => {
            painter.circle_stroke(c, r.width() * 0.5 - w, stroke);
            let bang = egui::Stroke::new(w * 1.15, ink);
            painter.line_segment(
                [
                    egui::pos2(c.x, r.top() + r.height() * 0.46),
                    egui::pos2(c.x, r.top() + r.height() * 0.72),
                ],
                bang,
            );
            painter.circle_filled(egui::pos2(c.x, r.top() + r.height() * 0.30), w * 0.65, ink);
        }
        Severity::Failure => {
            painter.circle_stroke(c, r.width() * 0.5 - w, stroke);
            let k = r.width() * 0.19;
            painter.line_segment(
                [egui::pos2(c.x - k, c.y - k), egui::pos2(c.x + k, c.y + k)],
                stroke,
            );
            painter.line_segment(
                [egui::pos2(c.x + k, c.y - k), egui::pos2(c.x - k, c.y + k)],
                stroke,
            );
        }
    }
}

/// A failed navigation, as the error page every browser has. Returns the button pressed.
///
/// Drawn in the reading column's measure, because that is where a reader's eye already is; the
/// glyph is large and muted (`guide`) so the page reads as a state and not as an article, the
/// heading is `notice_heading_font` (the one non-mono line the reader owns, see there), the
/// explanation is prose in the page's body face, and the code line under it is mono and dim: it
/// is for the bug report, and it names what went wrong in the words the code uses.
pub fn error_page(
    ui: &mut Ui,
    pal: &Palette,
    opts: &ReadOpts,
    column: Column,
    notice: &Notice,
) -> Option<Button> {
    let ink = theme::notice_ink(pal, notice.severity);
    let mut pressed = None;
    ui.horizontal_top(|ui| {
        ui.add_space(column.gutter);
        ui.vertical(|ui| {
            ui.set_max_width(column.measure);
            ui.set_min_width(column.measure);
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            ui.add_space(theme::snap(opts.base_size_pt * 5.0));
            let d = theme::snap(opts.base_size_pt * 2.6);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(d, d), egui::Sense::hover());
            icon(ui.painter(), rect, notice.severity, ink.icon);
            ui.add_space(theme::snap(opts.base_size_pt * 0.6));
            if let Some(h) = notice.heading {
                ui.label(
                    RichText::new(h.trim_end_matches('.'))
                        .font(theme::notice_heading_font(opts))
                        .color(ink.heading),
                );
            }
            ui.add_space(theme::snap(opts.base_size_pt * 0.3));
            ui.label(
                RichText::new(notice.body.as_str())
                    .font(theme::body_font(opts))
                    .color(ink.body),
            );
            ui.add_space(theme::snap(opts.base_size_pt * 0.4));
            ui.style_mut().override_font_id = Some(theme::chrome_font(opts));
            let mut details = notice.detail.iter();
            let first = details.next().map(String::as_str).unwrap_or("");
            let code = match notice.code {
                Some(c) => format!("{c} · {first}"),
                None => first.to_owned(),
            };
            ui.label(RichText::new(code).color(ink.aside));
            for d in details {
                ui.label(RichText::new(d.as_str()).color(ink.aside));
            }
            ui.add_space(theme::snap(opts.base_size_pt * 1.2));
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.spacing_mut().button_padding = egui::vec2(12.0, 5.0);
                for (i, b) in notice.actions.iter().enumerate() {
                    // The first action is the one the page most wants pressed, and it gets the
                    // one fill a control here has; the rest keep their border alone.
                    let button = if i == 0 {
                        egui::Button::new(b.label()).fill(pal.code_bg)
                    } else {
                        egui::Button::new(b.label())
                    };
                    if ui.add(button).clicked() {
                        pressed = Some(*b);
                    }
                }
            });
        });
    });
    pressed
}

/// The waveform from `assets/logo/hww.svg`, painted rather than textured.
///
/// Two polylines on a 64 grid: the decaying wave, then the rust tail that runs under its last
/// diagonal so the butt caps do not leave a wedge at the join. A PNG would freeze the Dark
/// pairing, and SVG is refused in `image_decode` for the reason given there; paint takes
/// `fg` and `notice_fg`, which *are* those two colours on Dark and invert with the page on
/// Light. No ground: the mark sits on the page the way the wordmark does, and `hww-mark.svg`
/// is the same decision in the file that drew it.
fn logo(painter: &egui::Painter, rect: egui::Rect, pal: &Palette) {
    const GRID: f32 = 64.0;
    // `assets/logo/hww.svg`, the master. Do not scale a raster down to invent vertices.
    const WAVE: [(i8, i8); 9] = [
        (0, 18),
        (7, 50),
        (14, 14),
        (21, 50),
        (28, 18),
        (35, 45),
        (42, 27),
        (48, 37),
        (54, 32),
    ];
    const TAIL: [(i8, i8); 3] = [(48, 37), (54, 32), (64, 32)];
    let w = rect.width() * 4.0 / GRID;
    let inner = rect.shrink(w * 0.5);
    let pt = |x: i8, y: i8| {
        egui::pos2(
            inner.left() + inner.width() * (x as f32) / GRID,
            inner.top() + inner.height() * (y as f32) / GRID,
        )
    };
    painter.add(egui::Shape::line(
        TAIL.iter().map(|&(x, y)| pt(x, y)).collect(),
        egui::Stroke::new(w, pal.notice_fg),
    ));
    painter.add(egui::Shape::line(
        WAVE.iter().map(|&(x, y)| pt(x, y)).collect(),
        egui::Stroke::new(w, pal.fg),
    ));
}

/// The idle screen: waveform, wordmark, tagline, and the two keys that matter, as keycaps.
pub fn splash(ui: &mut Ui, pal: &Palette, opts: &ReadOpts, column: Column, s: &notice::Splash) {
    ui.horizontal_top(|ui| {
        ui.add_space(column.gutter);
        ui.vertical(|ui| {
            ui.set_max_width(column.measure);
            ui.set_min_width(column.measure);
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            ui.add_space(theme::snap(opts.base_size_pt * 5.0));
            // Mark and wordmark on one line, laid out by hand rather than by a horizontal
            // strip: the mark's ink fills only the middle 36 of its 64 grid, so a row that
            // centres two boxes hangs it above the letters. Measure the word, allocate one
            // rect for both, and centre each on the same axis, which is what makes the wave
            // read as the same height as `hww` instead of floating over it.
            let wordmark = ui.painter().layout_no_wrap(
                s.wordmark.to_owned(),
                theme::wordmark_font(opts),
                pal.fg,
            );
            let d = theme::snap(opts.base_size_pt * 4.4);
            let gap = theme::snap(opts.base_size_pt * 0.8);
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(
                    d + gap + wordmark.rect.width(),
                    d.max(wordmark.rect.height()),
                ),
                egui::Sense::hover(),
            );
            if ui.is_rect_visible(rect) {
                let mark = egui::Rect::from_center_size(
                    egui::pos2(rect.left() + d * 0.5, rect.center().y),
                    egui::vec2(d, d),
                );
                logo(ui.painter(), mark, pal);
                ui.painter().galley(
                    egui::pos2(
                        rect.left() + d + gap,
                        rect.center().y - wordmark.rect.height() * 0.5,
                    ),
                    wordmark,
                    pal.fg,
                );
            }
            ui.add_space(theme::snap(opts.base_size_pt * 0.3));
            ui.label(
                RichText::new(s.tagline)
                    .font(theme::body_font(opts))
                    .color(pal.dim),
            );
            ui.add_space(theme::snap(opts.base_size_pt * 1.2));
            ui.style_mut().override_font_id = Some(theme::chrome_font(opts));
            egui::Grid::new("hww-splash-keys")
                .num_columns(2)
                .spacing([
                    theme::snap(opts.base_size_pt),
                    theme::snap(opts.base_size_pt * 0.5),
                ])
                .show(ui, |ui| {
                    for (keys, what) in s.keys {
                        ui.horizontal(|ui| keycaps(ui, pal, opts, keys));
                        ui.label(RichText::new(*what).color(pal.dim));
                        ui.end_row();
                    }
                });
        });
    });
}

/// One dim line in the middle of an empty page, for a navigation with nothing under it.
pub fn loading_line(ui: &mut Ui, pal: &Palette, opts: &ReadOpts, text: &str) {
    let size = ui.available_size();
    ui.allocate_ui_with_layout(
        size,
        egui::Layout::centered_and_justified(egui::Direction::TopDown),
        |ui| {
            ui.label(
                RichText::new(text)
                    .font(theme::chrome_font(opts))
                    .color(pal.dim),
            );
        },
    );
}

/// A transient status, as a pill floating above the status strip, centred.
///
/// `lift` is how far above the window's bottom edge it sits: the strip's height, plus the hover
/// URL overlay's measured height when one is showing, so a wrapping address cannot cover the
/// pill. An `Area` and not a panel, for
/// the reason `hover_strip` gives: a panel takes its height out of the reading column and the
/// page would jump every time a status appeared. `interactable(false)`, so it takes no hover or
/// click from the page under it. It is the one chrome surface that borrows nothing from the
/// page and nothing from the other chrome: `toast_bg` / `toast_fg`, the palette's inversion.
pub fn toast(ctx: &egui::Context, pal: &Palette, opts: &ReadOpts, text: &str, lift: f32) {
    let width = (ctx.content_rect().width() - 48.0).max(120.0);
    egui::Area::new(egui::Id::new("hww-toast"))
        .order(egui::Order::Foreground)
        .interactable(false)
        .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -(lift + 14.0)))
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(pal.toast_bg)
                .corner_radius(egui::CornerRadius::same(theme::RADIUS))
                .inner_margin(egui::Margin::symmetric(14, 8))
                .show(ui, |ui| {
                    ui.set_max_width(width);
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                    ui.label(
                        RichText::new(text)
                            .font(theme::chrome_font(opts))
                            .color(pal.toast_fg),
                    );
                });
        });
}

/// A key, drawn as a keycap: a bordered, rounded box on the page ground, in the chrome font.
///
/// The reader's one convention for "press this", shared by the help card and the splash. The
/// border is `guide`, because with no fill the edge is the whole affordance, which is `guide`'s
/// definition; the ground is `bg` so a cap reads as lifted off the `chrome_bg` card it sits on.
pub fn keycap(ui: &mut Ui, pal: &Palette, opts: &ReadOpts, key: &str) {
    egui::Frame::new()
        .fill(pal.bg)
        .stroke(egui::Stroke::new(1.0, pal.guide))
        .corner_radius(egui::CornerRadius::same(theme::RADIUS))
        .inner_margin(egui::Margin::symmetric(5, 1))
        .show(ui, |ui| {
            ui.label(
                RichText::new(key)
                    .font(theme::chrome_font(opts))
                    .color(pal.strong),
            );
        });
}

/// A key spec from `HELP` or the splash, as caps and separators: `Ctrl+L or o` is two caps
/// joined by `+`, the word `or` dim, and a third cap. Whitespace separates alternatives; `/`,
/// `or`, and `then` are separators when they are not the first token, and `/` *is* the key
/// when it is (`/ then n / N`).
pub fn keycaps(ui: &mut Ui, pal: &Palette, opts: &ReadOpts, spec: &str) {
    ui.spacing_mut().item_spacing.x = 4.0;
    let dim = |ui: &mut Ui, t: &str| {
        ui.label(
            RichText::new(t)
                .font(theme::chrome_font(opts))
                .color(pal.dim),
        );
    };
    for (i, token) in spec.split_whitespace().enumerate() {
        let separator = matches!(token, "/" | "or" | "then" | "·") && i > 0;
        if separator {
            dim(ui, token);
            continue;
        }
        // `Ctrl+L` is two caps and a plus; a bare `+` would be a key and is not in the table.
        let mut parts = token.split('+').filter(|p| !p.is_empty()).peekable();
        while let Some(part) = parts.next() {
            keycap(ui, pal, opts, part);
            if parts.peek().is_some() {
                dim(ui, "+");
            }
        }
    }
}
