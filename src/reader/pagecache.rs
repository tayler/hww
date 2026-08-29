//! The documents Back and Forward can open without asking the site again.
//!
//! # Why this is not a field on `history::Entry`
//!
//! Three reasons, in ascending order of how much they cost to ignore. `session::Loaded` has no
//! derives at all, so a page inside `Entry` takes `#[derive(Debug)]` off both `Entry` and
//! [`History`](crate::reader::history::History), whose twenty tests read `assert_eq!` output.
//! `history` is compiled unconditionally, so a reader page inside it drags `egui` into the
//! fast CI job or puts back and forward behind `gui`, and the default job stops testing them
//! either way. And the two lifetimes differ: an entry dies only by truncation, a kept page dies
//! by truncation *or* by budget, so `History::push` would be performing byte arithmetic, which
//! is not what "back and forward, over a stack with a cursor" means.
//!
//! The payload is therefore generic. The caller says how big a page is, which keeps
//! `ir::Document` out of here and lets the budget be tested with a `&'static str`.
//!
//! # What bounds it
//!
//! The reachable stack, first and mostly: [`PageCache::retain`] drops every page whose entry
//! `History` no longer holds, so a session's cache is a session's back stack. [`BUDGET`] is the
//! second bound, for the document that is not ordinary. [`stash_under`] is the third half of the
//! rule and the one that runs first: it decides which entry a page leaving the column is kept
//! for, and answers `None` for a reload, which navigates over the entry it is already on.
//!
//! Eviction is in insertion order, and here that *is* an LRU: nothing reads a page without
//! removing it, so recency is insertion order, and a page put back on screen and later left
//! again is re-inserted at the back. A `touch` would be dead code.
//!
//! # Why the options have to match
//!
//! The cache may only answer with the page the fetch it replaced would have produced. A bare
//! reload reads a page with both builtin tables off; handing that document back on an ordinary Back
//! would show the reader a page they did not ask for and could not tell apart. The rule is one
//! equality test at [`PageCache::take`] rather than a special case at each call site.
//!
//! # Why pictures are not kept
//!
//! `ImageStore::clear_page` drops textures and the per-page disclosure counters when a page
//! commits, and both are right to die with the page they account for. A restored page therefore
//! comes back with placeholders, and the reader loads them by click, `i`, or `Shift+I` as on any
//! page — which is what the image charter requires and what keeps a Back from silently
//! re-contacting every host on the page. `ImageStore::dims` outlives `clear_page`, so the
//! placeholders reserve the boxes the pictures had and nothing reflows.

use crate::reader::history::EntryId;
use crate::session::LoadOptions;

/// Which entry the page leaving the column should be kept under, if any.
///
/// Not simply the entry the outgoing page occupies. A reload navigates over that same entry, so
/// keeping it there hands a later Back the document the reload replaced: the entry has no second
/// page to come back to and must hold none. `cursor` is where the arriving page will land, which
/// during a Back is already the entry *ahead* of the one on screen.
///
/// Here rather than beside `ReaderApp::commit`, its one caller: it is arithmetic on two ids,
/// needs no `egui::Context`, and is the cache's own filing rule. Its failure is invisible to the
/// reader directory — filing the outgoing page under the entry the arriving one is about to
/// occupy compiles, photographs identically, and shows up only as a Back several navigations
/// later that opens a document the reader replaced.
pub fn stash_under(shown: Option<EntryId>, cursor: Option<EntryId>) -> Option<EntryId> {
    shown.filter(|id| Some(*id) != cursor)
}

/// Visible text held across the whole cache, in [`ir::Document::text_len`] characters.
///
/// The reachable stack is the real bound; this is the guard against the document that is not
/// ordinary. `fetch::MAX_BYTES` caps one document's *source* at 5 MB and extracted text is a
/// subset of that, so eight million characters is a stack of several near-maximal documents and
/// never binds on ordinary reading. Past it the most recent survive and the rest are fetched
/// again, which is what every Back did before this module existed.
///
/// Characters rather than bytes because `Document::text_len` is this crate's one length
/// currency (`ir::THIN_TEXT`, `thread::comments_text_len`), and one number to be wrong about
/// beats two. It is a proxy for memory, not a measure of it: the `String` per inline run, the
/// hrefs, the block vectors, and the outline are all outside it, so resident cost is some
/// multiple of this and the ceiling is tens of megabytes rather than eight. That is the reason
/// to set it well under what the reader could actually afford.
///
/// [`ir::Document::text_len`]: crate::ir::Document::text_len
pub const BUDGET: usize = 8_000_000;

struct Cached<T> {
    id: EntryId,
    /// What the page was fetched under. See the module doc: the cache may only answer with the
    /// page the fetch it replaced would have produced.
    opts: LoadOptions,
    cost: usize,
    page: T,
}

/// Pages kept for the entries a back stack can still return to. Oldest first.
pub struct PageCache<T> {
    entries: Vec<Cached<T>>,
}

/// By hand rather than derived: `#[derive(Default)]` adds a `T: Default` bound, and the payload
/// this is instantiated with is a boxed page that has no default.
impl<T> Default for PageCache<T> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}

impl<T> PageCache<T> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Keep `page` for `id`, replacing whatever was held for it.
    ///
    /// Refuse before evicting. A page bigger than the whole budget cannot be held whatever else
    /// goes, so evicting the rest of the cache on its way to dropping it too would spend every
    /// other page for nothing.
    pub fn insert(&mut self, id: EntryId, page: T, cost: usize, opts: LoadOptions) {
        // Unconditional: a replacement counts once, and an entry too big to keep must not be
        // left holding its previous, smaller document under the same id.
        self.pull(id);
        if cost > BUDGET {
            return;
        }
        self.entries.push(Cached {
            id,
            opts,
            cost,
            page,
        });
        while self.held() > BUDGET {
            // `held() > BUDGET` means at least one entry, and the push above guarantees it.
            self.entries.remove(0);
        }
    }

    /// Take the entry held for `id` out of the cache.
    ///
    /// The one place an entry leaves by name, so [`PageCache::insert`]'s replace step and
    /// [`PageCache::take`] cannot disagree about what removing one means.
    fn pull(&mut self, id: EntryId) -> Option<Cached<T>> {
        let at = self.entries.iter().position(|e| e.id == id)?;
        Some(self.entries.remove(at))
    }

    /// Hand back the page kept for `id`, if it is held and was fetched under `opts`.
    ///
    /// The entry is removed either way. `ReaderApp::opts` is fixed for the session, so the only
    /// caller that can ever ask for this id asks with the same options every time: a page filed
    /// under others can never be handed out, and keeping it only spends budget.
    pub fn take(&mut self, id: EntryId, opts: LoadOptions) -> Option<T> {
        let got = self.pull(id)?;
        (got.opts == opts).then_some(got.page)
    }

    /// Drop every page whose entry the stack no longer holds.
    ///
    /// A predicate rather than the list: disjoint field captures let the caller write
    /// `self.pages.retain(|id| self.history.holds(id))`, so the stack answers for one id at a
    /// time and neither side has to build a `Vec` of the whole thing.
    pub fn retain(&mut self, live: impl Fn(EntryId) -> bool) {
        self.entries.retain(|e| live(e.id));
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Characters currently held: what [`BUDGET`] is spent against, and what the tests assert.
    ///
    /// Summed rather than carried as a running total. The cache is bounded by the back stack, so
    /// this is a walk of tens of `usize`s, and the one arithmetic error this module could make —
    /// a total that drifts from what is actually held — becomes unrepresentable.
    pub fn held(&self) -> usize {
        self.entries.iter().map(|e| e.cost).sum()
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
    use crate::reader::history::History;
    use url::Url;

    /// A `&'static str` payload, so these exercise the genericity and nothing here needs a
    /// document, a window, or the `gui` feature to be wrong in an interesting way.
    fn stack(urls: &[&str]) -> History {
        let mut h = History::new();
        for u in urls {
            h.push(Url::parse(u).expect("a fixture URL parses"));
        }
        h
    }

    /// The stack and the ids it minted, in order. `History` names only the entry under the
    /// cursor, so a test wanting to file a page under each entry collects them as they are made.
    fn stack_ids(urls: &[&str]) -> (History, Vec<EntryId>) {
        let mut h = History::new();
        let mut ids = Vec::new();
        for u in urls {
            h.push(Url::parse(u).expect("a fixture URL parses"));
            ids.push(id(&h));
        }
        (h, ids)
    }

    fn id(h: &History) -> EntryId {
        h.current_id().expect("a pushed stack has a current entry")
    }

    #[test]
    fn a_page_comes_back_out_under_the_entry_it_was_kept_for() {
        let h = stack(&["https://a/"]);
        let a = id(&h);
        let mut c = PageCache::new();
        c.insert(a, "page a", 10, LoadOptions::default());
        assert_eq!(c.take(a, LoadOptions::default()), Some("page a"));
        assert_eq!(c.take(a, LoadOptions::default()), None, "taken, not copied");
        assert_eq!(c.held(), 0);
    }

    /// The rule the module doc states: only the page the fetch it replaced would have produced.
    /// A bare reload reads a page with both tables off, and an ordinary Back must not be handed
    /// it.
    #[test]
    fn a_page_kept_under_other_options_is_not_handed_back() {
        let h = stack(&["https://a/"]);
        let a = id(&h);
        let mut c = PageCache::new();
        c.insert(a, "bare", 10, LoadOptions::BARE);
        assert_eq!(c.take(a, LoadOptions::default()), None);
        assert!(
            c.is_empty(),
            "and it is dropped rather than left to be asked for again"
        );
        assert_eq!(c.held(), 0);
    }

    #[test]
    fn the_oldest_page_goes_when_the_budget_is_spent() {
        let (_h, ids) = stack_ids(&["https://a/", "https://b/", "https://c/"]);
        let mut c = PageCache::new();
        for (i, e) in ids.iter().enumerate() {
            c.insert(*e, "page", BUDGET / 2, LoadOptions::default());
            assert!(c.held() <= BUDGET, "over budget after {i} inserts");
        }
        assert_eq!(c.len(), 2);
        assert!(c.take(ids[0], LoadOptions::default()).is_none());
        assert!(c.take(ids[2], LoadOptions::default()).is_some());
    }

    /// Refuse before evicting. A page nothing can hold must not cost the pages that fit.
    #[test]
    fn a_page_larger_than_the_whole_budget_is_not_kept() {
        let (_h, ids) = stack_ids(&["https://a/", "https://b/"]);
        let mut c = PageCache::new();
        c.insert(ids[0], "small", 100, LoadOptions::default());
        c.insert(ids[1], "enormous", BUDGET + 1, LoadOptions::default());
        assert_eq!(c.len(), 1);
        assert_eq!(c.held(), 100);
        assert_eq!(c.take(ids[0], LoadOptions::default()), Some("small"));
    }

    /// Re-inserting must replace rather than accumulate, or the budget counts one page twice
    /// and the older copy can still be handed out.
    #[test]
    fn keeping_the_same_entry_twice_counts_it_once() {
        let h = stack(&["https://a/"]);
        let a = id(&h);
        let mut c = PageCache::new();
        c.insert(a, "first", 100, LoadOptions::default());
        c.insert(a, "second", 250, LoadOptions::default());
        assert_eq!(c.len(), 1);
        assert_eq!(c.held(), 250);
        assert_eq!(c.take(a, LoadOptions::default()), Some("second"));
    }

    /// An entry too big to keep must not leave its previous, smaller document behind under the
    /// same id: a Back would then open the page before the one the reader last read.
    #[test]
    fn a_refused_page_does_not_leave_the_one_it_replaces() {
        let h = stack(&["https://a/"]);
        let a = id(&h);
        let mut c = PageCache::new();
        c.insert(a, "old", 100, LoadOptions::default());
        c.insert(a, "enormous", BUDGET + 1, LoadOptions::default());
        assert!(c.is_empty());
        assert_eq!(c.held(), 0);
    }

    #[test]
    fn a_page_whose_entry_the_stack_dropped_is_not_kept() {
        let (mut h, ids) = stack_ids(&["https://a/", "https://b/"]);
        let mut c = PageCache::new();
        for e in &ids {
            c.insert(*e, "page", 100, LoadOptions::default());
        }
        h.back();
        h.push(Url::parse("https://c/").expect("a fixture URL parses"));
        c.retain(|id| h.holds(id));
        assert_eq!(c.len(), 1, "b's entry was truncated away");
        assert_eq!(c.held(), 100);
        assert!(c.take(ids[1], LoadOptions::default()).is_none());
    }

    /// `present` hands the reader a whole new stack. Ids are monotonic for the process, so
    /// nothing collides; what would go wrong is the budget carrying pages nothing can reach.
    #[test]
    fn an_id_from_a_stack_that_no_longer_exists_names_nothing() {
        let h = stack(&["https://a/"]);
        let a = id(&h);
        let mut c = PageCache::new();
        c.insert(a, "page a", 100, LoadOptions::default());
        let fresh = stack(&["https://a/"]);
        assert_ne!(a, id(&fresh), "ids must not restart with a new stack");
        assert!(c.take(id(&fresh), LoadOptions::default()).is_none());
        c.retain(|id| fresh.holds(id));
        assert!(c.is_empty());
        assert_eq!(c.held(), 0);
    }

    // ------------------------------------------------- which entry a page is kept for

    /// The rule `stash_under` exists for. A reload navigates over the entry the page on screen
    /// already occupies, and filing the outgoing page there would hand a later Back the document
    /// the reload replaced.
    #[test]
    fn a_reload_keeps_no_page_for_the_entry_it_navigates_over() {
        let mut h = stack(&["https://a/"]);
        let a = h.current_id();

        // A reload: the cursor never leaves the entry the page on screen occupies.
        assert_eq!(stash_under(a, a), None);

        // A link followed: the cursor has moved on, and the page being left is worth keeping.
        h.push(Url::parse("https://b/").expect("a fixture URL parses"));
        assert_eq!(stash_under(a, h.current_id()), a);

        // A Back: the cursor moved at dispatch, so the page on screen is the entry ahead.
        let b = h.current_id();
        h.back();
        assert_eq!(stash_under(b, h.current_id()), b);
    }

    /// An error screen and an empty stack both leave the reader with no entry on screen, and
    /// there is no page to file for either.
    #[test]
    fn a_page_with_no_entry_is_kept_for_nothing() {
        let h = stack(&["https://a/"]);
        assert_eq!(stash_under(None, None), None);
        assert_eq!(stash_under(None, h.current_id()), None);
    }

    #[test]
    fn clearing_forgets_everything_and_its_cost() {
        let (_h, ids) = stack_ids(&["https://a/", "https://b/"]);
        let mut c = PageCache::new();
        for e in ids {
            c.insert(e, "page", 100, LoadOptions::default());
        }
        c.clear();
        assert!(c.is_empty());
        assert_eq!(c.held(), 0);
    }
}
