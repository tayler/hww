//! Texture upload and the LRU. **No decoding happens here**: that is
//! `reader::image_decode`, deliberately outside this directory so it is tested rather than
//! merely compiled.
//!
//! Nothing in this module touches disk, and that line is still exact now that something does.
//! The store is `reader::iconcache`, outside `ui/` where the fast CI job tests it, and this
//! module has no idea it exists: `ImageStore` still holds textures that die with the process,
//! `ReaderApp` decides what may be drawn from a file and what may be written to one, and a
//! worker does the reading. What arrived with the archive is the *rule* about which bytes may be
//! kept, and it is kept in one place rather than spread across a texture cache.
//!
//! # The placeholder is the whole privacy argument
//!
//! Article images live on CDN hosts constantly, while README promises no automatic third-party
//! requests. Loading one is allowed on exactly the reasoning `rewrite` uses: the placeholder
//! **names the host before the click**, the click is a
//! deliberate user action, the load is counted in the provenance strip, and the capability
//! lives behind the `gui` feature, so core tests and local tools retain compile-time absence
//! of it.
//!
//! Article images are never prefetched and never loaded on hover. They are auto-loaded under
//! exactly one policy, `ImagePolicy::Auto`, which a reader has to choose: the default still
//! waits for a click. That policy changes who presses the button and nothing else — the same
//! choke point, the same host named before the bytes, the same counters — and it stays bounded
//! by the layout band, so it is a screenful of requests rather than a document of them.
//!
//! The page favicon beside the masthead eyebrow is automatic under every policy but
//! `NoRequests`: that is chrome identity, fetched as soon as the document names a URL (or the
//! well-known `/favicon.ico` fallback), counted the same way, and omitted from the column until
//! the bytes decode.
//!
//! It is also the one picture in the program that may come off the disk rather than off a host,
//! for a site the archive already names — see `reader::iconcache`, which argues why that is the
//! opposite of a privacy cost. A mark read from there reaches [`ImageStore::insert`] with
//! `from_cache` set, so it moves `icons_from_cache` and never `loaded`: the panel prints
//! `loaded` beside a list of hosts, and a picture that contacted nobody counted there would have
//! it report images loaded from no host at all.
//!
//! The bookmarks' left column is the same exception with the same policy, one page over
//! ([`site_icon`]): each row's mark is the identity of the site that row leads to, stored when
//! the reader kept the page and fetched when the row is drawn. It is the one automatic request
//! that is *not* about the page on screen, which is why it is bounded twice — only the bookmarks
//! draws the column (`archive::Mark`), and only the rows inside the layout band are asked for
//! (`RenderCtx::note_icon`). Page info reports the hosts, on a page that otherwise reports no
//! request at all.
//!
//! # Two classes, and why the page's cache has something in it that is not the page's
//!
//! Everything here is a picture and most of them belong to the page on screen, arriving with it
//! and going with it. Site marks do not. A mark belongs to a *list* — the bookmarks, the reading
//! list, the sidebar down the left edge — and those surfaces outlive any page: the sidebar in
//! particular is up while the reader navigates under it. So a mark is filed apart by
//! [`ImageStore::note_mark`] and treated differently in exactly two places, both of which are
//! about that difference and neither of which is about the bytes.
//!
//! [`ImageStore::clear_page`] keeps a settled mark, because dropping it would blank the strip on
//! every navigation and rebuild it from the disk. [`MARK_CAPACITY`] evicts marks against a
//! ceiling of their own, because the two classes share one recency queue and a page of
//! photographs would otherwise push out every mark on screen — and then be asked for again, and
//! evicted again, for as long as the panel is open. Recency is the right question to ask of a
//! picture the reader scrolled past and the wrong one to ask of a mark that is on screen because
//! a panel is.
//!
//! What does *not* change is the account: the counters are the page's and are reset with it, and
//! a surviving texture makes no request, so it cannot be counted twice.
//!
//! # Which heights an image invalidates
//!
//! Every state transition here re-sizes the block the picture sits in, so [`ImageStore`] keeps
//! the `src`s whose state moved rather than a change counter. A counter meant one arrival threw
//! away the height of every block on the page (`measure::Heights`), which is affordable at one
//! click and quadratic at thirty; the list lets `ReaderApp` forget the blocks that actually
//! moved. Every writer of `entries` goes through [`ImageStore::set`] or names itself in
//! [`ImageStore::take_changed`]'s contract, so a new call site cannot forget to record.

use crate::ir;
use crate::reader::image_decode::{DecodeError, Decoded};
use crate::reader::image_formats::declined_by_url;
use crate::reader::pageinfo::Counts;
use crate::reader::ui::{Action, RenderCtx, theme};
use eframe::egui::{self, RichText, TextureHandle, Ui};
use std::collections::{HashMap, HashSet};

/// Textures held at once.
///
/// Bounded because a photo-heavy page that filled an unbounded cache would be a GPU-memory
/// problem driven by untrusted input, but not so tight that `I` on a page with thirty images
/// silently reverts the first ones to placeholders while the reader is still scrolling toward
/// them. Each texture is already downscaled to the reading column, so the ceiling is tens of
/// megabytes, not hundreds.
const LRU_CAPACITY: usize = 48;

/// How many settled declines `evict` will hold past [`LRU_CAPACITY`] before dropping the oldest
/// anyway.
///
/// Each one is a `src` and a sentence, not a texture, so the ceiling is about the queue rather
/// than about memory: without it a page of unloadable pictures grows `order` without bound and
/// the cap stops being a cap. One page's worth of them, so the answer survives a page that is
/// nothing but declines and `clear_page` still takes them all at the next commit.
const DECLINES_KEPT: usize = LRU_CAPACITY;

/// Textures held for site marks, budgeted apart from [`LRU_CAPACITY`].
///
/// A separate ceiling and not a share of the same one, because the two compete on a page where
/// the reader can least afford it. The bookmarks sidebar is on screen for the whole session while
/// article pictures churn under it, and one recency-ordered queue would let a photo-heavy page
/// evict every mark in the strip: `RenderCtx::note_icon` would record each evicted mark again,
/// the strip would re-read them off the disk, and the page's next pictures would evict them
/// again. Recency is the wrong question to ask of a mark, which is on screen because a panel is
/// open rather than because it was scrolled past.
///
/// Sized to a screenful and a bit: the strip only ever asks for the rows in its band, so this is
/// the working set with room for the reader to scroll and come back.
const MARK_CAPACITY: usize = 64;

pub enum State {
    Offered,
    Loading(Source),
    Ready(Ready),
    Failed(Failure),
}

/// Where the bytes of a pending image are coming from.
///
/// The distinction exists for one number: `autoload::MAX_OUTSTANDING` bounds how many *hosts*
/// may be mid-request at once, and a site mark being read out of `reader::iconcache` contacts
/// none. Counting a disk read against that ceiling made a bookmarks list of thirty rows draw
/// itself eight rows a frame for no reason at all — the throttle is about connections, and a
/// read from this machine is not one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Network,
    Disk,
}

/// Which ceiling an entry is evicted against.
///
/// Not a property of the bytes — a mark and a thumbnail are the same kind of picture — but of
/// what put it on screen and what takes it off. See [`MARK_CAPACITY`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Picture,
    Mark,
}

/// Why a picture is not on screen, and whether asking again could change that.
///
/// The reader draws its one control on this. A retry offered for a decline that is a property
/// of the file, or of the address, is a button that re-fetches and re-fails for as long as the
/// page is open, and there was no way to say so while this was a bare `String`.
///
/// Built through the two constructors rather than a public flag, so every call site says which
/// kind it means at the point it knows, and asked as a named question rather than by reading
/// the flag back out.
pub struct Failure {
    pub why: String,
    retryable: bool,
}

impl Failure {
    /// A second request cannot change this: a declined format, an address that will not
    /// resolve, a file too big to decode.
    pub fn permanent(why: String) -> Self {
        Self {
            why,
            retryable: false,
        }
    }

    /// A second request might succeed: the transport failed, or the bytes were cut short.
    pub fn transient(why: String) -> Self {
        Self {
            why,
            retryable: true,
        }
    }

    pub fn offers_retry(&self) -> bool {
        self.retryable
    }
}

/// What one queued request added to the disclosure counters, so it can be taken back if the
/// request turns out never to have been made.
struct Disclosed {
    host: String,
    referer: bool,
    /// A site mark rather than a picture in the page, so [`ImageStore::forget`] knows which
    /// counter to take back. Without it a dropped mark would leave `icons_requested` claiming a
    /// host was asked for an icon that was never asked for.
    icon: bool,
}

pub struct Ready {
    pub texture: TextureHandle,
    pub width: u32,
    pub height: u32,
    pub source_width: u32,
    pub source_height: u32,
    /// Set when the bytes were incomplete. Always shown: a half-gray photograph the user
    /// cannot tell from the real one is worse than no photograph.
    pub partial: Option<String>,
}

#[derive(Default)]
pub struct ImageStore {
    entries: HashMap<String, State>,
    /// Most-recently-touched last.
    order: Vec<String>,
    /// Decoded dimensions per `src`, kept across navigation so a revisit reserves the right
    /// box before the bytes arrive. Cheap, and it is the only "layout reservation" available
    /// without putting dimensions in `ir::Image`, which is contract creep the charter guards.
    dims: HashMap<String, (u32, u32)>,
    /// Hosts contacted for images on the page being read and how many requests stand against
    /// each, for the status strip and the page-info panel.
    ///
    /// Counted rather than a set because a request can be taken back: a job the pool dropped
    /// without dispatching contacted nobody, and a set could not tell "the only request to this
    /// host was never made" from "one of four was".
    hosts: HashMap<String, usize>,
    /// What each outstanding request has already added to the disclosures above.
    ///
    /// The disclosure is recorded when the job is queued, because that is where the host is
    /// known and where the reader is told about it, and a job can still be dropped after that
    /// (`net::Msg::ImageDropped`). This is what lets that be undone exactly, instead of by
    /// subtracting one and hoping. Cleared without undoing when a page commits, along
    /// with the counters themselves: the account is about the page on screen.
    in_flight: HashMap<String, Disclosed>,
    pub loaded: usize,
    /// The `src`s whose state has moved since the reading column last looked. The column caches
    /// a height per block (`reader::measure`), and an image that arrives, fails, is dropped, or
    /// is evicted re-sizes the block it sits in — including blocks nowhere near the window,
    /// which is why the column cannot discover this by drawing.
    changed: Vec<String>,
    pub cookie_attempts: usize,
    pub referer_requests: usize,
    /// Site marks drawn from `reader::iconcache`, counted when the texture lands rather than
    /// when the read is queued: a read that finds the file gone becomes an ordinary request, and
    /// counting it here first would need taking back at the other end.
    pub icons_from_cache: usize,
    /// Site marks asked of a host, counted beside `hosts` in `record_request`'s own call — where
    /// the request is disclosed — so the two can never disagree about whether one went out.
    pub icons_requested: usize,
    /// Which `src`s are site marks rather than pictures in a page.
    ///
    /// Two things hang off this and both are about a surface that outlives the page: the marks a
    /// list or the sidebar drew survive [`Self::clear_page`], and they are evicted against
    /// [`MARK_CAPACITY`] rather than against the pictures' [`LRU_CAPACITY`].
    ///
    /// Recorded by the caller through [`Self::note_mark`] rather than inferred from anything
    /// here, for the reason `ui::app::Mark` exists: whether an address is a mark is a question
    /// about the archive, and this module cannot see it.
    marks: HashSet<String>,
}

impl ImageStore {
    pub fn state(&self, src: &str) -> Option<&State> {
        self.entries.get(src)
    }

    pub fn reserved(&self, src: &str) -> Option<(u32, u32)> {
        self.dims.get(src).copied()
    }

    /// The `src`s whose state has moved since the last call, drained.
    ///
    /// Called once a frame by the reading column, which turns each one into the blocks that
    /// contain it and forgets their remembered heights. Draining rather than reading is what
    /// keeps the list from growing for the length of a session.
    pub fn take_changed(&mut self) -> Vec<String> {
        std::mem::take(&mut self.changed)
    }

    pub fn begin(&mut self, src: &str, source: Source) {
        self.set(src, State::Loading(source));
        self.touch(src);
        self.evict();
    }

    /// Record that `src` is a site mark and not a picture in a page.
    ///
    /// Call it before the state is set, at every door that knows the answer — which is every
    /// caller holding a `ui::app::Mark` other than `Mark::No`, the settled `Known::Nothing`
    /// refusal included, because that one never reaches [`Self::begin`] at all and would
    /// otherwise be filed and evicted as a picture.
    pub fn note_mark(&mut self, src: &str) {
        self.marks.insert(src.to_owned());
    }

    /// The only writer that *adds* to `entries`, so the record cannot be forgotten at a new
    /// call site: the field doc promises every change is recorded, and this is where that is
    /// kept. The two removers, [`Self::forget`] and [`Self::evict`], record for themselves.
    fn set(&mut self, src: &str, state: State) {
        self.changed.push(src.to_owned());
        self.entries.insert(src.to_owned(), state);
    }

    /// Drop what is known about `src` without recording an attempt, for an image request that
    /// was queued and then never dispatched (`net::Msg::ImageDropped`).
    ///
    /// The entry goes back to having no state at all rather than to `Failed`, because nothing
    /// was fetched: a failure here would tell the reader that a host had been contacted and
    /// had refused, which is a different and untrue thing. For the same reason the disclosure
    /// recorded when the job was queued is taken back — a host that was named and then not
    /// contacted must not be left standing in the page-info panel, which is the panel claiming
    /// a request that never left. `dims` survives, as it does
    /// across a whole navigation: it is layout memory, not a claim about the network.
    pub fn forget(&mut self, src: &str) {
        if let Some(was) = self.in_flight.remove(src) {
            if was.referer {
                self.referer_requests = self.referer_requests.saturating_sub(1);
            }
            if was.icon {
                self.icons_requested = self.icons_requested.saturating_sub(1);
            }
            if let Some(n) = self.hosts.get_mut(&was.host) {
                *n = n.saturating_sub(1);
                if *n == 0 {
                    self.hosts.remove(&was.host);
                }
            }
        }
        if self.entries.remove(src).is_some() {
            self.changed.push(src.to_owned());
        }
        self.order.retain(|s| s != src);
        // With the entry, so a mark handed back keeps no claim on the marks' budget. It is
        // recorded again by the door that re-asks for it.
        self.marks.remove(src);
    }

    pub fn is_pending(&self, src: &str) -> bool {
        matches!(self.entries.get(src), Some(State::Loading(_)))
    }

    /// Anything at all outstanding, for the repaint floor. Both sources: a disk read still ends
    /// in a texture, and a frame that never comes is a mark that never appears.
    pub fn any_pending(&self) -> bool {
        self.entries
            .values()
            .any(|s| matches!(s, State::Loading(_)))
    }

    /// How many *requests* are outstanding, for `autoload::MAX_OUTSTANDING`. Counted rather than
    /// tracked alongside, because the count has to fall when a reply arrives, when a job is
    /// dropped, and when a page commits, and a separate counter would have to be told about
    /// all three.
    ///
    /// [`Source::Disk`] is not one. See that variant.
    pub fn pending(&self) -> usize {
        self.entries
            .values()
            .filter(|s| matches!(s, State::Loading(Source::Network)))
            .count()
    }

    /// A decoded picture is ready. `from_cache` says the bytes came off this machine.
    ///
    /// The flag decides which counter moves, and that is the whole of it: `loaded` is what the
    /// panel prints beside a host list, so a mark read from `reader::iconcache` counting there
    /// would report images loaded from no host at all. This is the one place either number
    /// changes, so the two cannot drift.
    pub fn insert(&mut self, ctx: &egui::Context, src: &str, decoded: Decoded, from_cache: bool) {
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [decoded.width as usize, decoded.height as usize],
            &decoded.rgba,
        );
        let texture = ctx.load_texture(
            format!("hww-image-{src}"),
            image,
            egui::TextureOptions::LINEAR,
        );
        self.dims
            .insert(src.to_owned(), (decoded.width, decoded.height));
        self.set(
            src,
            State::Ready(Ready {
                texture,
                width: decoded.width,
                height: decoded.height,
                source_width: decoded.source_width,
                source_height: decoded.source_height,
                partial: decoded.partial,
            }),
        );
        if from_cache {
            self.icons_from_cache += 1;
        } else {
            self.loaded += 1;
        }
        self.settle_request(src);
        self.touch(src);
        self.evict();
    }

    /// The disclosure counters for the page being read, for the page-info panel.
    ///
    /// Assembled here so the field list lives beside the fields it reads: `pageinfo::Counts`
    /// is one directory up, where egui cannot reach, and a second hand-written copy of these
    /// four names is one that goes stale the next time a counter is added.
    pub fn counts(&self) -> Counts {
        let mut hosts: Vec<String> = self.hosts.keys().cloned().collect();
        hosts.sort_unstable();
        Counts {
            images: self.loaded,
            hosts,
            cookie_attempts: self.cookie_attempts,
            referer_requests: self.referer_requests,
            icons_from_cache: self.icons_from_cache,
            icons_requested: self.icons_requested,
        }
    }

    /// Record what a request about to be queued discloses: one more request to `host`, and one
    /// more `Referer` if the setting is on.
    ///
    /// Ahead of the fetch, because the host is named to the reader before the bytes and the
    /// account has to agree with what they were told. [`Self::forget`] is the one thing that
    /// takes it back.
    pub fn record_request(&mut self, src: &str, host: &str, referer: bool, icon: bool) {
        *self.hosts.entry(host.to_owned()).or_default() += 1;
        if referer {
            self.referer_requests += 1;
        }
        if icon {
            self.icons_requested += 1;
        }
        self.in_flight.insert(
            src.to_owned(),
            Disclosed {
                host: host.to_owned(),
                referer,
                icon,
            },
        );
    }

    /// The request resolved, one way or another. Whatever it disclosed stands.
    fn settle_request(&mut self, src: &str) {
        self.in_flight.remove(src);
    }

    pub fn fail(&mut self, src: &str, failure: Failure) {
        self.settle_request(src);
        self.set(src, State::Failed(failure));
        self.touch(src);
    }

    /// True when this `src` already failed in a way a second request cannot change.
    ///
    /// Asked by `ReaderApp::request_image` beside `is_pending`, so the paths that do not draw
    /// a control — "load all", and a click on a placeholder the LRU has since evicted and
    /// redrawn — cannot re-issue a request whose answer is already known.
    pub fn refuses_retry(&self, src: &str) -> bool {
        matches!(self.entries.get(src), Some(State::Failed(f)) if !f.offers_retry())
    }

    /// Everything the outgoing page owned: its textures, and the account of what its images
    /// disclosed.
    ///
    /// Dropped when a new page commits, not when one is asked for: the outgoing page stays on
    /// screen for the length of the fetch and its pictures have to stay with it. `dims`
    /// survives, because it is layout memory rather than a claim about the network — a revisit
    /// reserves the right box before the bytes arrive.
    ///
    /// The counters go with the textures because the panel that reads them answers how *this
    /// page* arrived. A running session total is an answer to a question the panel does not
    /// ask, and it reads as this page's number to anyone who misses the qualifier.
    ///
    /// **A settled site mark survives too**, and that is not an exception to the paragraph
    /// above so much as the same reasoning applied to a surface that is not the page. The
    /// bookmarks sidebar is drawn from `archive.json` and stays on screen across every
    /// navigation, so marks dropped here would blank the whole strip on each one and rebuild it
    /// from the disk — visible flicker, and a decode per row per navigation, for a picture that
    /// did not change and was never about the outgoing page. The counters are still reset, and
    /// they stay honest for free: a surviving texture issues no request, so it cannot be counted
    /// twice.
    ///
    /// **Settled, though — not pending.** A mark still `Loading` is dropped with the pictures.
    /// Its reply is matched against the outgoing page's `ReqId` and will be discarded, so a
    /// retained `Loading` entry would be a mark that is permanently mid-flight: `note_icon`
    /// skips a `src` that has any state at all, so nothing would ever ask for it again and the
    /// row would stay empty for the rest of the session. Dropped, it is re-asked on the next
    /// frame, off the disk.
    pub fn clear_page(&mut self) {
        // Destructured rather than read through `self`: `retain`'s closure cannot borrow a
        // field of the same struct it is draining, and naming the three fields is what lets the
        // first pass consult `marks` while the second and third follow `entries`.
        let Self {
            entries,
            order,
            marks,
            ..
        } = self;
        entries.retain(|src, state| {
            marks.contains(src) && matches!(state, State::Ready(_) | State::Failed(_))
        });
        order.retain(|src| entries.contains_key(src));
        marks.retain(|src| entries.contains_key(src));
        self.hosts.clear();
        self.loaded = 0;
        self.cookie_attempts = 0;
        self.referer_requests = 0;
        self.icons_from_cache = 0;
        self.icons_requested = 0;
        // Cleared, not undone: subtracting these back out of counters that are themselves being
        // reset is the same nothing, and the page they belonged to is gone.
        self.in_flight.clear();
        // Not recorded one `src` at a time: the caller is `ReaderApp::commit`, which clears the
        // whole height table on its own line, and the blocks these belonged to are a document
        // that is no longer on screen.
        self.changed.clear();
    }

    fn touch(&mut self, src: &str) {
        self.order.retain(|s| s != src);
        self.order.push(src.to_owned());
    }

    /// How many entries of one class the queue is holding. Test-only, and beside `evict` rather
    /// than in the test module: what it counts is `order` and `class_of`, which are this
    /// module's business, and a test that walked them itself would pass by restating the bug.
    #[cfg(test)]
    fn count_of(&self, class: Class) -> usize {
        self.order
            .iter()
            .filter(|s| self.class_of(s) == class)
            .count()
    }

    /// Never evicts an in-flight request.
    ///
    /// `begin` pushes into `order` without evicting, so `I` on a page with more than
    /// `LRU_CAPACITY` images left the queue over capacity while requests were outstanding;
    /// the next `insert` then dropped the oldest, which was still `Loading`. That made
    /// `is_pending` false, so the placeholder offered itself again and a second click issued a
    /// duplicate third-party request, counted twice in `hosts`, `loaded`, and
    /// `referer_requests`, which are the reader's account of what it disclosed. Skipping
    /// pending entries can leave `order` briefly over capacity; it drains as they resolve.
    fn evict(&mut self) {
        // Two passes per class, against that class's own ceiling. Both classes share one
        // `order`, so a ceiling each is a budget rather than a second queue: what it prevents
        // is a page of photographs pushing a sidebar's worth of marks out of a cache they were
        // never competing for. See `MARK_CAPACITY`.
        //
        // The first pass spares settled declines. One holds no texture, so evicting it frees
        // nothing this cache exists to free, and it costs the memory `refuses_retry` is:
        // dropped, the entry reverts to `None`, the placeholder offers "load from host" again
        // and the next "load all" re-issues a request whose answer was already known. That is
        // the eviction bug one function down, in the one shape the pending guard cannot see.
        //
        // The second pass is why sparing them is not keeping them forever. A page can carry
        // more declines than the cache holds entries, and skipping every one of them left
        // `order` growing with the page instead of bounded by the cap. Past `DECLINES_KEPT` the
        // oldest goes anyway: an offer to re-request one picture is the cheaper failure, and it
        // is the one already accepted for a pending entry.
        //
        // Written as a loop over the classes rather than four calls, so a third class is a row
        // here and cannot be given one pass and not the other.
        for (class, cap) in [(Class::Picture, LRU_CAPACITY), (Class::Mark, MARK_CAPACITY)] {
            self.trim(class, cap, true);
            self.trim(class, cap + DECLINES_KEPT, false);
        }
    }

    /// Evict from the front of `order` until `class` fits `limit`. `spare_declines` is which of
    /// the two passes in [`evict`](Self::evict) this is; a pending entry is spared by both,
    /// because re-offering one is a duplicate third-party request rather than a repeated answer.
    ///
    /// One queue, walked once per class, so recency still decides *which* entry goes while the
    /// ceiling it is measured against is its own. The running `have` is what keeps this linear:
    /// counting the class afresh on every turn of the loop made a full cache quadratic, and this
    /// runs on every `begin`.
    fn trim(&mut self, class: Class, limit: usize, spare_declines: bool) {
        let mut have = self
            .order
            .iter()
            .filter(|s| self.class_of(s) == class)
            .count();
        let mut i = 0;
        while have > limit && i < self.order.len() {
            // The three guards borrow out of `order` and the removal takes ownership out of it,
            // so nothing is cloned: a pass that cloned every candidate it then skipped
            // allocated a `String` per entry per pass, four times over, on every `begin`.
            let src = &self.order[i];
            if self.class_of(src) != class
                || self.is_pending(src)
                || (spare_declines && self.refuses_retry(src))
            {
                i += 1;
                continue;
            }
            let src = self.order.remove(i);
            self.entries.remove(&src);
            self.marks.remove(&src);
            self.changed.push(src);
            have -= 1;
        }
    }

    fn class_of(&self, src: &str) -> Class {
        if self.marks.contains(src) {
            Class::Mark
        } else {
            Class::Picture
        }
    }
}

enum Snapshot {
    Offered,
    Loading,
    Ready,
    /// A format the address names before anything is fetched. Apart from [`Snapshot::Failed`]
    /// so the line can still carry the alt: nothing was ever asked for here, so the alt is the
    /// whole account of the picture the reader gets.
    Declined(&'static str),
    /// The message, and whether to draw the retry control beside it. Both halves, because this
    /// exists to be read out of the store before the drawing closures borrow `ctx`.
    Failed(String, bool),
}

/// The format an address names as one this reader declines, or `None`.
///
/// The three controls below are drawn on the strength of this, and it is the question
/// `autoload::plan` and `ReaderApp::load_all_images` already ask before requesting anything.
/// `ReaderApp::request_image` refuses such a `src` without contacting a host, so a control
/// offering it is a button whose entire effect is to print an answer that was available before
/// it was pressed — and naming what a press will do is the placeholder's whole claim.
///
/// The address and not the bytes, so this is half the declines at most: a format an
/// extensionless address hides is knowable only from what comes back, and [`Snapshot::Failed`]
/// is what says so afterwards. Resolved against the page like `inline_ui::image_host`, because
/// a relative `src` carries the extension and no host.
fn declined_by_address(src: &str, base: &url::Url) -> Option<&'static str> {
    declined_by_url(&base.join(src).ok()?)
}

/// The `[image]` mark and the alt beside it: the line a placeholder opens with whether it is
/// about to offer the picture or to say why it will not.
fn alt_line(ui: &mut Ui, alt: &str, pal: &theme::Palette) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.label(RichText::new("[image] ").color(pal.dim));
        if !alt.is_empty() {
            ui.label(RichText::new(alt).color(pal.fg));
        }
    });
}

/// The first [`ALT_SHOWN`] characters of an alt, cut at a word and marked.
fn short_alt(alt: &str) -> String {
    if alt.chars().count() <= ALT_SHOWN {
        return alt.to_owned();
    }
    let cut: String = alt.chars().take(ALT_SHOWN).collect();
    let cut = cut.rsplit_once(' ').map(|(h, _)| h).unwrap_or(&cut);
    format!("{cut}\u{2026}")
}

/// Alt text beyond this is read as a caption would be, which is not what a placeholder line
/// is for; the full alt stays in the IR and in the caption if the page has one.
const ALT_SHOWN: usize = 90;

/// The frame a figure shows before it is loaded: a filled, rounded box the measure wide and a
/// few lines tall, with `[image]`, the alt, and the one control, "load from host".
///
/// A few lines and not the image's height, because no dimensions live in `ir::Image`, so a
/// taller placeholder would reserve the wrong box and shift the page when it filled; a revisit
/// reserves the right box from [`ImageStore::reserved`]. The host is named *before* the click,
/// which is the whole argument for allowing the request at all, and the ground is `code_bg`
/// with a `rule` edge, which is what says "a picture goes here" on a page that has not fetched
/// one, in the shape every reader app uses for it.
pub fn placeholder(ui: &mut Ui, img: &ir::Image, ctx: &mut RenderCtx<'_>) {
    let alt_full = img.alt.as_deref().unwrap_or("").trim();
    let alt_short = short_alt(alt_full);
    let alt = alt_short.as_str();
    let host = super::inline_ui::image_host(img, &ctx.base);
    let pal = ctx.pal;
    let font = theme::chrome_font(ctx.opts);

    if !ctx.opts.images.offers_loading() {
        // Silently dropping the image is how a reader lies about a page: say it was there.
        // Both policies that decline to load still get this line; what separates them is
        // whether the favicon is fetched, which is `ensure_favicon`'s business, not this one's.
        let label = if alt.is_empty() {
            "[image] not shown".to_owned()
        } else {
            format!("[image] {alt} (not shown)")
        };
        ui.label(RichText::new(label).color(pal.dim).italics());
        return;
    }

    // Read the state out before drawing: the closures below need `ctx` mutably, and holding a
    // borrow of the store across them would be a second one.
    let state = match ctx.images.state(&img.src) {
        Some(State::Loading(_)) => Snapshot::Loading,
        Some(State::Failed(f)) => Snapshot::Failed(f.why.clone(), f.offers_retry()),
        Some(State::Ready(_)) => Snapshot::Ready,
        // Nothing has been asked about this picture, and for a format the address names
        // nothing will be. Worded through `DecodeError::Unsupported` rather than a second
        // literal, for the reason `request_image` gives at the same test: the reader must not
        // learn one phrase for a declined format that was fetched and another for one that
        // never left.
        None | Some(State::Offered) => match declined_by_address(&img.src, &ctx.base) {
            Some(name) => Snapshot::Declined(name),
            None => Snapshot::Offered,
        },
    };
    if let Snapshot::Ready = state {
        show_ready(ui, img, ctx);
        return;
    }
    ctx.note_unloaded_image(ui.next_widget_position().y, &img.src);
    // A revisit knows the box; reserve it so the page does not jump when it fills.
    let (w, h) = ctx.images.reserved(&img.src).unwrap_or((0, 0));
    let min_h = if w > 0 && h > 0 {
        ui.available_width().min(w as f32) * h as f32 / w as f32
    } else {
        theme::snap(ctx.opts.base_size_pt * 3.4)
    };
    let mut act = false;
    let mut focused = false;
    egui::Frame::new()
        .fill(pal.code_bg)
        .stroke(egui::Stroke::new(1.0, pal.rule))
        .corner_radius(egui::CornerRadius::same(theme::RADIUS))
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.set_min_height(min_h - 20.0);
            ui.style_mut().override_font_id = Some(font.clone());
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            ui.vertical_centered(|ui| {
                ui.spacing_mut().item_spacing.y = theme::snap(ctx.opts.base_size_pt * 0.2);
                match &state {
                    Snapshot::Loading => {
                        ui.label(RichText::new("[image]").color(pal.dim));
                        ui.label(RichText::new(format!("loading from {host}…")).color(pal.dim));
                    }
                    Snapshot::Failed(why, retryable) => {
                        ui.label(RichText::new(format!("[image] {why}")).color(pal.notice_fg));
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&host).color(pal.dim));
                            // No control for a failure a second request cannot change: the
                            // button re-fetched and re-failed for as long as the page was
                            // open, and taking Tab focus meant `i` landed on it too. The line
                            // above still says the picture was there and why it is not shown.
                            if *retryable {
                                let resp = ui.small_button("retry");
                                super::follow_focus(&resp);
                                super::advance_focus(&resp);
                                act = resp.clicked();
                                focused = resp.has_focus();
                            }
                        });
                    }
                    // The alt, then the reason, and no control at all. `request_image` would
                    // answer this press by contacting nobody and writing the second line, so
                    // a button here is one the placeholder cannot name a host for: it says
                    // "load from …" about a request that will not be made. The alt is drawn
                    // the same way an offer draws it, because this picture was on the page
                    // and the alt is now the only thing that says so.
                    Snapshot::Declined(name) => {
                        alt_line(ui, alt, &pal);
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(DecodeError::Unsupported(name).to_string())
                                    .color(pal.notice_fg),
                            );
                            ui.label(RichText::new(&host).color(pal.dim));
                        });
                    }
                    Snapshot::Offered | Snapshot::Ready => {
                        alt_line(ui, alt, &pal);
                        let resp = ui.add(
                            egui::Button::new(
                                RichText::new(format!("load from {host}"))
                                    .color(pal.link)
                                    .underline(),
                            )
                            .frame(false),
                        );
                        super::follow_focus(&resp);
                        super::advance_focus(&resp);
                        theme::focus_ring(ui, &resp, &pal);
                        act = resp.clicked();
                        focused = resp.has_focus();
                    }
                }
            });
        });
    // Reported like a link's focus: `i` then loads or retries, and the block stays laid out
    // while Tab rests here.
    if focused {
        ctx.focus_image = Some(img.src.clone());
    }
    if act {
        ctx.act(Action::LoadImage(img.src.clone()));
    }
}

fn show_ready(ui: &mut Ui, img: &ir::Image, ctx: &mut RenderCtx<'_>) {
    let pal = ctx.pal;
    let host = super::inline_ui::image_host(img, &ctx.base);
    let Some(State::Ready(ready)) = ctx.images.state(&img.src) else {
        return;
    };
    let max_w = ui.available_width().min(ready.width as f32);
    let height = max_w * ready.height as f32 / ready.width as f32;
    let source = (ready.source_width, ready.source_height);
    let partial = ready.partial.clone();
    ui.add(
        egui::Image::new(&ready.texture)
            .fit_to_exact_size(egui::vec2(max_w, height))
            .corner_radius(egui::CornerRadius::same(theme::RADIUS)),
    );
    let mut note = format!("{} × {} · {host}", source.0, source.1);
    if let Some(p) = &partial {
        note = format!("{p} · {note}");
    }
    ui.label(
        RichText::new(note)
            .color(if partial.is_some() {
                pal.notice_fg
            } else {
                pal.dim
            })
            .small(),
    );
    if partial.is_some() {
        let resp = ui.small_button("retry");
        super::follow_focus(&resp);
        super::advance_focus(&resp);
        if resp.has_focus() {
            ctx.focus_image = Some(img.src.clone());
        }
        if resp.clicked() {
            ctx.act(Action::LoadImage(img.src.clone()));
        }
    }
}

/// An entry's thumbnail: drawn at a few lines' height once loaded, and not at all before.
/// No placeholder, no button; the headline beside it is the card's affordance and `I` is how
/// the page's images arrive.
pub fn thumbnail(ui: &mut Ui, img: &ir::Image, ctx: &mut RenderCtx<'_>) {
    if let Some(State::Ready(ready)) = ctx.images.state(&img.src) {
        let h = ctx.opts.base_size_pt * 3.6;
        let w = (h * ready.width as f32 / ready.height.max(1) as f32).min(ui.available_width());
        ui.add(
            egui::Image::new(&ready.texture)
                .fit_to_exact_size(egui::vec2(w, h))
                .corner_radius(egui::CornerRadius::same(theme::RADIUS)),
        );
    }
}

/// The page favicon beside the masthead eyebrow. Drawn only when ready: a missing or failed
/// icon leaves the site label alone, which is what the line was before favicons existed.
pub fn favicon(ui: &mut Ui, src: &str, ctx: &mut RenderCtx<'_>) {
    let Some(State::Ready(ready)) = ctx.images.state(src) else {
        return;
    };
    let h = theme::snap(ctx.opts.base_size_pt * 1.1);
    let w = h * ready.width as f32 / ready.height.max(1) as f32;
    ui.add(
        egui::Image::new(&ready.texture)
            .fit_to_exact_size(egui::vec2(w, h))
            .corner_radius(egui::CornerRadius::same(theme::RADIUS)),
    );
}

/// One list row's site mark, in the column left of its headline.
///
/// The masthead favicon's sibling and not a thumbnail: `box_size` square whatever the file's
/// proportions, no placeholder, no control, and no widget — the headline beside it is the row's
/// only affordance and its only Tab stop. The space is allocated whether or not the bytes are
/// here, because a column that closed up around a missing icon would re-align every row under
/// it as the marks arrived one by one.
///
/// Requesting is `ctx.note_icon`'s business and, past it, `ReaderApp::load_site_icons`. This
/// function neither asks the network for anything nor decides whether it may.
pub fn site_icon(ui: &mut Ui, src: &str, box_size: f32, ctx: &mut RenderCtx<'_>) {
    // Allocated first and unconditionally, so the fitting arithmetic can be [`paint_mark`]'s
    // alone: the box is the same square whether or not there are bytes for it, which is what
    // keeps the rows under a missing mark from re-aligning as the marks arrive. `allocate_space`
    // and not `allocate_exact_size`, so this stays the widgetless column the doc above claims.
    let (_, rect) = ui.allocate_space(egui::vec2(box_size, box_size));
    if !paint_mark(ui, ctx.images, src, rect) {
        ctx.note_icon(rect.top(), src);
    }
}

/// Paint a ready site mark to fit `rect`, or answer `false` when there is nothing to paint.
///
/// Every mark hww draws is fitted here, [`site_icon`]'s included, because the two surfaces
/// differ only in who allocates and who records a want: the sidebar has already allocated a
/// control the reader can click and tab to, and it collects its own band rather than going
/// through `RenderCtx`. Fitted inside the box rather than stretched to it, in the one place:
/// a mark that is not square is a wide wordmark often enough, and a stretched one is worse
/// than a small one.
///
/// It reads the store and never asks it for anything, which is the same permission
/// `reader::iconcache` grants every reader of a kept mark.
pub fn paint_mark(ui: &Ui, store: &ImageStore, src: &str, rect: egui::Rect) -> bool {
    let Some(State::Ready(ready)) = store.state(src) else {
        return false;
    };
    let box_size = rect.width().min(rect.height());
    let scale = (box_size / ready.width.max(1) as f32).min(box_size / ready.height.max(1) as f32);
    let size = egui::vec2(ready.width as f32 * scale, ready.height as f32 * scale);
    egui::Image::new(&ready.texture)
        .corner_radius(egui::CornerRadius::same(theme::RADIUS))
        .paint_at(ui, egui::Rect::from_center_size(rect.center(), size));
    true
}

/// The one control an entry's thumbnail gets: a small dim `[image]` beside the time, which
/// loads that one picture, and nothing once it is loaded. `Never` shows nothing at all.
pub fn load_control(ui: &mut Ui, img: &ir::Image, ctx: &mut RenderCtx<'_>) {
    // Nothing to offer once it is loaded, and nothing to offer for a decline a second request
    // cannot change: this is a control, and both of those make it a dead one.
    // The third case is the same dead control one step earlier: an address that names a
    // declined format is refused by `request_image` without a request, so the button would
    // take Tab focus and do nothing the first time it was pressed rather than the second.
    if !ctx.opts.images.offers_loading()
        || matches!(ctx.images.state(&img.src), Some(State::Ready(_)))
        || ctx.images.refuses_retry(&img.src)
        || declined_by_address(&img.src, &ctx.base).is_some()
    {
        return;
    }
    let pal = ctx.pal;
    ctx.note_unloaded_image(ui.next_widget_position().y, &img.src);
    let resp =
        ui.add(egui::Button::new(RichText::new("[image]").color(pal.dim).small()).frame(false));
    super::follow_focus(&resp);
    super::advance_focus(&resp);
    theme::focus_ring(ui, &resp, &pal);
    if resp.has_focus() {
        ctx.focus_image = Some(img.src.clone());
    }
    if resp.clicked() {
        ctx.act(Action::LoadImage(img.src.clone()));
    }
}

/// An image inside a paragraph. Always the compact strip: an icon-sized image mid-sentence
/// that expanded to the column width would break the line it sits in.
pub fn inline_placeholder(ui: &mut Ui, img: &ir::Image, ctx: &mut RenderCtx<'_>) {
    let alt = img.alt.as_deref().unwrap_or("image");
    let pal = ctx.pal;
    // Paired with the state rather than asked inside a guard, so the arm below can name the
    // format instead of unwrapping it back out.
    let declined = declined_by_address(&img.src, &ctx.base);
    match (ctx.images.state(&img.src), declined) {
        (Some(State::Ready(ready)), _) => {
            let h = ctx.opts.base_size_pt * 1.4;
            let w = h * ready.width as f32 / ready.height.max(1) as f32;
            ui.add(egui::Image::new(&ready.texture).fit_to_exact_size(egui::vec2(w, h)));
        }
        // A policy that declines to load gets a label, not a button: the click was already
        // guarded, but a `Button` still takes Tab focus and still writes `focus_image`, so `i`
        // landed on a picture the setting had ruled out and the block placeholder two
        // functions up had stopped offering. The mark stays either way — saying nothing is how
        // a reader lies about a page.
        _ if !ctx.opts.images.offers_loading() => {
            ui.label(RichText::new(format!("[{alt}]")).color(pal.dim).italics());
        }

        // Same shape, for the same reason, one case further on: a decline a second request
        // cannot change had no arm here at all, so the mark stayed a `Button` that took Tab
        // focus, wrote `focus_image`, and did nothing at all when pressed. A label says the
        // picture was there and why it is not shown, and takes no focus.
        (Some(State::Failed(f)), _) if !f.offers_retry() => {
            ui.label(
                RichText::new(format!("[{alt}] ({})", f.why))
                    .color(pal.dim)
                    .italics(),
            );
        }
        // And the same shape once more, one step before the request: a decline the address
        // already names reaches the arm above only after a press that contacts nobody, so
        // until then the mark was a `Button` offering a picture hww will not open.
        (None | Some(State::Offered), Some(name)) => {
            ui.label(
                RichText::new(format!("[{alt}] ({})", DecodeError::Unsupported(name)))
                    .color(pal.dim)
                    .italics(),
            );
        }
        _ => {
            ctx.note_unloaded_image(ui.next_widget_position().y, &img.src);
            let resp = ui.add(
                egui::Button::new(RichText::new(format!("[{alt}]")).color(pal.dim)).frame(false),
            );
            super::follow_focus(&resp);
            super::advance_focus(&resp);
            theme::focus_ring(ui, &resp, &pal);
            if resp.has_focus() {
                ctx.focus_image = Some(img.src.clone());
            }
            if resp.clicked() {
                ctx.act(Action::LoadImage(img.src.clone()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use url::Url;

    /// Admitted under `ui/` on the same argument as the store tests below: the failure is
    /// otherwise invisible. Three surfaces drop their control on the strength of this, and each
    /// holds the `src` a page wrote rather than a resolved URL, so resolving against the wrong
    /// base — or not resolving at all — puts the button back on every relative address, which is
    /// most of them. The only symptom is a press that contacts nobody and prints what it knew
    /// before it was pressed, which is exactly what a screenshot cannot show.
    #[test]
    fn a_declined_address_is_recognised_relative_to_the_page_it_came_from() {
        let base = Url::parse("https://example.org/post/one").expect("a literal URL parses");
        assert_eq!(declined_by_address("../img/logo.svg", &base), Some("SVG"));
        assert_eq!(declined_by_address("/hero.avif", &base), Some("AVIF"));
        assert_eq!(declined_by_address("photo.jpg", &base), None);
        // The case the bytes still have to answer, and the reason the control cannot be dropped
        // on suspicion: an image CDN taking its source in a query says nothing about what it
        // will serve, since the same address answers a browser with WebP.
        assert_eq!(declined_by_address("/_i?url=%2Fa.avif&w=800", &base), None);
    }

    /// A settled mark outlives the page; a pending one does not; a picture never does.
    ///
    /// Admitted under `ui/` because the failure is otherwise invisible rather than because
    /// there is precedent above. Every branch here is reachable without an `egui::Context` —
    /// `note_mark`, `begin`, `fail` and `clear_page` upload nothing — and each way of getting it
    /// wrong shows up as something no test would catch and a reader would blame on the network:
    /// dropping settled marks is a sidebar that blanks on every navigation, keeping a pending
    /// one is a row that stays empty for the rest of the session because `note_icon` skips a
    /// `src` that has any state at all, and keeping a picture is the page's memory outliving the
    /// page.
    #[test]
    fn a_settled_mark_survives_a_commit_and_a_pending_one_does_not() {
        let mut store = ImageStore::default();

        store.note_mark("https://a.example/favicon.ico");
        store.fail(
            "https://a.example/favicon.ico",
            Failure::permanent("no icon".to_owned()),
        );
        store.note_mark("https://b.example/favicon.ico");
        store.begin("https://b.example/favicon.ico", Source::Disk);
        store.begin("/article-photo.jpg", Source::Network);

        store.clear_page();

        assert!(
            store.state("https://a.example/favicon.ico").is_some(),
            "a settled mark is what keeps the strip from blanking on every navigation"
        );
        assert!(
            store.state("https://b.example/favicon.ico").is_none(),
            "a pending mark's reply is matched against the page being left, so keeping the \
             entry would strand the row for the session"
        );
        assert!(
            store.state("/article-photo.jpg").is_none(),
            "the page's own pictures still go with the page"
        );
    }

    /// A page of photographs must not evict the sidebar.
    ///
    /// The two classes share one recency queue, so without a budget of its own every mark in the
    /// strip is pushed out by the pictures of whatever the reader happens to be looking at — and
    /// then asked for again, and evicted again, for as long as the panel is open. Recency is the
    /// wrong question to ask of a mark: it is on screen because a panel is, not because it was
    /// scrolled past.
    ///
    /// The pictures are *transient* declines and `evict` is called per picture, and both of
    /// those are the test rather than the fixture. `fail` alone never evicts — only `begin` and
    /// `insert` do — and a permanent decline is spared by the first pass, so a fixture built the
    /// obvious way passed with every `Class::Mark` line deleted from `evict`: it asserted the
    /// survival of an entry against a queue nothing had ever trimmed. The picture count below is
    /// what says the trimming happened at all.
    #[test]
    fn a_page_full_of_pictures_does_not_evict_the_marks() {
        let mut store = ImageStore::default();
        store.note_mark("https://kept.example/favicon.ico");
        store.fail(
            "https://kept.example/favicon.ico",
            Failure::permanent("no icon".to_owned()),
        );
        // Comfortably past the pictures' own ceiling, which is what used to take the mark with
        // it. Settled failures rather than textures so the test needs no window.
        for i in 0..LRU_CAPACITY * 2 {
            let src = format!("/photo-{i}.jpg");
            store.fail(&src, Failure::transient("reset".to_owned()));
            store.evict();
        }
        assert!(
            store.count_of(Class::Picture) <= LRU_CAPACITY,
            "the pictures were never trimmed, so nothing else here is about eviction"
        );
        assert!(
            store.state("https://kept.example/favicon.ico").is_some(),
            "the mark was evicted against the pictures' ceiling"
        );
    }

    /// And the marks are bounded by a ceiling of their own, which is the other half of it.
    ///
    /// A budget that only ever spares is not a budget: the strip's entries are settled and
    /// survive `clear_page`, so with nothing trimming them a long session's worth of bookmarks
    /// drawn and scrolled past would hold every texture it ever decoded. This is the assertion
    /// that fails when `evict` stops asking about `Class::Mark`.
    #[test]
    fn the_marks_are_bounded_by_their_own_ceiling() {
        let mut store = ImageStore::default();
        for i in 0..MARK_CAPACITY * 2 {
            let src = format!("https://s{i}.example/favicon.ico");
            store.note_mark(&src);
            store.fail(&src, Failure::transient("reset".to_owned()));
            store.evict();
        }
        assert!(
            store.count_of(Class::Mark) <= MARK_CAPACITY,
            "a mark is spared the pictures' ceiling, not every ceiling"
        );
    }

    /// The second test under `ui/`, admitted on the same argument as the first.
    ///
    /// `counts` and `fail` take no `egui::Context`, unlike `insert`, which uploads a texture.
    /// The directory's rule is "split by what a test can reach", and these are reachable; the
    /// half that needs a window stays merely compiled.
    ///
    /// What this does **not** reach is `net::load_image`, where a cookie attempt is actually
    /// counted: that needs a live `Session`. The guard there is the signature,
    /// `(usize, Result<Decoded, ImageError>)`, which has no arm that can drop the count.
    #[test]
    fn a_failed_image_still_reports_what_its_request_disclosed() {
        let mut store = ImageStore {
            cookie_attempts: 2,
            ..ImageStore::default()
        };
        store.record_request("/a.png", "img.example.org", true, false);
        store.record_request("/b.png", "cdn.example.net", false, false);

        // The decode failed. Nothing about what the request disclosed is undone by that.
        store.fail("/a.png", Failure::transient("not an image".to_owned()));

        let counts = store.counts();
        assert_eq!(counts.cookie_attempts, 2);
        assert_eq!(counts.referer_requests, 1);
        assert_eq!(counts.images, 0, "nothing decoded");
        assert_eq!(
            counts.hosts,
            vec!["cdn.example.net", "img.example.org"],
            "sorted here, so the panel never reorders between frames"
        );
    }

    /// A job the pool dropped without dispatching contacted nobody, so the panel must not go on
    /// naming its host for the rest of the page.
    ///
    /// The one place the two halves can disagree: the disclosure is recorded when the job is
    /// *queued*, because that is where the host is known and where the reader is told about it,
    /// and `net::Msg::ImageDropped` arrives afterwards. Failing is not this — a failed fetch
    /// contacted the host — which is why the test above and this one have to both hold.
    #[test]
    fn a_dropped_request_takes_its_disclosure_back() {
        let mut store = ImageStore::default();
        store.record_request("/a.png", "cdn.example.net", true, false);
        store.record_request("/b.png", "cdn.example.net", true, false);
        store.record_request("/c.png", "img.example.org", false, false);
        assert_eq!(store.counts().referer_requests, 2);

        store.forget("/a.png");
        let counts = store.counts();
        assert_eq!(counts.referer_requests, 1, "one Referer never went out");
        assert_eq!(
            counts.hosts,
            vec!["cdn.example.net", "img.example.org"],
            "the host still stands: its other request did go out"
        );

        store.forget("/b.png");
        assert_eq!(
            store.counts().hosts,
            vec!["img.example.org"],
            "nothing left that contacted it"
        );
        assert_eq!(store.counts().referer_requests, 0);
    }

    /// A mark counts as a mark and never as a picture, and a mark that was never sent takes its
    /// own number back with the host and the Referer.
    ///
    /// The panel prints these two side by side — "Site icons: 0 from disk, 2 fetched" over
    /// "Images loaded: 2, from 1 host" — so a mark counted in the wrong one is the panel giving
    /// two different answers to the same question about the same request.
    #[test]
    fn a_site_mark_is_counted_as_one_and_is_taken_back_like_one() {
        let mut store = ImageStore::default();
        store.record_request("/mark.png", "kept.example", false, true);
        store.record_request("/photo.jpg", "cdn.example.net", true, false);
        let counts = store.counts();
        assert_eq!(counts.icons_requested, 1, "one of the two was a mark");
        assert_eq!(counts.icons_from_cache, 0, "and it was not on this machine");
        assert_eq!(counts.images, 0, "neither has decoded yet");

        store.forget("/mark.png");
        let counts = store.counts();
        assert_eq!(counts.icons_requested, 0, "that request never went out");
        assert_eq!(counts.hosts, vec!["cdn.example.net"]);

        // And a page commit clears both, like every other per-page counter: the panel answers
        // how the page on screen arrived, and a session total under that heading is a different
        // quantity wearing the same label.
        store.icons_from_cache = 3;
        store.clear_page();
        let counts = store.counts();
        assert_eq!((counts.icons_from_cache, counts.icons_requested), (0, 0));
    }

    /// The retry control is drawn from this, and so is the guard that stops "load all" from
    /// re-issuing a request whose answer is already known.
    #[test]
    fn only_a_permanent_failure_refuses_a_retry() {
        let mut store = ImageStore::default();
        store.fail("/a.png", Failure::transient("connection reset".to_owned()));
        store.fail(
            "/b.svg",
            Failure::permanent("SVG images are not supported".to_owned()),
        );
        assert!(!store.refuses_retry("/a.png"), "a second request may work");
        assert!(store.refuses_retry("/b.svg"), "a second request cannot");
        // A picture nobody has asked about yet is not a refusal.
        assert!(!store.refuses_retry("/c.png"));
    }

    /// A page's disclosures are the page's. The panel reading these answers how the document
    /// on screen arrived, so the counters cannot outlive it; `dims` is the one thing that does,
    /// because reserving the right box on a revisit is layout memory rather than a network claim.
    #[test]
    fn committing_a_page_drops_what_the_last_one_disclosed() {
        let mut store = ImageStore {
            cookie_attempts: 4,
            loaded: 2,
            ..ImageStore::default()
        };
        store.dims.insert("/a.png".to_owned(), (800, 600));
        store.record_request("/a.png", "cdn.example.net", true, false);
        store.fail("/a.png", Failure::transient("connection reset".to_owned()));

        store.clear_page();

        assert_eq!(
            store.counts(),
            Counts::default(),
            "a fresh page discloses nothing yet"
        );
        assert!(store.state("/a.png").is_none());
        assert_eq!(
            store.reserved("/a.png"),
            Some((800, 600)),
            "layout memory survives"
        );
    }

    /// A settled decline must outlive the cache pressure that has nothing to do with it.
    ///
    /// `evict` bounds *textures*, and a failure holds none. Dropping one reverted the entry to
    /// `None`, which made the placeholder offer itself again and let "load all" re-issue the
    /// request. Only reachable for a decline the address cannot predict — an extensionless
    /// AVIF, a file over the pixel ceiling — because `declined_by_url` catches the rest before
    /// a request is made at all.
    #[test]
    fn eviction_does_not_forget_that_a_picture_is_settled() {
        let mut store = ImageStore::default();
        store.fail(
            "/big.png",
            Failure::permanent("image is 9000x9000".to_owned()),
        );
        // Fill well past the cache with entries that are evictable.
        for i in 0..LRU_CAPACITY * 2 {
            store.fail(
                &format!("/n{i}.png"),
                Failure::transient("reset".to_owned()),
            );
        }
        store.evict();
        assert!(
            store.order.len() <= LRU_CAPACITY + 1,
            "the cache is still bounded"
        );
        assert!(
            store.refuses_retry("/big.png"),
            "the one entry that must not be re-requested is the one that was dropped"
        );
    }

    /// And sparing them has a ceiling of its own.
    ///
    /// Every entry on this page is a decline the first pass steps over, so before
    /// `DECLINES_KEPT` the queue grew with the page and `LRU_CAPACITY` bounded nothing. What
    /// survives is the newest, which is the part of the page a reader is still near.
    #[test]
    fn a_page_of_settled_declines_is_still_bounded() {
        let mut store = ImageStore::default();
        let n = (LRU_CAPACITY + DECLINES_KEPT) * 2;
        for i in 0..n {
            store.fail(
                &format!("/n{i}.png"),
                Failure::permanent("image is 9000x9000".to_owned()),
            );
            store.evict();
        }
        assert!(
            store.order.len() <= LRU_CAPACITY + DECLINES_KEPT,
            "declines are spared, not unbounded"
        );
        assert!(
            store.refuses_retry(&format!("/n{}.png", n - 1)),
            "the newest decline is the one still worth remembering"
        );
    }

    /// And a request that resolved cannot be taken back afterwards. `forget` is reachable from
    /// an eviction as well as from a drop, and an eviction is about a texture, not a host.
    #[test]
    fn forgetting_a_resolved_request_leaves_the_disclosure_standing() {
        let mut store = ImageStore::default();
        store.record_request("/a.png", "cdn.example.net", true, false);
        store.fail("/a.png", Failure::transient("not an image".to_owned()));
        store.forget("/a.png");
        let counts = store.counts();
        assert_eq!(counts.hosts, vec!["cdn.example.net"]);
        assert_eq!(counts.referer_requests, 1);
    }

    /// Enter on a load control moves the focus on, the way Tab would.
    ///
    /// The control removes itself when it is pressed — the placeholder redraws as "loading
    /// from host…" — so egui's dead-man's switch used to clear the focus, and the next Tab
    /// began again at the first link on the page. A reader loading the pictures of a long
    /// article paid one press per picture and then a press per link above it.
    ///
    /// It needs an `egui::Context` and no display, and it belongs here on `inline_ui`'s
    /// argument rather than on its precedent: what it pins is egui's focus machinery, and the
    /// failure is invisible to everything else. Where the focus is after a key is not a value
    /// any function returns, and `hww-shot` photographs one frame and cannot say a key moved
    /// anything. The bug looked, in a screenshot, like a placeholder that had started loading.
    #[test]
    fn enter_on_a_load_control_hands_the_focus_to_the_next_one() {
        let ctx = egui::Context::default();
        crate::reader::ui::fonts::install(&ctx);
        let opts = crate::reader::opts::ReadOpts::default();
        let collapsed = HashSet::new();
        let threads = HashMap::new();
        let base = url::Url::parse("https://example.com/").unwrap();
        let pal = theme::palette(crate::reader::opts::Theme::Light, false);
        let first = ir::Image {
            src: "https://cdn.example.net/first.jpg".to_owned(),
            alt: Some("the first picture".to_owned()),
        };
        let second = ir::Image {
            src: "https://cdn.example.net/second.jpg".to_owned(),
            alt: Some("the second picture".to_owned()),
        };
        let mut images = ImageStore::default();
        // Where the focus was found in each frame, and what the last of them acted on.
        let mut landed = Vec::new();
        let mut last_action = None;
        for key in [
            None,
            None,
            Some(egui::Key::Tab),
            None,
            Some(egui::Key::Enter),
        ] {
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::pos2(0.0, 0.0),
                    egui::vec2(400.0, 900.0),
                )),
                ..Default::default()
            };
            if let Some(key) = key {
                for pressed in [true, false] {
                    input.events.push(egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
            }
            let mut rctx =
                RenderCtx::new(pal, &opts, base.clone(), &mut images, &collapsed, &threads);
            let mut out = ctx.run_ui(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    placeholder(ui, &first, &mut rctx);
                    placeholder(ui, &second, &mut rctx);
                });
            });
            // The test never uploads them, and epaint panics on a delta dropped unhandled.
            out.textures_delta.clear();
            landed.push(rctx.focus_image.clone());
            last_action = rctx.action.take();
            eprintln!(
                "frame key={key:?} focus_image={:?} mem={:?}",
                rctx.focus_image,
                ctx.memory(|m| m.focused())
            );
        }
        assert_eq!(
            landed[3].as_deref(),
            Some(first.src.as_str()),
            "Tab lands on the first picture's control"
        );
        assert_eq!(
            landed[4].as_deref(),
            Some(second.src.as_str()),
            "Enter loads the first picture and leaves the focus on the next control, not nowhere"
        );
        assert!(
            matches!(&last_action, Some(Action::LoadImage(src)) if src == &first.src),
            "the press belongs to the control it was aimed at: the one it hands the focus to \
             must not read the same key as a click on itself"
        );
    }
}
