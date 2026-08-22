//! Remembered block heights, so a page longer than the window costs a window's worth of layout.
//!
//! egui lays out every widget it is handed, every frame; only *painting* is clipped by the
//! scroll area. A long changelog is a thousand `ir::Block`s, and laying all of them out to
//! show the thirty on screen measured 8 ms a frame in release and 77 ms in debug, which is
//! the whole of a 60 Hz budget spent on text nobody can see. The fix is the one every long
//! document renderer makes: remember what a block measured the last time it was laid out, and
//! hand egui empty space of that height for as long as the block is outside the window.
//!
//! A remembered height is a *measurement*, so anything that changes what a block would
//! measure throws the whole table away: the column width, the zoom, the reading settings, and
//! the image store, because a placeholder that becomes a photograph is a different height. A
//! stale entry that survived would read as the page jumping when its block scrolls back in.
//!
//! The table lives here rather than in `ui/` because the arithmetic is the part that can be
//! wrong, and it needs no `egui::Context`: what the reader hands in is four floats.

use crate::reader::opts::ReadOpts;

/// Everything a remembered height depends on. Any change clears the table.
#[derive(Clone, PartialEq, Debug)]
pub struct Layout {
    pub opts: ReadOpts,
    /// Width of the reading column, in pixels.
    pub measure: f32,
    /// egui's zoom, which scales every height without changing the settings.
    pub pixels_per_point: f32,
    /// [`crate::reader::ui::images::ImageStore::changes`]: a load or a failure re-sizes the
    /// block it lands in, and `I` starts loads in blocks that are nowhere near the window.
    pub images: u64,
    /// The find query, because a match is drawn at a tighter line height than the page reads
    /// at: a galley row made entirely of matched text is *shorter* while the find bar is open.
    /// Those heights are measured (a non-empty query lays the page out whole, so every block
    /// records one), and nothing else in this key moves when the bar closes, so without this
    /// the page would keep the compressed heights for every block off the window and shift
    /// under the reader as they scroll back in.
    pub find: Option<String>,
    /// [`egui::Id::value`] of the link a click is pending on, which wears `notice::PENDING` and
    /// is therefore a different height while the fetch is in flight.
    pub pending_link: Option<u64>,
}

/// Heights measured under one [`Layout`], indexed by block.
#[derive(Default)]
pub struct Heights {
    layout: Option<Layout>,
    h: Vec<Option<f32>>,
}

impl Heights {
    /// Forget everything. A new page has its own blocks, and the old indices mean nothing.
    pub fn clear(&mut self) {
        self.layout = None;
        self.h.clear();
    }

    /// Point the table at the layout this frame is drawing under, clearing it if that differs
    /// from the one the heights in it were measured under.
    pub fn under(&mut self, layout: &Layout) {
        if self.layout.as_ref() != Some(layout) {
            self.layout = Some(layout.clone());
            self.h.clear();
        }
    }

    /// What block `i` measured last time it was laid out, if it has been.
    pub fn get(&self, i: usize) -> Option<f32> {
        self.h.get(i).copied().flatten()
    }

    pub fn set(&mut self, i: usize, height: f32) {
        if self.h.len() <= i {
            self.h.resize(i + 1, None);
        }
        self.h[i] = Some(height);
    }
}

/// A band of the page that must be laid out for real, in the scroll area's coordinates.
///
/// Wider than the window on purpose. The margin is what keeps everything that walks the page
/// by *widget* working across the edge of the viewport: a Tab from the last visible link, a
/// drag-selection running past the bottom, egui's own scroll-focus-into-view. One window's
/// worth either way is two extra screens of layout, which is still a fortieth of the page
/// this was written for.
#[derive(Clone, Copy, Debug)]
pub struct Band {
    pub top: f32,
    pub bottom: f32,
}

impl Band {
    /// `top`/`bottom` are the visible window; the margin is added here.
    pub fn around(top: f32, bottom: f32) -> Self {
        let margin = (bottom - top).max(0.0);
        Self {
            top: top - margin,
            bottom: bottom + margin,
        }
    }

    /// May a block of `height` starting at `y` be replaced by empty space?
    ///
    /// Touching the band at all is enough to be laid out: a block that starts above the band
    /// and ends inside it is the one on screen, and it is the case an `is-inside` test gets
    /// wrong.
    pub fn skips(&self, y: f32, height: f32) -> bool {
        y + height < self.top || y > self.bottom
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(measure: f32) -> Layout {
        Layout {
            opts: ReadOpts::default(),
            measure,
            pixels_per_point: 1.0,
            images: 0,
            find: None,
            pending_link: None,
        }
    }

    #[test]
    fn a_height_survives_a_frame_under_the_same_layout() {
        let mut h = Heights::default();
        h.under(&layout(600.0));
        h.set(3, 42.0);
        h.under(&layout(600.0));
        assert_eq!(h.get(3), Some(42.0));
    }

    #[test]
    fn a_narrower_column_forgets_every_height() {
        let mut h = Heights::default();
        h.under(&layout(600.0));
        h.set(3, 42.0);
        h.under(&layout(500.0));
        assert_eq!(h.get(3), None);
    }

    #[test]
    fn a_loaded_image_forgets_every_height() {
        let mut h = Heights::default();
        let mut l = layout(600.0);
        h.under(&l);
        h.set(3, 42.0);
        l.images += 1;
        h.under(&l);
        assert_eq!(h.get(3), None);
    }

    #[test]
    fn a_changed_reading_setting_forgets_every_height() {
        let mut h = Heights::default();
        let mut l = layout(600.0);
        h.under(&l);
        h.set(3, 42.0);
        l.opts.line_height += 0.1;
        h.under(&l);
        assert_eq!(h.get(3), None);
    }

    /// A match is drawn at a tighter line height than the page reads at, so every height
    /// measured while the find bar was open belongs to a page that is no longer on screen.
    /// Nothing else in the key moves when the bar closes, so this field is the only thing
    /// that can notice.
    #[test]
    fn a_closed_find_bar_forgets_every_height() {
        let mut h = Heights::default();
        let mut l = layout(600.0);
        l.find = Some("cabbage".to_owned());
        h.under(&l);
        h.set(3, 42.0);
        l.find = None;
        h.under(&l);
        assert_eq!(h.get(3), None);
    }

    /// Same shape, for the `[loading]` mark a click appends to the link it was aimed at.
    #[test]
    fn a_pending_link_forgets_every_height() {
        let mut h = Heights::default();
        let mut l = layout(600.0);
        h.under(&l);
        h.set(3, 42.0);
        l.pending_link = Some(7);
        h.under(&l);
        assert_eq!(h.get(3), None);
    }

    #[test]
    fn an_unmeasured_block_has_no_height() {
        let mut h = Heights::default();
        h.under(&layout(600.0));
        h.set(9, 10.0);
        assert_eq!(h.get(4), None);
        assert_eq!(h.get(99), None);
    }

    #[test]
    fn a_block_straddling_the_edge_of_the_band_is_laid_out() {
        let band = Band::around(1000.0, 1500.0);
        // The margin is one window either way: 500..2000.
        assert_eq!((band.top, band.bottom), (500.0, 2000.0));
        // Starts above the band, ends inside it. The `is-inside` mistake.
        assert!(!band.skips(100.0, 600.0));
        // Starts inside, runs off the bottom.
        assert!(!band.skips(1900.0, 4000.0));
        // Taller than the band, and covering all of it.
        assert!(!band.skips(0.0, 9000.0));
    }

    #[test]
    fn a_block_clear_of_the_band_is_skipped() {
        let band = Band::around(1000.0, 1500.0);
        assert!(band.skips(0.0, 100.0));
        assert!(band.skips(2100.0, 100.0));
        // A zero-height block on the boundary is not off it.
        assert!(!band.skips(500.0, 0.0));
    }
}
