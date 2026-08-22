//! `notice::Notice` -> a band across the page. **The one place the reader draws its own text.**
//!
//! # Why a function and not a convention
//!
//! Before this, a notice was a colour applied by hand at fifteen call sites, and a convention
//! drifts: `accent` had picked up five unrelated jobs, one of which was the find-match
//! background, which is not a notice at all. A function is one place. Adding something new for
//! the reader to report means calling [`band`], and there is nowhere else to put it.
//!
//! # What makes a band unmistakable
//!
//! **Width, not colour.** A band spans the whole central panel; the reading column never can,
//! because every `ir::Block` is laid out inside a fixed measure. That is a property the page
//! cannot forge at any colour, in any theme, in a screenshot, or to a reader who cannot
//! distinguish the hues. `notice::MARK` is the second such signal. Colour is support, and
//! `theme::every_notice_ink_is_legible_on_its_own_ground` is what keeps it honest.
//!
//! Only `Caution` has a ground of its own, because it is the only severity with a page under it.
//! The other two fill the viewport, and a notice that *is* the viewport needs no mark to be told
//! apart from a page that is not there. See `theme::notice_ink`.
//!
//! # One band per severity, not per notice
//!
//! `notice::about_page` returns up to four cautions about the same page. Drawn as four bands they
//! sit flush, with no edge between them, and merge into one block of ground: the reader sees one
//! remark where there are four. So [`band`] takes a group that shares a severity and draws it as
//! one surface with one line each, and the count stays legible because the lines are separate.
//!
//! It follows that a band borrows **no page idiom**: not `heading_font`, not `body_font`, not
//! the painted left rule of a quote, not the rounded fill of a code block.
//! `theme::notice_ink` and `theme::notice_heading_font` are where its type and paint come from,
//! and both belong to the reader rather than to any page.
//!
//! # The line that does the work
//!
//! `egui::Frame` paints its background around `content_ui.min_rect()`, which shrinks to fit.
//! Claiming the full width *before* drawing anything is therefore what turns a box into a
//! band; without it the fill comes out column-width and this whole module quietly does
//! nothing. The gutter cannot be an `inner_margin` either, because `egui::Margin` is `i8` and
//! a gutter is routinely 350 px.
//!
//! # Find does not search notices
//!
//! Bands are drawn outside `ready_screen`, so they never reach `RenderCtx::begin_list` and
//! never count towards `find_total`. That is deliberate: find is the reader searching the
//! page, and a notice is not the page.

use crate::reader::notice::{self, Button, Notice};
use crate::reader::opts::ReadOpts;
use crate::reader::ui::theme::{self, Palette};
use eframe::egui::{self, RichText, Ui};

/// The reading column's geometry: the width a band's fill spans, and the measure the page
/// keeps to.
///
/// One `Column` is built per frame and feeds *both* the bands and the article column, so the two
/// cannot disagree about a left edge. If a band's text ever fails to line up with the prose, it
/// is because something computed a second measure instead of taking this one.
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

/// Draw one group of same-severity notices as a single band. Returns the button pressed this
/// frame, if any.
///
/// The group is the unit because a severity is what a band's whole dress is derived from, and
/// because two adjacent bands of one severity are indistinguishable from one. Callers group;
/// mixing severities in one call is a bug, and in debug builds it is an assertion.
pub fn band(
    ui: &mut Ui,
    pal: &Palette,
    opts: &ReadOpts,
    column: Column,
    group: &[Notice],
) -> Option<Button> {
    let first = group.first()?;
    debug_assert!(
        group.iter().all(|n| n.severity == first.severity),
        "a band is one severity: {:?}",
        group.iter().map(|n| n.severity).collect::<Vec<_>>()
    );
    let dress = theme::notice_ink(pal, first.severity);
    // A blocking severity has no page under it, so it takes the rest of the viewport. Letting it
    // shrink to its content would leave a strip of hww floating on empty page ground, which
    // reads as a very short article: the exact confusion this module exists to remove.
    let claim_height = first.severity.fills_the_viewport();
    // `Frame::end` grows the painted rect by the inner margin *after* the content `Ui` has been
    // sized, so a claim of the whole remaining height allocates that height plus both margins
    // and overflows the scroll viewport. The overflow is small (12-48 px) and entirely
    // visible: every idle, loading, and failure screen grows a scrollbar and a little dead
    // scroll for `j`/`k` to find. Subtract the margin here so the claim is exact.
    let pad = 2.0 * f32::from(theme::band_pad(opts));
    let rest = (ui.available_height() - pad).max(0.0);
    let mut pressed = None;

    egui::Frame::new()
        .fill(dress.fill)
        // No stroke. `Frame` strokes all four sides, so a full-bleed band would grow vertical
        // hairlines at the viewport edges and read as a boxed page. The severities whose fill is
        // the page's own ground carry no edge either; see `theme::notice_ink`.
        .inner_margin(egui::Margin::symmetric(0, theme::band_pad(opts)))
        .show(ui, |ui| {
            // A band is entirely the reader's words, so the chrome family is set once here.
            // The labels below that name a font still win, which is what keeps the heading at
            // `notice_heading_font`; what this catches is the widgets that name none, which is
            // every `ui.button()` in the actions row. `TextStyle::Button` is not enough on its
            // own: a `Button`'s style is only a fallback, and it loses to nothing here because
            // there is nothing else to lose to, which is exactly why it went unnoticed.
            ui.style_mut().override_font_id = Some(theme::chrome_font(opts));
            // See the module doc: without this the fill is column-wide and the band is a box.
            ui.set_min_width(ui.available_width());
            if claim_height {
                ui.set_min_height(rest);
            }
            ui.horizontal_top(|ui| {
                ui.add_space(column.gutter);
                ui.vertical(|ui| {
                    ui.set_max_width(column.measure);
                    ui.set_min_width(column.measure);
                    for (i, notice) in group.iter().enumerate() {
                        // Between remarks, not around them: the band's own padding is already
                        // the gap at the top and bottom.
                        if i > 0 {
                            ui.add_space(theme::snap(opts.base_size_pt * 0.45));
                        }
                        if let Some(h) = notice.heading {
                            ui.label(
                                RichText::new(h)
                                    .font(theme::notice_heading_font(opts))
                                    .color(dress.heading),
                            );
                            ui.add_space(theme::snap(opts.base_size_pt * 0.5));
                        }
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing.x = theme::snap(opts.base_size_pt * 0.35);
                            // The mark, so the band survives a screenshot and a monochrome eye.
                            ui.label(
                                RichText::new(notice::MARK)
                                    .color(dress.aside)
                                    .font(theme::chrome_font(opts)),
                            );
                            // The severity, spelled out. Colour is the obvious way to carry
                            // loudness and the one that fails quietest; see `Severity::word`.
                            ui.label(
                                RichText::new(notice.severity.word())
                                    .color(dress.aside)
                                    .font(theme::chrome_font(opts)),
                            );
                            ui.label(
                                RichText::new(notice.body.as_str())
                                    .color(dress.body)
                                    .font(theme::chrome_font(opts)),
                            );
                        });
                        for d in &notice.detail {
                            ui.label(
                                RichText::new(d.as_str())
                                    .color(dress.aside)
                                    .font(theme::chrome_font(opts)),
                            );
                        }
                        if !notice.actions.is_empty() {
                            ui.add_space(theme::snap(opts.base_size_pt * 0.75));
                            ui.horizontal_wrapped(|ui| {
                                for b in notice.actions {
                                    if ui.button(b.label()).clicked() {
                                        pressed = Some(*b);
                                    }
                                }
                            });
                        }
                    }
                });
            });
        });

    pressed
}
