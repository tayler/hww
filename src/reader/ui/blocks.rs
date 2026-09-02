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
///
/// **Every call to this is a nested run**, and clearing [`RenderCtx::block`] for the length of
/// it is what says so: the reading column draws a top-level block by calling [`block_ui`]
/// itself, and the four ways one block gets inside another — a list item, a quotation, an
/// entry's summary, a comment's own body — all arrive here. See the field's own doc for what
/// the three tables keyed by it would do with a nested run's rows.
pub fn blocks_ui(
    ui: &mut Ui,
    blocks: &[ir::Block],
    ctx: &mut RenderCtx<'_>,
    scroll_to: Option<usize>,
) {
    // Restored rather than left cleared: a run of blocks inside one top-level block is followed
    // by the rest of that block, and `Block::Quote` puts one in the middle of a page.
    let outer = ctx.block.take();
    for (i, b) in blocks.iter().enumerate() {
        let resp = ui.scope(|ui| block_ui(ui, b, ctx)).response;
        if scroll_to == Some(i) {
            resp.scroll_to_me(Some(egui::Align::TOP));
        }
    }
    ctx.block = outer;
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
                    // A page that chose one family for the whole of itself keeps it here too,
                    // the rule `theme::heading_role` follows; only the sans page borrows the
                    // display face.
                    FontChoice::Sans => FontChoice::Serif,
                    other => other,
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
    // Asked of the run and not of the row, so one site's mark failing to arrive cannot indent
    // its neighbours differently. A run with no marks in it — every list a page produced, and
    // the history — is laid out exactly as it was before the column existed.
    let icon = entries
        .iter()
        .any(|e| e.icon.is_some())
        .then(|| theme::mark_box(ctx.opts));
    // `None` inside a card's summary or a list item, and the window rule is off for the whole
    // run when it is: the heights are keyed by top-level block, a nested run has no such index,
    // and borrowing the containing block's would have the two runs overwriting each other's
    // rows. Nothing is lost by laying a nested run out whole — it is a card's own body, held to
    // `budget::lead_in` and to `html::MAX_CARD_NESTING`, not the five thousand rows of
    // `hww:history` this rule exists for.
    let keyed = ctx.block;
    for (row, e) in entries.iter().enumerate() {
        ui.add_space(gap);
        ui.separator();
        ui.add_space(gap);
        // The window rule, at the row: `hww:history` is one `Block::Entries` of up to five
        // thousand rows, so the block-level skip in `ready_screen` can never fire for it — the
        // block *is* the page. Without this, every row was laid out on every frame, each one a
        // galley for its timestamp and a handful of allocations for its headline, and scrolling
        // a full history did that work five thousand times over.
        //
        // The rule is `thread_ui`'s, with the same shape and the same three guards: the gaps and
        // the hairline above are laid out either way so the rows sit where they sat, `ctx.band`
        // is `None` whenever the page is being laid out whole (find, Tab, selection, AccessKit),
        // and a row with no remembered height is drawn. `note_icon` reads the cursor, so a
        // skipped row has to claim its space rather than collapse — `allocate_space`, not
        // `add_space`.
        let y = ui.next_widget_position().y;
        let skip = keyed
            .zip(ctx.band)
            .and_then(|(block, band)| {
                ctx.entry_heights
                    .get(&(block, row))
                    .copied()
                    .map(|h| (band, h))
            })
            .and_then(|(band, h)| band.skips(y, h).then_some(h));
        // What is allocated back is the row's own height, which is not the same number as its
        // rect: see [`remember`], which is where that is measured and argued.
        if let Some(h) = skip {
            ui.allocate_space(egui::vec2(0.0, h));
            continue;
        }
        {
            let Some(box_size) = icon else {
                entry_ui(ui, e, ctx);
                remember(ui, ctx, keyed, row, y);
                continue;
            };
            // Two columns: the mark, then everything the row says. The gutter is the mark
            // plus the same space the time is given at the other end of the headline, and
            // the right-hand column is a `Ui` of its own so the headline, the address, and
            // the dek wrap and align to it rather than to the page — which is what keeps
            // them in the relationship to each other they have in a list with no marks in
            // it.
            let pad = theme::snap(ctx.opts.base_size_pt * 0.8);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let avail = ui.available_width();
                ui.allocate_ui_with_layout(
                    egui::vec2(box_size, box_size),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| match &e.icon {
                        Some(src) => super::images::site_icon(ui, src, box_size, ctx),
                        // A row in a marked run that has no mark of its own — a page kept before
                        // this column existed, or one whose site named an icon hww will not
                        // request. The space is claimed anyway, because
                        // `allocate_ui_with_layout` advances the cursor by what its contents
                        // *used*: a closure that draws nothing is zero wide, and that row's
                        // headline would start where the marks are.
                        None => {
                            ui.allocate_space(egui::vec2(box_size, box_size));
                        }
                    },
                );
                // `add_space` and not a wider allocation: `allocate_ui_with_layout` hands back the
                // space its contents *used*, so a gutter padded from the inside collapses onto the
                // mark and the two columns touch.
                ui.add_space(pad);
                ui.allocate_ui_with_layout(
                    egui::vec2((avail - box_size - pad).max(avail * 0.5), 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| entry_ui(ui, e, ctx),
                );
            });
        }
        remember(ui, ctx, keyed, row, y);
    }
}

/// Record how far a drawn row moved the cursor, and throw the run's whole table away if that
/// disagrees with what was remembered.
///
/// **The disagreement is the point.** A row is measured on the first frame the page is drawn,
/// which is the only frame every row is laid out — and on that frame the chrome font's metrics
/// are not ready, so `entry_ui` measures a zero-width timestamp, gives the headline the whole
/// column, and the row comes out a line shorter than it will ever be again. Rows still near the
/// window are re-measured on the next frame and correct themselves; rows the reader has not
/// scrolled to would keep that first wrong height for as long as the page is open, and a long
/// list would end short of its last row.
///
/// So a drawn row that measures differently invalidates the run rather than just itself: the
/// rows it cannot see are wrong for the same reason it was, and they are only re-measurable by
/// being drawn. One frame of laying the run out whole, and the table is right from then on.
/// Costs one `f32` comparison per drawn row, and says nothing about a row whose height really
/// did change — an arriving thumbnail already goes through `measure::Heights::forget`.
fn remember(ui: &Ui, ctx: &mut RenderCtx<'_>, keyed: Option<usize>, row: usize, y: f32) {
    // Nothing recorded for a nested run, matching the skip that was not offered it. A height
    // filed under the containing block's index is worse than no height: it is the outer run's
    // row `row`, and the next frame reads it back as one.
    let Some(block) = keyed else {
        return;
    };
    // **How far the cursor moved, not how tall the row's rect was.** The two are different
    // numbers: a `Ui`'s rect is the union of what it drew, and the trailing space `blocks_ui`
    // leaves under the last paragraph of a dek is cursor movement with nothing in it. Recording
    // the rect and allocating it back lost a couple of points a row, which over sixty rows is a
    // page that ends early, with `Shift+G` landing above the last story rather than on it.
    //
    // And the gap comes off, because `allocate_space` adds it back: the cursor after a row is
    // the row plus one item spacing, and the spacing is the layout's to put there. Counting it
    // in the stored height counts it twice, which is the same bug in the other direction.
    let used = (ui.next_widget_position().y - y - ui.spacing().item_spacing.y).max(0.0);
    let key = (block, row);
    // Sub-pixel drift is rounding, not a relayout. A line of text is twenty-odd points.
    if ctx
        .entry_heights
        .get(&key)
        .is_some_and(|had| (had - used).abs() > 0.5)
    {
        ctx.entry_heights.retain(|(b, _), _| *b != block);
    }
    ctx.entry_heights.insert(key, used);
}

/// One entry's own column: the thumbnail, the headline with its time, the address, the dek.
///
/// Split out of [`entries_ui`] when the site mark arrived, and it draws the same thing at the
/// same widths whether it was called directly or inside the right-hand column of a two-column
/// row. Everything in here measures against `ui.available_width()`, which is how one function
/// serves both.
fn entry_ui(ui: &mut Ui, e: &ir::Entry, ctx: &mut RenderCtx<'_>) {
    if let Some(img) = &e.image {
        super::images::thumbnail(ui, img, ctx);
    }
    // Flattened straight under the href rather than cloned into an `ir::Inline::Link` to be
    // flattened after: this is drawn per row per frame, and `hww:history` is five thousand rows.
    let title = inline::flatten_linked(&e.title, e.href.as_deref());
    let set = Setting::headline(ctx.opts, &ctx.pal);
    // Through `short_date`, as the masthead's dateline is: a feed writes RFC 822, which is
    // 31 characters of mostly zone offset, and the headline below gets whatever width this
    // stamp leaves it. See `title::short_date`.
    let when = e
        .published
        .as_deref()
        .map(crate::reader::title::short_date)
        .filter(|p| !p.is_empty());
    // The right-hand end of the headline's first line holds the time and, for a card with
    // a picture, the small `[image]` control (`images::load_control`). Measure the time,
    // give the headline the rest, and let the headline wrap under both.
    let font = theme::chrome_font(ctx.opts);
    let time_w = when.as_deref().map_or(0.0, |p| {
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
    // The reading list's own control, reserved like the picture's and drawn beside it. A run
    // of entries anywhere else is the page's own rows, and hww has nothing to take them off;
    // see `RenderCtx::reading_list`. A width rather than a measurement, as `control_w` is: the
    // label is one short word in a face the painter would have to be handed to measure, and
    // the headline beside it wraps into whatever is left either way.
    let remove_w = if ctx.reading_list && e.href.is_some() {
        theme::snap(ctx.opts.base_size_pt * 4.5)
    } else {
        0.0
    };
    if time_w + control_w + remove_w > 0.0 {
        let pad = theme::snap(ctx.opts.base_size_pt * 0.8);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let avail = ui.available_width();
            ui.allocate_ui_with_layout(
                egui::vec2(
                    (avail - time_w - control_w - remove_w - pad).max(avail * 0.5),
                    0.0,
                ),
                egui::Layout::top_down(egui::Align::Min),
                |ui| runs_flat(ui, &title, &set, ctx),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                ui.spacing_mut().item_spacing.x = theme::snap(ctx.opts.base_size_pt * 0.5);
                // First, so it sits at the outside edge under the header's own control. The
                // other two are about the row's page; this one is about the row.
                if remove_w > 0.0
                    && let Some(href) = &e.href
                {
                    remove_control(ui, href, ctx);
                }
                if let Some(p) = &when {
                    ui.label(
                        RichText::new(p.as_str())
                            .font(font.clone())
                            .color(ctx.pal.dim),
                    );
                }
                if let Some(img) = &e.image {
                    super::images::load_control(ui, img, ctx);
                }
            });
        });
    } else {
        runs_flat(ui, &title, &set, ctx);
    }
    // Under the headline and above the dek, in the same dim as the date across from it.
    // `small_font` and not `chrome_font`: the address belongs to the page the row points at,
    // and this module's rule is that the page's words are set in the page's family. It is
    // not a link — the headline is the link, and a row with two of them would give Tab two
    // stops to the same place.
    //
    // Through `runs` rather than `ui.label`, which is the only way it reaches
    // `RenderCtx::begin_list` and so the only way find-in-page can see it. The history and
    // the bookmarks show an address at all because their titles repeat — half a dozen `Home`s
    // are one list of identical rows without it — so `/` and a hostname is the first thing a
    // reader tries on a long one, and a search that matched the visible text of every row
    // except the one distinguishing part of it would be a wrong answer rather than a missing
    // feature.
    if let Some(addr) = &e.address {
        let set = Setting {
            font: theme::small_font(ctx.opts),
            color: ctx.pal.dim,
            role: ctx.opts.family,
            underline: false,
        };
        // One copy of the address rather than two: this was an `ir::Inline::Text` holding a
        // clone, which `flatten` then copied again into the `Run::Text` it produced.
        runs_flat(
            ui,
            &[inline::Run::Text {
                text: addr.clone(),
                style: inline::Style::default(),
            }],
            &set,
            ctx,
        );
    }
    // Through `budget::lead_in`, not a loop here: `app::walk_blocks` reads the same
    // function, so the pictures `I` offers to load are exactly the pictures on screen.
    // See the module doc there.
    let shown = crate::reader::budget::lead_in(&e.summary, ctx.opts.entry_summary_bytes);
    blocks_ui(ui, shown, ctx, None);
}

/// The control at the trailing edge of a reading-list row: take this one link off the list.
///
/// A framed button, like the `forget all` above it and the panel's three, rather than the dim
/// bracketed label `images::load_control` uses. The two are different offers: `[image]` asks a
/// page whether to fetch something and has to stay quiet enough to sit in the middle of it, and
/// this one deletes something the reader owns. A border is what says a press has a consequence,
/// and hww's other deleting controls all have one.
///
/// No `theme::focus_ring`: a framed button keeps egui's own focused border, and the ring exists
/// for the controls that have no frame to change. It writes `focus_other` and not `focus_href`,
/// because the row's headline is the link — a second widget claiming to be one would hand
/// `Shift+Y` and `Shift+L` a link the reader is not pointing at.
fn remove_control(ui: &mut Ui, href: &str, ctx: &mut RenderCtx<'_>) {
    let resp = ui.small_button(crate::reader::notice::READING_LIST_REMOVE);
    super::follow_focus(&resp);
    if resp.has_focus() {
        ctx.focus_other = true;
    }
    if resp.clicked() {
        ctx.act(Action::ForgetNext(href.to_owned()));
    }
}

/// Flatten, register the run list with find-in-page, then set it.
fn runs(ui: &mut Ui, inlines: &[ir::Inline], set: &Setting, ctx: &mut RenderCtx<'_>) {
    runs_flat(ui, &inline::flatten(inlines), set, ctx);
}

/// [`runs`] for a run list that is already flat, so a caller that had to build one anyway does
/// not build it twice. See `blocks::entry_ui`.
fn runs_flat(ui: &mut Ui, flat: &[inline::Run], set: &Setting, ctx: &mut RenderCtx<'_>) {
    ctx.begin_runs(flat);
    inline_ui::runs_ui(ui, flat, set, ctx);
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
                super::follow_focus(&resp);
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
    // The reading list's one header control, and the only thing hww ever draws in a document
    // header that is not a fact about the document. It belongs here rather than only in the
    // settings panel because a reader looking at a list they want gone is looking at the list, not
    // at three groups of preferences; the panel's button stays where it is, and both press the
    // same door so both say the same sentence afterwards.
    //
    // Only while there is something to empty. On an empty list the document is one sentence about
    // how to fill it, and a button offering to clear nothing under it would be the page inventing
    // a state it is not in.
    if ctx.reading_list && has_entries(doc) {
        ui.add_space(theme::snap(base * 0.25));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
            let resp = ui
                .small_button(crate::reader::notice::READING_LIST_FORGET_ALL)
                .on_hover_text(crate::reader::notice::READING_LIST_FORGET_ALL_HOVER);
            super::follow_focus(&resp);
            if resp.has_focus() {
                ctx.focus_other = true;
            }
            if resp.clicked() {
                ctx.act(Action::ForgetAllNext);
            }
        });
    }
    ui.add_space(theme::snap(base * 0.5));
    ui.separator();
    ui.add_space(theme::snap(base * 0.6));
}

/// Whether a document has a row in it, which for one of hww's own list views is the difference
/// between a list and the sentence it shows instead of one.
fn has_entries(doc: &ir::Document) -> bool {
    doc.blocks
        .iter()
        .any(|b| matches!(b, ir::Block::Entries(e) if !e.is_empty()))
}
