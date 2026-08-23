//! The circled `i` on the status strip, and the panel it opens.
//!
//! # Why the icon is painted rather than typed
//!
//! `ⓘ` (U+24D8) is in none of the ten faces `reader::ui::fonts` embeds, and it was in none of
//! the four epaint used to embed before them. Enclosed Alphanumerics is a symbol-font block:
//! swapping the whole font stack did not move it, and neither did checking a dozen more faces,
//! including one with 5,918 glyphs. The only info glyph anything here carries is `ℹ` (U+2139,
//! in NotoEmoji), a bare serif *i* with no circle at all, which does not say what this
//! affordance is.
//!
//! This outlived the gap it was written next to. `→` was tofu under the old faces and is not
//! under these; `ⓘ` was tofu under both.
//!
//! Two arcs and a letter cost about ten lines and have no font to be wrong about. They also
//! take the palette exactly, and scale with `chrome_font`, so the icon tracks `[`/`]` and
//! egui's zoom the way every other piece of chrome does. Do not "simplify" this back to a
//! character.
//!
//! # Why the panel is chrome and not a bar
//!
//! [`notice_ui`](super::notice_ui) owns the reader's interruptions, and everything the reader
//! says *about* a page that it needs seen goes through it. This panel says things about a page
//! and does not go through it, which looks like a hole in that rule and is not: a bar is shown
//! without asking, and this is summoned. It is a `chrome_bg` window floating over the column, in
//! the same class as the help overlay, and it keeps the record a dismissed bar no longer shows:
//! closing a caution loses nothing, because the row is still here.
//!
//! The rows come grouped (`pageinfo::Group`): where am I, how did the request get here, what
//! came back, who else was contacted. A heading per group and a hairline between them, so the
//! panel reads as four short answers rather than one long ledger, and every value has a label
//! beside it.

use crate::reader::opts::ReadOpts;
use crate::reader::pageinfo::{Group, Row};
use crate::reader::ui::theme::{self, Palette};
use eframe::egui::{self, RichText, Ui};

/// The status strip's one remaining right-hand affordance.
///
/// Uniformly dim, in every state. An icon that brightened when a page had loaded an image or
/// discarded a cookie would rebuild, one pixel at a time, the ribbon of dim text this panel
/// exists to remove; and the two facts that genuinely cannot wait for a click are bars now,
/// where nobody has to notice a badge to find them.
pub fn icon(ui: &mut Ui, pal: &Palette, opts: &ReadOpts) -> egui::Response {
    let d = theme::snap(theme::chrome_font(opts).size * 1.35);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(d, d), egui::Sense::click());
    let ink = if resp.hovered() { pal.fg } else { pal.dim };
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        // Inset by the stroke width, or the circle is clipped on the pixel grid at small sizes.
        p.circle_stroke(rect.center(), d * 0.5 - 1.0, egui::Stroke::new(1.0, ink));
        p.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "i",
            egui::FontId::proportional(theme::snap(d * 0.62)),
            ink,
        );
    }
    resp.on_hover_text("Page info (p)")
}

/// The panel itself. `open` is `Chrome::info_open`, so `Esc`, `p`, and the window's own close
/// button all reach the same bit.
///
/// `rows` is empty when no page has loaded, which is a state worth saying out loud: an empty
/// window reads as a panel that failed rather than one with nothing to report.
pub fn panel(
    ctx: &egui::Context,
    pal: &Palette,
    opts: &ReadOpts,
    rows: &[Row],
    strip_height: f32,
    open: &mut bool,
) {
    let font = theme::chrome_font(opts);
    // An `Area` and not a `Window`, for the reason the help card is one: a `Window` paints its
    // title bar from egui's own visuals rather than from the palette, which left this panel
    // wearing a grey band no theme here chose. The title and the close button are drawn below
    // instead, which also puts the close on the same monospace, labelled footing as the rest of
    // the chrome; egui's own is a painted glyph.
    egui::Area::new(egui::Id::new("hww-pageinfo-panel"))
        .order(egui::Order::Foreground)
        // Anchored above the icon that opens it, rather than centred like the help overlay:
        // this panel is about the strip, and rising from the affordance says so. `strip_height`
        // is written by `status_strip`, which runs earlier in the same frame.
        .anchor(
            egui::Align2::RIGHT_BOTTOM,
            egui::vec2(-8.0, -(strip_height + 8.0)),
        )
        .constrain(true)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(pal.chrome_bg)
                .stroke(egui::Stroke::new(1.0, pal.guide))
                .corner_radius(egui::CornerRadius::same(theme::RADIUS))
                .inner_margin(egui::Margin::symmetric(14, 12))
                .show(ui, |ui| {
                    ui.style_mut().override_font_id = Some(font.clone());
                    ui.horizontal(|ui| {
                        ui.label(theme::label_job("Page info", opts, pal.dim));
                        // Opened by a pointer, so it must be closable by one. `Esc` and `p` still work;
                        // this is what a mouse-only reader has.
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("close").clicked() {
                                *open = false;
                            }
                        });
                    });
                    ui.add_space(theme::snap(opts.base_size_pt * 0.4));
                    if rows.is_empty() {
                        ui.label(
                            RichText::new("No page loaded.")
                                .color(pal.dim)
                                .font(font.clone()),
                        );
                        return;
                    }
                    // One grid per group, a heading over each, a hairline between. `rows` comes
                    // grouped in reading order, so this is one pass and never a sort. Inside a
                    // scroll area with a ceiling, because a page with a redirect chain, a
                    // profile, and images makes a panel taller than a short window, and a
                    // floating panel that runs off the top cannot be read or closed.
                    let max_h = (ctx.content_rect().height() - strip_height - 96.0).max(120.0);
                    egui::ScrollArea::vertical()
                        .max_height(max_h)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            let mut current: Option<Group> = None;
                            let label_w = theme::snap(opts.base_size_pt * 8.5);
                            for (i, row) in rows.iter().enumerate() {
                                if current != Some(row.group) {
                                    if current.is_some() {
                                        ui.add_space(theme::snap(opts.base_size_pt * 0.3));
                                        ui.separator();
                                        ui.add_space(theme::snap(opts.base_size_pt * 0.3));
                                    }
                                    ui.label(theme::label_job(row.group.title(), opts, pal.dim));
                                    ui.add_space(theme::snap(opts.base_size_pt * 0.2));
                                    current = Some(row.group);
                                }
                                ui.horizontal_top(|ui| {
                                    ui.spacing_mut().item_spacing.x =
                                        theme::snap(opts.base_size_pt);
                                    ui.allocate_ui_with_layout(
                                        egui::vec2(label_w, 0.0),
                                        egui::Layout::top_down(egui::Align::Min),
                                        |ui| {
                                            ui.set_min_width(label_w);
                                            ui.label(
                                                RichText::new(row.label)
                                                    .color(pal.dim)
                                                    .font(font.clone()),
                                            );
                                        },
                                    );
                                    ui.vertical(|ui| {
                                        // A URL and a redirect chain are the long values here, and both
                                        // are read left to right for a host: wrapping keeps the whole
                                        // address legible where the strip could only truncate it.
                                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                                        ui.set_max_width(theme::snap(opts.base_size_pt * 26.0));
                                        ui.label(
                                            RichText::new(&row.value)
                                                .color(pal.fg)
                                                .font(font.clone()),
                                        );
                                        if let Some(note) = &row.note {
                                            // `chrome_font`, not `.small()`: every other row here already
                                            // asks for it by name, and after `Small` moved to the page's
                                            // family this was the one proportional line in a monospace
                                            // panel.
                                            ui.label(
                                                RichText::new(note)
                                                    .color(pal.dim)
                                                    .font(font.clone()),
                                            );
                                        }
                                    });
                                });
                                if i + 1 < rows.len() {
                                    ui.add_space(theme::snap(opts.base_size_pt * 0.25));
                                }
                            }
                        });
                });
        });
}
