//! `ir::Block` -> egui widgets.
//!
//! One match arm per variant, and the match stays exhaustive on purpose: a variant that
//! renders as nothing is a page the reader silently lies about.
//!
//! # Blocks that do not fit the measure
//!
//! `Table` and `Code` routinely exceed 66–78 characters, and a single reading column has
//! nowhere to put the overflow. Each gets its **own horizontal `ScrollArea`**, sized to the
//! measure: the column stays honest, wide content stays reachable, and the page never scrolls
//! sideways as a whole. Code additionally keeps its monospace metric rather than being
//! re-wrapped: a wrapped code line is a changed code line.

use crate::ir;
use crate::reader::face::Face;
use crate::reader::inline::{self, Run};
use crate::reader::opts::FontChoice;
use crate::reader::ui::inline_ui::{self, Setting};
use crate::reader::ui::{Action, RenderCtx, fonts, theme};
use eframe::egui::{self, RichText, Ui};

/// Render a block list. `first_block` is the index of `blocks[0]` in the document, so outline
/// jumps and fragment scrolls can name a target; nested lists pass `None`.
pub fn blocks_ui(
    ui: &mut Ui,
    blocks: &[ir::Block],
    ctx: &mut RenderCtx<'_>,
    scroll_to: Option<usize>,
) {
    for (i, b) in blocks.iter().enumerate() {
        let resp = ui.scope(|ui| block_ui(ui, b, ctx)).response;
        if scroll_to == Some(i) {
            resp.scroll_to_me(Some(egui::Align::TOP));
        }
    }
}

pub fn block_ui(ui: &mut Ui, b: &ir::Block, ctx: &mut RenderCtx<'_>) {
    match b {
        ir::Block::Heading { level, inlines } => {
            let set = Setting::heading(ctx.opts, &ctx.pal, *level);
            // More air above than below, so the heading belongs to what follows it. Inside the
            // block, where `measure::Heights` measures it.
            ui.add_space(theme::heading_space_above(ctx.opts, *level));
            runs(ui, inlines, &set, ctx);
        }
        ir::Block::Paragraph(inlines) => {
            let set = Setting::body(ctx.opts, &ctx.pal);
            runs(ui, inlines, &set, ctx);
        }
        ir::Block::List { ordered, items } => {
            for (i, item) in items.iter().enumerate() {
                let marker = if *ordered {
                    format!("{}.", i + 1)
                } else {
                    "•".to_owned()
                };
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = theme::snap(ctx.opts.base_size_pt * 0.45);
                    ui.add_space(theme::snap(ctx.opts.base_size_pt * 0.6));
                    ui.label(RichText::new(marker).color(ctx.pal.dim));
                    ui.vertical(|ui| {
                        blocks_ui(ui, item, ctx, None);
                    });
                });
            }
        }
        ir::Block::Quote { blocks, cite } => {
            // A large opening quotation mark hung in the indent, in `guide`, and the quotation
            // set upright after it. A painted mark rather than an indent alone: an indented
            // paragraph in a narrow column reads as a paragraph, not as a quotation. The mark
            // is a glyph, so it needs a face that carries it, and it takes the serif's (the
            // display face over a sans page, and the page's own over a serif one) at a size
            // that makes it a mark rather than a character; `guide` because it identifies
            // structure and is held to 3:1 for that.
            let base = ctx.opts.base_size_pt;
            let indent = theme::snap(base * 1.8);
            ui.horizontal_top(|ui| {
                let (slot, _) =
                    ui.allocate_exact_size(egui::vec2(indent, 0.0), egui::Sense::hover());
                let role = match ctx.opts.family {
                    FontChoice::Mono => FontChoice::Mono,
                    _ => FontChoice::Serif,
                };
                ui.painter().text(
                    egui::pos2(slot.left(), slot.top() - theme::snap(base * 0.35)),
                    egui::Align2::LEFT_TOP,
                    "\u{201C}",
                    egui::FontId::new(
                        theme::snap(base * 2.7),
                        fonts::family(Face::new(role, false, false)),
                    ),
                    ctx.pal.guide,
                );
                ui.vertical(|ui| {
                    blocks_ui(ui, blocks, ctx, None);
                    if let Some(c) = cite {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                            ui.label(RichText::new(format!("— {c}")).color(ctx.pal.dim).small());
                        });
                    }
                });
            });
        }
        ir::Block::Code { lang, text } => code_ui(ui, lang.as_deref(), text, ctx),
        ir::Block::Figure { image, caption } => {
            // A caption that repeats the alt is drawn once, as the caption: the placeholder
            // says "[image]" and the host, and the words follow in the caption face.
            let caption_is_alt = caption.as_ref().is_some_and(|c| {
                image
                    .alt
                    .as_deref()
                    .is_some_and(|a| a.trim() == ir::plain_text(c).trim())
            });
            if caption_is_alt {
                let bare = ir::Image {
                    src: image.src.clone(),
                    alt: None,
                };
                super::images::placeholder(ui, &bare, ctx);
            } else {
                super::images::placeholder(ui, image, ctx);
            }
            if let Some(c) = caption {
                let set = Setting::dim(ctx.opts, &ctx.pal);
                runs(ui, c, &set, ctx);
            }
        }
        ir::Block::Table { headers, rows } => table_ui(ui, headers, rows, ctx),
        ir::Block::Rule => {
            ui.add_space(theme::snap(ctx.opts.base_size_pt * 0.5));
            ui.separator();
        }
        ir::Block::Embed { kind, url } => embed_ui(ui, *kind, url, ctx),
        ir::Block::Thread(comments) => super::thread_ui::thread_ui(ui, comments, ctx),
        ir::Block::Entries(entries) => entries_ui(ui, entries, ctx),
    }
}

/// A run of story cards, as a ruled list: a hairline above the first and between the rest,
/// each card a linked headline in the smallest heading face with no underline (the card is
/// the affordance), its time at the right of the headline's first line in the dim chrome face,
/// its dek in body under. The hairline above the first card is also what rules off the section
/// heading a front page sets over the list. The thumbnail is drawn small above the headline
/// once it is loaded (`I` loads every image on the page, entries included) and is otherwise not
/// there at all: a column of one placeholder per card was the worst thing about these pages.
fn entries_ui(ui: &mut Ui, entries: &[ir::Entry], ctx: &mut RenderCtx<'_>) {
    let gap = theme::snap(ctx.opts.base_size_pt * 0.35);
    for e in entries {
        ui.add_space(gap);
        ui.separator();
        ui.add_space(gap);
        if let Some(img) = &e.image {
            super::images::thumbnail(ui, img, ctx);
        }
        let title = match &e.href {
            Some(href) => vec![ir::Inline::Link {
                href: href.clone(),
                inlines: e.title.clone(),
            }],
            None => e.title.clone(),
        };
        let set = Setting::headline(ctx.opts, &ctx.pal);
        let when = e
            .published
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty());
        // The right-hand end of the headline's first line holds the time and, for a card with
        // a picture, the small `[image]` control (`images::load_control`). Measure the time,
        // give the headline the rest, and let the headline wrap under both.
        let font = theme::chrome_font(ctx.opts);
        let time_w = when.map_or(0.0, |p| {
            ui.painter()
                .layout_no_wrap(p.to_owned(), font.clone(), ctx.pal.dim)
                .rect
                .width()
        });
        let control_w = if e.image.is_some() {
            theme::snap(ctx.opts.base_size_pt * 4.0)
        } else {
            0.0
        };
        if time_w + control_w > 0.0 {
            let pad = theme::snap(ctx.opts.base_size_pt * 0.8);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let avail = ui.available_width();
                ui.allocate_ui_with_layout(
                    egui::vec2((avail - time_w - control_w - pad).max(avail * 0.5), 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| runs(ui, &title, &set, ctx),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                    ui.spacing_mut().item_spacing.x = theme::snap(ctx.opts.base_size_pt * 0.5);
                    if let Some(p) = when {
                        ui.label(RichText::new(p).font(font.clone()).color(ctx.pal.dim));
                    }
                    if let Some(img) = &e.image {
                        super::images::load_control(ui, img, ctx);
                    }
                });
            });
        } else {
            runs(ui, &title, &set, ctx);
        }
        blocks_ui(ui, &e.summary, ctx, None);
    }
}

/// Flatten, register the run list with find-in-page, then set it.
fn runs(ui: &mut Ui, inlines: &[ir::Inline], set: &Setting, ctx: &mut RenderCtx<'_>) {
    let flat = inline::flatten(inlines);
    ctx.begin_list(&inline::plain_of(&flat));
    inline_ui::runs_ui(ui, &flat, set, ctx);
}

fn code_ui(ui: &mut Ui, lang: Option<&str>, text: &str, ctx: &mut RenderCtx<'_>) {
    let pal = ctx.pal;
    egui::Frame::new()
        .fill(pal.code_bg)
        .inner_margin(egui::Margin::symmetric(12, 9))
        .corner_radius(egui::CornerRadius::same(theme::RADIUS))
        .show(ui, |ui| {
            if let Some(l) = lang.filter(|l| !l.is_empty()) {
                ui.label(RichText::new(l).color(pal.dim).small());
            }
            // Its own horizontal scroll area, so a long line stays reachable without the page
            // scrolling sideways, and no re-wrapping, because a wrapped code line is a
            // changed code line.
            egui::ScrollArea::horizontal()
                .id_salt(("code", text.len(), text.as_ptr() as usize))
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(text)
                                .font(theme::mono_font(ctx.opts))
                                .color(pal.fg),
                        )
                        .wrap_mode(egui::TextWrapMode::Extend),
                    );
                });
        });
}

fn table_ui(
    ui: &mut Ui,
    headers: &[Vec<ir::Inline>],
    rows: &[Vec<Vec<ir::Inline>>],
    ctx: &mut RenderCtx<'_>,
) {
    let columns = headers
        .len()
        .max(rows.iter().map(Vec::len).max().unwrap_or(0));
    if columns == 0 {
        return;
    }
    // Cells must be told how wide they may be. `horizontal_wrapped` inside a grid cell wraps
    // at the *grid's* remaining width, which is the whole row, so without this every cell
    // lays out as one long line, the grid mis-measures its rows, and they overlap.
    let measure = ui.max_rect().width().max(120.0);
    let cell_width = (measure / columns as f32).max(measure * 0.25);
    // `egui_extras::TableBuilder` is excluded (its image loaders would take fetching out of
    // the audited `Fetcher`), so this is `egui::Grid`, which clips rather than scrolls. Hence
    // the scroll area around it.
    egui::ScrollArea::horizontal()
        .id_salt(("table", headers.len(), rows.len()))
        .auto_shrink([false, true])
        .show(ui, |ui| {
            egui::Grid::new(("table-grid", headers.len(), rows.len()))
                .num_columns(columns)
                .striped(true)
                .spacing(egui::vec2(
                    ctx.opts.base_size_pt,
                    ctx.opts.base_size_pt * 0.3,
                ))
                .show(ui, |ui| {
                    if !headers.is_empty() {
                        for cell in headers {
                            let mut set = Setting::body(ctx.opts, &ctx.pal);
                            set.color = ctx.pal.strong;
                            cell_ui(ui, cell_width, cell, &set, ctx);
                        }
                        ui.end_row();
                    }
                    for row in rows {
                        for cell in row {
                            let set = Setting::body(ctx.opts, &ctx.pal);
                            cell_ui(ui, cell_width, cell, &set, ctx);
                        }
                        ui.end_row();
                    }
                });
        });
}

fn cell_ui(ui: &mut Ui, width: f32, cell: &[ir::Inline], set: &Setting, ctx: &mut RenderCtx<'_>) {
    ui.scope(|ui| {
        ui.set_max_width(width);
        runs(ui, cell, set, ctx);
    });
}

/// `Block::Embed` is never constructed today (`html.rs` drops iframes as noise), but the arm
/// exists so the match stays exhaustive and the variant goes live the day it stops.
fn embed_ui(ui: &mut Ui, kind: ir::EmbedKind, url: &str, ctx: &mut RenderCtx<'_>) {
    let pal = ctx.pal;
    let label = url::Url::parse(url).ok().map_or_else(
        || url.to_owned(),
        |u| format!("{}{}", u.host_str().unwrap_or("?"), u.path()),
    );
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, pal.guide))
        .inner_margin(egui::Margin::same(8))
        .corner_radius(egui::CornerRadius::same(theme::RADIUS))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(format!("[{kind:?}]")).color(pal.dim));
                ui.label(RichText::new(label).color(pal.fg));
                ui.label(RichText::new("(not loaded)").color(pal.dim).small());
                let resp = ui.small_button("copy URL");
                if resp.has_focus() {
                    ctx.focus_other = true;
                }
                if resp.clicked() {
                    ctx.act(Action::Copy(url.to_owned()));
                }
            });
        });
}

/// The document's own header: an eyebrow naming the site, the title, a byline row, and a rule.
/// Not a `Block`: `doc.title` is metadata, and rendering it as an H1 is the reader's decision.
///
/// The eyebrow is the site's name when the page gives one and its host when it does not, set as
/// a chrome label (`theme::label_job`): it is the one line of the header that is about where
/// the page came from rather than what it says. The page's favicon sits beside it when the
/// bytes have arrived; until then the label stands alone. The title takes `heading_font(1)`,
/// which is the display role's bold; the byline row is the page's own small face, dim; and the
/// hairline under the three is `rule`, drawn by `separator`, which closes the header off from
/// the body the way a well-set page does.
pub fn document_header(ui: &mut Ui, doc: &ir::Document, ctx: &mut RenderCtx<'_>) {
    let base = ctx.opts.base_size_pt;
    let site = doc
        .site_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or_else(|| ctx.base.host_str().map(str::to_owned));
    if doc.favicon.is_some() || site.is_some() {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = theme::snap(base * 0.35);
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            if let Some(src) = doc.favicon.as_deref() {
                super::images::favicon(ui, src, ctx);
            }
            if let Some(site) = site {
                ui.label(theme::label_job(&site, ctx.opts, ctx.pal.dim));
            }
        });
        ui.add_space(theme::snap(base * 0.2));
    }
    if let Some(t) = crate::reader::title::display(doc) {
        let set = Setting::heading(ctx.opts, &ctx.pal, 1);
        let flat = vec![Run::Text {
            text: t.clone(),
            style: Default::default(),
        }];
        ctx.begin_list(&t);
        inline_ui::runs_ui(ui, &flat, &set, ctx);
    }
    let meta: Vec<String> = [
        doc.byline.clone(),
        doc.published
            .as_deref()
            .map(crate::reader::title::short_date),
    ]
    .into_iter()
    .flatten()
    .filter(|s| !s.trim().is_empty())
    .collect();
    if !meta.is_empty() {
        ui.add_space(theme::snap(base * 0.1));
        ui.label(RichText::new(meta.join(" · ")).color(ctx.pal.dim).small());
    }
    ui.add_space(theme::snap(base * 0.5));
    ui.separator();
    ui.add_space(theme::snap(base * 0.6));
}
