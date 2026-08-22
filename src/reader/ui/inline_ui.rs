//! `Vec<Run>` -> wrapped, selectable, and clickable text.
//!
//! # The approach
//!
//! One `Label` per maximal non-link stretch, `LayoutJob` sections for emphasis inside it, one
//! `Label` per link, all in `ui.horizontal_wrapped` with `item_spacing.x = 0.0`.
//!
//! The two standard objections to splitting a paragraph across several labels are both false
//! in modern egui, and both were checked against 0.36.1's source rather than assumed:
//!
//! - *Wrapping quality.* `Label::layout_in_ui` has an explicit branch for wrapped
//!   left-to-right layouts: it computes `first_row_indentation`, sets it as the first section's
//!   `leading_space`, and merges one rect per row into a single `Response`. A label that starts
//!   mid-line continues from that point and wraps at full width, exactly what flowed text
//!   needs. (The branch is skipped for `WidgetText::Galley`, which is why nothing here hands
//!   egui a pre-laid galley.)
//! - *Selection across the paragraph.* `LabelSelectionState` is a `Context` plugin whose
//!   cursors carry `widget_id`; selection is *designed* to span labels, with drag, keyboard,
//!   and Ctrl+C. Splitting a paragraph into five labels costs nothing here.
//!
//! What it buys is that every link is a real `Response`: hover cursor, hover-href in the
//! status strip, `clicked()` and `secondary_clicked()` for free, a stable `Id` to focus for
//! Tab-cycling, and correct hit regions on a link that wraps across rows.
//!
//! The one genuine advantage of a single galley is guaranteed identical baselines, and the
//! mitigation is one line: every section of every segment carries the same
//! [`theme::line_height_px`] and the same `valign`.
//!
//! The other thing a single galley would buy is full justification, which this cannot do, so
//! there is deliberately no `justify` field in `ReadOpts`. If justification is ever wanted
//! badly enough it arrives together with the galley renderer, and the two are one decision.

use crate::ir;
use crate::reader::face::Face;
use crate::reader::inline::{Run, Style};
use crate::reader::notice;
use crate::reader::opts::{FontChoice, ReadOpts};
use crate::reader::ui::theme::Palette;
use crate::reader::ui::{Action, RenderCtx, fonts, theme};
use eframe::egui::{
    self, Align, Color32, FontId, Label, RichText, Sense, Stroke, TextFormat, TextWrapMode, Ui,
    text::LayoutJob, text_selection::LabelSelectionState,
};

/// How one run list should be set. Headings, captions, and body prose differ only here.
#[derive(Clone)]
pub struct Setting {
    pub font: FontId,
    pub color: Color32,
    /// The role the runs are set in, so `Strong` and `Emph` can resolve to real faces. Carried
    /// separately from `font` because a `FontId` names one family and an inline style needs the
    /// other three of that role's four.
    pub role: FontChoice,
}

impl Setting {
    pub fn body(opts: &ReadOpts, pal: &Palette) -> Self {
        Self {
            font: theme::body_font(opts),
            color: pal.fg,
            role: opts.family,
        }
    }

    /// Chrome, so monospace regardless of what the page is set in.
    pub fn dim(opts: &ReadOpts, pal: &Palette) -> Self {
        Self {
            font: theme::chrome_font(opts),
            color: pal.dim,
            role: FontChoice::Mono,
        }
    }

    pub fn heading(opts: &ReadOpts, pal: &Palette, level: u8) -> Self {
        Self {
            font: theme::heading_font(opts, level),
            color: pal.fg,
            role: opts.family,
        }
    }
}

/// Lay out one run list. Actions and hover/focus state land on `ctx`.
pub fn runs_ui(ui: &mut Ui, runs: &[Run], set: &Setting, ctx: &mut RenderCtx<'_>) {
    ui.horizontal_wrapped(|ui| {
        // Word spacing comes from the text itself; `flatten` preserves boundary whitespace
        // for exactly this reason. Any horizontal item spacing here would double it.
        ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
        ui.style_mut().wrap_mode = Some(TextWrapMode::Wrap);

        let mut job = new_job();
        // Byte offset of the next run within this list's plain text, for find-in-page.
        let mut offset = 0usize;
        for run in runs {
            match run {
                Run::Text { text, style } => {
                    append(&mut job, text, *style, set, ctx, offset);
                    offset += text.len();
                }
                Run::Break => {
                    append(&mut job, "\n", Style::default(), set, ctx, offset);
                    offset += 1;
                }
                Run::Link { href, runs } => {
                    flush(ui, &mut job, ctx);
                    link_ui(ui, href, runs, set, ctx, &mut offset);
                }
                Run::Image(img) => {
                    flush(ui, &mut job, ctx);
                    super::images::inline_placeholder(ui, img, ctx);
                    offset += img.alt.as_deref().map_or(0, str::len);
                }
            }
        }
        flush(ui, &mut job, ctx);
    });
}

/// Multiple of the font size at which an underline sits. Just past the descender.
///
/// Both things this file underlines want it: a link, and a find match that is not the current
/// one. See [`link_ui`] for what epaint does without it.
const UNDERLINE_LINE_HEIGHT: f32 = 1.22;

fn new_job() -> LayoutJob {
    LayoutJob {
        // `Label` overwrites this with the available width on the wrapping path; leaving it at
        // zero would make an unwrapped job wrap after every glyph if that path is ever missed.
        wrap: egui::text::TextWrapping {
            max_width: f32::INFINITY,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Emit the accumulated sections as one `Label`, if there are any.
fn flush(ui: &mut Ui, job: &mut LayoutJob, ctx: &mut RenderCtx<'_>) {
    if job.is_empty() {
        return;
    }
    let job = std::mem::replace(job, new_job());
    let carries_current_match = ctx.job_has_current_match && ctx.find_scroll;
    ctx.job_has_current_match = false;
    let resp = ui.add(Label::new(job).wrap());
    if carries_current_match {
        // A match lives inside a galley section, not in a widget of its own, so the honest
        // granularity for `n`/`N` is the label that contains it. Splitting the label at the
        // match instead would change where the line wraps, which is worse than a scroll that
        // lands a few words early.
        resp.scroll_to_me(Some(Align::Center));
    }
}

fn append(
    job: &mut LayoutJob,
    text: &str,
    style: Style,
    set: &Setting,
    ctx: &mut RenderCtx<'_>,
    offset: usize,
) {
    let (pal, line_height) = (ctx.pal, ctx.line_height);
    let base = format_for(style, set, &pal, line_height);
    // Find highlighting splits a run into background-carrying pieces; with no query, this is
    // one piece and one section.
    for (piece, hit) in ctx.split_by_matches(text, offset) {
        let mut fmt = base.clone();
        if let Some(is_current) = hit {
            if is_current {
                fmt.background = ctx.pal.find_current_bg;
                fmt.color = ctx.pal.bg;
                ctx.job_has_current_match = true;
            } else {
                // An underline rather than a wash. The wash could not carry `dim` at AA on any
                // theme (4.25 / 3.78 / 3.74), and lowering its alpha far enough to clear that
                // leaves a tint too faint to read as a hit at all. An underline changes no ink,
                // so every run keeps the contrast it already had on the page, and it survives
                // greyscale, which a second background never did.
                //
                // The weight comes from the run's own size, not from `base_size_pt`: a code run
                // is at 0.92x and a level-1 heading at 1.65x, and one fixed weight looks heavy
                // on the first and thin on the second. `background` is deliberately left alone,
                // so a match inside a code run keeps `code_bg` and takes the underline on top.
                fmt.underline = Stroke::new((fmt.font_id.size * 0.09).max(1.0), ctx.pal.notice_fg);
                // And the tighter line height that `link_ui` explains: epaint puts an underline
                // at the bottom of the section's *logical* rect, so at a reading line height of
                // 1.55 the rule lands most of a line below the match, next to the following
                // row, where it marks the wrong words. `valign: TOP` in `format_for` is what
                // keeps this from moving the text with it.
                fmt.line_height = Some(fmt.font_id.size * UNDERLINE_LINE_HEIGHT);
            }
        }
        job.append(piece, 0.0, fmt);
    }
}

fn format_for(style: Style, set: &Setting, pal: &Palette, line_height: f32) -> TextFormat {
    // Code is monospace whatever the page is set in, and smaller for the reason on
    // `theme::CODE_SCALE`.
    let (role, size) = if style.code {
        (FontChoice::Mono, set.font.size * theme::CODE_SCALE)
    } else {
        (set.role, set.font.size)
    };
    let face = Face::new(role, style.strong, style.emph);
    TextFormat {
        font_id: FontId::new(size, fonts::family(face)),
        color: set.color,
        background: if style.code {
            pal.code_bg
        } else {
            Color32::TRANSPARENT
        },
        // A real italic face is shipped for every role but mono, so the skew is asked for in
        // exactly the one case where no face exists. Both at once would slant it twice.
        italics: face.synthesises_italics(),
        // Shared by every section of every segment: this is what keeps the several labels of
        // one paragraph on one baseline. A segment that forgets it sits a pixel off its
        // neighbours, and the eye reads that as a broken line rather than as a subtle bug.
        line_height: Some(line_height),
        // TOP, everywhere, and the choice is load-bearing. epaint positions a glyph at
        // `font_ascent + valign_factor * (row_height - section_line_height)`. With TOP the
        // factor is zero, so every section sits on the same baseline *no matter what line
        // height it carries*, which is what lets the link sections below run a tighter line
        // height purely to move their underline, without moving the words.
        valign: Align::TOP,
        ..Default::default()
    }
}

fn link_ui(
    ui: &mut Ui,
    href: &str,
    runs: &[Run],
    set: &Setting,
    ctx: &mut RenderCtx<'_>,
    offset: &mut usize,
) {
    let mut job = new_job();
    let underline = Stroke::new(1.0, ctx.pal.link);
    // epaint draws an underline at the bottom of the glyph's *logical* rect, whose height is
    // the section's line height, so at a reading line height of 1.55 the rule lands most of a
    // line below the words, close enough to the next row to read as a stray horizontal line.
    // A tighter line height on the link's own sections puts it back under the text, and
    // `valign: TOP` above is what keeps that from also moving the text.
    let underline_line_height = set.font.size * UNDERLINE_LINE_HEIGHT;
    for run in runs {
        let (text, style) = match run {
            Run::Text { text, style } => (text.as_str(), *style),
            Run::Break => ("\n", Style::default()),
            // A link's own runs are already flattened; an image inside one contributes its alt.
            Run::Image(img) => (img.alt.as_deref().unwrap_or(""), Style::default()),
            Run::Link { .. } => continue,
        };
        let mut link_set = set.clone();
        link_set.color = ctx.pal.link;
        let start = job.sections.len();
        append(&mut job, text, style, &link_set, ctx, *offset);
        // Underline is per-section within one galley, so it continues correctly across a
        // wrapped row rather than stopping at the row break.
        for section in &mut job.sections[start..] {
            section.format.underline = underline;
            section.format.line_height = Some(underline_line_height);
        }
        *offset += text.len();
    }
    if ctx.opts.show_link_urls {
        let mut fmt = format_for(Style::default(), set, &ctx.pal, ctx.line_height);
        fmt.color = ctx.pal.dim;
        fmt.font_id.size *= 0.85;
        job.append(&format!(" <{href}>"), 0.0, fmt);
    }
    if job.is_empty() {
        return;
    }

    let carries_current_match = ctx.job_has_current_match && ctx.find_scroll;
    ctx.job_has_current_match = false;

    // `layout_in_ui` and not `ui.add`, because the hover rule below needs the galley and a
    // `Response` cannot give one up. A wrapped `Label` allocates one rect per row and unions
    // the responses, so `resp.rect` for a link that wraps is a *bounding box*: its bottom edge
    // runs the full width of the widest row, out past the link's own words and under whatever
    // text follows them on the last row. Everything after this call is what `Label::ui` does,
    // less the elision tooltip (nothing here truncates) and the style-driven text colour (every
    // section sets its own, so the fallback is never reached).
    let (galley_pos, galley, resp) = Label::new(job)
        .wrap()
        .sense(Sense::click())
        .layout_in_ui(ui);
    // A link, not a label: this is the whole of what a screen reader is told about it.
    resp.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Link, ui.is_enabled(), galley.text())
    });
    if ui.is_rect_visible(resp.rect) {
        // `Stroke::NONE`, because egui's own focus underline is not the mark this reader uses:
        // the focus ring below is.
        if ui.style().interaction.selectable_labels {
            LabelSelectionState::label_text_selection(
                ui,
                &resp,
                galley_pos,
                galley.clone(),
                ctx.pal.link,
                Stroke::NONE,
            );
        } else {
            ui.painter().add(egui::epaint::TextShape::new(
                galley_pos,
                galley.clone(),
                ctx.pal.link,
            ));
        }
    }

    // `Sense::click()` alone does not put a widget in egui's tab order; focusability is a
    // separate property. Only links ask for it; if plain text did, Tab would walk every label
    // on the page.
    ui.memory_mut(|m| m.interested_in_focus(resp.id, ui.layer_id()));

    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        ctx.hover_href = Some(href.to_owned());
        // A second, heavier underline on hover: the link is already underlined, so the change
        // has to be in weight rather than in presence. Per row, and bounded by the row's own
        // glyphs, which is the one thing `resp.rect` cannot say: a link that wraps ends its
        // last row after a word or two, and a rule drawn to the bounding box underlines the
        // rest of that line as well.
        for row in &galley.rows {
            let r = row
                .rect_without_leading_space()
                .translate(galley_pos.to_vec2());
            ui.painter().line_segment(
                [r.left_bottom(), r.right_bottom()],
                Stroke::new(1.5, ctx.pal.link),
            );
        }
    }
    if resp.has_focus() {
        ctx.focus_href = Some(href.to_owned());
        ui.painter().rect_stroke(
            resp.rect.expand(2.0),
            2.0,
            Stroke::new(1.0, ctx.pal.link),
            egui::StrokeKind::Outside,
        );
    }
    if resp.clicked() {
        ctx.act(Action::Follow(href.to_owned()));
        // Which link, so the mark below lands on the one that was clicked rather than on every
        // inline that happens to share its href. `app` reads it back once the action is applied.
        ctx.clicked_link = Some(resp.id);
    }
    // The page this link points at is being fetched. A bracket, in the page's own flow, because
    // the reading column admits the reader's words only as a positional mark carrying that
    // convention: `[image]` two modules over is the same shape, and it is set the same way.
    // Not a colour or a second underline: the hover rule is already painted at this exact
    // baseline, and the pointer is still on the link in the moment this matters most.
    if ctx.pending == Some(resp.id) {
        // The leading space is not cosmetic. `runs_ui` sets `item_spacing.x = 0.0`, because word
        // spacing on a wrapped line comes from the text itself, so a bare label here renders as
        // `the quiet web[loading]`. The `[image]` placeholder gets away with no space only
        // because it is an `egui::Button` and carries `button_padding`.
        ui.label(
            RichText::new(format!(" {}", notice::PENDING))
                .color(ctx.pal.dim)
                .font(theme::chrome_font(ctx.opts)),
        );
    }
    // Two items, both of which already exist as keyboard actions. A second route to them, not
    // new capability.
    resp.context_menu(|ui| {
        if ui.button("Copy link").clicked() {
            ctx.act(Action::Copy(href.to_owned()));
            ui.close();
        }
        if ui.button("Open without rewrite").clicked() {
            ctx.act(Action::FollowBare(href.to_owned()));
            ui.close();
        }
    });
    if carries_current_match {
        resp.scroll_to_me(Some(Align::Center));
    }
}

/// A one-line strip standing in for an image, used by figures and inline images alike.
///
/// Deliberately compact: no dimensions live in `ir::Image` (the extractor does not capture
/// them and adding them is contract creep in the direction the charter guards), so a tall
/// placeholder would reserve the wrong box and shift the page when it loaded. A single line
/// shifts by almost nothing, and once an image *has* been decoded its dimensions are cached
/// per `src`, so a revisit reserves the right box for free.
pub fn image_host(img: &ir::Image, base: &url::Url) -> String {
    base.join(&img.src)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_else(|| "an unknown host".to_owned())
}
