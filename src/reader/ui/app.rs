//! `ReaderApp`: state machine, keymap, chrome, and error screens.
//!
//! # Chrome, and the one piece of it that never hides
//!
//! The URL bar, the outline, the find bar, and the help overlay all hide on `Esc`. The status
//! strip at the bottom does not, and cannot, because charter point 4 requires that a rewrite is
//! reported *before* the request goes out, unconditionally, so a hang cannot swallow the
//! notice. A permanently visible one-line strip is what makes "chrome hidden until summoned"
//! and that requirement compatible, and the GUI actually realizes the invariant better than
//! the CLI does: the notice travels the channel before the blocking fetch starts, so it is on
//! screen for the whole of a 15 s hang instead of being interleaved with page text in a
//! terminal after the fact.
//!
//! The strip earns a second job from being there anyway. It is the one piece of chrome a
//! pointer can always reach, so back/forward and "summon the URL bar" live on it, and pointer
//! navigation costs no new persistent chrome. **No affordance is reachable by keyboard alone**:
//! a mouse-only reader must be able to follow a link, come back, and type a new URL.
//!
//! What it does *not* carry is provenance. Seven facts `·`-joined into one dim right-aligned
//! line crowded the URL, and with `wrap_mode = Truncate` on a narrow window the tail was not
//! drawn at all, which is a poor way to report anything. They are labelled rows in the
//! page-info panel now (`p`, or the circled `i`, `reader::ui::pageinfo_ui`). The two that could
//! not wait for a click went the other way and became bands: a truncated body and a rewrite
//! rule that landed on the wrong host both change how the page in front of the reader should be
//! read. The dead rule in particular used to be a twelve-second `flash` here, which is to say
//! it was unrecoverable by anyone who looked away.
//!
//! # Zoom is egui's, not ours
//!
//! egui already binds `Ctrl`+`+`/`=`/`-`/`0` to `zoom_factor` and folds `Ctrl`+wheel and pinch
//! into the same value. Binding font size to those keys does not resolve the collision; it
//! lands on top of it, because egui consumes the shortcut only *after* `update()` has run. So
//! the reader implements no zoom at all: the built-in is browser-identical, brings `Ctrl`+`0`
//! and trackpad pinch for free, and scales chrome with text the way full-page zoom should.
//! `[` and `]` (the reading measure) stay hww's own, because that is the knob a browser does
//! not have.
//!
//! Every `Ctrl` in the keymap is `Modifiers::COMMAND`, which is Cmd on macOS and Ctrl
//! elsewhere. Back and forward are the one place the platforms genuinely disagree, so both
//! `Alt`+arrows and `Cmd`+`[`/`]` are bound rather than picking a winner.

use crate::ir;
use crate::reader::history::History;
use crate::reader::measure::{self, Heights};
use crate::reader::notice::{self, Button, Notice};
use crate::reader::outline::{self, OutlineEntry};
use crate::reader::pageinfo;
use crate::reader::settings::{self, Settings};
use crate::reader::thread_tree::CommentKey;
use crate::reader::title;
use crate::reader::ui::images::ImageStore;
use crate::reader::ui::{
    Action, Launch, RenderCtx, blocks, fonts, net, notice_ui, pageinfo_ui, theme,
};
use crate::session::{self, LoadError, LoadOptions, Loaded, Rewrite, Target};
use eframe::egui::{self, Align, Key, Layout, Modifiers, RichText, Ui};
use net::{Job, Msg, Net, ReqId};
use std::collections::HashSet;
use std::time::{Duration, Instant};
use url::Url;

const URL_BAR_ID: &str = "hww-url-bar";
const FIND_ID: &str = "hww-find";

pub fn run(launch: Launch) -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("hww")
            .with_inner_size([900.0, 800.0])
            .with_min_inner_size([420.0, 320.0])
            .with_app_id("hww"),
        ..Default::default()
    };
    eframe::run_native(
        "hww",
        options,
        Box::new(|cc| Ok(Box::new(ReaderApp::new(cc, launch)?))),
    )
}

enum Page {
    /// Nothing asked for yet, so there is nothing to draw and nothing in flight.
    ///
    /// `ReaderApp::new` opens the URL bar over it when the session starts with no page; it is
    /// also what `take_shown` leaves behind for the length of one render, which is why the bar
    /// is not part of the state itself.
    Idle,
    Loading {
        url: Url,
        started: Instant,
        /// The page still on screen while this one is fetched, and the whole of what makes a
        /// navigation stop being a screen change.
        ///
        /// A click used to blank the reading column on the frame it happened: `Page::Loading`
        /// replaced the page, and `notice::loading` is a `Quiet` band, which fills the viewport.
        /// On a fast host that flashes for two frames and cannot be read; on a slow one it is a
        /// blank screen, so following a link reads as arriving somewhere empty and then being
        /// moved again. Keeping the outgoing page is the browser model and the one readers
        /// already have: the column does not change until a new document commits.
        ///
        /// `None` when there is genuinely nothing to keep: the session's first navigation, or a
        /// retry from a failure. Only then is the loading band drawn.
        from: Option<Box<Ready>>,
        /// Whether this navigation added the history entry now at the cursor.
        ///
        /// Only then may the next navigation overwrite that entry instead of appending, because
        /// only then does it belong to a page that was never rendered. A reload, a Back, and a
        /// Forward all reach `navigate` with `push: false` and leave a `Loading` standing over an
        /// entry that is somebody's real history, so overwriting on "a fetch is in flight" alone
        /// deletes it: `[A]`, press `r`, click a link before it answers, and A is gone.
        pushed: bool,
    },
    Ready(Box<Ready>),
    Failed {
        url: Url,
        error: LoadError,
        /// Whether the attempt that failed applied rewrite rules, so Retry repeats the same
        /// request rather than quietly making a different one.
        rewrite: bool,
    },
}

impl Page {
    /// Take the visible page out for the length of one render, leaving the rest of the state
    /// intact.
    ///
    /// `ready_screen` needs the page and the image store at once and both sit behind one
    /// `&mut self`, so it borrows the page by moving it and putting it back. The placeholder left
    /// behind has to be the **same variant**: swapping `Idle` into a `Loading` would drop the
    /// `url` and `started` that the status strip and the progress rule read on that very frame.
    ///
    /// A method on `Page` rather than on `ReaderApp` because it is a pure transition over this
    /// enum and nothing else, which is what makes it reachable by a test. `ReaderApp` needs a
    /// window.
    fn take_shown(&mut self) -> Option<Box<Ready>> {
        match self {
            Page::Loading { from, .. } => from.take(),
            Page::Ready(_) => match std::mem::replace(self, Page::Idle) {
                Page::Ready(r) => Some(r),
                _ => unreachable!("just matched"),
            },
            Page::Idle | Page::Failed { .. } => None,
        }
    }

    /// Whether a new navigation should overwrite the history entry at the cursor rather than
    /// append after it.
    ///
    /// True only for a navigation that is still in flight *and* put that entry there itself, so
    /// the entry belongs to a page that was never rendered. Clicking a second link before the
    /// first answers is the case it exists for. The near-miss is testing only that a fetch is in
    /// flight: `reload`, `go_back`, and `go_forward` all reach `navigate` with `push: false` and
    /// leave a `Loading` standing over an entry that is real history, and overwriting that
    /// deletes a page the reader actually visited.
    fn supersedes_its_history_entry(&self) -> bool {
        matches!(self, Page::Loading { pushed: true, .. })
    }

    /// Put back what [`Page::take_shown`] took.
    ///
    /// Must run before any deferred action: `apply` can reach `navigate`, which writes the page,
    /// and restoring after that would clobber the navigation with the page it replaced.
    fn restore_shown(&mut self, ready: Box<Ready>) {
        match self {
            Page::Loading { from, .. } => *from = Some(ready),
            // `take_shown` left `Idle` behind, and nothing since has set a page.
            _ => *self = Page::Ready(ready),
        }
    }
}

struct Ready {
    loaded: Loaded,
    outline: Vec<OutlineEntry>,
    /// `Some(i)` => `blocks[i]` repeats `doc.title` and is skipped.
    skip_block: Option<usize>,
    /// The navigation this page arrived on, and the id every image request made *from* it
    /// carries.
    ///
    /// Not `ReaderApp::current`, which is the navigation in flight: those are the same id only
    /// while nothing is loading. Once the outgoing page stays on screen it also stays clickable,
    /// so an image placeholder can be clicked on a page that is already being replaced. Stamping
    /// the job with `current` would hand that reply to whatever committed next, and if the two
    /// pages share an image `src`, which a logo, a sprite, or a CDN path routinely does on the
    /// same site, the new page would display a picture nobody clicked on it. Nothing is ever auto-loaded is
    /// the first line of the images charter; this field is what keeps it true.
    req: ReqId,
}

#[derive(Default)]
struct Chrome {
    url_bar: Option<String>,
    outline_open: bool,
    find: Option<String>,
    help_open: bool,
    info_open: bool,
}

pub struct ReaderApp {
    net: Net,
    settings: Settings,
    settings_dirty: bool,
    last_saved: Instant,
    history: History,
    page: Page,
    /// The navigation every reply is matched against. Anything older is stale by definition.
    current: Option<ReqId>,
    /// Sticky for the life of one navigation, and shown whether it succeeds or hangs.
    rewrite_notice: Option<String>,
    /// Transient, with the instant it was set so it can fade.
    status: Option<(String, Instant)>,
    /// The link under the pointer right now. Its own slot rather than a status message, or a
    /// moment's hover would bury whatever the reader was actually told.
    hover_href: Option<String>,
    /// Last frame's status-strip height, so the hover overlay can sit on its top edge.
    strip_height: f32,
    chrome: Chrome,
    images: ImageStore,
    /// How tall each block was the last time it was laid out, so the blocks off the window can
    /// be skipped. See `reader::measure` for why, and `ready_screen` for when it is ignored.
    heights: Heights,
    /// Toggles against the auto-collapse default (see `thread_ui::is_collapsed`).
    collapsed: HashSet<CommentKey>,
    find_current: usize,
    find_total: usize,
    /// One-shot: bring the current match into view on the next frame.
    ///
    /// Set by the steps that *move* the match (`n`, `N`, Enter, a changed query) and by
    /// nothing else, so scrolling away from a highlight leaves it where the reader put it.
    /// Re-centring every frame the match is on screen would make the page unscrollable while
    /// find is open.
    find_scroll: bool,
    /// Whether the *default* for this session applies rewrites. `--no-rewrite` clears it.
    rewrite: bool,
    /// Whether the navigation in flight applied them, which `R` makes differ from the default.
    last_rewrite: bool,
    /// One-shot: block index to bring into view on the next frame.
    scroll_to_block: Option<usize>,
    /// One-shot: pixels to scroll by, from `j`/`k`/`Space`.
    scroll_delta: f32,
    /// One-shot: absolute offset, from `g`/`G`.
    scroll_offset: Option<f32>,
    viewport_height: f32,
    /// Last frame's distance from the top of the page to the foot of it, which is where
    /// `Shift+G` and `End` go.
    ///
    /// Measured rather than spelled `f32::INFINITY`, which is the obvious spelling and panics:
    /// egui places the content at `inner_rect.min - offset` *before* it clamps the offset to
    /// the content, so an infinite offset puts that corner at `-inf`, and `-inf` against an
    /// unbounded content size is `NaN`. The reader then asks for a `NaN`-tall row and egui
    /// rejects it. A merely large finite offset does not panic but loses the layout to f32
    /// precision and lands somewhere in the middle of the document. The page has already been
    /// through a frame by the time anyone can press a key, so last frame's measurement is the
    /// answer, and it is the exact one.
    max_scroll: f32,
    /// Deferred to the end of the frame: `ctx.copy_text` inside a closure that already borrows
    /// the app would be a second mutable borrow.
    pending_copy: Option<String>,
    /// What Tab last landed on, read back by `Enter`, `Y`, `i`, and `z`. Set during render.
    focused_href: Option<String>,
    focused_image: Option<String>,
    focused_comment: Option<CommentKey>,
    /// Whether the *previous* frame's render found focus on the page, taken before the three
    /// above are cleared for the frame in flight.
    ///
    /// `lays_out_whole_page` runs inside that render, downstream of the clear and upstream of
    /// the write, so reading the fields themselves gets `None` on every frame and the guard
    /// they were meant to be never fires. The snapshot is the value the guard wanted: focus was
    /// on a link when this frame began, so the widget carrying it has to be drawn again.
    focus_was_on_page: bool,
    /// Frames that a Tab reaches into, counted down in `ui`.
    ///
    /// Two, not one. egui moves focus among the widgets of the frame the key arrived in, so
    /// the link Tab is reaching for has to be drawn on it; and when Tab arrives at the *last*
    /// focusable widget nothing takes focus that pass and `give_to_next` carries the request
    /// into the next one, where a page laid out in part would hand the wrap the first link in
    /// the band instead of the first in the document.
    tab_frames: u8,
    /// The link a click has been dispatched for, marked in place until the navigation ends.
    ///
    /// A widget id rather than an href: `html::link_block` copies one block-level anchor's href
    /// onto every inline in the block, and a front page repeats the same href in nav, headline,
    /// and footer, so matching on the string marks every occurrence instead of the one that was
    /// clicked. The id is stable across the mark being appended, because a new widget only
    /// shifts the ids of widgets after it.
    pending_link: Option<egui::Id>,
    /// One-shot: give the URL bar keyboard focus on the next frame, after it exists.
    focus_url_bar: bool,
    /// One-shot: the same for the find field. Focusing it from the keymap instead would hand
    /// the field the `/` that opened it: `consume_key` takes the key event, but the text event
    /// that arrives with it is a separate event, and a field focused earlier in the same frame
    /// still reads it.
    focus_find: bool,
}

/// Distance one wheel notch moves, in points.
///
/// egui reports a discrete wheel in lines and multiplies by `line_scroll_speed`, whose default
/// 40 points is a display measurement rather than a reading one: at the default body it is a
/// line and a half, which registers as the page not quite having moved. Deriving it from
/// [`theme::line_height_px`] instead makes a notch and a press of `j` the same distance, and
/// keeps them the same distance when the body size changes.
fn apply_scroll_speed(ctx: &egui::Context, settings: &Settings) {
    let step = scroll_step(settings);
    ctx.options_mut(|o| o.input_options.line_scroll_speed = step);
}

fn scroll_step(settings: &Settings) -> f32 {
    theme::line_height_px(&settings.read) * settings.scroll_lines()
}

/// Whether egui is building an AccessKit tree this pass, which is to say whether an assistive
/// technology is attached.
///
/// egui has no getter for it, so this asks the one question that answers it: `accesskit_node_builder`
/// returns `None` when accesskit is off, and the root node is the single id that always exists
/// while it is on, so asking for *that* one reads the state rather than creating a node nobody
/// wanted. The closure writes nothing.
fn accesskit_is_building(ctx: &egui::Context) -> bool {
    ctx.accesskit_node_builder(egui::accesskit_root_id(), |_| ())
        .is_some()
}

impl ReaderApp {
    /// Public so an embedder can own the app without owning `run`: `bin/shot.rs` wraps it to
    /// drive scenes and work the shutter. Nothing else calls it.
    pub fn new(cc: &eframe::CreationContext<'_>, launch: Launch) -> Result<Self, LoadError> {
        let net = Net::new(cc.egui_ctx.clone())?;
        // Before anything draws. `eframe` is built without `default_fonts`, so until this runs
        // there is no face registered at all and every string is blank rather than tofu.
        fonts::install(&cc.egui_ctx);
        cc.egui_ctx.set_zoom_factor(launch.settings.zoom_factor);
        apply_scroll_speed(&cc.egui_ctx, &launch.settings);
        let mut app = Self {
            net,
            settings: launch.settings,
            settings_dirty: false,
            last_saved: Instant::now(),
            history: History::new(),
            page: Page::Idle,
            current: None,
            rewrite_notice: None,
            status: launch.settings_note.map(|n| (n, Instant::now())),
            hover_href: None,
            strip_height: 0.0,
            chrome: Chrome::default(),
            images: ImageStore::default(),
            heights: Heights::default(),
            collapsed: HashSet::new(),
            find_current: 0,
            find_total: 0,
            find_scroll: false,
            rewrite: launch.rewrite,
            last_rewrite: launch.rewrite,
            scroll_to_block: None,
            scroll_delta: 0.0,
            scroll_offset: None,
            viewport_height: 600.0,
            max_scroll: 0.0,
            pending_copy: None,
            focused_href: None,
            focused_image: None,
            focused_comment: None,
            focus_was_on_page: false,
            tab_frames: 0,
            pending_link: None,
            focus_url_bar: false,
            focus_find: false,
        };
        match launch.start {
            Some(url) => app.navigate(url, app.rewrite, true),
            // With nothing to read, the URL bar is the only sensible thing on screen.
            None => app.chrome.url_bar = Some(String::new()),
        }
        Ok(app)
    }

    // ---------------------------------------------------------------- navigation

    /// Dispatch a navigation. **Nothing on screen changes here except the chrome.**
    ///
    /// The resets that belong to the page being *fetched* are not done here; they are
    /// [`ReaderApp::commit`]'s, and they run when a document actually takes the column. Doing
    /// them at dispatch is what used to clear the textures, collapse state, and scroll offset of
    /// the page the reader was still looking at.
    fn navigate(&mut self, url: Url, rewrite: bool, push: bool) {
        let req = self.net.mint();
        // The page on screen stays on screen. Taken rather than borrowed so a chain of
        // abandoned loads, Back pressed twice inside one slow fetch, carries the same page
        // forward instead of falling to blank on the second hop.
        // Whether the navigation being replaced owns the entry at the cursor: still in flight,
        // and it put that entry there itself. Read before the `take`, which leaves `Idle` behind.
        let superseding = self.page.supersedes_its_history_entry();
        let from = match std::mem::replace(&mut self.page, Page::Idle) {
            Page::Ready(r) => Some(r),
            Page::Loading { from, .. } => from,
            // A failure owns the viewport already; there is nothing under it to keep.
            Page::Idle | Page::Failed { .. } => None,
        };
        self.last_rewrite = rewrite;
        self.current = Some(req);
        self.rewrite_notice = None;
        self.pending_link = None;
        if push {
            // A navigation that never committed gets no entry of its own. Clicking a second link
            // before the first has answered is ordinary now that the page stays clickable, and
            // `push` on both would leave a middle entry for a page that was never rendered.
            if superseding {
                self.history.replace(url.clone());
            } else {
                self.history.push(url.clone());
            }
        }
        self.page = Page::Loading {
            url: url.clone(),
            started: Instant::now(),
            from,
            pushed: push,
        };
        self.net.submit(Job::Load {
            req,
            url,
            opts: LoadOptions { rewrite },
        });
    }

    /// Follow a link. The scheme gate runs here, before anything is dispatched: one place, so
    /// no widget can route around it.
    fn follow(&mut self, href: &str, rewrite: bool) {
        match session::classify_link(href) {
            Target::Navigate(url) => {
                // A `#fragment` on the page already open is a scroll, not a fetch.
                if let Some(fragment) = url.fragment()
                    && self.same_page(&url)
                {
                    self.jump_to_fragment(fragment);
                    return;
                }
                self.navigate(url, rewrite, true);
            }
            Target::OfferCopy { url, note } => {
                self.copy(&url);
                self.flash(format!("{url} is {note}, copied instead"));
            }
            Target::Refuse { url, reason } => {
                self.flash(format!("{reason}: {url}"));
            }
        }
    }

    /// Whether `url` addresses the page already on screen, fragment aside.
    ///
    /// The visible page, not the one being fetched. Against `Page::Ready` alone this returns
    /// `false` all through a navigation, and `follow` then treats a `#fragment` link on the page
    /// the reader is looking at as a fresh document fetch, superseding the navigation in flight
    /// to re-download the article already on screen.
    fn same_page(&self, url: &Url) -> bool {
        let Some(ready) = self.shown() else {
            return false;
        };
        let mut a = url.clone();
        a.set_fragment(None);
        let mut b = ready.loaded.prov.final_url.clone();
        b.set_fragment(None);
        a == b
    }

    fn jump_to_fragment(&mut self, fragment: &str) {
        let Some(ready) = self.shown() else {
            return;
        };
        match outline::find_fragment(&ready.outline, fragment) {
            Some(entry) => self.scroll_to_block = Some(entry.block),
            None => {
                // There are no block ids in the IR, so this cannot resolve exactly. Saying so
                // is the difference between a heuristic and a lie. It goes to the status strip
                // and nowhere else: an unresolved anchor is a navigation event, not a fact
                // about the page's content, and saying it twice was saying it badly.
                self.flash(format!("no section matching #{fragment} on this page"));
            }
        }
    }

    fn reload(&mut self, rewrite: bool) {
        if let Some(url) = self.history.current().cloned() {
            self.navigate(url, rewrite, false);
        }
    }

    fn go_back(&mut self) {
        if let Some(url) = self.history.back().cloned() {
            self.navigate(url, self.rewrite, false);
        }
    }

    fn go_forward(&mut self) {
        if let Some(url) = self.history.forward().cloned() {
            self.navigate(url, self.rewrite, false);
        }
    }

    // ---------------------------------------------------------------- images

    fn load_image(&mut self, ctx: &egui::Context, src: &str) {
        // The visible page, and *its* request id rather than `self.current`: see `Ready::req`.
        let Some(ready) = self.shown() else {
            return;
        };
        let page_req = ready.req;
        if self.images.is_pending(src) {
            return;
        }
        let base = ready.loaded.prov.final_url.clone();
        let Ok(url) = base.join(src) else {
            self.images.fail(src, "unusable image address".to_owned());
            return;
        };
        let host = url.host_str().unwrap_or("?").to_owned();
        self.images.begin(src);
        self.images.hosts.insert(host.clone());
        if self.settings.send_image_referer {
            self.images.referer_requests += 1;
        }
        // Decode straight to the reading column: without this, "load images" on a photo-heavy
        // page is a GPU-memory denial of service driven by untrusted input.
        let max_width =
            (theme::measure_px(ctx, &self.settings.read) * ctx.pixels_per_point()).max(64.0) as u32;
        self.net.submit(Job::Image {
            req: self.net.mint(),
            page: page_req,
            src: src.to_owned(),
            url,
            referrer: base,
            max_width,
            send_referer: self.settings.send_image_referer,
        });
        self.flash(format!("loading one image from {host}"));
    }

    fn load_all_images(&mut self, ctx: &egui::Context) {
        let Some(ready) = self.shown() else {
            return;
        };
        let srcs = collect_image_srcs(&ready.loaded.doc);
        let base = ready.loaded.prov.final_url.clone();
        let mut hosts: Vec<String> = srcs
            .iter()
            .filter_map(|s| base.join(s).ok())
            .filter_map(|u| u.host_str().map(str::to_owned))
            .collect();
        hosts.sort();
        hosts.dedup();
        if srcs.is_empty() {
            self.flash("no images on this page".to_owned());
            return;
        }
        // Naming the distinct hosts is the point: "load all" must not be the one place the
        // reader stops saying who it is about to contact.
        let count = srcs.len();
        for src in srcs {
            self.load_image(ctx, &src);
        }
        // Last, not first: `load_image` ends with its own "loading one image from …", so
        // saying this up front meant the strip only ever showed the final host and "load all"
        // became the one place the reader does not say who it is about to contact. Nothing has
        // been drawn yet (these are queued jobs, not completed ones), so the disclosure still
        // precedes any pixels.
        self.flash(format!(
            "loading {count} image(s) from {}",
            hosts.join(", ")
        ));
    }

    // ---------------------------------------------------------------- messages

    fn drain(&mut self, ctx: &egui::Context) {
        while let Some(msg) = self.net.try_recv() {
            match msg {
                Msg::Rewrote { req, rewrite } => {
                    if self.current == Some(req) {
                        let Rewrite::Applied { .. } = &rewrite;
                        self.rewrite_notice = Some(rewrite.to_string());
                    }
                }
                Msg::Loaded { req, loaded } => {
                    if self.current != Some(req) {
                        continue; // stale: the user moved on while this was in flight
                    }
                    self.settle(*loaded);
                }
                Msg::Failed { req, error } => {
                    if self.current != Some(req) {
                        continue;
                    }
                    let url = self.loading_url().unwrap_or_else(|| {
                        Url::parse("about:blank").expect("a literal URL parses")
                    });
                    // A failure replaces the page it was fetched over, so this is a commit too.
                    self.commit();
                    self.page = Page::Failed {
                        url,
                        error,
                        rewrite: self.last_rewrite,
                    };
                }
                Msg::Image {
                    page,
                    src,
                    cookies,
                    result,
                    ..
                } => {
                    // Against the page the request was made *from*, not the navigation in
                    // flight: see `Ready::req`. A picture whose page has been replaced is
                    // dropped, which is the whole reason the id is stamped there.
                    if self.shown().map(|r| r.req) != Some(page) {
                        continue;
                    }
                    // Counted before the result is looked at: a cookie the CDN tried to set is
                    // a thing that happened whether or not the picture ever decoded.
                    self.images.cookie_attempts += cookies;
                    match result {
                        Ok(decoded) => self.images.insert(ctx, &src, decoded),
                        Err(e) => self.images.fail(&src, e.to_string()),
                    }
                }
                Msg::Panicked { req, page } => {
                    if self.current != Some(page) {
                        continue;
                    }
                    self.flash("a worker failed while handling that request".to_owned());
                    // `req == page` only for a document load; see `Job::page`. Nothing is coming
                    // for it and nothing will time out either, because no worker still holds the
                    // job, so a `Page::Loading` left standing here spins the repaint floor at
                    // 10 fps for the rest of the session behind a progress rule pegged at the far
                    // edge, which `notice_ui` documents as meaning the request is at its deadline.
                    // An image panic is not this: its placeholder states its own case and the
                    // page is fine.
                    if req == page {
                        self.commit();
                        self.page = Page::Failed {
                            url: self.loading_url().unwrap_or_else(|| {
                                Url::parse("about:blank").expect("a literal URL parses")
                            }),
                            error: LoadError::Fetch(crate::fetch::FetchError::Io(
                                std::io::Error::other(
                                    "a worker failed while handling this request",
                                ),
                            )),
                            rewrite: self.last_rewrite,
                        };
                    }
                }
            }
        }
    }

    fn settle(&mut self, loaded: Loaded) {
        // A dead rewrite rule used to flash here for twelve seconds and then be unrecoverable.
        // It is a band now (`notice::about_page`), which lasts as long as the page it is about:
        // "this may not be the page you asked for" is not a thing to say once and withdraw.
        let outline = outline::build(&loaded.doc.blocks);
        let skip_block = title::dedupe(&loaded.doc);
        let fragment = self
            .history
            .current()
            .and_then(|u| u.fragment().map(str::to_owned));
        let req = self.current.unwrap_or_else(|| self.net.mint());
        // Before the page is installed, so `jump_to_fragment` below overrides the reset rather
        // than being overridden by it.
        self.commit();
        self.page = Page::Ready(Box::new(Ready {
            loaded,
            outline,
            skip_block,
            req,
        }));
        if let Some(f) = fragment {
            self.jump_to_fragment(&f);
        }
    }

    /// Put a page on screen that no fetch produced.
    ///
    /// The screenshot tool's one injection point, and the reason it can photograph a refused
    /// downgrade, a dead rewrite rule, or a body cut at the cap without arranging for a server
    /// that behaves that way. It takes the same `Loaded`/`LoadError` the worker sends back and
    /// runs the same `settle`, so a scene cannot drift into a page shape the pipeline cannot
    /// produce: everything downstream of here is the code a real navigation runs.
    ///
    /// The page-scoped resets are [`ReaderApp::commit`]'s, which both arms below reach; what is
    /// left here is the dispatch state a real navigation would have set, plus the history. A
    /// photographed page has to stand alone: pushing onto whatever came before would put the
    /// back arrow in the status strip in a different state depending on how many scenes ran
    /// ahead of this one, which is a picture that changes without the page changing.
    pub fn present(
        &mut self,
        url: Url,
        rewrite_notice: Option<String>,
        result: Result<Loaded, LoadError>,
    ) {
        self.current = Some(self.net.mint());
        self.rewrite_notice = rewrite_notice;
        self.history = History::new();
        self.history.push(url.clone());
        match result {
            Ok(loaded) => self.settle(loaded),
            Err(error) => {
                self.commit();
                self.page = Page::Failed {
                    url,
                    error,
                    rewrite: self.last_rewrite,
                }
            }
        }
    }

    /// Everything a new page takes over from the one it replaces.
    ///
    /// These used to run in `navigate`, on the frame of the click. That was correct while a
    /// navigation blanked the column and wrong the moment it stopped: the outgoing page is still
    /// being read, and it still needs its textures, its collapsed threads, its find position, and
    /// above all its scroll offset. Resetting the offset at dispatch is what made a click jolt
    /// the page to the top before anything had arrived.
    ///
    /// Called from both places a page actually takes the column, `settle` and the `Msg::Failed`
    /// arm of `drain`, and from `present`, so the screenshot tool cannot drift from the
    /// navigation path.
    fn commit(&mut self) {
        // Textures die with the page they were loaded for; the disclosure counters do not.
        self.images.clear_textures();
        self.heights.clear();
        self.collapsed.clear();
        self.find_current = 0;
        self.find_total = 0;
        self.pending_link = None;
        self.scroll_offset = Some(0.0);
        // The other two scroll one-shots, or a `Shift+G` pressed on the very frame a page
        // commits is applied against the *outgoing* page's `max_scroll` and opens the new one
        // at an arbitrary offset.
        self.scroll_delta = 0.0;
        self.scroll_to_block = None;
    }

    /// The page the reading column is drawing, which during a navigation is the outgoing one.
    ///
    /// Everything that describes what is *on screen* goes through this: the outline, the info
    /// panel, find, image loads, `y`, and the cautions in `notices`. The handful of things that
    /// describe the navigation instead, the strip's URL, the progress rule, and the repaint
    /// floor, match on `self.page` directly and are meant to.
    fn shown(&self) -> Option<&Ready> {
        match &self.page {
            Page::Ready(r) => Some(r),
            Page::Loading { from, .. } => from.as_deref(),
            Page::Idle | Page::Failed { .. } => None,
        }
    }

    fn loading_url(&self) -> Option<Url> {
        match &self.page {
            Page::Loading { url, .. } => Some(url.clone()),
            _ => self.history.current().cloned(),
        }
    }

    fn flash(&mut self, text: String) {
        self.status = Some((text, Instant::now()));
    }

    fn copy(&mut self, text: &str) {
        self.pending_copy = Some(text.to_owned());
    }

    // ---------------------------------------------------------------- keymap

    fn handle_keys(&mut self, ctx: &egui::Context) {
        // While a text field has focus, letters are text. Only Esc and Enter are ours.
        let typing = ctx
            .memory(|m| m.focused())
            .is_some_and(|id| id == egui::Id::new(URL_BAR_ID) || id == egui::Id::new(FIND_ID));

        let esc = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        if esc {
            // Never the status strip.
            self.chrome.url_bar = None;
            self.chrome.find = None;
            self.chrome.help_open = false;
            self.chrome.outline_open = false;
            self.chrome.info_open = false;
            ctx.memory_mut(|m| m.surrender_focus(egui::Id::new(URL_BAR_ID)));
            return;
        }
        if typing {
            return;
        }

        // Pointer side buttons: "typically corresponds to the Browser back/forward button".
        let (extra1, extra2) = ctx.input(|i| {
            (
                i.pointer.button_clicked(egui::PointerButton::Extra1),
                i.pointer.button_clicked(egui::PointerButton::Extra2),
            )
        });
        if extra1 {
            self.go_back();
        }
        if extra2 {
            self.go_forward();
        }

        let step = scroll_step(&self.settings);
        let line = theme::line_height_px(&self.settings.read);
        let page = (self.viewport_height - line * 2.0).max(line);
        // `consume_key` ignores *extra* Shift and Alt, so every shifted binding must be tested
        // before its unshifted twin or `G` would fire `g` on the way past.
        let k = |mods: Modifiers, key: Key| ctx.input_mut(|i| i.consume_key(mods, key));

        // --- shifted and modified bindings first
        if k(Modifiers::SHIFT, Key::G) || k(Modifiers::NONE, Key::End) {
            self.scroll_offset = Some(self.max_scroll);
        }
        if k(Modifiers::SHIFT, Key::R) {
            self.reload(false);
            self.flash("reloaded without rewrite rules".to_owned());
        }
        if k(Modifiers::SHIFT, Key::I) {
            self.load_all_images(ctx);
        }
        if k(Modifiers::SHIFT, Key::Y) {
            match self.focused_href.clone() {
                Some(h) => {
                    self.copy(&h);
                    self.flash(format!("copied {h}"));
                }
                None => self.flash("no link focused; Tab to one first".to_owned()),
            }
        }
        if k(Modifiers::SHIFT, Key::Z) {
            self.collapse_all();
        }
        if self.find_total > 0 && k(Modifiers::SHIFT, Key::N) {
            self.find_current = (self.find_current + self.find_total - 1) % self.find_total;
            self.find_scroll = true;
        }
        if k(Modifiers::SHIFT, Key::Space) || k(Modifiers::NONE, Key::PageUp) {
            self.scroll_delta += page;
        }
        if k(Modifiers::COMMAND, Key::L) {
            self.open_url_bar();
        }
        // Alt+arrows on Linux and Windows, Cmd+[ / ] on macOS. Both bound rather than picking
        // a winner; the mouse side buttons work identically everywhere.
        if k(Modifiers::ALT, Key::ArrowLeft) || k(Modifiers::COMMAND, Key::OpenBracket) {
            self.go_back();
        }
        if k(Modifiers::ALT, Key::ArrowRight) || k(Modifiers::COMMAND, Key::CloseBracket) {
            self.go_forward();
        }
        // `?` arrives as its own logical key, shift included.
        if k(Modifiers::NONE, Key::Questionmark) {
            self.chrome.help_open = !self.chrome.help_open;
        }

        // --- unshifted bindings
        if k(Modifiers::NONE, Key::J) || k(Modifiers::NONE, Key::ArrowDown) {
            self.scroll_delta -= step;
        }
        if k(Modifiers::NONE, Key::K) || k(Modifiers::NONE, Key::ArrowUp) {
            self.scroll_delta += step;
        }
        if k(Modifiers::NONE, Key::Space) || k(Modifiers::NONE, Key::PageDown) {
            self.scroll_delta -= page;
        }
        if k(Modifiers::NONE, Key::G) || k(Modifiers::NONE, Key::Home) {
            self.scroll_offset = Some(0.0);
        }
        if k(Modifiers::NONE, Key::O) {
            self.open_url_bar();
        }
        if k(Modifiers::NONE, Key::Backspace) {
            self.go_back();
        }
        if k(Modifiers::NONE, Key::R) {
            self.reload(self.rewrite);
        }
        if k(Modifiers::NONE, Key::T) {
            self.chrome.outline_open = !self.chrome.outline_open;
        }
        if k(Modifiers::NONE, Key::P) {
            self.chrome.info_open = !self.chrome.info_open;
        }
        if k(Modifiers::NONE, Key::Slash) {
            self.chrome.find = Some(self.chrome.find.take().unwrap_or_default());
            self.focus_find = true;
        }
        if self.find_total > 0 && k(Modifiers::NONE, Key::N) {
            self.find_current = (self.find_current + 1) % self.find_total;
            self.find_scroll = true;
        }
        if k(Modifiers::NONE, Key::I) {
            match self.focused_image.clone() {
                Some(src) => self.load_image(ctx, &src),
                None => self
                    .flash("no image focused; Tab to a placeholder, or Shift+I for all".to_owned()),
            }
        }
        // The page on screen, not `history.current()`. Two reasons: during a navigation the
        // history tip is the destination while the reader is still looking at the old article,
        // and even at rest the tip is the URL that was *requested* rather than the one the
        // redirects landed on. `HELP` calls this "copy page URL", so it should be the page's.
        if k(Modifiers::NONE, Key::Y)
            && let Some(u) = self
                .shown()
                .map(|r| r.loaded.prov.final_url.clone())
                .or_else(|| self.history.current().cloned())
        {
            self.copy(u.as_str());
            self.flash("page URL copied".to_owned());
        }
        if k(Modifiers::NONE, Key::OpenBracket) {
            self.settings.read.narrow();
            self.settings_dirty = true;
        }
        if k(Modifiers::NONE, Key::CloseBracket) {
            self.settings.read.widen();
            self.settings_dirty = true;
        }
        if k(Modifiers::NONE, Key::D) {
            self.settings.read.theme = self.settings.read.theme.next();
            self.settings_dirty = true;
            self.flash(format!("theme: {:?}", self.settings.read.theme));
        }
        if k(Modifiers::NONE, Key::Z) {
            self.collapse_focused();
        }
        if k(Modifiers::NONE, Key::Q) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        // Enter follows the focused link. egui's own Tab order does the cycling, because links
        // and comment toggles (and nothing else) are made focusable.
        if k(Modifiers::NONE, Key::Enter)
            && let Some(h) = self.focused_href.clone()
        {
            self.follow(&h, self.rewrite);
        }
    }

    fn open_url_bar(&mut self) {
        let current = self
            .history
            .current()
            .map(Url::to_string)
            .unwrap_or_default();
        self.chrome.url_bar = Some(current);
        self.focus_url_bar = true;
    }

    fn collapse_focused(&mut self) {
        match self.focused_comment.clone() {
            Some(key) => toggle(&mut self.collapsed, key),
            None => self.flash("no comment focused".to_owned()),
        }
    }

    fn collapse_all(&mut self) {
        let Some(ready) = self.shown() else {
            return;
        };
        let mut keys = Vec::new();
        for block in &ready.loaded.doc.blocks {
            if let ir::Block::Thread(comments) = block {
                let tree = crate::reader::thread_tree::build(comments);
                for (i, node) in tree.nodes.iter().enumerate() {
                    if !node.children.is_empty() && !tree.starts_collapsed(i) {
                        keys.push(node.key.clone());
                    }
                }
            }
        }
        if keys.is_empty() {
            self.flash("nothing to collapse".to_owned());
            return;
        }
        for k in keys {
            self.collapsed.insert(k);
        }
    }
}

fn toggle(set: &mut HashSet<CommentKey>, key: CommentKey) {
    if !set.remove(&key) {
        set.insert(key);
    }
}

fn collect_image_srcs(doc: &ir::Document) -> Vec<String> {
    fn walk_blocks(blocks: &[ir::Block], out: &mut Vec<String>) {
        for b in blocks {
            match b {
                ir::Block::Figure { image, .. } => out.push(image.src.clone()),
                ir::Block::Paragraph(inlines) | ir::Block::Heading { inlines, .. } => {
                    walk_inlines(inlines, out)
                }
                ir::Block::List { items, .. } => items.iter().for_each(|i| walk_blocks(i, out)),
                ir::Block::Quote { blocks, .. } => walk_blocks(blocks, out),
                ir::Block::Table { headers, rows } => {
                    headers.iter().for_each(|c| walk_inlines(c, out));
                    rows.iter().flatten().for_each(|c| walk_inlines(c, out));
                }
                ir::Block::Thread(cs) => cs.iter().for_each(|c| walk_blocks(&c.blocks, out)),
                ir::Block::Code { .. } | ir::Block::Rule | ir::Block::Embed { .. } => {}
            }
        }
    }
    fn walk_inlines(inlines: &[ir::Inline], out: &mut Vec<String>) {
        for i in inlines {
            match i {
                ir::Inline::Image(img) => out.push(img.src.clone()),
                ir::Inline::Emph(v) | ir::Inline::Strong(v) => walk_inlines(v, out),
                ir::Inline::Link { inlines, .. } => walk_inlines(inlines, out),
                ir::Inline::Text(_) | ir::Inline::Code(_) | ir::Inline::Break => {}
            }
        }
    }
    let mut out = Vec::new();
    walk_blocks(&doc.blocks, &mut out);
    // `Vec::dedup` only drops *adjacent* duplicates, and a page that reuses one `src` in two
    // places (a logo in header and footer, a repeated sprite) does not put them side by side.
    // The count feeds the "loading N image(s) from …" disclosure, so an inflated N is a
    // misstatement of what is about to be fetched. Sort by src to make the pairs adjacent, but
    // hand back document order: the reading column loads top to bottom.
    let mut seen = std::collections::HashSet::new();
    out.retain(|s| seen.insert(s.clone()));
    out
}

impl eframe::App for ReaderApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let pal = theme::apply(&ctx, &self.settings.read);
        // Cheap, and the body size it is derived from can change under `[`/`]` and a settings
        // reload, so it is re-derived rather than trusted from startup.
        apply_scroll_speed(&ctx, &self.settings);
        self.drain(&ctx);
        self.handle_keys(&ctx);

        // Focus and hover are discovered during render; start each frame not knowing. Hover
        // matters most: `ready_screen` is the only writer, so a value left standing survives
        // into `Loading` and onto the failure screen, where it pins the previous page's link
        // over hww's own words for as long as the fetch takes.
        self.focus_was_on_page = self.focused_href.is_some()
            || self.focused_image.is_some()
            || self.focused_comment.is_some();
        self.focused_href = None;
        self.focused_image = None;
        self.focused_comment = None;
        self.hover_href = None;

        // Read here rather than where it is used, so it can be held for the frame after as
        // well; see `tab_frames`. `handle_keys` has already run and does not consume Tab, and
        // egui's own focus handling reads it without taking it out of `events`.
        self.tab_frames = if ctx.input(|i| i.key_pressed(egui::Key::Tab)) {
            2
        } else {
            self.tab_frames.saturating_sub(1)
        };

        // Order matters: the strip is laid out first so it keeps its line at the bottom no
        // matter what the rest of the chrome does. It is the one thing that never hides.
        self.status_strip(ui, &pal);
        self.url_bar(ui, &pal);
        self.find_bar(ui, &pal);
        self.outline_panel(ui, &pal);
        self.central(ui, &pal);
        // *After* the page, because the page is what discovers the hover. An overlay takes no
        // space from anything, so nothing above depends on it having been drawn. See
        // `hover_strip`.
        self.hover_strip(ui, &pal);
        self.help_overlay(&ctx, &pal);
        self.info_panel(&ctx, &pal);

        if let Some(text) = self.pending_copy.take() {
            ctx.copy_text(text);
        }
        self.persist(&ctx);

        // While anything is in flight, keep repainting so the elapsed-time indicator animates
        // without a busy loop. Workers also repaint on every message, so this is the *floor*
        // on responsiveness, not the mechanism.
        // 100 ms while a document is in flight, 250 for images. The bar is why: at 4 fps a 15 s
        // rule on a wide window advances fifteen pixels a frame and visibly jumps, and "is it
        // stuck?" is the one question the indicator exists to answer. Bounded by the fetch
        // timeout, so there is no busy loop to worry about; an image has a placeholder that says
        // its own state and needs no such rate.
        if matches!(self.page, Page::Loading { .. }) {
            ctx.request_repaint_after(Duration::from_millis(100));
        } else if self.images.any_pending() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    /// The debounce in [`ReaderApp::persist`] exists so holding `[` does not write the file
    /// once per frame. It must not be the reason a change is *lost*: `last_saved` starts at
    /// construction, so changing the measure and pressing `q` inside the window discarded it
    /// silently. Quitting is exactly when a pending write has to land.
    fn on_exit(&mut self) {
        if self.settings_dirty {
            self.save_settings();
        }
    }
}

impl ReaderApp {
    /// How long `[`/`]` can be held down before a write lands.
    const SAVE_DEBOUNCE: Duration = Duration::from_millis(1500);

    /// Persist settings, but not on every keystroke of `[`/`]`.
    fn persist(&mut self, ctx: &egui::Context) {
        let zoom = ctx.zoom_factor();
        if (zoom - self.settings.zoom_factor).abs() > f32::EPSILON {
            self.settings.zoom_factor = zoom;
            self.settings_dirty = true;
        }
        if !self.settings_dirty {
            return;
        }
        let waited = self.last_saved.elapsed();
        if waited > Self::SAVE_DEBOUNCE {
            self.save_settings();
        } else {
            // egui only runs a frame when something asks it to. Without this the debounce
            // never expires on its own: change the measure, touch nothing else, and the write
            // waits for the next unrelated input instead of for 1.5s.
            ctx.request_repaint_after(Self::SAVE_DEBOUNCE - waited);
        }
    }

    fn save_settings(&mut self) {
        self.settings_dirty = false;
        self.last_saved = Instant::now();
        if let Err(e) = settings::save(&self.settings) {
            self.flash(format!("could not save settings: {e}"));
        }
    }

    // ---------------------------------------------------------------- chrome

    /// The one piece of chrome that never hides. See the module doc.
    fn status_strip(&mut self, ui: &mut Ui, pal: &theme::Palette) {
        let mut action: Option<StripAction> = None;
        let strip = egui::Panel::bottom("hww-status")
            // egui paints its own separator on a panel's inner edge, from
            // `noninteractive.bg_stroke`, and reserves outer-margin room for it. That is `rule`,
            // and next to `panel_edge`'s `guide` it made every docked panel a two-tone double
            // hairline. One edge, and it is the one this crate chose.
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(pal.bg)
                    .inner_margin(egui::Margin::symmetric(8, 4)),
            )
            .show(ui, |ui| {
                // Every word in this panel is the reader's, so the family is set once here
                // rather than at each label. Only `RichText::font` still wins: egui resolves
                // `override_font_id` *before* a `TextStyle`, so `.small()` and `.monospace()`
                // lose to it, and the labels that also set a colour name `chrome_font` outright
                // rather than relying on a style.
                ui.style_mut().override_font_id = Some(theme::chrome_font(&self.settings.read));
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                    // Pointer-reachable back/forward: a mouse-only reader must be able to
                    // return, and `Esc` may have hidden everything else.
                    if ui
                        .add_enabled(
                            self.history.can_go_back(),
                            egui::Button::new("◀").frame(false),
                        )
                        .on_hover_text("Back (Alt+Left)")
                        .clicked()
                    {
                        action = Some(StripAction::Back);
                    }
                    if ui
                        .add_enabled(
                            self.history.can_go_forward(),
                            egui::Button::new("▶").frame(false),
                        )
                        .on_hover_text("Forward (Alt+Right)")
                        .clicked()
                    {
                        action = Some(StripAction::Forward);
                    }

                    // The URL itself summons the URL bar: the pointer's route to typing one.
                    let shown = self.status_url();
                    if ui
                        .add(egui::Button::new(RichText::new(shown).color(pal.fg)).frame(false))
                        .on_hover_text("Click to edit (Ctrl+L)")
                        .clicked()
                    {
                        action = Some(StripAction::OpenUrlBar);
                    }

                    // Charter point 4: this line is on screen from before the request is sent
                    // until the navigation ends, whatever the fetch does.
                    if let Some(n) = &self.rewrite_notice {
                        ui.label(RichText::new(n).color(pal.notice_fg));
                    }
                    if let Some((text, at)) = &self.status
                        && at.elapsed() < Duration::from_secs(12)
                    {
                        ui.label(RichText::new(text).color(pal.notice_fg));
                    }

                    // The wrapper is what pins to the right edge; the icon alone would sit
                    // wherever the notices happened to end. What used to be here was seven
                    // facts `·`-joined into one dim truncated line, which crowded the URL and
                    // on a narrow window was not drawn at all. It is a panel now.
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if pageinfo_ui::icon(ui, pal, &self.settings.read).clicked() {
                            action = Some(StripAction::ToggleInfo);
                        }
                    });
                });
            });
        panel_edge(ui, strip.response.rect, Side::Top, pal);
        self.strip_height = strip.response.rect.height();
        // After the panel, so the rule is not clipped by its inner margin, and only while a
        // document is in flight. Images have their own placeholders and do not get a bar.
        if let Page::Loading { started, .. } = &self.page {
            notice_ui::progress_rule(
                ui.ctx(),
                pal,
                strip.response.rect,
                notice::elapsed_fraction(started.elapsed()),
            );
        }
        match action {
            Some(StripAction::Back) => self.go_back(),
            Some(StripAction::Forward) => self.go_forward(),
            Some(StripAction::OpenUrlBar) => self.open_url_bar(),
            Some(StripAction::ToggleInfo) => self.chrome.info_open = !self.chrome.info_open,
            None => {}
        }
    }

    /// The link under the pointer, on its own line *above* the strip.
    ///
    /// A URL is the one string in the chrome that has to be readable in full before the reader
    /// acts on it: a truncated host is exactly what a lookalike domain needs to pass. So it
    /// gets a line of its own, wrapping to as many rows as the address takes, instead of
    /// competing for the strip's single truncated row with the status message it used to bury.
    ///
    /// It is a **floating overlay, not a panel**, and that is load-bearing rather than
    /// stylistic. A panel takes its height out of the reading column, so the column reflows the
    /// instant the pointer touches a link. The link then moves out from under the pointer, the
    /// hover ends, the panel goes away, the column reflows back, and the pointer is over the
    /// link again: the URL strobes at frame rate for as long as the pointer is on the link.
    /// An overlay changes no other rect, and `interactable(false)` keeps it out of the hit test
    /// as well, so it cannot take the hover it exists to report.
    fn hover_strip(&mut self, ui: &mut Ui, pal: &theme::Palette) {
        let Some(href) = &self.hover_href else {
            return;
        };
        // A page can hand us an arbitrarily long `data:` href, and the overlay grows upward
        // without bound. Real addresses are nowhere near this; only a pathological one is cut.
        const MAX: usize = 2000;
        let shown = if href.chars().count() > MAX {
            let head: String = href.chars().take(MAX).collect();
            format!("{head}…")
        } else {
            href.clone()
        };
        let ctx = ui.ctx().clone();
        // Sits on the strip's top edge. `status_strip` runs earlier in the same frame, so
        // `strip_height` is this frame's measurement, not the previous one's.
        let width = (ctx.content_rect().width() - 16.0).max(64.0);
        egui::Area::new(egui::Id::new("hww-hover"))
            .order(egui::Order::Foreground)
            .interactable(false)
            .anchor(
                egui::Align2::LEFT_BOTTOM,
                egui::vec2(0.0, -self.strip_height),
            )
            .show(&ctx, |ui| {
                egui::Frame::new()
                    .fill(pal.chrome_bg)
                    .inner_margin(egui::Margin::symmetric(8, 4))
                    .show(ui, |ui| {
                        ui.style_mut().override_font_id =
                            Some(theme::chrome_font(&self.settings.read));
                        // An `Area` offers unbounded width, so wrapping needs an explicit one.
                        ui.set_max_width(width);
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                        ui.label(RichText::new(shown).color(pal.link));
                    });
            });
    }

    fn status_url(&self) -> String {
        match &self.page {
            // The address bar, so it names the destination rather than what is still on screen.
            Page::Loading { url, started, .. } => {
                format!(
                    "loading {} ({:.1}s)",
                    notice::host_and_path(url),
                    started.elapsed().as_secs_f32()
                )
            }
            Page::Ready(r) => notice::host_and_path(&r.loaded.prov.final_url),
            Page::Failed { url, .. } => notice::host_and_path(url),
            Page::Idle => "hww: press Ctrl+L for a URL, ? for help".to_owned(),
        }
    }

    fn url_bar(&mut self, ui: &mut Ui, pal: &theme::Palette) {
        let Some(mut text) = self.chrome.url_bar.take() else {
            return;
        };
        let mut go = false;
        let mut keep = true;
        let bar = egui::Panel::top("hww-url")
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(pal.bg)
                    .inner_margin(egui::Margin::symmetric(8, 4)),
            )
            .show(ui, |ui| {
                ui.style_mut().override_font_id = Some(theme::chrome_font(&self.settings.read));
                ui.horizontal(|ui| {
                    ui.label("URL");
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut text)
                            .id(egui::Id::new(URL_BAR_ID))
                            .desired_width(f32::INFINITY)
                            .hint_text("https://…"),
                    );
                    if self.focus_url_bar {
                        resp.request_focus();
                        self.focus_url_bar = false;
                    }
                    if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        go = true;
                        keep = false;
                    }
                });
            });
        panel_edge(ui, bar.response.rect, Side::Bottom, pal);
        if go {
            let typed = text.trim().to_owned();
            // A bare host is what people type. Assuming https is the only assumption that does
            // not silently downgrade.
            let candidate = if typed.contains("://") {
                typed.clone()
            } else {
                format!("https://{typed}")
            };
            match Url::parse(&candidate) {
                Ok(url) => self.navigate(url, self.rewrite, true),
                Err(e) => {
                    self.flash(format!("{typed} is not a usable address ({e})"));
                    keep = true;
                }
            }
        }
        if keep {
            self.chrome.url_bar = Some(text);
        }
    }

    fn find_bar(&mut self, ui: &mut Ui, pal: &theme::Palette) {
        let Some(mut query) = self.chrome.find.take() else {
            return;
        };
        let total = self.find_total;
        let current = self.find_current;
        let mut next = None;
        let bar = egui::Panel::top("hww-find")
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(pal.bg)
                    .inner_margin(egui::Margin::symmetric(8, 4)),
            )
            .show(ui, |ui| {
                ui.style_mut().override_font_id = Some(theme::chrome_font(&self.settings.read));
                ui.horizontal(|ui| {
                    ui.label("Find");
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut query)
                            .id(egui::Id::new(FIND_ID))
                            .desired_width(240.0),
                    );
                    if self.focus_find {
                        resp.request_focus();
                        self.focus_find = false;
                    }
                    if resp.changed() {
                        next = Some(0);
                    }
                    if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) && total > 0 {
                        next = Some((current + 1) % total);
                        resp.request_focus();
                    }
                    let shown = if query.is_empty() {
                        String::new()
                    } else if total == 0 {
                        "no matches".to_owned()
                    } else {
                        format!("{} of {total}", current + 1)
                    };
                    ui.label(shown);
                    ui.label(
                        RichText::new("n / N to step · Esc to dismiss")
                            .color(pal.dim)
                            .font(theme::chrome_font(&self.settings.read)),
                    );
                });
            });
        panel_edge(ui, bar.response.rect, Side::Bottom, pal);
        if let Some(n) = next {
            self.find_current = n;
            self.find_scroll = true;
        }
        self.chrome.find = Some(query);
    }

    fn outline_panel(&mut self, ui: &mut Ui, pal: &theme::Palette) {
        if !self.chrome.outline_open {
            return;
        }
        let Some(ready) = self.shown() else {
            return;
        };
        let entries: Vec<OutlineEntry> = ready.outline.clone();
        let mut jump = None;
        let panel = egui::Panel::left("hww-outline")
            .resizable(true)
            // See `status_strip`. A resizable panel still shows its transient line while the
            // edge is hovered or dragged, which is the affordance rather than the separation.
            .show_separator_line(false)
            .default_size(240.0)
            .frame(
                egui::Frame::new()
                    .fill(pal.bg)
                    .inner_margin(egui::Margin::symmetric(8, 6)),
            )
            .show(ui, |ui| {
                ui.style_mut().override_font_id = Some(theme::chrome_font(&self.settings.read));
                ui.label(
                    RichText::new("Outline")
                        .color(pal.dim)
                        .font(theme::chrome_font(&self.settings.read)),
                );
                ui.separator();
                if entries.is_empty() {
                    ui.label(RichText::new("no headings on this page").color(pal.dim));
                }
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for e in &entries {
                        ui.horizontal(|ui| {
                            ui.add_space(f32::from(e.level - 1) * 12.0);
                            if ui
                                .add(egui::Button::new(RichText::new(&e.text)).frame(false))
                                .clicked()
                            {
                                jump = Some(e.block);
                            }
                        });
                    }
                });
            });
        panel_edge(ui, panel.response.rect, Side::Right, pal);
        if let Some(b) = jump {
            self.scroll_to_block = Some(b);
        }
    }

    /// The page-info panel, and the rows it draws.
    ///
    /// Built only while the panel is open: `pageinfo::rows` allocates a dozen `String`s, and
    /// this runs every frame.
    fn info_panel(&mut self, ctx: &egui::Context, pal: &theme::Palette) {
        if !self.chrome.info_open {
            return;
        }
        // The page on screen, which during a navigation is the outgoing one. Its rows are all
        // true of what is being read, and its first row is the address, the reader's one route
        // to that while the status strip is showing the destination instead. "No page loaded."
        // over a plainly visible article is the lie here, not the rows.
        let rows = match self.shown() {
            Some(r) => pageinfo::rows(&r.loaded.prov, &self.images.counts()),
            None => Vec::new(),
        };
        let mut open = true;
        pageinfo_ui::panel(
            ctx,
            pal,
            &self.settings.read,
            &rows,
            self.strip_height,
            &mut open,
        );
        self.chrome.info_open = open;
    }

    fn help_overlay(&mut self, ctx: &egui::Context, pal: &theme::Palette) {
        if !self.chrome.help_open {
            return;
        }
        // An `Area` and not a `Window`. A `Window` brings a title bar egui paints from its own
        // visuals, which is why this was the one piece of chrome that did not take the reader's
        // palette. Two things `Window` was giving away for free have to be asked for: the order,
        // or this draws under `hover_strip`, which is `Foreground`; and the constraint, or a
        // help table taller than a short window runs off the bottom edge with no way back.
        egui::Area::new(egui::Id::new("hww-help"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .constrain(true)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(pal.chrome_bg)
                    .stroke(egui::Stroke::new(1.0, pal.guide))
                    .inner_margin(egui::Margin::same(12))
                    .show(ui, |ui| {
                        ui.style_mut().override_font_id =
                            Some(theme::chrome_font(&self.settings.read));
                        egui::Grid::new("hww-help").num_columns(2).show(ui, |ui| {
                            for (keys, what) in HELP {
                                ui.label(RichText::new(*keys).color(pal.strong).monospace());
                                ui.label(*what);
                                ui.end_row();
                            }
                        });
                        ui.separator();
                        ui.label(
                    RichText::new(
                        "Ctrl means Cmd on macOS. Zoom is egui's: Ctrl +/-/0, Ctrl+wheel, pinch.",
                    )
                    .color(pal.dim)
                    .font(theme::chrome_font(&self.settings.read)),
                );
                    });
            });
    }

    // ---------------------------------------------------------------- the page

    fn central(&mut self, ui: &mut Ui, pal: &theme::Palette) {
        // Rounded to whole *pixels*, not points: at a fractional zoom a column on a half-pixel
        // boundary makes every glyph in it resample, which reads as soft text rather than as a
        // bug, and egui's debug build paints an "Unaligned" warning over the page to say so.
        let wanted = theme::measure_px(ui.ctx(), &self.settings.read);
        self.wheel_during_drag(ui.ctx());
        let mut scroll = egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            // Drag-to-select depends on a mouse drag never being taken by the scroll area, so
            // this is pinned on purpose rather than left to a default that could change.
            .scroll_source(egui::containers::scroll_area::ScrollSource {
                drag: egui::containers::scroll_area::DragScroll::OnTouch,
                ..Default::default()
            });
        if let Some(offset) = self.scroll_offset.take() {
            scroll = scroll.vertical_scroll_offset(offset);
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(pal.bg))
            .show(ui, |ui| {
                self.viewport_height = ui.available_height();
                let out = scroll.show(ui, |ui| {
                    if self.scroll_delta != 0.0 {
                        ui.scroll_with_delta(egui::vec2(0.0, self.scroll_delta));
                        self.scroll_delta = 0.0;
                    }
                    // One geometry, two consumers. Computing the measure twice is how a band
                    // and the column would come to disagree about a left edge. The outline
                    // panel takes width from the column, so the measure is a ceiling rather
                    // than a promise; without the clamp inside `Column` the text is simply cut
                    // off on the right when the panel opens.
                    let column = notice_ui::Column::new(ui, wanted);

                    // Everything above this line is the reader. Below it is the page.
                    self.notices(ui, pal, column);

                    if self.shown().is_some() {
                        ui.horizontal_top(|ui| {
                            ui.add_space(column.gutter);
                            ui.vertical(|ui| {
                                ui.set_max_width(column.measure);
                                ui.set_min_width(column.measure);
                                ui.add_space(theme::snap(
                                    theme::line_height_px(&self.settings.read) * 1.5,
                                ));
                                self.ready_screen(ui);
                            });
                        });
                    }
                });
                self.viewport_height = out.inner_rect.height().max(1.0);
                self.max_scroll = (out.content_size.y - out.inner_rect.height()).max(0.0);
            });
    }

    /// The wheel, for the one case egui hands the page nothing: a live drag.
    ///
    /// `ScrollArea` tests `ctx.dragged_id().is_none()` before it will look at the wheel, so
    /// that a drag inside it never scrolls two things at once. A selectable `Label` senses
    /// `click_and_drag`, so holding the button down to select text sets that id and turns the
    /// wheel off for as long as the selection lasts, which is exactly when a selection running
    /// past the bottom of the window needs it. Take the wheel here instead and put it through
    /// the same one-shot `scroll_delta` that `j`/`k`/`Space` use; the reading column is the
    /// only vertical scroll in the window, so there is no second area for it to belong to.
    ///
    /// Zoom is unaffected: egui turns Ctrl+wheel into `zoom_delta` before this ever sees it.
    fn wheel_during_drag(&mut self, ctx: &egui::Context) {
        if ctx.dragged_id().is_none() {
            return;
        }
        // Zeroing it is what keeps the delta from being spent twice: `smooth_scroll_delta` is
        // what `ScrollArea` reads, and a drag that ends mid-smoothing would otherwise hand the
        // tail of this notch to the scroll area a second time.
        self.scroll_delta += ctx.input_mut(|i| std::mem::take(&mut i.smooth_scroll_delta.y));
    }

    /// Every notice for the page on screen, in the one place they are drawn.
    ///
    /// What this replaces is each screen drawing its own banner where it happened, which put
    /// the reader's words inside the reading column at the prose's own left edge and left
    /// colour as the only thing telling them apart.
    ///
    /// "On screen" rather than "current" is load-bearing during a navigation: the cautions belong
    /// to the article being read, so a truncated body or a dead rewrite rule must not blink out
    /// the moment a link is clicked and reappear when the next page lands.
    fn notices(&mut self, ui: &mut Ui, pal: &theme::Palette, column: notice_ui::Column) {
        // Owned, so the borrow of `self.page` ends here rather than running through the draw
        // loop and colliding with the `&mut self` that acting on a button needs.
        let notices: Vec<Notice> = match &self.page {
            Page::Idle => vec![notice::idle()],
            // The cautions belong to the page on screen and must not blink out under it: a
            // truncated body or a dead rewrite rule is still true of the article being read.
            // `notice::loading` returns `None` when there is one, so the two cannot both appear.
            Page::Loading {
                url, started, from, ..
            } => match from {
                Some(r) => notice::about_page(&r.loaded.prov, r.loaded.doc.text_len()),
                None => notice::loading(url, started.elapsed(), false)
                    .into_iter()
                    .collect(),
            },
            Page::Failed { url, error, .. } => vec![notice::failure(error, url)],
            Page::Ready(r) => notice::about_page(&r.loaded.prov, r.loaded.doc.text_len()),
        };
        let mut pressed = None;
        // Consecutive notices of one severity are one band. `about_page` can return four
        // cautions about the same page, and four bands of one ground sit flush and read as one
        // remark; see the `notice_ui` module doc. Grouping is by run rather than by sorting,
        // because the order `about_page` chose is the order the reader should read them in.
        for group in notices.chunk_by(|a, b| a.severity == b.severity) {
            if let Some(b) = notice_ui::band(ui, pal, &self.settings.read, column, group) {
                pressed = Some(b);
            }
        }

        // Acting is deferred past the borrow, the same shape `status_strip` uses for
        // `StripAction`: widgets record an intent, `app` acts on it once per frame.
        let (Some(b), Page::Failed { url, rewrite, .. }) = (pressed, &self.page) else {
            return;
        };
        let (url, rewrite) = (url.to_string(), *rewrite);
        match b {
            Button::Retry => self.reload(rewrite),
            Button::RetryWithoutRewrite => self.reload(false),
            Button::CopyUrl => {
                self.copy(&url);
                self.flash("URL copied".to_owned());
            }
        }
    }

    /// Draw the page the reading column is showing, which during a navigation is the outgoing
    /// one rather than the one being fetched.
    /// Whether this frame has to lay the page out whole, skipping nothing.
    ///
    /// Skipping a block off the window is invisible to the eye and is not invisible to
    /// everything that walks the page by *widget*. Each of these is a case where a block
    /// nowhere near the window still has to be one of this frame's widgets:
    ///
    /// - **Find.** `find_total` counts matches across the whole document, and `n` scrolls to
    ///   one that is by definition somewhere the reader is not looking.
    /// - **Tab.** egui moves focus among the widgets of the frame the key arrived in, so the
    ///   link Tab is reaching for has to be drawn on it, and on the frame after as well; see
    ///   `tab_frames`.
    /// - **Focus already on the page.** A focused widget that stops being drawn loses focus,
    ///   and `Enter`, `Y`, `i`, and `z` all read what Tab last landed on. `focus_was_on_page`
    ///   and not the three fields, which this is downstream of the clearing of.
    /// - **A selection.** egui builds the copy buffer out of the labels drawn this frame, so
    ///   a selection running off the window would put only the visible part on the clipboard.
    /// - **A screen reader.** egui builds the AccessKit tree out of the widgets drawn this
    ///   frame too, and a block replaced by empty space contributes no text to it. The eye
    ///   loses nothing to skipping and an assistive technology would lose the page, so the
    ///   whole optimization turns off for as long as one is attached.
    ///
    /// A jump (`scroll_to_block`) is the sixth and is tested at the call site: it scrolls to a
    /// `Response`, which a block replaced by empty space does not have.
    fn lays_out_whole_page(&self, ui: &Ui) -> bool {
        self.chrome.find.as_deref().is_some_and(|q| !q.is_empty())
            || self.focus_was_on_page
            || self.tab_frames > 0
            || ui
                .ctx()
                .plugin_opt::<egui::text_selection::LabelSelectionState>()
                .is_some_and(|p| p.lock().has_selection())
            || accesskit_is_building(ui.ctx())
    }

    fn ready_screen(&mut self, ui: &mut Ui) {
        // The page is behind `&mut self`, and rendering needs both it and the image store, so
        // take it for the duration and put it back. Cheaper than cloning a document.
        let Some(ready) = self.page.take_shown() else {
            unreachable!("checked by the caller")
        };
        // Both before `RenderCtx`, which borrows `self.images` for the rest of the function.
        let whole = self.scroll_to_block.is_some() || self.lays_out_whole_page(ui);
        self.heights.under(&measure::Layout {
            opts: self.settings.read.clone(),
            measure: ui.max_rect().width(),
            pixels_per_point: ui.ctx().pixels_per_point(),
            images: self.images.changes(),
            find: self.chrome.find.clone(),
            pending_link: self.pending_link.map(|id| id.value()),
        });
        let base = ready.loaded.prov.final_url.clone();
        let mut ctx = RenderCtx::new(
            theme::palette(self.settings.read.theme, theme::system_is_dark(ui.ctx())),
            &self.settings.read,
            base,
            &mut self.images,
            &self.collapsed,
        );
        ctx.find = self
            .chrome
            .find
            .as_deref()
            .map(str::to_lowercase)
            .unwrap_or_default();
        ctx.find_current = self.find_current;
        ctx.find_scroll = self.find_scroll;
        ctx.pending = self.pending_link;

        let doc = &ready.loaded.doc;
        blocks::document_header(ui, doc, &mut ctx);
        let scroll_to = self.scroll_to_block.take();
        let band = measure::Band::around(ui.clip_rect().top(), ui.clip_rect().bottom());
        for (i, b) in doc.blocks.iter().enumerate() {
            // The article's <h1> is usually also its <title>; rendering both shows the
            // headline twice.
            if ready.skip_block == Some(i) {
                continue;
            }
            // Empty space of the height this block measured last time, for as long as it is
            // clear of the window. `allocate_space` and not `add_space`, so the item spacing
            // either side of it is the spacing the block itself would have had.
            if !whole
                && let Some(h) = self.heights.get(i)
                && band.skips(ui.next_widget_position().y, h)
            {
                ui.allocate_space(egui::vec2(0.0, h));
                continue;
            }
            let resp = ui.scope(|ui| blocks::block_ui(ui, b, &mut ctx)).response;
            self.heights.set(i, resp.rect.height());
            if scroll_to == Some(i) {
                resp.scroll_to_me(Some(Align::TOP));
            }
        }
        ui.add_space(theme::snap(
            theme::line_height_px(&self.settings.read) * 4.0,
        ));

        self.find_total = ctx.find_seen;
        self.find_scroll = false;
        self.focused_href = ctx.focus_href.clone();
        self.focused_image = ctx.focus_image.clone();
        self.focused_comment = ctx.focus_comment.clone();
        self.hover_href = ctx.hover_href.clone();
        let action = ctx.action.take();
        let clicked = ctx.clicked_link;
        self.page.restore_shown(ready);
        // `current` changes only in `navigate`, so comparing it across `apply` is what says a
        // *new* fetch started. `matches!(self.page, Page::Loading { .. })` does not: with an
        // earlier navigation still in flight that is true whatever the click did, so a
        // `#fragment` link that only scrolled, or a `mailto:` that was copied and refused, would
        // drag the mark off the link actually loading and onto one that is not.
        let before = self.current;
        if let Some(a) = action {
            self.apply(ui.ctx(), a);
        }
        // After `apply`, because `navigate` clears `pending_link` and would otherwise wipe this
        // on the same line of control flow.
        if clicked.is_some() && self.current != before {
            self.pending_link = clicked;
        }
    }

    fn apply(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::Follow(href) => self.follow(&href, self.rewrite),
            Action::FollowWithoutRewrite(href) => self.follow(&href, false),
            Action::Copy(text) => {
                self.copy(&text);
                self.flash(format!("copied {text}"));
            }
            Action::LoadImage(src) => self.load_image(ctx, &src),
            Action::GoToBlock(b) => self.scroll_to_block = Some(b),
            Action::ToggleComment(key) => toggle(&mut self.collapsed, key),
        }
    }
}

/// Which side of a docked panel faces the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Top,
    Bottom,
    Right,
}

/// One rule on one side of a docked panel.
///
/// A docked panel takes `bg` and this; only a surface that *floats* over the page takes
/// `chrome_bg`. Position is what separates a docked panel from the page, and a fill on top of
/// position was saying the same thing twice in a client whose whole argument is that the page
/// gets no chrome treatment of its own.
///
/// It cannot be `Frame::stroke`, which draws four sides: the other three are the window's own
/// edges, and a box drawn around the status strip is not what this means. `guide` rather than
/// `rule`, because with the fill gone this hairline is the entire separation and `rule` measures
/// 1.41 / 1.41 / 1.48 against `bg`.
fn panel_edge(ui: &Ui, rect: egui::Rect, side: Side, pal: &theme::Palette) {
    let stroke = egui::Stroke::new(1.0, pal.guide);
    // Clipped to the panel's own rect rather than to the `Ui`'s, which has already shrunk around
    // the panel by the time its rect is known.
    let painter = ui.painter().with_clip_rect(rect);
    // Inset by half the stroke, so the whole hairline lands *inside* the panel. Centred on the
    // boundary, its outer half falls in the neighbouring panel's rect, and that panel's own fill
    // is added to this layer later in the frame and paints over it: the `guide` held to 3:1 by
    // `theme::guide_is_visible_where_it_is_drawn` rendered at half its width.
    let inset = 0.5 * stroke.width;
    match side {
        Side::Top => {
            painter.hline(rect.x_range(), theme::snap(rect.top()) + inset, stroke);
        }
        Side::Bottom => {
            painter.hline(rect.x_range(), theme::snap(rect.bottom()) - inset, stroke);
        }
        Side::Right => {
            painter.vline(theme::snap(rect.right()) - inset, rect.y_range(), stroke);
        }
    }
}

enum StripAction {
    Back,
    Forward,
    OpenUrlBar,
    ToggleInfo,
}

/// Written with the glyphs egui's embedded fonts actually carry. `→` is not one of them; it
/// renders as tofu, while `←`, `↑`, and `↓` are, which is why this table looks asymmetric and
/// is meant to.
const HELP: &[(&str, &str)] = &[
    ("j k ↓ ↑", "scroll"),
    ("Space / PgDn", "page down · Shift+Space pages up"),
    ("g / G", "top / bottom"),
    ("Ctrl+L or o", "URL bar · Enter navigates"),
    ("Tab / Shift+Tab", "cycle links · Enter follows"),
    ("Alt+Left / Backspace", "back"),
    ("Alt+Right", "forward · the mouse side buttons do both"),
    ("r / R", "reload · reload without rewrite rules"),
    ("i / I", "load focused image · load all, naming their hosts"),
    ("t", "outline"),
    ("p", "page info: how this page arrived"),
    ("/ then n / N", "find in page · next / previous match"),
    ("z / Z", "collapse focused reply · collapse all"),
    ("y / Y", "copy page URL · copy focused link"),
    ("[ / ]", "narrow / widen the reading measure"),
    ("d", "cycle theme"),
    ("? / Esc / q", "this help · dismiss chrome · quit"),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The third test under `ui/`, and the argument is made again rather than the precedent
    /// cited, as AGENTS.md requires.
    ///
    /// [`Page::take_shown`] and [`Page::restore_shown`] take no `egui::Context`: they are a pure
    /// transition over one enum, which is why they are methods on `Page` and not on `ReaderApp`.
    /// And the failure they can have is invisible to everything else that guards this directory.
    /// Losing the page in the round trip compiles, and `hww-shot` catches it only if a scene
    /// happens to photograph the one frame it occurs on. The render borrows the page and puts it
    /// back inside a single frame, so a picture taken before or after looks perfect.
    fn ready(url: &str, req: u64) -> Box<Ready> {
        let url = Url::parse(url).expect("a fixture URL parses");
        // Through the real extractor rather than hand-built, so this cannot drift into a page
        // shape the pipeline could not produce.
        let doc = crate::html::extract("<article><h1>Title</h1><p>Words.</p></article>", &url);
        Box::new(Ready {
            skip_block: title::dedupe(&doc),
            outline: outline::build(&doc.blocks),
            loaded: Loaded {
                doc,
                prov: crate::session::Provenance::fixture(url.as_str()),
            },
            req: ReqId::for_test(req),
        })
    }

    fn loading_pushed(from: Option<Box<Ready>>, pushed: bool) -> Page {
        Page::Loading {
            url: Url::parse("https://example.com/next").expect("a fixture URL parses"),
            started: Instant::now(),
            from,
            pushed,
        }
    }

    fn loading(from: Option<Box<Ready>>) -> Page {
        Page::Loading {
            url: Url::parse("https://example.com/next").expect("a fixture URL parses"),
            started: Instant::now(),
            from,
            pushed: true,
        }
    }

    #[test]
    fn taking_and_restoring_a_ready_page_is_the_identity() {
        let mut page = Page::Ready(ready("https://example.com/a", 1));
        let taken = page.take_shown().expect("a page to take");
        assert!(matches!(page, Page::Idle), "the placeholder is Idle");
        page.restore_shown(taken);
        let Page::Ready(r) = &page else {
            panic!("expected Ready back");
        };
        assert_eq!(r.req, ReqId::for_test(1));
    }

    /// The placeholder must keep the variant. Swapping `Idle` in would drop `url` and `started`,
    /// which the status strip and the progress rule read on the same frame the page is out.
    #[test]
    fn taking_the_page_out_of_a_load_keeps_the_load() {
        let mut page = loading(Some(ready("https://example.com/a", 1)));
        let started = match &page {
            Page::Loading { started, .. } => *started,
            _ => unreachable!(),
        };
        let taken = page.take_shown().expect("the outgoing page");
        match &page {
            Page::Loading {
                url,
                started: s,
                from,
                ..
            } => {
                assert_eq!(url.as_str(), "https://example.com/next");
                assert_eq!(*s, started);
                assert!(from.is_none());
            }
            _ => panic!("the load must survive its page being borrowed"),
        }
        page.restore_shown(taken);
        assert_eq!(
            page.take_shown().map(|r| r.req),
            Some(ReqId::for_test(1)),
            "restore must put it back where it came from"
        );
    }

    /// A reload, a Back, and a Forward leave a `Loading` over an entry they did not add. Only
    /// the in-flight navigation that added the entry may overwrite it; anything looser deletes a
    /// page the reader really visited.
    #[test]
    fn only_a_navigation_that_added_its_own_entry_may_overwrite_it() {
        assert!(loading_pushed(None, true).supersedes_its_history_entry());
        assert!(
            !loading_pushed(None, false).supersedes_its_history_entry(),
            "a reload or a Back is in flight over somebody else's entry"
        );
        assert!(!Page::Idle.supersedes_its_history_entry());
        assert!(
            !Page::Ready(ready("https://example.com/a", 1)).supersedes_its_history_entry(),
            "a settled page owns its entry and the next click appends after it"
        );
    }

    #[test]
    fn the_three_empty_states_have_nothing_to_show() {
        assert!(Page::Idle.take_shown().is_none());
        assert!(loading(None).take_shown().is_none());
        let mut failed = Page::Failed {
            url: Url::parse("https://example.com/a").expect("a fixture URL parses"),
            error: LoadError::Fetch(crate::fetch::FetchError::LikelyBlocked),
            rewrite: false,
        };
        assert!(failed.take_shown().is_none());
    }
}
