//! Texture upload and the LRU. **No decoding happens here**: that is
//! `reader::image_decode`, deliberately outside this directory so it is tested rather than
//! merely compiled.
//!
//! Nothing in this module touches disk. The texture cache is the only thing a revisit hits and
//! it dies with the process; a content-addressed on-disk cache is the archive's problem and
//! arrives with it.
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
//! The bookmarks' left column is the same exception with the same policy, one page over
//! ([`site_icon`]): each row's mark is the identity of the site that row leads to, stored when
//! the reader kept the page and fetched when the row is drawn. It is the one automatic request
//! that is *not* about the page on screen, which is why it is bounded twice — only the bookmarks
//! draws the column (`archive::Mark`), and only the rows inside the layout band are asked for
//! (`RenderCtx::note_icon`). Page info reports the hosts, on a page that otherwise reports no
//! request at all.
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
use crate::reader::image_decode::Decoded;
use crate::reader::pageinfo::Counts;
use crate::reader::ui::{Action, RenderCtx, theme};
use eframe::egui::{self, RichText, TextureHandle, Ui};
use std::collections::HashMap;

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

pub enum State {
    Offered,
    Loading,
    Ready(Ready),
    Failed(Failure),
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

    pub fn begin(&mut self, src: &str) {
        self.set(src, State::Loading);
        self.touch(src);
        self.evict();
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
    }

    pub fn is_pending(&self, src: &str) -> bool {
        matches!(self.entries.get(src), Some(State::Loading))
    }

    pub fn any_pending(&self) -> bool {
        self.entries.values().any(|s| matches!(s, State::Loading))
    }

    /// How many requests are outstanding, for `autoload::MAX_OUTSTANDING`. Counted rather than
    /// tracked alongside, because the count has to fall when a reply arrives, when a job is
    /// dropped, and when a page commits, and a separate counter would have to be told about
    /// all three.
    pub fn pending(&self) -> usize {
        self.entries
            .values()
            .filter(|s| matches!(s, State::Loading))
            .count()
    }

    pub fn insert(&mut self, ctx: &egui::Context, src: &str, decoded: Decoded) {
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
        self.loaded += 1;
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
        }
    }

    /// Record what a request about to be queued discloses: one more request to `host`, and one
    /// more `Referer` if the setting is on.
    ///
    /// Ahead of the fetch, because the host is named to the reader before the bytes and the
    /// account has to agree with what they were told. [`Self::forget`] is the one thing that
    /// takes it back.
    pub fn record_request(&mut self, src: &str, host: &str, referer: bool) {
        *self.hosts.entry(host.to_owned()).or_default() += 1;
        if referer {
            self.referer_requests += 1;
        }
        self.in_flight.insert(
            src.to_owned(),
            Disclosed {
                host: host.to_owned(),
                referer,
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
    /// screen for the length of the fetch and its pictures have to stay with it. `dims` alone
    /// survives, because it is layout memory rather than a claim about the network — a revisit
    /// reserves the right box before the bytes arrive.
    ///
    /// The counters go with the textures because the panel that reads them answers how *this
    /// page* arrived. A running session total is an answer to a question the panel does not
    /// ask, and it reads as this page's number to anyone who misses the qualifier.
    pub fn clear_page(&mut self) {
        self.entries.clear();
        self.order.clear();
        self.hosts.clear();
        self.loaded = 0;
        self.cookie_attempts = 0;
        self.referer_requests = 0;
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
        // A settled decline holds no texture, so evicting one frees nothing this cache exists
        // to free, and it costs the memory `refuses_retry` is. Dropped, the entry reverts to
        // `None`: the placeholder offers "load from host" again and the next "load all"
        // re-issues a request whose answer was already known. That is the eviction bug two
        // functions down, in the one shape the pending guard cannot see.
        self.trim(LRU_CAPACITY, true);
        // Sparing them is not keeping them forever. A page can carry more declines than the
        // cache holds entries, and skipping every one of them left `order` growing with the
        // page instead of bounded by the cap — the queue that the pass above exists to bound.
        // Past `DECLINES_KEPT` the oldest goes anyway: an offer to re-request one picture is
        // the cheaper failure, and it is the one already accepted for a pending entry.
        self.trim(LRU_CAPACITY + DECLINES_KEPT, false);
    }

    /// Evict from the front of `order` until it fits `limit`. `spare_declines` is which of the
    /// two passes in [`evict`](Self::evict) this is; a pending entry is spared by both, because
    /// re-offering one is a duplicate third-party request rather than a repeated answer.
    fn trim(&mut self, limit: usize, spare_declines: bool) {
        let mut i = 0;
        while self.order.len() > limit && i < self.order.len() {
            if self.is_pending(&self.order[i])
                || (spare_declines && self.refuses_retry(&self.order[i]))
            {
                i += 1;
                continue;
            }
            let oldest = self.order.remove(i);
            self.entries.remove(&oldest);
            self.changed.push(oldest);
        }
    }
}

enum Snapshot {
    Offered,
    Loading,
    Ready,
    /// The message, and whether to draw the retry control beside it. Both halves, because this
    /// exists to be read out of the store before the drawing closures borrow `ctx`.
    Failed(String, bool),
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
        Some(State::Loading) => Snapshot::Loading,
        Some(State::Failed(f)) => Snapshot::Failed(f.why.clone(), f.offers_retry()),
        Some(State::Ready(_)) => Snapshot::Ready,
        None | Some(State::Offered) => Snapshot::Offered,
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
                                act = resp.clicked();
                                focused = resp.has_focus();
                            }
                        });
                    }
                    Snapshot::Offered | Snapshot::Ready => {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            ui.label(RichText::new("[image] ").color(pal.dim));
                            if !alt.is_empty() {
                                ui.label(RichText::new(alt).color(pal.fg));
                            }
                        });
                        let resp = ui.add(
                            egui::Button::new(
                                RichText::new(format!("load from {host}"))
                                    .color(pal.link)
                                    .underline(),
                            )
                            .frame(false),
                        );
                        super::follow_focus(&resp);
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
    if let Some(State::Ready(ready)) = ctx.images.state(src) {
        // Fitted inside the box rather than stretched to it: a mark that is not square is a
        // wide wordmark often enough, and a stretched one is worse than a small one.
        let scale =
            (box_size / ready.width.max(1) as f32).min(box_size / ready.height.max(1) as f32);
        let size = egui::vec2(ready.width as f32 * scale, ready.height as f32 * scale);
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(box_size, box_size), egui::Sense::hover());
        let at = egui::Rect::from_center_size(rect.center(), size);
        egui::Image::new(&ready.texture)
            .corner_radius(egui::CornerRadius::same(theme::RADIUS))
            .paint_at(ui, at);
        return;
    }
    ctx.note_icon(ui.next_widget_position().y, src);
    ui.allocate_space(egui::vec2(box_size, box_size));
}

/// The one control an entry's thumbnail gets: a small dim `[image]` beside the time, which
/// loads that one picture, and nothing once it is loaded. `Never` shows nothing at all.
pub fn load_control(ui: &mut Ui, img: &ir::Image, ctx: &mut RenderCtx<'_>) {
    // Nothing to offer once it is loaded, and nothing to offer for a decline a second request
    // cannot change: this is a control, and both of those make it a dead one.
    if !ctx.opts.images.offers_loading()
        || matches!(ctx.images.state(&img.src), Some(State::Ready(_)))
        || ctx.images.refuses_retry(&img.src)
    {
        return;
    }
    let pal = ctx.pal;
    ctx.note_unloaded_image(ui.next_widget_position().y, &img.src);
    let resp =
        ui.add(egui::Button::new(RichText::new("[image]").color(pal.dim).small()).frame(false));
    super::follow_focus(&resp);
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
    match ctx.images.state(&img.src) {
        Some(State::Ready(ready)) => {
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
        Some(State::Failed(f)) if !f.offers_retry() => {
            ui.label(
                RichText::new(format!("[{alt}] ({})", f.why))
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
        store.record_request("/a.png", "img.example.org", true);
        store.record_request("/b.png", "cdn.example.net", false);

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
        store.record_request("/a.png", "cdn.example.net", true);
        store.record_request("/b.png", "cdn.example.net", true);
        store.record_request("/c.png", "img.example.org", false);
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
        store.record_request("/a.png", "cdn.example.net", true);
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
        store.record_request("/a.png", "cdn.example.net", true);
        store.fail("/a.png", Failure::transient("not an image".to_owned()));
        store.forget("/a.png");
        let counts = store.counts();
        assert_eq!(counts.hosts, vec!["cdn.example.net"]);
        assert_eq!(counts.referer_requests, 1);
    }
}
