//! The egui view. Everything here needs a `Context`; everything that does not lives one
//! directory up, where `cargo test` can reach it.
//!
//! The directory is `ui/`, not `egui/`, so `use egui::…` stays unambiguous.

mod app;
mod blocks;
pub mod fonts;
pub mod images;
mod inline_ui;
mod menu_ui;
pub mod net;
mod notice_ui;
mod pageinfo_ui;
mod prefs_ui;
pub mod theme;
mod thread_ui;

use crate::reader::inline::{self, Run};
use crate::reader::opts::ReadOpts;
use crate::reader::settings::Settings;
use crate::reader::thread_tree::CommentKey;
use crate::reader::ui::theme::Palette;
use eframe::egui;
use std::collections::{HashMap, HashSet};
use url::Url;

/// What a click asked for. Widgets never navigate; they record an intent that `app` acts on
/// once per frame, which is what keeps the scheme gate and the history stack in one place.
#[derive(Debug, Clone)]
pub enum Action {
    Follow(String),
    /// The context menu's second item, and the bare reload's per-link equivalent.
    /// Follow with neither table applied: the page as the host sends it.
    FollowBare(String),
    Copy(String),
    /// The context menu's third item: mark this link to read later.
    ///
    /// It carries the link's words as well as its address, and it is the only `Action` that
    /// carries two strings. The reason is that the thing being filed has never been fetched, so
    /// there is no `<title>` anywhere to name it — the words in the link are all the name it will
    /// ever have, and they are in scope here and nowhere downstream.
    ReadLater {
        href: String,
        title: String,
    },
    /// The `remove` button on a row of `hww:reading-list`: take that one link off it.
    ///
    /// A separate `Action` from [`Action::ReadLater`] rather than the same one pressed on a link
    /// that is already listed, because the two mean different things. `ReadLater` is a toggle
    /// aimed at a link on someone else's page and cannot know which way it will go; this is a
    /// control drawn from an item hww holds, so the only thing pressing it can mean is "take it
    /// off", and a door that could still add would put back a row a stale frame had removed.
    ForgetNext(String),
    /// The reading list's own "forget all": empty it, as the settings panel's button does.
    ForgetAllNext,
    /// Load one image, named by its `src` as it appears in the IR.
    LoadImage(String),
    /// Jump to a block index: outline entries and resolved fragments.
    GoToBlock(usize),
    ToggleComment(CommentKey),
}

/// State threaded through block and inline rendering for one frame.
pub struct RenderCtx<'a> {
    pub pal: Palette,
    pub opts: &'a ReadOpts,
    /// The page being read, for resolving image `src` and naming hosts.
    pub base: Url,
    pub images: &'a mut images::ImageStore,
    pub collapsed: &'a HashSet<CommentKey>,

    /// Lowercased find query, empty when find is closed.
    pub find: String,
    /// Which match Enter and Shift+Enter step through, page-wide.
    ///
    /// Matches are counted in render order, so a collapsed subtree contributes none; find
    /// looks for what the reader is actually showing, which is also the only definition that
    /// can be scrolled to.
    pub find_current: usize,
    /// How many matches have been emitted so far this frame. After the frame it is the total.
    pub find_seen: usize,
    /// Match ranges within the run list currently being rendered, and the page-wide index of
    /// the first of them.
    find_ranges: Vec<(usize, usize)>,
    find_base: usize,
    /// Set when the current match lands in the job being built; consumed by the flush that
    /// emits it, which is the widget the find bar scrolls to.
    pub job_has_current_match: bool,
    /// Whether this frame should bring the current match into view. True only on the frame
    /// after the match moved; the highlight itself never steers the scroll.
    pub find_scroll: bool,

    pub action: Option<Action>,
    /// The link whose page is being fetched, marked in place by `inline_ui::link_ui`.
    ///
    /// An id, not an href: `html::link_block` copies a block-level anchor's href onto every
    /// inline in the block, and a front page repeats one href in nav, headline, and footer, so
    /// matching the string marks every occurrence rather than the one that was clicked.
    pub pending: Option<egui::Id>,
    /// The same mark for a navigation that began without a widget: Enter on a focused link,
    /// or the shot harness following one. An href rather than an id, so it marks every inline
    /// carrying that href, which is the trade the id avoids for a click and the right one when
    /// there was no click.
    pub pending_href: Option<String>,
    /// Which link the click this frame came from, read back by `app` once the action it queued
    /// has been applied. The counterpart to [`RenderCtx::pending`], one navigation earlier.
    pub clicked_link: Option<egui::Id>,
    pub hover_href: Option<String>,
    /// What Tab landed on this frame. Links, image placeholders, and comment toggles are the
    /// focusable things on the page; if plain text were focusable, Tab would walk every label.
    pub focus_href: Option<String>,
    /// The words of the link Tab landed on, taken beside [`RenderCtx::focus_href`].
    ///
    /// The keyboard route's half of what [`Action::ReadLater`] carries by hand. `Shift+L` happens
    /// in `handle_keys`, which is nowhere near the galley, so the text has to be picked up during
    /// the render like the href beside it or a reading list is a column of bare addresses.
    pub focus_text: Option<String>,
    pub focus_image: Option<String>,
    pub focus_comment: Option<CommentKey>,
    /// Focus is on a page widget that none of the three above describes (a button with no
    /// keyboard action of its own). `app` reads it only to keep that widget drawn: a focused
    /// widget that is not laid out loses its focus.
    pub focus_other: bool,
    /// The page being drawn is hww's own reading list, whose rows carry a control of their own.
    ///
    /// The renderer knows this one view by name because the control is not a general fact about a
    /// run of entries: a card on a front page and a search result are the page's rows, and hww has
    /// nothing to remove them from. These rows are items in `reader::archive`, and taking one off
    /// is the second half of the switch that put it there. `app` sets it from `builtin_view`, so
    /// no fetched page can turn it on.
    pub reading_list: bool,
    /// The window the reading column may skip outside of, or `None` when this block is laid
    /// out whole. Consulted by `thread_ui`, which skips comment by comment inside one block.
    pub band: Option<crate::reader::measure::Band>,
    /// Which top-level block is being drawn, the first half of a comment- or entry-height key,
    /// and the key `RenderCtx::threads` is looked up by.
    ///
    /// **`None` means "not a top-level block", and every consumer has to say what it does with
    /// that.** `blocks_ui` clears it for the length of a nested run, because the three things
    /// keyed by it are all keyed by *document position* and a nested run has none: `Block::List`
    /// items, `Block::Quote` bodies, `Entry::summary` and a comment's own blocks all recurse
    /// through it, and `html::summary_of` really does pass a `Block::Entries` into an
    /// `Entry::summary`. Left set, a nested run of entries would restart at `row = 0` and
    /// overwrite the outer run's heights under the outer run's key — which `remember` reads as a
    /// relayout and answers by throwing the whole table away, every frame. A nested thread would
    /// be handed the containing block's tree, or no tree at all.
    pub block: Option<usize>,
    /// Where `ImagePolicy::Auto` may fetch, or `None` under every other policy.
    ///
    /// Deliberately not [`RenderCtx::band`], which is `None` whenever a block is laid out
    /// whole and is therefore no answer to "is this near the window".
    pub autoload_band: Option<crate::reader::measure::Band>,
    /// The `src`s of unloaded pictures drawn inside [`RenderCtx::autoload_band`] this frame.
    ///
    /// Gathered where each picture draws rather than from the block that contains it. A feed is
    /// one `ir::Block::Entries` and a discussion is one `ir::Block::Thread`, so "which top-level
    /// blocks touch the band" answers *the whole page* for exactly the two shapes that carry
    /// the most pictures — which is the bound `reader::autoload` exists to keep.
    pub autoload_srcs: Vec<String>,
    /// Where a list row's site mark may be drawn from, set on every page whatever the policy.
    ///
    /// A second band rather than [`RenderCtx::autoload_band`], because the two answer different
    /// questions: that one is `None` unless the reader chose `ImagePolicy::Auto`, and a site
    /// mark is fetched under every policy but `NoRequests`, exactly as the masthead favicon is.
    /// A band at all, and not the whole document, because the bookmarks is one
    /// `ir::Block::Entries` that may hold two thousand rows — which is the bound `autoload`
    /// exists to keep, arrived at from the other direction.
    pub icon_band: Option<crate::reader::measure::Band>,
    /// The `src`s of unfetched site marks drawn inside [`RenderCtx::icon_band`] this frame.
    pub icon_srcs: Vec<String>,
    /// The comment heights for the frame, lent by `measure::Heights`.
    pub comment_heights: HashMap<(usize, usize), f32>,
    /// The entry-row heights for the frame, lent the same way.
    pub entry_heights: HashMap<(usize, usize), f32>,
    /// The comment tree of each `ir::Block::Thread`, by top-level block index, built once with
    /// the page. Borrowed rather than lent: unlike the height tables nothing writes it.
    pub threads: &'a HashMap<usize, crate::reader::thread_tree::ThreadTree>,
}

impl RenderCtx<'_> {
    pub fn new<'a>(
        pal: Palette,
        opts: &'a ReadOpts,
        base: Url,
        images: &'a mut images::ImageStore,
        collapsed: &'a HashSet<CommentKey>,
        threads: &'a HashMap<usize, crate::reader::thread_tree::ThreadTree>,
    ) -> RenderCtx<'a> {
        RenderCtx {
            pal,
            opts,
            base,
            images,
            collapsed,
            threads,
            find: String::new(),
            find_current: 0,
            find_seen: 0,
            find_ranges: Vec::new(),
            find_base: 0,
            job_has_current_match: false,
            find_scroll: false,
            action: None,
            pending: None,
            pending_href: None,
            clicked_link: None,
            hover_href: None,
            focus_href: None,
            focus_text: None,
            focus_image: None,
            focus_comment: None,
            focus_other: false,
            reading_list: false,
            band: None,
            block: None,
            autoload_band: None,
            autoload_srcs: Vec::new(),
            icon_band: None,
            icon_srcs: Vec::new(),
            comment_heights: HashMap::new(),
            entry_heights: HashMap::new(),
        }
    }

    /// Record an unloaded picture about to be drawn at `y`, if the automatic policy is on and
    /// `y` is inside its band.
    ///
    /// The top edge alone, because a placeholder's height is not known until it is drawn and
    /// every placeholder is a few lines tall. The band already reaches a full window past the
    /// viewport in both directions, which is far more than that error.
    ///
    /// Called by the three places an unloaded picture draws — `images::placeholder`,
    /// `images::inline_placeholder`, and `images::load_control` for a feed card — and by
    /// nothing else. A picture already loaded is not recorded: `autoload::plan` would drop it,
    /// and a page of loaded pictures should not walk a list of them every frame.
    pub fn note_unloaded_image(&mut self, y: f32, src: &str) {
        if let Some(band) = self.autoload_band
            && !band.skips(y, 0.0)
        {
            self.autoload_srcs.push(src.to_owned());
        }
    }

    /// Record a site mark about to be drawn at `y` that hww does not have yet.
    ///
    /// **The policy is not asked here.** It used to be, as a way of not building a list of hosts
    /// for a request nobody would make — and that stopped being harmless when the marks grew a
    /// store on disk: under `ImagePolicy::NoRequests` this returned before recording anything,
    /// so `ReaderApp::load_site_icons` was handed an empty list and the one policy that most
    /// wants a column drawn without contacting anybody was the one policy that could not have
    /// one. The question belongs to the door, which asks it in its own name, and which can now
    /// answer it differently for a mark that is already on this machine.
    ///
    /// Only a mark with no state at all is recorded. One that failed has a state, so a dead
    /// icon address is asked for once and not once per frame, which is the difference between
    /// a column with a hole in it and a host contacted sixty times a second.
    pub fn note_icon(&mut self, y: f32, src: &str) {
        if self.images.state(src).is_some() {
            return;
        }
        if let Some(band) = self.icon_band
            && !band.skips(y, 0.0)
        {
            self.icon_srcs.push(src.to_owned());
        }
    }

    /// First action wins. A frame in which two widgets both fire is a frame with a bug in it,
    /// and silently taking the last one would hide it.
    pub fn act(&mut self, action: Action) {
        if self.action.is_none() {
            self.action = Some(action);
        }
    }

    /// Find every match in one run list, before any of it is drawn.
    ///
    /// Matching runs over the *flattened* text, which is correct, because that is the text the
    /// reader shows, and the ranges are byte offsets into it. Highlighting then re-emits the
    /// affected segments with a per-match background, so a match that straddles an emphasis
    /// boundary (`the <em>quick</em> brown`) is painted as two ranges of one logical hit rather
    /// than being missed for spanning two labels.
    pub fn begin_list(&mut self, plain: &str) {
        self.find_ranges.clear();
        self.find_base = self.find_seen;
        if self.find.is_empty() {
            return;
        }
        self.scan(plain);
    }

    /// [`Self::begin_list`] for a run list whose plain text has not been built yet.
    ///
    /// Which is the point: `inline::plain_of` is a `String` per run plus the concatenation, and
    /// the caller was paying it on every run list of every page on every frame to hand it to a
    /// function that returns immediately unless the find bar is open. The bookkeeping above the
    /// early return still has to happen either way — `find_base` is what makes the
    /// current-match index in `split_by_matches` mean anything — so this is the same two lines
    /// and a build that is skipped, not a call that is.
    pub fn begin_runs(&mut self, runs: &[Run]) {
        self.find_ranges.clear();
        self.find_base = self.find_seen;
        if self.find.is_empty() {
            return;
        }
        self.scan(&inline::plain_of(runs));
    }

    /// Whether the run list `begin_list`/`begin_runs` last looked at holds any match.
    ///
    /// Asked before `split_by_matches`, which answers "no matches" with a `Vec` holding the
    /// whole text — an allocation per text run per frame for a page nobody is searching. Keyed
    /// on the ranges and not on the query: a live search that hit nothing in *this* list is the
    /// same single-section case.
    pub fn has_matches(&self) -> bool {
        !self.find_ranges.is_empty()
    }

    fn scan(&mut self, plain: &str) {
        let hay = plain.to_lowercase();
        // `to_lowercase` can change byte length, so a case-insensitive search over the folded
        // string cannot be trusted to give offsets into the original. When the fold is
        // length-preserving (which it is for the overwhelming majority of prose), use it;
        // otherwise fall back to an exact search, which is honest about what it can find.
        let (hay, needle) = if hay.len() == plain.len() {
            (hay, self.find.clone())
        } else {
            (plain.to_owned(), self.find.clone())
        };
        let mut from = 0usize;
        while let Some(at) = hay[from..].find(&needle) {
            let start = from + at;
            let end = start + needle.len();
            self.find_ranges.push((start, end));
            self.find_seen += 1;
            from = end.max(start + 1);
            if from >= hay.len() {
                break;
            }
        }
    }

    /// Split `text` at find-match boundaries.
    ///
    /// `(piece, Some(is_current))` for a matched piece, `(piece, None)` otherwise. `offset` is
    /// the byte position of `text` within the run list's plain text.
    pub fn split_by_matches<'t>(
        &self,
        text: &'t str,
        offset: usize,
    ) -> Vec<(&'t str, Option<bool>)> {
        if self.find_ranges.is_empty() || text.is_empty() {
            return vec![(text, None)];
        }
        let mut out = Vec::new();
        let mut cut = 0usize;
        for (i, (start, end)) in self.find_ranges.iter().enumerate() {
            let (start, end) = (*start, *end);
            if end <= offset || start >= offset + text.len() {
                continue;
            }
            let lo = start.saturating_sub(offset).min(text.len());
            let hi = (end - offset).min(text.len());
            if !(text.is_char_boundary(lo) && text.is_char_boundary(hi)) || hi <= cut {
                continue;
            }
            if lo > cut {
                out.push((&text[cut..lo], None));
            }
            out.push((&text[lo..hi], Some(self.find_base + i == self.find_current)));
            cut = hi;
        }
        if cut < text.len() {
            out.push((&text[cut..], None));
        }
        if out.is_empty() {
            out.push((text, None));
        }
        out
    }
}

/// Keep the widget Tab just landed on inside the window.
///
/// egui 0.36 scrolls to a focused widget for an AccessKit `ScrollIntoView` request and for
/// nothing else, so a Tab that walks off the bottom of the reading column is hww's to follow:
/// without this the ring leaves the window and the reader is left pressing a key that appears
/// to do nothing. Only on the frame focus arrives — repeating it every frame would pin the
/// page against the reader's own scrolling — and only for widgets *inside* the column, because
/// `scroll_to_me` posts a target that the next scroll area drawn will honour, and a chrome
/// widget posting one would scroll the page to a rect that is not on it.
pub fn follow_focus(resp: &egui::Response) {
    if resp.gained_focus() {
        resp.scroll_to_me(None);
    }
}

pub use app::{ReaderApp, run};

/// Launch arguments, kept out of `app` so `bin/hww.rs` stays a thin main.
pub struct Launch {
    pub start: Option<Url>,
    pub settings: Settings,
    pub settings_note: Option<String>,
    pub opts: crate::session::LoadOptions,
}
