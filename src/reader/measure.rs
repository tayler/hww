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
//! measure throws the whole table away: the column width, the zoom, and the reading settings.
//! A stale entry that survived would read as the page jumping when its block scrolls back in.
//!
//! An image is the one such change that is *local*. A placeholder becoming a photograph is a
//! different height for one block and no different for any other, and it used to be in the key
//! above, so a page loading thirty pictures re-measured every block thirty times — the cost
//! rising with the square of how much the reader asked for, and worst under the policy that
//! asks for the most. [`Heights::forget`] drops the blocks that actually moved; `ReaderApp`
//! maps a changed `src` to them.
//!
//! The table lives here rather than in `ui/` because the arithmetic is the part that can be
//! wrong, and it needs no `egui::Context`: what the reader hands in is four floats.

use crate::reader::opts::ReadOpts;
use std::collections::HashMap;

/// Everything a remembered height depends on. Any change clears the table.
#[derive(Clone, PartialEq, Debug)]
pub struct Layout {
    pub opts: ReadOpts,
    /// Width of the reading column, in points.
    pub measure: f32,
    /// egui's zoom, which scales every height without changing the settings.
    pub pixels_per_point: f32,
    /// Whether a find query is being drawn, because a match is drawn at a tighter line height
    /// than the page reads at: a galley row made entirely of matched text is *shorter* while
    /// the find bar is open. Those heights are measured (a non-empty query lays the page out
    /// whole, so every block records one), and nothing else in this key moves when the bar
    /// closes, so without this the page would keep the compressed heights for every block off
    /// the window and shift under the reader as they scroll back in. Which query does not
    /// matter: while one is drawn the table is never consulted.
    pub find: bool,
    /// `egui::Id::value` of the link a click is pending on, which wears `notice::PENDING` and
    /// is therefore a different height while the fetch is in flight.
    pub pending_link: Option<u64>,
    /// How many times the set of collapsed comments has changed. A `Block::Thread` is one
    /// block and one height, and `Shift+Z` collapses every thread on the page, including the
    /// ones clear of the band that nothing will lay out this frame.
    pub collapsed: u64,
}

/// Heights measured under one [`Layout`], indexed by block, and within a discussion by
/// comment: a thread is one block, and a long one used to cost the whole page every frame.
#[derive(Default)]
pub struct Heights {
    layout: Option<Layout>,
    h: Vec<Option<f32>>,
    /// `(block, comment node)` -> height, for the comments of a `Block::Thread`. Lent to the
    /// renderer for the frame (`take_comments`/`restore_comments`), because the block table
    /// and the comment table are read and written on opposite sides of one borrow.
    comments: HashMap<(usize, usize), f32>,
    /// `(block, row)` -> height, for the rows of a `Block::Entries`, on the same terms as
    /// [`Self::comments`] and for the same reason.
    ///
    /// A separate table rather than a shared one keyed by a tagged index: a block is a thread or
    /// a run of entries and never both, so nothing needs them merged, and two tables cannot
    /// collide on a key that means different things.
    entries: HashMap<(usize, usize), f32>,
}

impl Heights {
    /// Forget everything. A new page has its own blocks, and the old indices mean nothing.
    pub fn clear(&mut self) {
        self.layout = None;
        self.h.clear();
        self.comments.clear();
        self.entries.clear();
    }

    /// Point the table at the layout this frame is drawing under, clearing it if that differs
    /// from the one the heights in it were measured under.
    pub fn under(&mut self, layout: &Layout) {
        if self.layout.as_ref() != Some(layout) {
            self.layout = Some(layout.clone());
            self.h.clear();
            self.comments.clear();
            self.entries.clear();
        }
    }

    pub fn take_comments(&mut self) -> HashMap<(usize, usize), f32> {
        std::mem::take(&mut self.comments)
    }

    pub fn restore_comments(&mut self, comments: HashMap<(usize, usize), f32>) {
        self.comments = comments;
    }

    pub fn take_entries(&mut self) -> HashMap<(usize, usize), f32> {
        std::mem::take(&mut self.entries)
    }

    pub fn restore_entries(&mut self, entries: HashMap<(usize, usize), f32>) {
        self.entries = entries;
    }

    /// Forget one block's height, and the comment heights inside it.
    ///
    /// For a change that re-sizes one block and leaves the rest of the page alone, which in
    /// practice means an image arriving, failing, being dropped, or being evicted. Anything
    /// that would change what *every* block measures belongs in [`Layout`] instead, where it
    /// clears the table wholesale; the two are not interchangeable, and putting a local change
    /// in the key is what this method exists to undo.
    ///
    /// A block that has never been measured is not an error: an image can resolve in a block
    /// that has not been near the window since the page committed.
    pub fn forget(&mut self, i: usize) {
        if let Some(h) = self.h.get_mut(i) {
            *h = None;
        }
        // A thread is one block and many comments, and the picture is in one of them. Which
        // one is not worth tracking: a comment height is cheap to re-measure and the block
        // above it is being re-measured anyway.
        self.comments.retain(|(block, _), _| *block != i);
        // And the rows of a run of entries, for the same reason: a card's thumbnail arriving
        // re-sizes the row it is in.
        self.entries.retain(|(block, _), _| *block != i);
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
#[derive(Clone, Copy, Debug, PartialEq)]
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
            find: false,
            pending_link: None,
            collapsed: 0,
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

    /// The point of `forget`: one picture costs one block, not the page.
    #[test]
    fn a_loaded_image_forgets_only_its_own_block() {
        let mut h = Heights::default();
        h.under(&layout(600.0));
        h.set(3, 42.0);
        h.set(4, 17.0);
        h.forget(3);
        assert_eq!(h.get(3), None);
        assert_eq!(h.get(4), Some(17.0), "a neighbour was forgotten too");
    }

    /// A thread is one block and many comments, so forgetting the block has to take the
    /// comment heights inside it with it — and leave every other block's alone.
    #[test]
    fn forgetting_a_block_takes_its_comment_heights_and_no_others() {
        let mut h = Heights::default();
        h.under(&layout(600.0));
        let mut comments = h.take_comments();
        comments.insert((3, 0), 12.0);
        comments.insert((3, 9), 34.0);
        comments.insert((4, 0), 56.0);
        h.restore_comments(comments);
        h.forget(3);
        let left = h.take_comments();
        assert_eq!(left.get(&(3, 0)), None);
        assert_eq!(left.get(&(3, 9)), None);
        assert_eq!(left.get(&(4, 0)), Some(&56.0));
    }

    /// An image can resolve in a block that has not been laid out since the page committed.
    #[test]
    fn forgetting_an_unmeasured_block_is_not_an_error() {
        let mut h = Heights::default();
        h.under(&layout(600.0));
        h.set(1, 42.0);
        h.forget(0);
        h.forget(99);
        assert_eq!(h.get(1), Some(42.0));
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
        l.find = true;
        h.under(&l);
        h.set(3, 42.0);
        l.find = false;
        h.under(&l);
        assert_eq!(h.get(3), None);
    }

    /// `Shift+Z` changes the height of every thread on the page without laying one out.
    #[test]
    fn a_collapsed_thread_forgets_every_height() {
        let mut h = Heights::default();
        let mut l = layout(600.0);
        h.under(&l);
        h.set(3, 42.0);
        l.collapsed += 1;
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
