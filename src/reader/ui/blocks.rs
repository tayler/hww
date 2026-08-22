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
use crate::reader::inline::{self, Run};
use crate::reader::ui::inline_ui::{self, Setting};
use crate::reader::ui::{Action, RenderCtx};
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
            ui.add_space(super::theme::snap(ctx.opts.base_size_pt * 0.4));
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
                    ui.spacing_mut().item_spacing.x =
                        super::theme::snap(ctx.opts.base_size_pt * 0.45);
                    ui.add_space(super::theme::snap(ctx.opts.base_size_pt * 0.6));
                    ui.label(RichText::new(marker).color(ctx.pal.dim));
                    ui.vertical(|ui| {
                        blocks_ui(ui, item, ctx, None);
                    });
                });
            }
        }
        ir::Block::Quote { blocks, cite } => {
            ui.horizontal_top(|ui| {
                ui.add_space(super::theme::snap(ctx.opts.base_size_pt * 0.4));
                // A painted rule rather than an indent alone: an indented paragraph in a
                // narrow column reads as a paragraph, not as a quotation.
                //
                // The bar is as tall as the quotation, which is not known until the quotation
                // has been laid out. `available_height` here is the rest of the *viewport*, so
                // asking for it drew a rule from the quote to the bottom of the page; at
                // `rule`'s 1.41:1 that was invisible, and at `guide`'s 3.24:1 it is not. So the
                // shape is reserved, the content is laid out, and the rect is filled in after.
                let bar = ui.painter().add(egui::Shape::Noop);
                let (slot, _) = ui.allocate_exact_size(egui::vec2(2.0, 0.0), egui::Sense::hover());
                ui.add_space(super::theme::snap(ctx.opts.base_size_pt * 0.6));
                let quote = ui
                    .vertical(|ui| {
                        blocks_ui(ui, blocks, ctx, None);
                        if let Some(c) = cite {
                            ui.label(RichText::new(format!("— {c}")).color(ctx.pal.dim).small());
                        }
                    })
                    .response
                    .rect;
                ui.painter().set(
                    bar,
                    egui::Shape::rect_filled(
                        egui::Rect::from_min_max(
                            egui::pos2(slot.left(), quote.top()),
                            egui::pos2(slot.left() + 2.0, quote.bottom()),
                        ),
                        0.0,
                        ctx.pal.guide,
                    ),
                );
            });
        }
        ir::Block::Code { lang, text } => code_ui(ui, lang.as_deref(), text, ctx),
        ir::Block::Figure { image, caption } => {
            super::images::placeholder(ui, image, ctx);
            if let Some(c) = caption {
                let set = Setting::dim(ctx.opts, &ctx.pal);
                runs(ui, c, &set, ctx);
            }
        }
        ir::Block::Table { headers, rows } => table_ui(ui, headers, rows, ctx),
        ir::Block::Rule => {
            ui.add_space(super::theme::snap(ctx.opts.base_size_pt * 0.5));
            ui.separator();
        }
        ir::Block::Embed { kind, url } => embed_ui(ui, *kind, url, ctx),
        ir::Block::Thread(comments) => super::thread_ui::thread_ui(ui, comments, ctx),
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
        .inner_margin(egui::Margin::same(8))
        .corner_radius(0.0)
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
                                .font(super::theme::mono_font(ctx.opts))
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
        .corner_radius(0.0)
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

/// The document's own header: title, byline, and date. Not a `Block`: `doc.title` is metadata,
/// and rendering it as an H1 is the reader's decision.
pub fn document_header(ui: &mut Ui, doc: &ir::Document, ctx: &mut RenderCtx<'_>) {
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
        doc.site_name.clone(),
    ]
    .into_iter()
    .flatten()
    .filter(|s| !s.trim().is_empty())
    .collect();
    if !meta.is_empty() {
        ui.label(RichText::new(meta.join(" · ")).color(ctx.pal.dim).small());
    }
    ui.add_space(super::theme::snap(ctx.opts.base_size_pt * 0.5));
}
