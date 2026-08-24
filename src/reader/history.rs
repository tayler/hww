//! Back and forward, over a stack with a cursor.
//!
//! In memory only, and gone with the process. An on-disk history is the archive's problem and
//! arrives with it; reading history at rest is a whole second doctrine, and none of the
//! reading experience depends on it.
//!
//! An entry remembers where the reader was in it. Back re-fetches — there is no document cache —
//! so the arriving page would otherwise open at the top, which is the one thing every other
//! browser does not do. The offset is in points, written every frame by the reader and read once
//! when the page commits.
//!
//! Entries carry an [`EntryId`] because an index is not a name. `go_back` moves the cursor and
//! only then dispatches the fetch, so for the length of that fetch the page on screen belongs to
//! the entry *ahead* of the cursor; a link followed in that window truncates the entry away and
//! puts a different one at the same index. Keyed by index, the outgoing page's offset would land
//! on the incoming page. Keyed by id, the write misses and stops, which is the answer.

use std::sync::atomic::{AtomicU64, Ordering};
use url::Url;

/// Names one entry for as long as it exists, and never names the entry that replaces it.
///
/// Monotonic for the process rather than for one [`History`], which matters because `present`
/// hands the reader a whole new stack: a counter that restarted would mint an id the reader may
/// still be holding from the stack before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntryId(u64);

impl EntryId {
    fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        EntryId(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

#[derive(Debug)]
struct Entry {
    id: EntryId,
    url: Url,
    /// Where the reading column was left, in points. Clamped by the scroll area on arrival,
    /// so a page that comes back shorter opens at its own bottom rather than off the end.
    offset: f32,
}

#[derive(Debug, Default)]
pub struct History {
    entries: Vec<Entry>,
    /// Index of the current entry. Meaningless when `entries` is empty.
    cursor: usize,
}

impl History {
    pub fn new() -> Self {
        Self::default()
    }

    fn mint(url: Url) -> Entry {
        Entry {
            id: EntryId::next(),
            url,
            offset: 0.0,
        }
    }

    /// Follow a link. Everything ahead of the cursor is discarded, the same rule every
    /// browser uses, and the reason forward is not a second stack.
    pub fn push(&mut self, url: Url) {
        // Re-navigating to where you already are is a reload, not a history entry; without
        // this, `r` on a page makes Back a no-op that looks broken. The entry keeps its id and
        // its offset, which is what makes a reload land where the reader was.
        if self.current() == Some(&url) {
            return;
        }
        let entry = Self::mint(url);
        self.entries.truncate(self.cursor + 1);
        if !self.entries.is_empty() {
            self.cursor += 1;
        }
        self.entries.push(entry);
        self.cursor = self.entries.len() - 1;
    }

    /// Replace the current entry rather than adding one.
    ///
    /// A navigation that never committed does not deserve a history entry. Once the outgoing
    /// page stays on screen during a fetch it also stays clickable, so following a link and then
    /// following a second one before the first has answered is ordinary rather than impossible;
    /// `push` on both leaves a middle entry for a page that was never rendered, and Back lands on
    /// it. No browser does that, and the reason is this method.
    ///
    /// Falls back to `push` when there is nothing to replace. A fresh id and a zero offset,
    /// because the entry is being handed to a different page than the one it was minted for.
    pub fn replace(&mut self, url: Url) {
        if self.entries.is_empty() {
            self.push(url);
            return;
        }
        let entry = Self::mint(url);
        self.entries.truncate(self.cursor + 1);
        self.entries[self.cursor] = entry;
    }

    pub fn current(&self) -> Option<&Url> {
        self.entries.get(self.cursor).map(|e| &e.url)
    }

    /// Which entry the cursor is on, for a caller that must still recognise it after the stack
    /// has moved under it.
    pub fn current_id(&self) -> Option<EntryId> {
        self.entries.get(self.cursor).map(|e| e.id)
    }

    /// Where the page in `id` was left. Zero for an entry that has been discarded, which reads
    /// as "the top" and is the right answer for a page nothing remembers.
    pub fn offset_of(&self, id: EntryId) -> f32 {
        self.entries
            .iter()
            .find(|e| e.id == id)
            .map_or(0.0, |e| e.offset)
    }

    /// Remember where `id` is being read. A discarded id and a non-finite offset are both
    /// ignored: the first is the whole point of ids, and the second would reach
    /// `ScrollArea::vertical_scroll_offset`, which panics on an infinity.
    pub fn set_offset(&mut self, id: EntryId, offset: f32) {
        if !offset.is_finite() {
            return;
        }
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            e.offset = offset;
        }
    }

    pub fn can_go_back(&self) -> bool {
        self.cursor > 0
    }

    pub fn can_go_forward(&self) -> bool {
        self.cursor + 1 < self.entries.len()
    }

    pub fn back(&mut self) -> Option<&Url> {
        if !self.can_go_back() {
            return None;
        }
        self.cursor -= 1;
        self.current()
    }

    pub fn forward(&mut self) -> Option<&Url> {
        if !self.can_go_forward() {
            return None;
        }
        self.cursor += 1;
        self.current()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn empty_history_goes_nowhere() {
        let mut h = History::new();
        assert!(h.current().is_none());
        assert!(h.back().is_none());
        assert!(h.forward().is_none());
    }

    #[test]
    fn walks_back_and_forward_over_a_chain() {
        let mut h = History::new();
        for s in ["https://a/", "https://b/", "https://c/"] {
            h.push(u(s));
        }
        assert_eq!(h.current(), Some(&u("https://c/")));
        assert_eq!(h.back(), Some(&u("https://b/")));
        assert_eq!(h.back(), Some(&u("https://a/")));
        assert!(!h.can_go_back());
        assert_eq!(h.forward(), Some(&u("https://b/")));
        assert_eq!(h.forward(), Some(&u("https://c/")));
        assert!(!h.can_go_forward());
    }

    #[test]
    fn following_a_link_after_going_back_discards_the_forward_branch() {
        let mut h = History::new();
        h.push(u("https://a/"));
        h.push(u("https://b/"));
        h.back();
        h.push(u("https://c/"));
        assert!(!h.can_go_forward());
        assert_eq!(h.len(), 2);
        assert_eq!(h.back(), Some(&u("https://a/")));
    }

    /// A reload must not make Back a no-op that looks broken.
    #[test]
    fn reloading_the_current_page_is_not_a_history_entry() {
        let mut h = History::new();
        h.push(u("https://a/"));
        h.push(u("https://b/"));
        h.push(u("https://b/"));
        assert_eq!(h.len(), 2);
        assert!(h.can_go_back());
    }

    /// Back, then forward to the same page, must not grow the stack either.
    #[test]
    fn revisiting_a_page_that_is_already_current_is_idempotent() {
        let mut h = History::new();
        h.push(u("https://a/"));
        h.push(u("https://b/"));
        h.back();
        h.push(u("https://a/"));
        assert_eq!(h.len(), 2);
        assert!(h.can_go_forward());
    }

    /// The reason [`History::replace`] exists. Following a second link before the first has
    /// answered must not leave an entry for a page that was never on screen.
    #[test]
    fn a_navigation_that_never_committed_leaves_no_entry() {
        let mut h = History::new();
        h.push(u("https://x/"));
        h.push(u("https://a/"));
        h.replace(u("https://b/"));
        assert_eq!(h.len(), 2);
        assert_eq!(h.current(), Some(&u("https://b/")));
        assert_eq!(h.back(), Some(&u("https://x/")));
    }

    #[test]
    fn replacing_an_empty_history_is_a_push() {
        let mut h = History::new();
        h.replace(u("https://a/"));
        assert_eq!(h.len(), 1);
        assert_eq!(h.current(), Some(&u("https://a/")));
    }

    /// `replace` truncates forward for the same reason `push` does: the branch ahead belonged
    /// to the entry being overwritten.
    #[test]
    fn replacing_after_going_back_discards_the_forward_branch() {
        let mut h = History::new();
        h.push(u("https://a/"));
        h.push(u("https://b/"));
        h.back();
        h.replace(u("https://c/"));
        assert!(!h.can_go_forward());
        assert_eq!(h.len(), 1);
        assert_eq!(h.current(), Some(&u("https://c/")));
    }

    // ------------------------------------------------------ where the reader was

    #[test]
    fn a_new_entry_starts_at_the_top() {
        let mut h = History::new();
        h.push(u("https://a/"));
        assert_eq!(h.offset_of(h.current_id().unwrap()), 0.0);
    }

    #[test]
    fn an_entry_remembers_where_it_was_left() {
        let mut h = History::new();
        h.push(u("https://a/"));
        let a = h.current_id().unwrap();
        h.set_offset(a, 420.0);
        h.push(u("https://b/"));
        let b = h.current_id().unwrap();
        assert_eq!(h.offset_of(b), 0.0, "a fresh entry opens at the top");
        h.back();
        assert_eq!(h.current_id(), Some(a));
        assert_eq!(h.offset_of(a), 420.0);
        h.forward();
        assert_eq!(h.offset_of(b), 0.0);
    }

    /// Going one way must not disturb the entry at the other end, or forward has nothing to
    /// restore.
    #[test]
    fn both_directions_keep_their_own_position() {
        let mut h = History::new();
        h.push(u("https://a/"));
        let a = h.current_id().unwrap();
        h.push(u("https://b/"));
        let b = h.current_id().unwrap();
        h.set_offset(a, 500.0);
        h.set_offset(b, 300.0);
        h.back();
        assert_eq!(h.offset_of(a), 500.0);
        h.set_offset(a, 100.0);
        h.forward();
        assert_eq!(h.offset_of(b), 300.0, "back overwrote the entry ahead");
        h.back();
        assert_eq!(h.offset_of(a), 100.0);
    }

    #[test]
    fn going_back_does_not_clear_the_entry_ahead() {
        let mut h = History::new();
        h.push(u("https://a/"));
        h.push(u("https://b/"));
        let b = h.current_id().unwrap();
        h.set_offset(b, 250.0);
        h.back();
        assert_eq!(h.offset_of(b), 250.0);
    }

    /// The entry is being handed to a page it was not minted for, so it is a different entry.
    #[test]
    fn replacing_an_entry_starts_the_replacement_at_the_top() {
        let mut h = History::new();
        h.push(u("https://a/"));
        let a = h.current_id().unwrap();
        h.set_offset(a, 700.0);
        h.replace(u("https://b/"));
        let b = h.current_id().unwrap();
        assert_ne!(a, b);
        assert_eq!(h.offset_of(b), 0.0);
    }

    /// The reload case `push`'s early return covers: `r` keeps your place.
    #[test]
    fn re_pushing_the_current_url_keeps_its_offset() {
        let mut h = History::new();
        h.push(u("https://a/"));
        let a = h.current_id().unwrap();
        h.set_offset(a, 610.0);
        h.push(u("https://a/"));
        assert_eq!(h.current_id(), Some(a));
        assert_eq!(h.offset_of(a), 610.0);
    }

    /// Why entries are named rather than indexed. The page on screen during a Back belongs to
    /// the entry *ahead* of the cursor; following a link truncates it away and puts a different
    /// entry at the same index. An index would write the outgoing page's offset onto the
    /// incoming one.
    #[test]
    fn a_discarded_entry_stops_taking_an_offset() {
        let mut h = History::new();
        h.push(u("https://a/"));
        h.push(u("https://b/"));
        let b = h.current_id().unwrap();
        h.back();
        h.push(u("https://c/"));
        let c = h.current_id().unwrap();
        assert_ne!(b, c);
        h.set_offset(b, 500.0);
        assert_eq!(
            h.offset_of(c),
            0.0,
            "c took b's index and b's offset with it"
        );
        assert_eq!(h.offset_of(b), 0.0, "a discarded entry has no position");
    }

    /// `vertical_scroll_offset` panics on an infinity, and a `NaN` would stick.
    #[test]
    fn a_non_finite_offset_is_ignored() {
        let mut h = History::new();
        h.push(u("https://a/"));
        let a = h.current_id().unwrap();
        h.set_offset(a, 90.0);
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            h.set_offset(a, bad);
            assert_eq!(h.offset_of(a), 90.0);
        }
    }

    #[test]
    fn an_offset_for_an_unknown_entry_is_ignored() {
        let mut h = History::new();
        h.push(u("https://a/"));
        let a = h.current_id().unwrap();
        let mut other = History::new();
        other.push(u("https://z/"));
        let stranger = other.current_id().unwrap();
        assert_ne!(a, stranger, "ids must not restart with a new stack");
        h.set_offset(stranger, 800.0);
        assert_eq!(h.len(), 1);
        assert_eq!(h.offset_of(a), 0.0);
    }
}
