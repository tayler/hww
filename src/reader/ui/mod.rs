//! The egui view. Everything here needs a `Context`; everything that does not lives one
//! directory up, where `cargo test` can reach it.
//!
//! The directory is `ui/`, not `egui/`, so `use egui::…` stays unambiguous.

mod app;
mod blocks;
pub mod images;
mod inline_ui;
pub mod net;
mod notice_ui;
mod pageinfo_ui;
pub mod theme;
mod thread_ui;

use crate::reader::opts::ReadOpts;
use crate::reader::settings::Settings;
use crate::reader::thread_tree::CommentKey;
use crate::reader::ui::theme::Palette;
use eframe::egui;
use std::collections::HashSet;
use url::Url;

/// What a click asked for. Widgets never navigate; they record an intent that `app` acts on
/// once per frame, which is what keeps the scheme gate and the history stack in one place.
#[derive(Debug, Clone)]
pub enum Action {
    Follow(String),
    /// The context menu's second item, and `R`'s per-link equivalent.
    FollowWithoutRewrite(String),
    Copy(String),
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
    pub line_height: f32,
    pub images: &'a mut images::ImageStore,
    pub collapsed: &'a HashSet<CommentKey>,

    /// Lowercased find query, empty when find is closed.
    pub find: String,
    /// Which match `n`/`N` is on, page-wide.
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
    /// emits it, which is the widget `n`/`N` scrolls to.
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
    /// Which link the click this frame came from, read back by `app` once the action it queued
    /// has been applied. The counterpart to [`RenderCtx::pending`], one navigation earlier.
    pub clicked_link: Option<egui::Id>,
    pub hover_href: Option<String>,
    /// What Tab landed on this frame. Links and comment toggles are the only focusable things
    /// on the page; if plain text were focusable, Tab would walk every label.
    pub focus_href: Option<String>,
    pub focus_image: Option<String>,
    pub focus_comment: Option<CommentKey>,
}

impl RenderCtx<'_> {
    pub fn new<'a>(
        pal: Palette,
        opts: &'a ReadOpts,
        base: Url,
        images: &'a mut images::ImageStore,
        collapsed: &'a HashSet<CommentKey>,
    ) -> RenderCtx<'a> {
        RenderCtx {
            pal,
            line_height: theme::line_height_px(opts),
            opts,
            base,
            images,
            collapsed,
            find: String::new(),
            find_current: 0,
            find_seen: 0,
            find_ranges: Vec::new(),
            find_base: 0,
            job_has_current_match: false,
            find_scroll: false,
            action: None,
            pending: None,
            clicked_link: None,
            hover_href: None,
            focus_href: None,
            focus_image: None,
            focus_comment: None,
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

pub use app::{ReaderApp, run};

/// Launch arguments, kept out of `app` so `bin/hww-gui.rs` stays a thin main.
pub struct Launch {
    pub start: Option<Url>,
    pub settings: Settings,
    pub settings_note: Option<String>,
    pub rewrite: bool,
}
