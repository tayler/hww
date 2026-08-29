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
//! status strip, `clicked()` and `secondary_clicked()` for free, and correct hit regions on a
//! link that wraps across rows.
//!
//! The price is paid in [`link_ui`], and it is the row rects. One rect per row is also one
//! *widget id* per row, and `Sense::click()` in egui 0.36 is `CLICK | FOCUSABLE`, so a link
//! that wrapped used to be one tab stop per row: only the first carried the href, and the
//! others were blank stops that painted no ring and named nothing to a screen reader. The rows
//! now sense clicks without being focusable and a widget of hww's own, over all of them,
//! carries the focus. Keyboard reach is a property of the link, not of how the column happened
//! to break it.
//!
//! The one genuine advantage of a single galley is guaranteed identical baselines, and the
//! mitigation is one line: every section of every segment carries the same
//! [`theme::line_height_for`] (the body's row, or the heading's own when the runs are a
//! heading) and the same `valign`.
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
    /// Whether a link in these runs is underlined. True for prose, where the underline is the
    /// one cue that says "link" to an eye that does not see the hue (WCAG 1.4.1: the link
    /// colour is 2.1 / 1.4 from `fg`, short of the 3:1 a colour-only link needs); false for a
    /// story card's headline, where nothing non-link surrounds it and the card is the
    /// affordance, which is how every front page sets them.
    pub underline: bool,
}

impl Setting {
    pub fn body(opts: &ReadOpts, pal: &Palette) -> Self {
        Self {
            font: theme::body_font(opts),
            color: pal.fg,
            role: opts.family,
            underline: true,
        }
    }

    /// Chrome, so monospace regardless of what the page is set in.
    pub fn dim(opts: &ReadOpts, pal: &Palette) -> Self {
        Self {
            font: theme::chrome_font(opts),
            color: pal.dim,
            role: FontChoice::Mono,
            underline: true,
        }
    }

    /// A heading at `level`, in the role `theme::heading_role` gives it: the serif over a sans
    /// page for levels 1–3 (and the title), the body family past that.
    pub fn heading(opts: &ReadOpts, pal: &Palette, level: u8) -> Self {
        Self {
            font: theme::heading_font(opts, level),
            color: pal.fg,
            role: theme::heading_role(opts, level),
            underline: true,
        }
    }

    /// A story card's headline: the smallest heading, linked without an underline.
    pub fn headline(opts: &ReadOpts, pal: &Palette) -> Self {
        Self {
            underline: false,
            ..Self::heading(opts, pal, 4)
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
        // granularity for stepping matches is the label that contains it. Splitting the label at the
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
    let pal = ctx.pal;
    let base = format_for(style, set, &pal, ctx.opts);
    // Find highlighting splits a run into background-carrying pieces; with no query, this is
    // one piece and one section — and asking for it as a `Vec` was an allocation per text run
    // per frame for the pages nobody is searching, which is all of them almost all of the time.
    if !ctx.has_matches() {
        job.append(text, 0.0, base);
        return;
    }
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
                // is at 0.92x and a level-1 heading at 1.9x, and one fixed weight looks heavy
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

fn format_for(style: Style, set: &Setting, pal: &Palette, opts: &ReadOpts) -> TextFormat {
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
        // Sized from the runs' face, not from the body: a title at 1.9× used to wrap at the
        // body's pixels, which is 0.82 of its own size.
        line_height: Some(theme::line_height_for(opts, set.font.size)),
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
    // One *physical* pixel in the underline tint, or nothing: a hairline in `link_underline`
    // is the quiet underline long-form sites set (`theme::Palette::link_underline`), and a
    // logical point on a 2x display is two device pixels, which reads heavy.
    let underline = if set.underline {
        Stroke::new(
            (1.0 / ui.ctx().pixels_per_point()).max(0.5),
            ctx.pal.link_underline,
        )
    } else {
        Stroke::NONE
    };
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
        let mut fmt = format_for(Style::default(), set, &ctx.pal, ctx.opts);
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
    // section sets its own, so the fallback is never reached). `Sense::CLICK` rather than
    // `Sense::click()`, which is `CLICK | FOCUSABLE`: keyboard focus is the next widget's job.
    let (galley_pos, galley, resp) = Label::new(job).wrap().sense(Sense::CLICK).layout_in_ui(ui);
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

    // One tab stop for the link, over all of it, and not one of the rects above.
    //
    // The wrapped branch of `layout_in_ui` allocates a rect, and therefore a widget id, per
    // *row*, and `Sense::click()` is `CLICK | FOCUSABLE`. So every row of a headline that wraps
    // was its own tab stop, and `resp.id` is the first row's, which left the rest carrying no
    // href, painting no ring, and naming nothing to a screen reader. Registering `resp.id`
    // again after them closed the pair into a loop: Tab left the first row, egui gave focus to
    // the second, and the re-registration handed it straight back. A page whose headlines wrap,
    // which is what a bookmarks list, a history, and every front page are, trapped Tab on its first
    // link.
    //
    // So the rows sense clicks without being focusable, and one focusable, non-interactive
    // widget over the union of them carries the focus, the ring, and the label a screen reader
    // reads. Non-interactive because `hit_test` returns a click or a drag only to a widget that
    // senses one, so this cannot take the click off the rows underneath it.
    let focus = ui.interact(
        resp.rect,
        resp.id.with("focus"),
        Sense::focusable_noninteractive(),
    );
    // A link, not a label: this is the whole of what a screen reader is told about it, and it
    // belongs on the widget that takes the focus.
    focus.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Link, ui.is_enabled(), galley.text())
    });
    super::follow_focus(&focus);

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
    // The ring is `theme::focus_ring`, which the chrome's frameless controls draw as well:
    // one mark, learned once, wherever Tab can land. `focus` and not `resp`, and they share a
    // rect, so what is ringed is still the union of the link's rows.
    theme::focus_ring(ui, &focus, &ctx.pal);
    if focus.has_focus() {
        ctx.focus_href = Some(href.to_owned());
        // Beside the href, because the two are one fact about the focused link and a reading
        // list built from the href alone is a column of addresses.
        ctx.focus_text = Some(galley.text().to_owned());
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
    if ctx.pending == Some(resp.id) || ctx.pending_href.as_deref() == Some(href) {
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
    // Three items, all of which already exist as keyboard actions. A second route to them, not
    // new capability.
    //
    // The third one is why the rule is worth restating rather than just obeyed. `ui::app`'s
    // charter is that **no affordance is reachable by keyboard alone**, and the reading list is
    // the first feature in hww whose subject is a *link* rather than the page on screen — a
    // mouse-only reader cannot Tab to one, so without this row they could open the list, follow
    // things out of it, and never once put anything in it.
    resp.context_menu(|ui| {
        if ui.button("Copy link").clicked() {
            ctx.act(Action::Copy(href.to_owned()));
            ui.close();
        }
        if ui.button("Read later").clicked() {
            ctx.act(Action::ReadLater {
                href: href.to_owned(),
                title: galley.text().to_owned(),
            });
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::opts::{ReadOpts, Theme};
    use crate::reader::ui::{RenderCtx, fonts, images::ImageStore};
    use std::collections::HashSet;

    /// One link, set as a headline in a narrow column so it wraps.
    fn wrapping_links(n: usize) -> Vec<Run> {
        (0..n)
            .flat_map(|i| {
                [
                    Run::Link {
                        href: format!("https://example.com/{i}"),
                        runs: vec![Run::Text {
                            text: format!(
                                "Headline {i}, long enough that it has to wrap across more \
                                 than one row of the reading column"
                            ),
                            style: Style::default(),
                        }],
                    },
                    Run::Text {
                        text: " some prose between the links ".to_owned(),
                        style: Style::default(),
                    },
                ]
            })
            .collect()
    }

    /// Where Tab is after each press, named by the link it landed on.
    ///
    /// This test needs an `egui::Context` and no display: it runs the real [`runs_ui`] through
    /// `Context::run_ui` and reads back what the frame reported. It belongs under `ui/` because
    /// the thing it pins is egui's, and its failure is invisible to everything else — the tab
    /// order is not a value any function returns, `hww-shot` photographs one frame and cannot
    /// say a key moved anything, and the bug it was written for looked, in a screenshot, like a
    /// perfectly ordinary focus ring.
    fn tab_stops(runs: &[Run], presses: usize) -> Vec<Option<String>> {
        let ctx = egui::Context::default();
        fonts::install(&ctx);
        let opts = ReadOpts::default();
        let collapsed = HashSet::new();
        let base = url::Url::parse("https://example.com/").unwrap();
        let pal = crate::reader::ui::theme::palette(Theme::Light, false);
        let mut landed = Vec::new();
        // Two passes per press, and the answer is taken from the second. egui moves focus
        // among the widgets of the frame the key arrived in, but a Tab from the *last*
        // focusable widget finds nothing to give focus to and carries the request into the
        // next frame, where it wraps to the first. That second frame is why `ReaderApp` holds
        // its whole-page layout for two.
        for step in 0..presses * 2 {
            let tab = step % 2 == 0;
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::pos2(0.0, 0.0),
                    egui::vec2(400.0, 600.0),
                )),
                ..Default::default()
            };
            if tab {
                for pressed in [true, false] {
                    input.events.push(egui::Event::Key {
                        key: egui::Key::Tab,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
            }
            let mut images = ImageStore::default();
            let threads = std::collections::HashMap::new();
            let mut rctx =
                RenderCtx::new(pal, &opts, base.clone(), &mut images, &collapsed, &threads);
            let mut out = ctx.run_ui(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        let set = Setting::body(&opts, &pal);
                        runs_ui(ui, runs, &set, &mut rctx);
                    });
                });
            });
            // The test never uploads them, and epaint panics on a delta dropped unhandled.
            out.textures_delta.clear();
            if !tab {
                landed.push(rctx.focus_href.clone());
            }
        }
        landed
    }

    /// A link that wraps is one tab stop, not one per row.
    ///
    /// `Label::layout_in_ui` allocates a rect, and therefore a widget id, for every row a
    /// wrapped label takes, and `Sense::click()` is `CLICK | FOCUSABLE`. Every row of a
    /// headline used to be its own tab stop, and re-registering the first row after the last
    /// closed the pair into a loop: Tab moved from row one to row two and straight back, and a
    /// page of wrapped headlines — a bookmarks list, a history, any front page — trapped the reader on
    /// its first link.
    #[test]
    fn tab_visits_each_link_once_and_in_order() {
        let runs = wrapping_links(4);
        let visited = tab_stops(&runs, 4);
        let expected: Vec<Option<String>> = (0..4)
            .map(|i| Some(format!("https://example.com/{i}")))
            .collect();
        assert_eq!(visited, expected);
    }

    /// The prose between the links is not a tab stop. A `Label` that senses only hover is not
    /// focusable, which is what keeps Tab to the things it can act on.
    #[test]
    fn tab_skips_plain_text() {
        let runs = vec![
            Run::Text {
                text: "a paragraph of prose before the link ".to_owned(),
                style: Style::default(),
            },
            Run::Link {
                href: "https://example.com/only".to_owned(),
                runs: vec![Run::Text {
                    text: "the only link".to_owned(),
                    style: Style::default(),
                }],
            },
            Run::Text {
                text: " and a paragraph of prose after it".to_owned(),
                style: Style::default(),
            },
        ];
        assert_eq!(
            tab_stops(&runs, 2),
            vec![
                Some("https://example.com/only".to_owned()),
                Some("https://example.com/only".to_owned()),
            ],
            "one focusable widget on the page: Tab leaves it and wraps back to it"
        );
    }
}
