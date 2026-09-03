//! `ReaderApp`: state machine, keymap, chrome, and error screens.
//!
//! # Chrome, and the one piece of it that never hides
//!
//! The URL bar, the outline, the find bar, and the help overlay all hide on `Esc`. The status
//! strip at the bottom does not, and cannot, because charter point 4 requires that a rewrite is
//! reported *before* the request goes out, unconditionally, so a hang cannot swallow the
//! notice. A permanently visible one-line strip is what makes "chrome hidden until summoned"
//! and that requirement compatible: the notice travels the channel before the blocking fetch
//! starts, so it is on screen for the whole of a 15 s hang rather than arriving after it.
//!
//! The strip earns a second job from being there anyway. It is the one piece of chrome a
//! pointer can always reach, so back/forward and "summon the URL bar" live on it, and pointer
//! navigation costs no new persistent chrome. **No affordance is reachable by keyboard alone**:
//! a mouse-only reader must be able to follow a link, come back, and type a new URL.
//!
//! What it does *not* carry is provenance. Seven facts `·`-joined into one dim right-aligned
//! line crowded the URL, and with `wrap_mode = Truncate` on a narrow window the tail was not
//! drawn at all, which is a poor way to report anything. They are labelled rows in the
//! page-info panel now (its key, or the italic `i`, `reader::ui::pageinfo_ui`). The two that could
//! not wait for a click went the other way and became infobars under the top chrome: a truncated
//! body and a rewrite rule that landed on the wrong host both change how the page in front of
//! the reader should be read. The dead rule in particular used to be a twelve-second `flash`
//! here, which is to say it was unrecoverable by anyone who looked away.
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

use crate::fetch;
use crate::ir;
use crate::reader::archive::{self, Archive, Stored, Visited};
use crate::reader::autoload;
use crate::reader::desktop;
use crate::reader::history::{EntryId, History};
use crate::reader::iconcache;
use crate::reader::measure::{self, Heights};
use crate::reader::menu::{self, Command};
use crate::reader::notice::{
    self, Button, IMAGES_ALL_FAILED, IMAGES_ARE_DECLINED, IMAGES_ARE_DECLINED_OR_FAILED,
    IMAGES_ARE_OFF, NO_SITE_ICON, Notice,
};
use crate::reader::outline::{self, OutlineEntry};
use crate::reader::pagecache::{PageCache, stash_under};
use crate::reader::pageinfo;
use crate::reader::prefs;
use crate::reader::settings::{self, Settings};
use crate::reader::thread_tree::{self, CommentKey};
use crate::reader::title;
use crate::reader::ui::images::{Failure, ImageStore, Source};
use crate::reader::ui::{
    Action, Launch, RenderCtx, blocks, fonts, menu_ui, net, notice_ui, pageinfo_ui, prefs_ui, theme,
};
use crate::reader::{image_decode, image_formats};
use crate::session::{self, LoadError, LoadOptions, Loaded, Rewrite, Target};
use eframe::egui::{self, Align, Key, Layout, Modifiers, RichText, Ui};
use net::{Job, Kept, Msg, Net, ReqId};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use url::Url;

/// The width every site mark is decoded to, in physical pixels.
///
/// One number, so the masthead's mark, a list row's, and one read back out of
/// `reader::iconcache` cannot drift apart — a stored icon decoded at another size would be a
/// second, subtly different picture for the same address.
const ICON_PX: u32 = 64;

/// What one image request is, to the store on disk.
///
/// Three answers and not a bool, because "may be drawn from the store" and "may be written to
/// the store" are different permissions and the middle case is the one that matters:
/// `ReaderApp::load_site_icons` is handed addresses a marked list still names and so may write,
/// while `ReaderApp::ensure_favicon` fires on every page hww shows and may write only for the
/// one it can prove the archive names — `ReaderApp::favicon_mark` is that proof, and answers
/// `Read` for every other page. A bool would have collapsed the two into whichever answer was
/// written first, and the collapse in the permissive direction is a favicon on disk for every
/// host the reader visited.
///
/// It answers a third question with the same three words, and `Kept` is the only yes to that
/// one too: whether the entry belongs to a *list* rather than to the page on screen, which is
/// what decides the ceiling it is evicted against and whether it survives the next commit. See
/// `ReaderApp::note_list_mark`, and `images::MARK_CAPACITY` for the budget it buys.
///
/// The two questions agree on every caller hww has, and they are still asked apart: a later
/// surface that draws a bookmark's mark without writing one would need the class and not the
/// permission, and a `Mark` read as a single yes-or-no would hand it both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mark {
    /// Not a mark: an article picture, which never touches the store in either direction.
    No,
    /// A mark that may be drawn from the store and never written to it.
    Read,
    /// A mark for a row of a marked list: drawn from the store, and written to it.
    Kept,
}

/// Whether one image request may carry a Referer: the reader's setting, and never for a site
/// mark of any kind.
///
/// Until the sidebar existed every [`Mark::Kept`] request was made while a built page was on
/// screen — `hww:bookmarks` or `hww:reading-list` — and a built page's origin is opaque, so
/// `fetch::page_origin` answered `None` and the header went out empty however the setting was
/// set. The strip draws those same rows over an ordinary page, so `base` became whatever the
/// reader happens to be reading, and every bookmarked host's icon server would have been told it.
/// That is a cross-site disclosure carried by a request the reader did not make, in exchange for
/// nothing: a site mark is identity, and no host serves one differently for a referrer.
///
/// [`Mark::Read`] is refused for a second reason, and it is the sharper one. A page's own favicon
/// would carry only that page's origin, which tells its host what it already knows — but
/// `ReaderApp::favicon_mark` makes the *same* address a `Mark::Kept` request when a bookmark
/// names it, and a rule that sent the header for one and not the other would put the reader's
/// bookmarks in the request: a host watching for the missing Referer would learn which of its
/// pages this reader has kept. One answer for every mark is what closes that, and it costs a
/// header nobody needed.
fn sends_referer(setting: bool, mark: Mark) -> bool {
    setting && mark == Mark::No
}

/// Who owns a mark drawn with nothing in the reading column, which is only ever the sidebar.
///
/// [`Mark::Read`] is a page's own favicon and there is no page to draw it beside; [`Mark::No`]
/// is an article picture, which belongs to a page by definition. Neither has anything to draw
/// with no page, so neither gets an owner and neither reaches the door.
fn chrome_owner(mark: Mark) -> Option<ReqId> {
    (mark == Mark::Kept).then_some(ReqId::CHROME)
}

/// [`ReaderApp::awaits`], with the two facts it reads passed in: what is on screen, and whether
/// the image store is still waiting for this address.
///
/// Its own function so the rule can be tested. Getting it wrong is invisible in both directions
/// and neither shows up in a picture: too strict and the sidebar's marks are dropped on arrival
/// and the strip stays a column of empty squares, too loose and a texture from a page the reader
/// left is uploaded over the one they are reading.
fn reply_is_wanted(page: ReqId, shown: Option<ReqId>, pending: bool) -> bool {
    if page == ReqId::CHROME {
        return pending;
    }
    shown == Some(page)
}

const URL_BAR_ID: &str = "hww-url-bar";
const FIND_ID: &str = "hww-find";
const APP_ID: &str = "hww";

pub fn run(launch: Launch) -> eframe::Result {
    // Before the event loop, because the event that carries a link into a bundled application
    // on macOS is dispatched as soon as that loop turns. See `reader::mac_open`, which is also
    // where the argument for why `argv` is not enough on that platform lives.
    #[cfg(target_os = "macos")]
    crate::reader::mac_open::install();
    let icon =
        eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/logo/hww-256.png"))
            .expect("embedded hww icon is valid PNG");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(notice::WINDOW_NAME)
            .with_inner_size([900.0, 800.0])
            .with_min_inner_size([420.0, 320.0])
            .with_app_id(APP_ID)
            .with_icon(icon),
        ..Default::default()
    };
    eframe::run_native(
        "hww",
        options,
        Box::new(|cc| {
            let mut app = ReaderApp::new(cc, launch)?;
            // Only the application follows the desktop; see `ReaderApp::desktop_theme`. A
            // change arrives on another thread, so it has to ask for the frame that repaints
            // in the new palette.
            let ctx = cc.egui_ctx.clone();
            app.desktop_theme = desktop::watch(move || ctx.request_repaint());
            // The second half of `mac_open`, now that there is a window to wake: a link chosen
            // in Finder while hww is already open arrives as an Apple Event, which is no reason
            // for the event loop to draw, so without this the page would open on the reader's
            // next keystroke instead of on their click.
            #[cfg(target_os = "macos")]
            {
                let ctx = cc.egui_ctx.clone();
                crate::reader::mac_open::wake_with(move || ctx.request_repaint());
            }
            Ok(Box::new(app))
        }),
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
        /// replaced the page, and `notice::loading` took the viewport. On a fast host that
        /// flashes for two frames and cannot be read; on a slow one it is a blank screen, so
        /// following a link reads as arriving somewhere empty and then being moved again.
        /// Keeping the outgoing page is the browser model and the one readers already have: the
        /// column does not change until a new document commits.
        ///
        /// `None` when there is genuinely nothing to keep: the session's first navigation, or a
        /// retry from a failure. Only then is the loading line drawn.
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
        /// The options the failed attempt used, so Retry repeats the same request rather
        /// than quietly making a different one.
        opts: LoadOptions,
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
    /// Which leading blocks the masthead already says, and so are skipped.
    masthead: title::Masthead,
    /// Everything below this line is the document read once, where `outline` and `masthead`
    /// already are. Each was being recomputed on every frame from a document that cannot change
    /// while it is on screen: an image landing writes `ReaderApp::images`, collapsing a reply
    /// writes `ReaderApp::collapsed`, and find writes `ReaderApp::chrome`. None of them touches
    /// `loaded.doc`, and a page put away in `reader::pagecache` carries these back with it.
    ///
    /// `ir::Document::text_len`, for `notice::about_page`. **Not `prov.chars`**, which holds the
    /// same number for a fetched page and zero for a built one: `session::Provenance::built` is
    /// an address and zeros, and reading it here would put a zero one guard away from telling a
    /// reader that their own bookmarks list may need JavaScript to show its content.
    text_len: usize,
    /// Whether any of the page is right-to-left, for `notice::rtl` and the page-info panel.
    /// `notice::carries_rtl` walks every block and stops at the first hit, so it is the ordinary
    /// left-to-right page — every page, almost always — that pays for the whole walk.
    rtl: bool,
    /// The comment tree of each `ir::Block::Thread`, by top-level block index.
    ///
    /// `thread_tree::build` is a `CommentKey` allocation per comment and an
    /// `ir::blocks_text_len` walk of the whole thread for the auto-collapse measure, and it ran
    /// at the top of `thread_ui` — before the band was consulted, so a two-thousand-comment
    /// discussion rebuilt its tree on every frame it was on screen whether or not a single
    /// comment was drawn. Built here rather than in `ReaderApp` so `ReaderApp::collapse_all`,
    /// which was building its own, reads the same trees: two builds of one tree is two places
    /// for a `CommentKey` to be derived differently.
    threads: HashMap<usize, thread_tree::ThreadTree>,
    /// The navigation this page arrived on, and the id every image request made *from* it
    /// carries.
    ///
    /// Not `ReaderApp::current`, which is the navigation in flight: those are the same id only
    /// while nothing is loading. Once the outgoing page stays on screen it also stays clickable,
    /// so an image placeholder can be clicked on a page that is already being replaced. Stamping
    /// the job with `current` would hand that reply to whatever committed next, and if the two
    /// pages share an image `src`, which a logo, a sprite, or a CDN path routinely does on the
    /// same site, the new page would display a picture nobody asked for on it. That is the
    /// whole job of this field, and the automatic policy makes it matter more rather than less:
    /// `ReaderApp::autoload` plans against the page this id names, and the pool compares it
    /// with `Net::mint_page`'s value to drop what was queued for a page that has been left.
    /// The masthead favicon is stamped with the same id, so a late reply cannot land on the
    /// next page.
    req: ReqId,
    /// The options this document was fetched under, so a Back that would be made with different
    /// ones fetches instead of being answered from memory.
    ///
    /// **Not `ReaderApp::last_opts`**, which by the time this page is filed away already holds
    /// the options of the navigation replacing it: `navigate` writes it at dispatch. Filing with
    /// that would put a bare-reloaded page under the default options and hand it back on an
    /// ordinary
    /// Back. The rule is `reader::pagecache`'s: the cache may only answer with the page the
    /// fetch it replaced would have produced.
    opts: LoadOptions,
    /// When the document arrived from the network.
    ///
    /// Survives a trip through the page cache unchanged, so a page restored twice still reports
    /// the age of the fetch rather than the age of the last restore.
    fetched: Instant,
    /// Put back on screen from memory rather than fetched. Everything else this page reports
    /// describes the fetch above, which stays true because the page-info panel says this first.
    restored: bool,
    /// hww assembled this page itself; nothing was requested for it.
    ///
    /// Three things read it, and each would be a lie without it. `arrival` reports it, so the
    /// page-info panel draws no Response rows over a provenance that is all zeros; `commit`
    /// refuses to file it in the page cache, so Back rebuilds it and sees what has been kept
    /// since; and "keep this page" is gated on it being false, because `Needs::Page` is
    /// satisfied on the bookmarks view itself and would otherwise offer to save the bookmarks into the
    /// bookmarks.
    built: bool,
}

/// The three page-derived values [`Ready`] carries beside `outline` and `masthead`.
///
/// One function so the two places a page is installed — a fetch settling and a view hww built —
/// cannot come to different conclusions about the same document.
fn derived(doc: &ir::Document) -> (usize, bool, HashMap<usize, thread_tree::ThreadTree>) {
    let threads = doc
        .blocks
        .iter()
        .enumerate()
        .filter_map(|(i, b)| match b {
            ir::Block::Thread(comments) => Some((i, thread_tree::build(comments))),
            _ => None,
        })
        .collect();
    (doc.text_len(), notice::carries_rtl(doc), threads)
}

impl Ready {
    /// How this page got into the column, for `pageinfo::rows`.
    ///
    /// Here rather than at the panel so the two fields above are read in one place: the age is
    /// the *fetch*'s, so a page put back twice still reports how old the document is.
    fn arrival(&self) -> pageinfo::Arrival {
        // First, ahead of `restored`. A built page is never cached and so never restored, and
        // this ordering is what makes that a structural fact rather than a thing to remember:
        // no future path can make a built page report a fetch it did not make.
        if self.built {
            pageinfo::Arrival::Built
        } else if self.restored {
            pageinfo::Arrival::FromMemory {
                age: self.fetched.elapsed(),
            }
        } else {
            pageinfo::Arrival::Fetched
        }
    }
}

#[derive(Default)]
struct Chrome {
    url_bar: Option<String>,
    outline_open: bool,
    find: Option<String>,
    help_open: bool,
    about_open: bool,
    info_open: bool,
    settings_open: bool,
    /// Set by `F10`, consumed by the next `menu_bar` draw. `MenuButton` takes
    /// `ui.next_auto_id()`, so the title's id does not exist until the bar has been laid out
    /// and focus has to be requested on the response rather than on an id known in advance.
    focus_menu_bar: bool,
    /// Notice bars closed on the page on screen, by `Notice::key`. Cleared in `commit`, with
    /// every other per-page reset: a dismissal is about *this* page and no other.
    dismissed: HashSet<String>,
}

pub struct ReaderApp {
    net: Net,
    settings: Settings,
    settings_dirty: bool,
    last_saved: Instant,
    history: History,
    /// What the reader asked hww to keep, and the pages hww wrote down. Loaded once at start-up
    /// and written the moment it changes; see `reader::archive` for the doctrine and for why
    /// there is no debounce.
    archive: Archive,
    /// Whether the last automatic write of the archive failed.
    ///
    /// Only [`ReaderApp::save_visits`] reads it, and the noise is why: a keep is a keypress and
    /// says so every time it cannot be written, but a visit is written behind every navigation,
    /// and a config directory that cannot be written would otherwise put the same sentence over
    /// every page the reader opens until they learn to ignore toasts.
    archive_write_failed: bool,
    /// When the oldest visit that is not yet in the file was recorded, or `None` when the file
    /// is up to date.
    ///
    /// The instant of the *first* unwritten visit rather than the latest, which is what bounds
    /// the delay in both directions: a reader clicking steadily still gets a write every
    /// `SAVE_DEBOUNCE`, and a single navigation gets one that lands after the page is up rather
    /// than on the frame it appears. See [`ReaderApp::record_visit`].
    visits_dirty: Option<Instant>,
    /// Whether the page on screen is in the bookmarks.
    ///
    /// Cached rather than asked, and the cost is why: `Archive::is_bookmarked` parses every stored
    /// address, the menu bar wants the answer on every frame it draws, and the page-info panel
    /// wants it again. Two places can change it — the page, and the store — and
    /// [`ReaderApp::refresh_shown_kept`] is called from both.
    shown_bookmarked: bool,
    /// The documents Back and Forward can open without asking the site again, keyed by the
    /// entry each belongs to. Filled by [`ReaderApp::commit`] as a page leaves the column and
    /// emptied by [`ReaderApp::restore`] as one comes back; see `reader::pagecache`.
    pages: PageCache<Box<Ready>>,
    page: Page,
    /// The navigation every reply is matched against. Anything older is stale by definition.
    current: Option<ReqId>,
    /// Charter point 4: on the strip from before the request goes out until the navigation
    /// ends, whether it hangs or fails. Cleared in [`Self::settle`]: once a page is up,
    /// [`notice::rewrite`] is the readable one.
    rewrite_notice: Option<String>,
    /// Transient, with the instant it was set so it can fade: the toast (`notice_ui::toast`),
    /// up for `notice::TOAST_FOR`.
    status: Option<(String, Instant)>,
    /// The link under the pointer right now. Its own slot rather than a status message, or a
    /// moment's hover would bury whatever the reader was actually told.
    ///
    /// **One slot for every surface**, page and chrome alike. `ui` clears it before any panel
    /// runs and each surface writes it as the pointer reaches a link, so the write in `central`
    /// has to leave a value alone rather than assign over it: the bookmarks sidebar draws
    /// before the page and an unconditional assignment would erase its row every frame. The
    /// pointer is only ever in one place, so the surfaces never compete for it, and a later
    /// chrome surface that reports a hover needs no slot of its own.
    hover_href: Option<String>,
    /// Last frame's status-strip height, so the hover overlay can sit on its top edge.
    strip_height: f32,
    /// This frame's hover-URL overlay height, measured in [`Self::hover_strip`] before the
    /// toast is drawn, so a wrapping address lifts the pill by what was actually painted.
    hover_height: f32,
    chrome: Chrome,
    images: ImageStore,
    /// Which blocks of the page on screen contain each image `src`. Built when the page
    /// commits and rebuilt when the lead-in budget moves under it; see [`image_blocks`].
    image_blocks: HashMap<String, Vec<usize>>,
    /// The `entry_summary_bytes` [`Self::image_blocks`] was built under.
    ///
    /// The map is a function of the document *and* that budget: a longer lead-in draws figures
    /// a shorter one cut, so raising the setting with a feed open leaves the map describing
    /// blocks that are no longer the ones on screen. `collect_image_srcs` reads the live value
    /// and would go on to request a picture the stale map has no entry for, so its arrival
    /// invalidated no height and the entry kept a placeholder-sized row around a full-sized
    /// image. See [`Self::ready_screen`].
    image_blocks_budget: u16,
    /// How many times `ImagePolicy::Auto` has asked for each `src` on the page on screen.
    ///
    /// A count and not a set, because the LRU can drop a picture that is still on the page:
    /// one refill is right and an unbounded number is a request loop. `autoload::MAX_ATTEMPTS`
    /// holds the reasoning, and `autoload::plan` the other half of the pair.
    auto_attempts: HashMap<String, u32>,
    /// The site marks already asked for on the page in the column, so an eviction from the
    /// texture LRU cannot become a second request. Cleared with `auto_attempts`, which the same
    /// sentence describes: an attempt spent on the page going away must not count against the
    /// page arriving.
    icons_asked: HashSet<String>,
    /// The site marks kept on this machine, and where each one's bytes are.
    ///
    /// Consulted before anything is recorded, which is the disclosure and not an optimisation:
    /// `ImageStore::record_request` moves the host tally and the Referer count ahead of the
    /// request leaving, so a mark that is already here must take a path that never reaches it.
    /// See `reader::iconcache` for what may go in and why it is allowed at all.
    icons: iconcache::Cache,
    /// How tall each block was the last time it was laid out, so the blocks off the window can
    /// be skipped. See `reader::measure` for why, and `ready_screen` for when it is ignored.
    heights: Heights,
    /// What `theme::apply` last installed, so it installs again only when it would differ.
    style_installed: Option<theme::StyleKey>,
    /// Toggles against the auto-collapse default (see `thread_ui::is_collapsed`).
    collapsed: HashSet<CommentKey>,
    /// Bumped on every change to `collapsed`, for `measure::Layout`: a thread is one block and
    /// one remembered height, and `Shift+Z` changes it without laying the thread out.
    collapsed_changes: u64,
    find_current: usize,
    find_total: usize,
    /// One-shot: bring the current match into view on the next frame.
    ///
    /// Set by the steps that *move* the match (`n`, `N`, Enter, a changed query) and by
    /// nothing else, so scrolling away from a highlight leaves it where the reader put it.
    /// Re-centring every frame the match is on screen would make the page unscrollable while
    /// find is open.
    find_scroll: bool,
    /// The *default* options for this session. `--no-rewrite` and `--no-profile` clear a
    /// table each.
    opts: LoadOptions,
    /// The options of the navigation in flight, which a bare reload makes differ from the
    /// default.
    last_opts: LoadOptions,
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
    /// The words of the link [`Self::focused_href`] names, read back by `Shift+L`.
    ///
    /// Its own field rather than looked up from the document when the key arrives: `focused_href`
    /// is an href, and `html::link_block` copies one block-level anchor's href onto every inline
    /// in the block, so a search for "the link with this href" finds several and cannot say which
    /// carried the words Tab was actually on.
    focused_text: Option<String>,
    focused_image: Option<String>,
    focused_comment: Option<CommentKey>,
    /// Focus on a page widget none of the three above describes; see `RenderCtx::focus_other`.
    focused_other: bool,
    /// Whether the *previous* frame's render found focus on the page, taken before the four
    /// above are cleared for the frame in flight.
    ///
    /// `lays_out_whole_page` runs inside that render, downstream of the clear and upstream of
    /// the write, so reading the fields themselves gets `None` on every frame and the guard
    /// they were meant to be never fires. The snapshot is the value the guard wanted: focus was
    /// on a link when this frame began, so the widget carrying it has to be drawn again.
    focus_was_on_page: bool,
    /// The href focus was on when this frame began, taken beside [`Self::focus_was_on_page`]
    /// and for the same reason.
    ///
    /// The chrome is drawn *before* the page, so anything upstream of the render that asks
    /// "is a link focused?" reads `focused_href` after the clear and gets `None` every time.
    /// That is not a slow path the way the layout guard was; it is a menu item greyed for
    /// good. `Shift+Y` worked the whole time only because `handle_keys` runs ahead of the
    /// clear, which is what made the gap invisible.
    focus_href_was: Option<String>,
    /// The words that link was wearing, snapshotted beside [`Self::focus_href_was`] and for its
    /// reason: the chrome is drawn before the page, so the menu row and the key both read the
    /// snapshot rather than a field the render has already cleared.
    focus_text_was: Option<String>,
    /// Which block the focused widget was found in this frame, and the frame before: the one
    /// block that must be laid out for egui to keep the focus, which is what used to cost the
    /// whole page for as long as a link had it.
    focus_block: Option<usize>,
    focus_block_was: Option<usize>,
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
    /// The same for a navigation that began from the keyboard or the shot harness, where there
    /// is no widget to remember: the href that was followed. See `RenderCtx::pending_href`.
    pending_href: Option<String>,
    /// One-shot: give the URL bar keyboard focus on the next frame, after it exists.
    focus_url_bar: bool,
    /// One-shot: the same for the find field, which does not exist until `find_bar` runs, so
    /// the keymap cannot focus it directly. It also carries the selection: `find_bar` reads this
    /// to offer the last query back highlighted, the way the URL bar offers the address.
    focus_find: bool,
    /// The first block touching the top of the window last frame, which is what the outline
    /// marks as the current section. Found in `ready_screen`, a frame late like everything the
    /// render discovers.
    top_block: Option<usize>,
    /// Which history entry the page in the reading column belongs to, so `central` can keep
    /// writing where it is being read.
    ///
    /// Not the cursor. `go_back` moves the cursor at dispatch and the outgoing page stays on
    /// screen for the whole fetch, so through that window the visible page belongs to the entry
    /// *ahead* of the cursor. By id rather than index, because a link followed in that window
    /// truncates that entry away and puts the arriving page's entry at the same index.
    shown_entry: Option<EntryId>,
    /// What the window is currently called, so the title is sent when it changes and not on
    /// every frame. See [`ReaderApp::sync_window_title`].
    window_title: String,
    /// The desktop's light/dark preference, followed while the window is open, for
    /// `Theme::System`. See `reader::desktop`.
    ///
    /// Started by [`run`] and by nothing else, so it is `None` under `hww-shot`: a scene shot
    /// with `--theme system` photographs the same palette on every machine rather than the one
    /// the photographer's desktop happens to be in.
    desktop_theme: Option<desktop::Watch>,
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
/// A floating card centred in the window: the help card's frame, now that two things want it.
///
/// An `Area` and not a `Window`. A `Window` brings a title bar egui paints from its own
/// visuals, which is why this was the one piece of chrome that did not take the reader's
/// palette. Two things `Window` was giving away for free have to be asked for: the order, or
/// this draws under `hover_strip`, which is `Foreground`; and the constraint, or a card taller
/// than a short window runs off the bottom edge with no way back.
///
/// `chrome_bg`, a `guide` stroke and `RADIUS`, which is what the invariant asks of every
/// floating surface, and `chrome_font` overridden once so nothing inside has to remember that
/// the reader speaks in the monospace.
fn centred_card(
    ctx: &egui::Context,
    pal: &theme::Palette,
    opts: &crate::reader::opts::ReadOpts,
    id: &str,
    add_contents: impl FnOnce(&mut Ui),
) {
    egui::Area::new(egui::Id::new(id))
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .constrain(true)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(pal.chrome_bg)
                .stroke(egui::Stroke::new(1.0, pal.guide))
                .corner_radius(egui::CornerRadius::same(theme::RADIUS))
                .inner_margin(egui::Margin::symmetric(16, 14))
                .show(ui, |ui| {
                    ui.style_mut().override_font_id = Some(theme::chrome_font(opts));
                    add_contents(ui);
                });
        });
}

/// One of hww's own views: a page hww assembles rather than fetches.
///
/// One variant per tenant of `reader::archive`, and every one of them is a view of the same
/// file. It is `Copy` and carries nothing: what to draw is decided in
/// [`ReaderApp::show_builtin`], from the reader's own state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    Bookmarks,
    ReadingList,
    History,
}

/// Which of hww's own views an address names, or `None` for a page that has to be fetched.
///
/// Exact string equality against the one address, so no `http` or `https` URL can be diverted
/// from a real fetch — which is the failure worth testing for, and
/// `builtin_view_claims_one_address_and_diverts_no_fetch` is what pins it. Page content can
/// never reach here at all: `session::classify_link` refuses every scheme it does not handle,
/// so only a command the reader gave can name one of these.
fn builtin_view(url: &Url) -> Option<View> {
    match url.as_str() {
        archive::BOOKMARKS => Some(View::Bookmarks),
        archive::READING_LIST => Some(View::ReadingList),
        archive::HISTORY => Some(View::History),
        _ => None,
    }
}

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
        // Through the accessor, not the field: this is the one place a hand-edited `0`, a
        // negative, or a `NaN` reaches egui, which clamps none of them. `persist` writes the
        // applied value back on the first frame, so the file heals itself.
        cc.egui_ctx.set_zoom_factor(launch.settings.zoom_factor());
        apply_scroll_speed(&cc.egui_ctx, &launch.settings);
        // Read here rather than passed in through `Launch`, unlike the settings. Both binaries
        // point `HWW_CONFIG_DIR` where they mean to before this runs — `bin/shot.rs` at a
        // scratch or fixture directory — so the archive follows the settings to whichever
        // directory that is without either binary having to carry it. Its complaint joins the
        // settings' in the status strip rather than replacing it: two files failed to parse is
        // two things the reader is owed.
        let (archive, archive_note) = archive::load();
        let archive_icons: Vec<String> = archive.marked_icons().collect();
        let opening_note = {
            let both: Vec<String> = [launch.settings_note, archive_note]
                .into_iter()
                .flatten()
                .collect();
            (!both.is_empty()).then(|| both.join(" · "))
        };
        let mut app = Self {
            net,
            settings: launch.settings,
            settings_dirty: false,
            last_saved: Instant::now(),
            history: History::new(),
            archive,
            archive_write_failed: false,
            visits_dirty: None,
            shown_bookmarked: false,
            pages: PageCache::new(),
            page: Page::Idle,
            current: None,
            rewrite_notice: None,
            status: opening_note.map(|n| (n, Instant::now())),
            hover_href: None,
            strip_height: 0.0,
            hover_height: 0.0,
            chrome: Chrome::default(),
            images: ImageStore::default(),
            image_blocks: HashMap::new(),
            image_blocks_budget: 0,
            auto_attempts: HashMap::new(),
            icons_asked: HashSet::new(),
            // Handed the addresses the two marked lists name, so the sweep it does on the way
            // in deletes anything else it finds. That is what keeps an entry for a host nobody
            // marked from surviving a launch, whatever wrote it.
            icons: iconcache::Cache::open(
                archive::icons_path(),
                &archive_icons.iter().cloned().collect(),
            ),
            heights: Heights::default(),
            style_installed: None,
            collapsed: HashSet::new(),
            collapsed_changes: 0,
            find_current: 0,
            find_total: 0,
            find_scroll: false,
            opts: launch.opts,
            last_opts: launch.opts,
            scroll_to_block: None,
            scroll_delta: 0.0,
            scroll_offset: None,
            viewport_height: 600.0,
            max_scroll: 0.0,
            pending_copy: None,
            focused_href: None,
            focused_text: None,
            focused_image: None,
            focused_comment: None,
            focused_other: false,
            focus_was_on_page: false,
            focus_href_was: None,
            focus_text_was: None,
            focus_block: None,
            focus_block_was: None,
            tab_frames: 0,
            pending_link: None,
            pending_href: None,
            focus_url_bar: false,
            focus_find: false,
            top_block: None,
            shown_entry: None,
            // What `ui::run` opened the window under. Equal by construction so the first frame
            // sends nothing.
            window_title: notice::window_title(None),
            desktop_theme: None,
        };
        match launch.start {
            Some(url) => app.navigate(url, app.opts, true),
            None => {
                // The home screen, and it is no new surface: opening on the bookmarks is literally
                // "hww opened with the bookmarks", one extra branch in the match that was already
                // here. A real `Page::Ready` inherits find, the outline, scrolling, Tab focus,
                // and correct menu gating; a fourth empty state drawn into `empty_states` would
                // have inherited none of them and had to fake each in turn.
                //
                // An empty bookmarks list still opens on the splash whatever the setting says. Nothing
                // has happened yet, which is `notice::idle`'s whole argument for not being a
                // notice, and it means first run is unchanged.
                if archive::opens_on_launch(&app.settings, &app.archive) {
                    app.open_bookmarks(true);
                } else {
                    // Only on the splash. With nothing to read, the URL bar is the only sensible
                    // thing on screen and so the only sensible thing to be typing into — but
                    // `handle_keys` returns early while the bar has focus, so doing this over the
                    // bookmarks would hand the reader a page they cannot scroll, Tab into, find
                    // in, or reopen the bookmarks on until they think to press Esc. A page in
                    // the column is a
                    // page to read; the bar is one keystroke away when it is not.
                    app.chrome.url_bar = Some(String::new());
                    app.focus_url_bar = true;
                }
            }
        }
        Ok(app)
    }

    // ---------------------------------------------------------------- navigation

    /// Dispatch a navigation. **Nothing on screen changes here except the chrome.**
    ///
    /// Back and Forward reach this only when they have to: [`ReaderApp::arrive`] tries the page
    /// cache first and comes here on a miss. Every other navigation — a link, a typed URL,
    /// `reload`, a bare reload, Retry — arrives here directly and always asks the site.
    ///
    /// The resets that belong to the page being *fetched* are not done here; they are
    /// [`ReaderApp::commit`]'s, and they run when a document actually takes the column. Doing
    /// them at dispatch is what used to clear the textures, collapse state, and scroll offset of
    /// the page the reader was still looking at.
    fn navigate(&mut self, url: Url, opts: LoadOptions, push: bool) {
        self.close_url_bar();
        // **The chokepoint**, and it has to be here rather than at any caller. `navigate` is
        // reached by `reload`, by `follow`, and — the case that actually bites — by `arrive`,
        // which falls through to it whenever a history entry is not in the page cache. A built
        // view is deliberately never cached, so without this check, leaving the bookmarks and
        // pressing Back would hand `hww:bookmarks` to reqwest and land the reader on an error
        // page; `r` on the bookmarks is the same path, one caller over.
        if let Some(view) = builtin_view(&url) {
            self.show_builtin(view, url, opts, push);
            return;
        }
        // `mint_page`, not `mint`: it also tells the pool which page is current, so the images
        // queued on the one being left are dropped instead of run ahead of this fetch.
        let req = self.begin(opts);
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
        self.net.submit(Job::Load { req, url, opts });
    }

    /// Put one of hww's own views in the column: the synchronous half of [`ReaderApp::navigate`].
    ///
    /// It goes through `install`/`commit` like every other page rather than becoming a fourth
    /// `Page` variant, which would have left find, Tab, and the outline to special-case it.
    ///
    /// The view is **rebuilt on every arrival**, which is why it is not cached, and that is the
    /// behaviour rather than a workaround for a missing cache entry: the bookmarks has to reflect
    /// what has been kept or forgotten since it was last drawn, so Back onto it shows a current
    /// list instead of a stale one.
    fn show_builtin(&mut self, view: View, url: Url, opts: LoadOptions, push: bool) {
        // `begin`, and so `Net::mint_page`, for the reason its own doc gives: a second place
        // that starts a navigation cannot forget the second half. A built view is the third
        // place. The bookmarks has no article images of its own, so the symptom is not the one
        // that doc describes — it is that the pool's current-page id would still name the page
        // the reader just left, and images in flight for that page would go on being accepted
        // onto this one.
        let req = self.begin(opts);
        // Read before the history is touched, exactly as `navigate` does: a navigation still in
        // flight that put the entry at the cursor there itself is the one that may be
        // overwritten rather than appended after.
        let superseding = self.page.supersedes_its_history_entry();
        if push {
            if superseding {
                self.history.replace(url.clone());
            } else {
                // `History::push` treats a re-push of the current URL as a reload that keeps the
                // entry's id and offset, so the bookmarks key pressed on the bookmarks rebuilds
                // it in place with
                // the reading position intact rather than stacking a second entry.
                self.history.push(url.clone());
            }
        }
        let doc = match view {
            View::Bookmarks => archive::bookmarks_document(&self.archive),
            View::ReadingList => archive::reading_list_document(&self.archive, &self.settings),
            View::History => archive::history_document(&self.archive, &self.settings),
        };
        let outline = outline::build(&doc.blocks);
        let masthead = title::masthead(&doc);
        let (text_len, rtl, threads) = derived(&doc);
        self.install(Box::new(Ready {
            loaded: Loaded {
                // An address and zeros. `Arrival::Built` is what guarantees the zeros are never
                // drawn; see `session::Provenance::built`.
                prov: session::Provenance::built(url),
                doc,
            },
            outline,
            masthead,
            text_len,
            rtl,
            threads,
            req,
            opts,
            fetched: Instant::now(),
            restored: false,
            built: true,
        }));
    }

    /// Open the bookmarks, as the bookmarks key and the menu item do.
    fn open_bookmarks(&mut self, push: bool) {
        let url = Url::parse(archive::BOOKMARKS).expect("hww:bookmarks is a valid address");
        self.navigate(url, self.opts, push);
    }

    /// Open the reading list, as `l` and the menu item do.
    fn open_reading_list(&mut self, push: bool) {
        let url = Url::parse(archive::READING_LIST).expect("hww:reading-list is a valid address");
        self.navigate(url, self.opts, push);
    }

    /// Open the history, as the history key and the menu item do.
    fn open_history(&mut self, push: bool) {
        let url = Url::parse(archive::HISTORY).expect("hww:history is a valid address");
        self.navigate(url, self.opts, push);
    }

    /// Redraw a built view after the store behind it changed under it.
    ///
    /// A no-op on a fetched page, which is what makes it safe to call from anywhere the bookmarks
    /// can change. Through `navigate` with `push: false`, so the entry keeps its id and the
    /// reading position survives the rebuild.
    fn rebuild_builtin_view(&mut self) {
        let Some(url) = self
            .shown()
            .filter(|r| r.built)
            .map(|r| r.loaded.prov.final_url.clone())
        else {
            return;
        };
        self.navigate(url, self.last_opts, false);
    }

    /// `Ctrl+D`: keep the page on screen, or drop it if it is already kept.
    ///
    /// A toggle, because the menu row is an `Item::Check` and a check mark that could only ever
    /// be turned on would be a strange checkbox. The forget half runs whatever the keep switch
    /// says: turning keeping off stops hww writing anything new, and must never be a way to trap
    /// what is already in the bookmarks.
    fn bookmark_page(&mut self) {
        let Some(page) = self.shown().filter(|r| !r.built) else {
            // Reachable by key on the bookmarks, the splash, and an error screen, where the menu
            // row is greyed instead. A key that does nothing and says nothing reads as a broken
            // binding.
            self.flash(notice::NOTHING_TO_BOOKMARK.to_owned());
            return;
        };
        let url = page.loaded.prov.final_url.clone();
        let title = page.loaded.doc.title.clone();
        // The mark the page named for itself, stored so the bookmarks can draw its column
        // without asking the page again. `Archive::bookmark` refuses one that is not an absolute
        // http(s) address; see `archive::icon_to_store`.
        let icon = page.loaded.doc.favicon.clone();
        if self.archive.forget(&url) {
            // Before the write and whatever it answers: the row is gone from the store either
            // way, and a mark kept for a page hww has stopped remembering is a file with no
            // reason to exist. Another row may still name the same address, which is why this
            // asks the archive rather than deleting one file.
            self.prune_icons();
            if self.save_archive() {
                self.flash(notice::FORGOTTEN.to_owned());
            }
            return;
        }
        match self
            .archive
            .bookmark(&self.settings, &url, title.as_deref(), icon.as_deref())
        {
            Stored::Added => {
                self.keep_page_mark();
                if self.save_archive() {
                    // Named from what was *stored* rather than from the page: that string has
                    // been flattened to one line and capped, so the toast cannot be a paragraph
                    // and an untitled page is still named by its address.
                    let label = self
                        .archive
                        .bookmarks()
                        .next()
                        .map_or_else(|| url.to_string(), |i| i.label());
                    self.flash(notice::bookmarked(&label));
                }
            }
            // `forget` above answered `false` over the same predicate `holds` uses, so this door
            // pre-empts this arm. It is the store's own answer and the match is exhaustive, so
            // it gets the reader's words rather than a sentence invented here.
            Stored::Already => self.flash(notice::ALREADY_BOOKMARKED.to_owned()),
            Stored::Full => self.flash(notice::bookmarks_are_full(archive::MAX_ITEMS)),
            Stored::TooLong => self.flash(notice::ADDRESS_TOO_LONG.to_owned()),
            Stored::Off => self.flash(notice::BOOKMARKING_IS_OFF.to_owned()),
            // Unreachable here: only `Archive::add_next` inspects the scheme, because only it is
            // handed an address a page chose. `keep` is given `prov.final_url`, which a fetch
            // resolved and followed. The arm exists because the two doors share one answer type,
            // and saying so is better than an `unreachable!` that a later caller could reach.
            Stored::Refused => self.flash(notice::CANNOT_READ_LATER.to_owned()),
        }
    }

    /// Put the mark of the page just kept on the disk, if it is not there already.
    ///
    /// The other half of [`ReaderApp::favicon_mark`], for the order the reader actually does it
    /// in: read a page, then keep it. By then its favicon has already been fetched and drawn as
    /// the page's own — the bytes are a texture and nothing kept the file they came from — and
    /// every surface that would ask again skips a `src` the store already holds. So the strip's
    /// new row draws from this session's memory and has nothing to draw from in the next one.
    ///
    /// One small request to the host that just served the page, for the icon it just served,
    /// made when the reader asked hww to remember the page. It is counted and disclosed like
    /// every other, it is the same request opening the bookmarks would make, and it is skipped
    /// entirely when the file is already there or when the policy allows no request at all.
    /// Nothing here is reachable for a page that was not kept: the archive is asked again inside
    /// `favicon_mark`, so an address it does not name yields `Mark::Read` and no `cache_as`.
    fn keep_page_mark(&mut self) {
        let Some(src) = self.shown().and_then(|r| r.loaded.doc.favicon.as_deref()) else {
            return;
        };
        if !self.settings.read.images.allows_any_request() {
            return;
        }
        let src = src.to_owned();
        // Already answered, either way: the file is here, or this address was asked and served
        // nothing. `kept_mark` is the door for the first and does not need this one.
        if archive::icon_address(&src)
            .is_none_or(|key| !matches!(self.icons.known(&key), iconcache::Known::Unknown))
        {
            return;
        }
        let mark = self.favicon_mark(&src);
        if mark != Mark::Kept {
            return;
        }
        // `automatic`, because nobody asked for this picture — the reader asked for the page to
        // be kept. A worker that reaches it after the reader has navigated away answers
        // `Msg::ImageDropped` rather than fetching, which is the right answer for a mark whose
        // row can be drawn again whenever the strip or the list next wants it.
        self.load_favicon_job(&src, true, mark);
    }

    /// `Shift+L`, the context menu's "Read later", and the menu row: put one link on the reading
    /// list, or take it off if it is already there.
    ///
    /// **Public, and a fifth hook.** `follow_link` is public because Enter-on-a-focused-link is
    /// not something the screenshot harness can drive — `AGENTS.md` says a synthetic Tab does not
    /// reliably land focus, so anything reachable only by tabbing needs a door rather than key
    /// steps. This is the second such thing in the program and it needs the door for the same
    /// reason: a scene that pressed `Shift+L` would photograph an empty list and pass.
    ///
    /// `title` is the link's own words, which is all the name this item will ever have: nothing
    /// here has been fetched, so there is no `<title>` to fall back to. Empty is fine —
    /// `Item::label` falls back to the address, which is drawn under every row of this view
    /// anyway.
    ///
    /// The href goes through `session::classify_link` first, so a `mailto:` or `javascript:` link
    /// is answered in the words the rest of the reader already uses for it rather than refused in
    /// silence. That is a courtesy and not the guard: `Archive::add_next` refuses the same schemes
    /// inside the door, because a door guarded at one caller is a door guarded nowhere, and this
    /// one has two callers on the day it lands.
    pub fn read_later(&mut self, href: &str, title: &str) {
        let url = match session::classify_link(href) {
            Target::Navigate(url) => url,
            // The same two sentences a click on one of these gets. Copying is not offered here:
            // the reader asked to read something later, and an address that cannot be read later
            // is the answer, not a clipboard.
            Target::OfferCopy { url, note } => {
                self.flash(format!(
                    "{url} is {note}, so there is nothing to read later"
                ));
                return;
            }
            Target::Refuse { url, reason } => {
                self.flash(format!("{reason}: {url}"));
                return;
            }
        };
        // Off before on, as `bookmark_page` does: the removal half runs whatever the switch says, so
        // turning the reading list off can never be a way to trap what is already on it.
        if self.archive.forget_next(&url) {
            self.prune_icons();
            if self.save_archive() {
                self.flash(notice::READ_LATER_REMOVED.to_owned());
            }
            self.rebuild_builtin_view();
            return;
        }
        match self.archive.add_next(&self.settings, &url, Some(title)) {
            Stored::Added => {
                if self.save_archive() {
                    // Named from what was *stored*, for the reason `bookmark_page` gives: that string
                    // has been flattened and capped, so a link wearing a paragraph of text cannot
                    // become a paragraph of toast.
                    let label = self
                        .archive
                        .read_next()
                        .next()
                        .map_or_else(|| url.to_string(), |i| i.label());
                    self.flash(notice::read_later(&label));
                }
            }
            // `forget_next` above answered `false` over the same predicate `holds_next` uses, so
            // this door pre-empts this arm, exactly as `bookmark_page`'s does.
            Stored::Already => self.flash(notice::ALREADY_READ_LATER.to_owned()),
            Stored::Full => self.flash(notice::reading_list_is_full(archive::MAX_NEXT)),
            Stored::TooLong => self.flash(notice::ADDRESS_TOO_LONG_TO_READ_LATER.to_owned()),
            Stored::Off => self.flash(notice::READING_LIST_IS_OFF.to_owned()),
            // Unreachable through `classify_link`, which answers `Navigate` only for `http` and
            // `https`. The arm is the store's own answer and the match is exhaustive: the guard
            // inside the door is the one that has to hold, and this is what it says when it does.
            Stored::Refused => self.flash(notice::CANNOT_READ_LATER.to_owned()),
        }
        self.rebuild_builtin_view();
    }

    /// `Shift+L` with nothing focused, and the one path that is not about a link at all.
    ///
    /// Split out so the key and the two link-carrying callers do not share a `None` case that
    /// only one of them can reach.
    fn read_focused_link_later(&mut self) {
        // Both fields, because the two callers read at different points in the frame. `Shift+L`
        // arrives in `handle_keys`, which runs *ahead* of the clear, so the live field is the one
        // with the answer; the menu row is dispatched from the chrome, which is drawn before the
        // page, so by then only the snapshot has it. `Shift+Y` and `Command::CopyFocusedLink`
        // are the same pair one action over, and they read one field each — which is why the
        // menu row for copying a link was greyed for good until `focus_href_was` was added.
        let Some(href) = self
            .focused_href
            .clone()
            .or_else(|| self.focus_href_was.clone())
        else {
            self.flash(notice::no_link_to_read_later());
            return;
        };
        let title = self
            .focused_text
            .clone()
            .or_else(|| self.focus_text_was.clone())
            .unwrap_or_default();
        self.read_later(&href, &title);
    }

    /// The `remove` button on a row of `hww:reading-list`: take that one link off.
    ///
    /// It removes and never adds, which is what makes it a door of its own rather than
    /// [`ReaderApp::read_later`] with another label on it. The row was drawn from an item the
    /// archive holds, so the only thing a press can mean is "take it off" — a toggle reached from
    /// here would put back a row that a click landing a frame late had already removed.
    ///
    /// The address is parsed rather than trusted, like every other href the reader can press: it
    /// came out of the file, and a file this build did not write is a file that can hold anything.
    /// A row whose address no longer parses is one this control cannot act on, and doing nothing
    /// is the answer — `Archive::forget_next` matches by URL, so there is nothing to aim at.
    fn forget_next_link(&mut self, href: &str) {
        let Ok(url) = Url::parse(href) else {
            return;
        };
        if self.archive.forget_next(&url) {
            // Before the write and whatever it answers, exactly as the two toggles do it: this
            // is the fourth way a marked address leaves the file, and a mark kept for a row hww
            // has stopped remembering is a file with no reason to exist. Another row may still
            // name the same address, which is why this asks the archive rather than deleting one
            // file.
            self.prune_icons();
            if self.save_archive() {
                self.flash(notice::READ_LATER_REMOVED.to_owned());
            }
            // Whatever the write answered. The store no longer holds that link, and a list still
            // showing the row would be the page claiming something hww has stopped remembering;
            // if the file could not be written, `save_archive` has already said so.
            self.rebuild_builtin_view();
        }
    }

    /// Empty the reading list: the settings panel's "forget the reading list" and the view's own
    /// "forget all" both press this.
    ///
    /// One function and not two copies, unlike the other two tenants' arms below, and for the
    /// reason those two are written out separately: what may not be shared is a sentence about
    /// *which* list was emptied. These two buttons empty the same one, so a second copy could only
    /// ever drift from this one and start saying something else about it.
    fn forget_reading_list(&mut self) {
        // Asked before the forget, which is what lifts the seal; see the bookmarks' arm.
        let replaced = self.archive.refuses_to_be_written();
        let n = self.archive.forget_all_next();
        self.prune_icons();
        if self.save_archive() {
            self.flash(if replaced {
                notice::UNREADABLE_FILE_REPLACED.to_owned()
            } else {
                notice::reading_list_forgotten(n)
            });
        }
        self.rebuild_builtin_view();
    }

    /// Drop every kept site mark no marked row still names.
    ///
    /// Called wherever a marked address goes: un-bookmarking with the bookmark key, taking a
    /// link back off the reading list with `Shift+L`, the reading list's own per-row `remove`,
    /// and either forget button for the two chosen lists.
    /// Those five are not the only ways an entry becomes an orphan — re-bookmarking a page whose
    /// `<link rel=icon>` changed leaves the old one, and a hand-edited `archive.json` orphans
    /// whatever it likes — which is why `iconcache::Cache::open` sweeps the directory at every
    /// launch as well. This is the immediate half, so a reader who forgets a page does not have
    /// to relaunch before its mark is gone from the disk.
    ///
    /// By "no marked row still names this address" and never per tenant, because one icon serves
    /// rows on both lists: the same site can be bookmarked and queued to read again, and asking
    /// which of the two a file belongs to is a question with no answer.
    ///
    /// Emptied entirely rather than pruned when the two lists between them name nothing, which
    /// is both forget buttons pressed and the last row of the last list taken off. `prune` would
    /// leave the directory standing with nothing in it; `forget_all` takes that too, so a reader
    /// who has asked for all of it to go finds nothing left to wonder about.
    fn prune_icons(&mut self) {
        let keep: HashSet<String> = self.archive.marked_icons().collect();
        if keep.is_empty() {
            self.icons.forget_all();
        } else {
            self.icons.prune(&keep);
        }
    }

    /// Write the bookmarks now, and say so if it could not be written.
    ///
    /// No debounce, unlike the settings: a keep is a single deliberate act, and the reader who
    /// presses `Ctrl+D` and shuts the laptop should find it there. Returns whether the caller
    /// may report success, so a failure is never followed by a toast claiming the opposite.
    fn save_archive(&mut self) -> bool {
        // Before the write and whatever it answers: the store changed, and the cached answer is
        // about the store rather than about the file.
        self.refresh_shown_kept();
        // One file, so this write carries the pending visits too, whether or not it lands: a
        // failure here is the same failure they would have met, and leaving the mark set would
        // schedule a second attempt one debounce later for no new reason.
        self.visits_dirty = None;
        match archive::save(&self.archive) {
            Ok(()) => true,
            Err(e) => {
                self.flash(notice::archive_not_saved(&e.to_string()));
                false
            }
        }
    }

    /// Write down a page hww has just drawn — in the store now, in the file shortly.
    ///
    /// The switch is answered by `Archive::visit` rather than here, in the door's own name; what
    /// is decided here is *when* the file is written, and the answer is `settings`' answer.
    ///
    /// This used to write through on the spot, on the argument that a navigation is a fetch
    /// apart from the next one so there was no burst to absorb. That is true of how often the
    /// write happens and says nothing about what it costs: `settings::write_atomically` fsyncs
    /// the file and then the directory, the whole archive is re-serialized every time, and at
    /// `archive::MAX_VISITS` that is around a megabyte. Called from [`ReaderApp::install`], it
    /// put those two fsyncs on the UI thread on the exact frame the new page appeared — and
    /// again on every Back and Forward, which restore from the page cache in no time at all and
    /// then waited on a disk. Deferring costs at most one page of history to a kill that gives
    /// no chance to run [`ReaderApp::on_exit`], which is the trade `settings` already makes for
    /// a file the reader could retype and this one makes for the tenant nobody asked for. A keep
    /// is still written the instant it happens: that one is a keypress.
    ///
    /// Silent on success, unlike a keep. Every page produces one of these, and a status line
    /// about each would be the application narrating its own bookkeeping over the page the reader
    /// came to read.
    fn record_visit(&mut self, page: &Ready) {
        let url = page.loaded.prov.final_url.clone();
        let title = page.loaded.doc.title.clone();
        if self.archive.visit(&self.settings, &url, title.as_deref()) != Visited::Recorded {
            return;
        }
        // Only when nothing is pending, so the mark keeps the age of the oldest unwritten visit
        // and a steady reader cannot push the write out for ever.
        self.visits_dirty.get_or_insert_with(Instant::now);
    }

    /// Write the archive on the history's behalf, and complain at most once about a run of
    /// failures.
    ///
    /// This is why a visit does not go through [`ReaderApp::save_archive`]: that one is for a
    /// keypress and complains every time, correctly, because the reader just asked for something
    /// and did not get it. Nothing here asked for anything, so an unwritable config directory is
    /// one sentence rather than one over every page.
    fn save_visits(&mut self) {
        self.visits_dirty = None;
        match archive::save(&self.archive) {
            Ok(()) => self.archive_write_failed = false,
            Err(e) => {
                if !std::mem::replace(&mut self.archive_write_failed, true) {
                    self.flash(notice::archive_not_saved(&e.to_string()));
                }
            }
        }
    }

    /// Whether the page on screen is kept, for the page-info panel's disclosure row.
    ///
    /// **Membership is answered before the switch.** A page kept before the switch went off is
    /// still in the file and `Ctrl+D` still removes it, and the menu draws its tick from the same
    /// fact; a panel that answered "no" there would be the one surface in the program
    /// misstating what is stored, which is the disclosure this row exists to make.
    /// [`pageinfo::Bookmarked::Off`] is for a page that is genuinely not kept and cannot be, so that
    /// "no" does not read as something a keypress would change.
    fn kept_state(&self) -> pageinfo::Bookmarked {
        if self.shown_bookmarked {
            pageinfo::Bookmarked::Yes
        } else if !self.settings.keep_bookmarks {
            pageinfo::Bookmarked::Off
        } else {
            pageinfo::Bookmarked::No
        }
    }

    /// Recompute [`ReaderApp::shown_bookmarked`].
    ///
    /// Called from the three places that can change the answer and no others: the two that
    /// change the page, [`ReaderApp::install`] and [`ReaderApp::fail`], and
    /// [`ReaderApp::save_archive`], which every mutation of the store passes through on its way
    /// to the file. Each of the three is a door with one caller-facing name, so a fourth
    /// transition cannot be added without passing through one of them.
    fn refresh_shown_kept(&mut self) {
        self.shown_bookmarked = self
            .shown()
            .is_some_and(|r| !r.built && self.archive.is_bookmarked(&r.loaded.prov.final_url));
    }

    /// Follow a link. The scheme gate runs here, before anything is dispatched: one place, so
    /// no widget can route around it.
    /// Follow a link as Enter on it does: the navigation, and the pending mark on every inline
    /// carrying the href, because there is no widget to pin it to. Public for `hww-shot`, which
    /// is how the mark gets photographed; a synthetic Tab does not reliably land focus.
    pub fn follow_link(&mut self, href: &str) {
        let before = self.current;
        self.follow(href, self.opts);
        if self.current != before {
            self.pending_href = Some(href.to_owned());
        }
    }

    fn follow(&mut self, href: &str, opts: LoadOptions) {
        match session::classify_link(href) {
            Target::Navigate(url) => {
                // A `#fragment` on the page already open is a scroll, not a fetch.
                if let Some(fragment) = url.fragment()
                    && self.same_page(&url)
                {
                    self.jump_to_fragment(fragment);
                    return;
                }
                self.navigate(url, opts, true);
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
        archive::same_page_urls(url, &ready.loaded.prov.final_url)
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

    fn reload(&mut self, opts: LoadOptions) {
        if let Some(url) = self.history.current().cloned() {
            self.navigate(url, opts, false);
        }
    }

    fn go_back(&mut self) {
        if self.history.back().is_some() {
            self.arrive();
        }
    }

    fn go_forward(&mut self) {
        if self.history.forward().is_some() {
            self.arrive();
        }
    }

    /// The cursor has moved. Put the page it names into the column, from memory if the reader
    /// still has it and from the network if not.
    ///
    /// The one place the page cache is consulted, which is why no other navigation needs to opt
    /// out of it: `reload`, a bare reload, `Retry`, a link, and a typed URL all reach `navigate`
    /// directly. A check inside `navigate` would have needed a flag argument at every call site
    /// and would have been the wrong shape.
    fn arrive(&mut self) {
        // Here as well as in `navigate`, and not instead of it: this is the one navigation that
        // can finish without dispatching anything, so a Back onto a page still in
        // `reader::pagecache` would keep the bar that a Back onto a page that has to be fetched
        // closes.
        self.close_url_bar();
        let Some(id) = self.history.current_id() else {
            return;
        };
        if self.keep_shown(id) || self.restore(id) {
            return;
        }
        let Some(url) = self.history.current().cloned() else {
            return;
        };
        self.navigate(url, self.opts, false);
    }

    /// The cursor has landed on the entry whose page is *already in the column*. Cancel the
    /// navigation in flight over it and keep it.
    ///
    /// Following a link and pressing Back before the reply arrives is this case, and without it
    /// the reader re-downloads the page they are looking at. It fires only during a navigation:
    /// with nothing in flight the cursor has just moved off `shown_entry` by definition.
    ///
    /// Not [`Page::restore_shown`], which puts the page back into the `Loading` it is holding —
    /// that is the state being cancelled.
    fn keep_shown(&mut self, id: EntryId) -> bool {
        if self.shown_entry != Some(id) {
            return false;
        }
        let Some(page) = self.page.take_shown() else {
            return false;
        };
        self.resume(&page);
        // No `commit` here, deliberately: this page never left the column, so its textures, its
        // collapsed threads, its find position and its scroll offset are all still its own and
        // resetting them would be the jolt `commit` was moved out of `navigate` to avoid.
        //
        // The one half of `commit` that still has to run is the prune: this is a page
        // transition, `navigate` truncated the branch ahead when it dispatched the fetch being
        // cancelled, and the pages kept for the entries it dropped are charged against the
        // budget with nothing able to ask for them again.
        self.prune_pages();
        self.page = Page::Ready(page);
        true
    }

    /// Put a page back on screen without a request.
    ///
    /// Misses, and falls back to a fetch, when the entry's page was evicted, was never kept, or
    /// was read under other options; see `reader::pagecache` for why the last of those is a miss
    /// rather than a hit.
    fn restore(&mut self, id: EntryId) -> bool {
        let Some(mut page) = self.pages.take(id, self.opts) else {
            return false;
        };
        page.restored = true;
        self.adopt(&mut page);
        self.install(page);
        true
    }

    /// Re-stamp a page for a navigation of its own, and make whatever was in flight stale.
    ///
    /// A page that goes back on screen without a fetch still needs a live navigation id, for
    /// both of the jobs `navigate` mints one for: every reply still outstanding for the fetch
    /// this abandons fails its `self.current` guard and is dropped, and the pool is told which
    /// page is current so the images queued on the one being left are dropped too. The worker
    /// still runs its request to completion — correlation, not cancellation.
    ///
    /// [`Ready::req`] is re-stamped rather than carried, and that is the half worth stating.
    /// Left stale, `Msg::Image`'s match would drop every reply this page asked for, so
    /// `ImagePolicy::Auto` would go silently dead on restored pages and nowhere else; and a
    /// reply from the page's *earlier* visit could still be accepted, putting a picture on a
    /// page whose disclosure counters have just been cleared.
    fn adopt(&mut self, page: &mut Ready) {
        page.req = self.begin(self.opts);
    }

    /// Hand the column back to the page that never left it, and make the navigation over it
    /// stale.
    ///
    /// The mirror of [`ReaderApp::adopt`], and re-stamping here would be wrong for the reason
    /// it is right there. `commit` does not run on this path, so `ImageStore` still holds this
    /// page's entries, the disclosures recorded for its outstanding requests, and its counters.
    /// A fresh id orphans all three: every image reply in flight fails the `Ready::req` guard
    /// in `Msg::Image` and `Msg::ImageDropped`, which leaves a placeholder on `Loading` that
    /// `is_pending` refuses to re-request, `any_pending` pinning the repaint floor for the life
    /// of the page, a `Set-Cookie` uncounted, and a host standing in the panel for a request
    /// whose result was thrown away. `ImagePolicy::Auto` stops too, since `image_pass` asks
    /// whether the page on screen is the current navigation.
    ///
    /// The page's own id is still the live one. It is the fetch over it that has to go stale,
    /// which is what publishing an id other than its own achieves.
    fn resume(&mut self, page: &Ready) {
        self.net.resume_page(page.req);
        self.take_over(page.req, page.opts);
    }

    /// Mint the navigation id and make everything the one it replaces left behind stale.
    ///
    /// Shared by `navigate` and [`ReaderApp::adopt`] because the set is not obvious from any one
    /// field — it is "everything that makes an in-flight reply stale, plus the click marks on the
    /// link that started it". Written out at both, a sixth field added later lands in `navigate`
    /// alone and only pages put back from memory misbehave.
    fn begin(&mut self, opts: LoadOptions) -> ReqId {
        let req = self.net.mint_page();
        self.take_over(req, opts);
        req
    }

    /// Everything a navigation id takes over apart from minting it, shared with
    /// [`ReaderApp::resume`], which adopts the id of the page already in the column instead.
    fn take_over(&mut self, req: ReqId, opts: LoadOptions) {
        self.current = Some(req);
        self.last_opts = opts;
        self.rewrite_notice = None;
        self.pending_link = None;
        self.pending_href = None;
    }

    // ---------------------------------------------------------------- images

    /// One article image, by click, by `i`, or as one of `Shift+I`'s.
    ///
    /// The policy is enforced here rather than at each of those three, because here is where
    /// they meet: this is the only path an *article* image reaches the network by. The
    /// favicon takes `load_favicon` and answers to `allows_any_request` instead, which is the
    /// whole difference between `Never` and `NoRequests`.
    ///
    /// Drawing already respects the setting (`images::placeholder` returns a dim line under
    /// either policy), so before this guard the reader offered no control and still fetched
    /// the moment a key was pressed — `Shift+I` on a `NoRequests` page contacted every image
    /// host on it, which is the one thing that policy exists to promise it will not do.
    fn load_image(&mut self, ctx: &egui::Context, src: &str) {
        if !self.settings.read.images.offers_loading() {
            self.flash(IMAGES_ARE_OFF.to_owned());
            return;
        }
        self.request_image(src, self.column_texture_width(ctx), true, false, Mark::No);
    }

    /// The width every article image is decoded to: the reading column, in physical pixels.
    ///
    /// Shared by the click path and the automatic one rather than computed at each, so a
    /// picture cannot arrive at a different size depending on who asked for it.
    fn column_texture_width(&self, ctx: &egui::Context) -> u32 {
        (theme::measure_px(ctx, &self.settings.read) * ctx.pixels_per_point()).max(64.0) as u32
    }

    /// Request the pictures near the reading position, under `ImagePolicy::Auto`.
    ///
    /// The fourth door onto `request_image`, beside `load_image`, `load_all_images`, and
    /// `ensure_favicon`, and like them it answers the policy in its own name here rather than
    /// leaving it to the caller: a door that is guarded at some call sites and not others reads
    /// as guarded and is not. The caller's own test of the same question is an optimisation —
    /// it skips walking the band's blocks under the other three policies — and not the
    /// enforcement.
    ///
    /// `in_band` is what the column just drew inside `measure::Band`, so this is a screenful
    /// either side of the window and not the document. `autoload::plan` holds the rest of the
    /// reasoning, and is where the tests are.
    fn autoload(&mut self, ctx: &egui::Context, base: &Url, in_band: &[String]) {
        if !self.settings.read.images.loads_automatically() {
            return;
        }
        let srcs = autoload::plan(
            base,
            in_band,
            &self.auto_attempts,
            self.images.pending(),
            &|src| self.images.state(src).is_some(),
        );
        let max_width = self.column_texture_width(ctx);
        for src in srcs {
            // Counted whether or not the request survives: `request_image` declines a `src`
            // already in flight, and an entry that never resolves must not be planned again on
            // the next frame. `Msg::ImageDropped` is the one thing that takes a count back.
            *self.auto_attempts.entry(src.clone()).or_default() += 1;
            self.request_image(&src, max_width, false, true, Mark::No);
        }
    }

    /// The one place an icon's request size is chosen, so the masthead's mark and a bookmarks list
    /// row's are decoded to the same 64 px and cannot drift apart.
    ///
    /// Answers [`Self::request_image`]'s "did a request leave", which only `load_site_icons` reads.
    fn load_favicon_job(&mut self, src: &str, automatic: bool, mark: Mark) -> bool {
        self.request_image(src, ICON_PX, false, automatic, mark)
    }

    /// Whether a subresource reply stamped `page` is still wanted.
    ///
    /// Two rules, because [`ReaderApp::mark_owner`] hands out two kinds of owner. A page's reply
    /// is wanted while that page is the one on screen; that is the test this file has always
    /// made, and it is what stops a texture being uploaded for a document the reader has left.
    ///
    /// A [`net::ReqId::CHROME`] reply belongs to the sidebar, which no navigation makes stale,
    /// so the question is asked of the store instead: is the strip still waiting for this
    /// address? It is not, if a page committed in the meantime — `ImageStore::clear_page` drops
    /// every *pending* mark — and then this is exactly the old answer, the reply is dropped, and
    /// the row is asked for again through the ordinary door with a page to own it. Without the
    /// test the same bytes would arrive twice: once here and once for the page that re-asked.
    fn awaits(&self, page: ReqId, src: &str) -> bool {
        reply_is_wanted(
            page,
            self.shown().map(|r| r.req),
            self.images.is_pending(src),
        )
    }

    /// Which request a mark drawn now belongs to, or `None` when nothing may ask for one.
    ///
    /// A page's mark belongs to that page: its reply is wanted while the page is on screen and
    /// is thrown away when the reader leaves, which is what keeps a texture from being uploaded
    /// for a document nobody is reading. The bookmarks sidebar is the one surface that draws
    /// marks with *no* page under it — `OpenOn::Nothing`, and the frames of a launch before the
    /// first view commits — and it used to draw a column of empty squares there, because the
    /// reply had no page to be matched against and so the request was never made at all.
    ///
    /// [`net::ReqId::CHROME`] is what it belongs to instead: a surface that outlives every page,
    /// which no navigation makes stale. It is handed out here and nowhere else.
    ///
    /// **Only [`Mark::Kept`] gets it, and only the disk half of the door acts on it.** This
    /// answers for `ReaderApp::kept_mark`, which reads a file and contacts nobody;
    /// `request_image` still wants a page, so with none on screen the strip draws the marks this
    /// machine already holds and asks for no new ones. That is the disclosure rule and not
    /// timidity: a fetch is counted in the page's own account and reported by its page-info
    /// panel, and a fetch made with no page on screen is one the reader has no panel to read it
    /// in — the counters are the page's, so the next commit resets them and the request is
    /// reported nowhere at all. A read of a file needs no such panel, which is the same
    /// asymmetry `reader::iconcache` argues for the store itself.
    fn mark_owner(&self, mark: Mark) -> Option<ReqId> {
        match self.shown() {
            Some(ready) => Some(ready.req),
            None => chrome_owner(mark),
        }
    }

    /// File this address as a *list's* mark, which is the class question and not the disk one.
    ///
    /// [`Mark::Kept`] alone, and the third permission `Mark` carries: a mark drawn from the store
    /// may be any of the three, a mark written to it must be `Kept`, and a mark that outlives the
    /// page it was drawn on must be `Kept` as well. `ImageStore::note_mark` is what buys the last
    /// two of those — a settled entry that survives `clear_page` and is evicted against
    /// `MARK_CAPACITY` rather than the pictures' ceiling — and both are wrong for the page's own
    /// favicon, which is what [`Mark::Read`] is.
    ///
    /// Wrong twice over, and neither failure is visible. The favicon of every host visited would
    /// compete for the strip's budget, so a reader who browsed a few dozen sites with the sidebar
    /// up would have its rows evicted against the very ceiling written to protect them and
    /// re-read off the disk on the next frame. And a *transient* favicon failure would stick for
    /// the session: `clear_page` keeps a settled mark, and `ensure_favicon` returns early on an
    /// address with any state at all, so a host that answered badly once would never be asked
    /// again. A page's favicon goes with its page, which is what it did before the strip existed.
    fn note_list_mark(&mut self, src: &str, mark: Mark) {
        if mark == Mark::Kept {
            self.images.note_mark(src);
        }
    }

    /// Draw a mark from `reader::iconcache` if it is there, and say whether that settled it.
    ///
    /// **The one place a request is decided against before a request is recorded.** It has to be
    /// synchronous, and that is not an optimisation: `ImageStore::record_request` moves the host
    /// tally and the Referer count ahead of the bytes leaving, so a mark already on this machine
    /// has to take a path that never reaches it. A cached mark counted there would have page
    /// info naming hosts nobody was contacted at, which is precisely the bug the `Referer sent`
    /// row was carrying when it counted the reader's setting instead of what
    /// `fetch::page_origin` would actually send.
    ///
    /// Answers under **every** policy, `ImagePolicy::NoRequests` included. Nothing here contacts
    /// anybody, so nothing here breaks that promise — and it strengthens it, because until this
    /// existed the reader who asked for no requests got a permanently empty column and no way to
    /// have one that was not.
    ///
    /// `Known::Nothing` settles it too, and is why this returns a bool rather than an
    /// `Option<Job>`: an address hww has already asked and found empty must not be asked again
    /// for the rest of the week, and a permanent `Failure` is how the rest of this file already
    /// says "do not re-request this". The words never reach a reader — neither `images::favicon`
    /// nor `images::site_icon` draws anything but a ready texture — which is `notice`'s problem
    /// and not this door's.
    fn kept_mark(&mut self, src: &str, mark: Mark) -> bool {
        if self.icons.is_empty() {
            return false;
        }
        let Some(page_req) = self.mark_owner(mark) else {
            return false;
        };
        // Through the archive's own door, so a key can never be built from an address that
        // skipped the scheme test or kept its credentials. A list row's `src` is already this
        // string; the masthead's is whatever `html::favicon_of` joined, and normalising here is
        // what lets a bookmarked page draw its own mark off the disk.
        let Some(key) = archive::icon_address(src) else {
            return false;
        };
        match self.icons.known(&key) {
            iconcache::Known::Kept(path) => {
                self.note_list_mark(src, mark);
                self.images.begin(src, Source::Disk);
                self.net.submit(Job::KeptIcon {
                    req: self.net.mint(),
                    page: page_req,
                    src: src.to_owned(),
                    key,
                    path,
                    max_width: ICON_PX,
                });
                true
            }
            iconcache::Known::Nothing => {
                // Before the `fail`, and this is the arm that would be missed: a settled refusal
                // never reaches `begin`, so without this a row's mark is filed as a picture and
                // evicted against the pictures' ceiling.
                self.note_list_mark(src, mark);
                self.images
                    .fail(src, Failure::permanent(NO_SITE_ICON.to_owned()));
                true
            }
            iconcache::Known::Unknown => false,
        }
    }

    /// Which kind of mark the page's own favicon is: a row's, if a marked row names that exact
    /// address, and otherwise the page's alone.
    ///
    /// The doctrine's write rule is about the *address*, not the door: what may be kept on disk
    /// is a mark for a row of a marked list. Until this existed the door stood in for the rule —
    /// `ensure_favicon` fires on every page hww shows, so it was `Mark::Read` and never wrote —
    /// and the case that stands between the two is a bookmarked page being read. Its favicon and
    /// its row's mark are one address and one request, so the reader who kept a page, opened it
    /// again a week later and had the sidebar draw its mark the whole time still had nothing on
    /// disk for it: the strip skips a `src` the store already holds, so the request that could
    /// have written it never came from the list's door at all.
    ///
    /// Asking the archive is what makes the rule the rule. No page hww merely *visited* can
    /// answer yes here, so nothing changes for the host of an ordinary page, and the permission
    /// is re-asked when the write lands (see `Kept::Written`), because the reader may have
    /// pressed `Ctrl+D` again while the bytes were in flight.
    fn favicon_mark(&self, src: &str) -> Mark {
        match archive::icon_address(src) {
            Some(key) if self.archive.marks_icon(&key) => Mark::Kept,
            _ => Mark::Read,
        }
    }

    /// Put the mark of the page on screen beside its eyebrow, once.
    ///
    /// Two doors in one, in this order and not the other: the store first, under every policy,
    /// and the network second, under `allows_any_request` alone. Failures stay silent either
    /// way; the eyebrow label does not need a retry control for a missing mark.
    ///
    /// The fetch is the one image request the reader makes without being asked, which is why it
    /// is the one `ImagePolicy::NoRequests` exists to stop. `Never` deliberately does not: it
    /// turns off *article* images, and it meant that before the third policy arrived.
    ///
    /// **Writes only for an address a marked row names**, which [`ReaderApp::favicon_mark`]
    /// decides and this function does not. It fires on every page hww shows, marked or not, so a
    /// write for the page's sake would put a favicon on disk for every host the reader has
    /// visited — a record that outlives *forget history*, exists with `keep_history` off, and has
    /// no switch over it. What the archive names is the other half: that address is already
    /// written down, its file is deleted with the row that named it, and the reader kept the page
    /// on purpose.
    ///
    /// The class question follows the same answer: a `Mark::Read` entry goes with its page and is
    /// evicted against the pictures' ceiling, a `Mark::Kept` one is the strip's row and outlives
    /// the commit. See `ReaderApp::note_list_mark`.
    fn ensure_favicon(&mut self) {
        // Asked before it is owned. The clone exists only to end the borrow of `self.page`
        // before `kept_mark` takes `&mut self`, and on every frame but the first the answer
        // below is "already have it" — so the page that declares a favicon was cloning its
        // address sixty times a second to throw it away.
        let Some(src) = self.shown().and_then(|r| r.loaded.doc.favicon.as_deref()) else {
            return;
        };
        if self.images.state(src).is_some() {
            return;
        }
        let src = src.to_owned();
        let mark = self.favicon_mark(&src);
        if self.kept_mark(&src, mark) {
            return;
        }
        if !self.settings.read.images.allows_any_request() {
            return;
        }
        self.load_favicon_job(&src, false, mark);
    }

    /// Draw the site marks a marked list just laid out, for the rows near the window.
    ///
    /// The fifth door onto [`Self::request_image`], and the second one the reader did not push:
    /// `ensure_favicon` handles the mark of the page on screen and this handles the mark of each
    /// page a row on it points at. Same silence on failure — a column with a gap in it is what a
    /// site that serves no icon looks like, and there is nothing for the reader to retry.
    ///
    /// **Two doors, asked in that order.** The store answers under every policy, because a read
    /// contacts nobody; the network answers under `allows_any_request` alone, in this door's own
    /// name as before. So `ImagePolicy::NoRequests` on a list whose marks are already kept draws
    /// a full column and makes no request, which is the shape that policy always wanted.
    ///
    /// **Rows here are the only marks written down** ([`Mark::Kept`]), and that is the whole of
    /// the doctrine's *write* half: every `src` reaching this function came from
    /// `archive::mark_beside`, so every one of them is an address a bookmark or a reading-list
    /// row still names. `ensure_favicon` fires on every page instead, so it may only read.
    ///
    /// `srcs` is already bounded to the layout band by `RenderCtx::note_icon`, which is what
    /// makes this a screenful of requests on a two-thousand-row bookmarks rather than two
    /// thousand. `automatic: true`, so a worker that reaches a queued mark after the reader has
    /// navigated away drops it instead of contacting a host for a page nobody is looking at.
    fn load_site_icons(&mut self, srcs: &[String]) {
        // Nothing to ask for is the steady state of every surface that calls this — a list whose
        // marks have all settled, and every frame of the sidebar after the first few. The walk
        // below is `Images::pending` over the whole state map, so answering it before the walk
        // rather than inside the loop is what keeps an open strip off the frame budget.
        if srcs.is_empty() {
            return;
        }
        // The same guard `autoload` carries, for the same reason and against a sharper failure.
        // While a navigation is in flight the pool answers an automatic job for the outgoing
        // page with `Msg::ImageDropped`, and the handler hands the `src` back by forgetting its
        // state — so without this, a click on a bookmarks list row before its neighbours' marks have
        // resolved spins request-and-drop at frame rate for the whole of the load, pumping the
        // pool and moving the host counters for requests nobody wanted.
        if self.current != self.shown().map(|r| r.req) {
            return;
        }
        let may_fetch = self.settings.read.images.allows_any_request();
        // Counted once and kept: `Images::pending` walks the whole state map, and asking it per
        // candidate made the loop quadratic in a list whose marks are still arriving. It counts
        // network requests alone, so the marks `kept_mark` settles off the disk are outside it
        // in both builds, and `load_favicon_job` reports whether a request actually left — so
        // the running figure is the same figure `pending()` would answer with.
        let mut pending = self.images.pending();
        for src in srcs {
            // Asked once per page, and this is the line that stops the LRU from becoming a
            // request loop: an in-band mark evicted by a later one drops back to no state at
            // all, `note_icon` records it again, and without this record hww would re-fetch it
            // from a third-party host for as long as the bookmarks is on screen. A disk cache
            // makes that refetch cheap and not free, so the guard stays.
            if self.icons_asked.contains(src) {
                continue;
            }
            // Ahead of the throttle, deliberately: `MAX_OUTSTANDING` bounds how many *hosts* may
            // be mid-request at once, and reading a file on this machine contacts none of them.
            // Counting these against it drew a thirty-row band eight rows to a frame, waiting on
            // a ceiling that was never about them.
            if self.kept_mark(src, Mark::Kept) {
                self.icons_asked.insert(src.clone());
                continue;
            }
            if !may_fetch {
                // Not recorded as asked: nothing was, and a reader who turns the policy back on
                // while the list is open should see the column fill in rather than have to
                // reload for it.
                continue;
            }
            // Bounded like the automatic policy's, and by the same constant: the band is three
            // window-heights of rows, so a tall window on a long bookmarks would otherwise open
            // thirty connections in one frame. What is left over is recorded again next frame,
            // so the rest arrive as a drip rather than being dropped.
            if pending >= autoload::MAX_OUTSTANDING {
                break;
            }
            self.icons_asked.insert(src.clone());
            // `+= 1` on the answer and not on the call: `request_image` declines an address it
            // cannot join and a format it will not decode, and neither of those is a connection.
            // Counted regardless, a band holding a handful of SVG marks reached
            // `MAX_OUTSTANDING` having contacted nobody and left the rows under them undrawn
            // for the frame.
            if self.load_favicon_job(src, true, Mark::Kept) {
                pending += 1;
            }
        }
    }

    /// Ask for one picture, and say whether that put a request on the network.
    ///
    /// **False is not failure.** Every early return below is a request that did not leave — the
    /// answer is already known, the address will not join, the format is declined — and each one
    /// is a decision this function makes rather than one its callers can make for it.
    /// `load_site_icons` throttles on a running count of requests in flight, and a caller that
    /// assumed the call it just made was one of them counted an SVG mark it had refused to fetch
    /// against a ceiling that bounds open connections.
    fn request_image(
        &mut self,
        src: &str,
        max_width: u32,
        announce: bool,
        automatic: bool,
        mark: Mark,
    ) -> bool {
        // The visible page, and *its* request id rather than `self.current`: see `Ready::req`.
        let Some(ready) = self.shown() else {
            return false;
        };
        let page_req = ready.req;
        if self.images.is_pending(src) {
            return false;
        }
        // Beside `is_pending`, and for the same reason: this is the one door, and the paths
        // that reach it without drawing a control — "load all", and a click on a placeholder
        // the LRU evicted and redrew as an offer — would otherwise re-issue a request whose
        // answer is already known.
        if self.images.refuses_retry(src) {
            return false;
        }
        let base = ready.loaded.prov.final_url.clone();
        let Ok(url) = base.join(src) else {
            self.images
                .fail(src, Failure::permanent("unusable image address".to_owned()));
            return false;
        };
        // Ahead of the host, the disclosure, and the job. A format this reader declines was
        // being fetched in full and then refused from its bytes, which cost a third-party
        // request, up to the transfer cap in wasted transfer, and a host recorded in the
        // panel as if a picture had been loaded from it. Nothing below this line runs, so
        // nothing is contacted and nothing is claimed.
        //
        // Worded through `DecodeError::Unsupported` rather than a second literal: the reader
        // must not learn one phrase for a declined format fetched and another for one that
        // was not.
        if let Some(name) = image_formats::declined_by_url(&url) {
            let why = image_decode::DecodeError::Unsupported(name).to_string();
            self.images.fail(src, Failure::permanent(why));
            return false;
        }
        let host = url.host_str().unwrap_or("?").to_owned();
        // Where these bytes may be kept, decided here and carried to the worker. A worker sees
        // one kind of image job and cannot see the archive, so it can neither tell a mark from a
        // picture nor tell a marked address from a merely visited one. See `reader::iconcache`.
        let cache_as = (mark == Mark::Kept)
            .then(|| {
                archive::icon_address(src).and_then(|key| {
                    self.icons
                        .place_for(&key)
                        .map(|path| net::CacheAs { path, key })
                })
            })
            .flatten();
        self.note_list_mark(src, mark);
        self.images.begin(src, Source::Network);
        // Recorded here, where the host is known and where the reader is told it, and taken
        // back by `ImageStore::forget` if the pool answers `Msg::ImageDropped` for it. A
        // counter moved for a request that never left is the panel claiming a disclosure that
        // did not happen.
        let send_referer = sends_referer(self.settings.send_image_referer, mark);
        // The setting *and* whether this page can produce an origin at all, through the same
        // function the header is built with (`fetch::page_origin`). A page hww built itself has
        // an opaque origin, so a built view's site marks carry no Referer however the setting is
        // set, and a panel that counted the setting would report a disclosure that never left.
        let referer = send_referer && fetch::page_origin(&base).is_some();
        self.images
            .record_request(src, &host, referer, mark != Mark::No);
        // Decode straight to the reading column: without this, "load images" on a photo-heavy
        // page is a GPU-memory denial of service driven by untrusted input.
        self.net.submit(Job::Image {
            req: self.net.mint(),
            page: page_req,
            src: src.to_owned(),
            url,
            referrer: base,
            max_width,
            send_referer,
            automatic,
            cache_as,
        });
        if announce {
            self.flash(format!("loading one image from {host}"));
        }
        true
    }

    fn load_all_images(&mut self, ctx: &egui::Context) {
        // Ahead of the walk, so the answer is one line rather than one per image on the page.
        // `load_image` guards too, and has to: it is also the click and `i` path.
        if !self.settings.read.images.offers_loading() {
            self.flash(IMAGES_ARE_OFF.to_owned());
            return;
        }
        let Some(ready) = self.shown() else {
            return;
        };
        let all = collect_image_srcs(&ready.loaded.doc, self.settings.read.entry_summary_bytes);
        let base = ready.loaded.prov.final_url.clone();
        // Declined, and *only* declined. Testing the address the other way round dropped every
        // `src` that fails to join too, so a page whose images all had unusable addresses
        // reported them as a format hww does not display — a second wrong claim about the page,
        // in place of the one this removes. An unusable address still belongs in the list;
        // `request_image` has the honest message for it. Asked twice below: once to decide what
        // to request, and once to decide what to say when that comes to nothing.
        let is_declined = |s: &str| image_formats::declined_by_href(s, &base).is_some();
        // Before the hosts are gathered and before the count is taken. This remark names every
        // host it is about to contact, so a declined address left in would have it name one it
        // never reaches — the same claim about the network that `autoload::plan` refuses to
        // make for an address that will not resolve.
        let srcs: Vec<String> = all
            .iter()
            .filter(|s| !is_declined(s))
            // And the same argument one step later. `request_image` returns without asking for
            // a picture that has already failed permanently, so counting one here would put a
            // host in the remark that this press will not contact — "loading 3 image(s) from
            // …" for three addresses nothing re-requests.
            .filter(|s| !self.images.refuses_retry(s))
            .cloned()
            .collect();
        let mut hosts: Vec<String> = srcs
            .iter()
            .filter_map(|s| base.join(s).ok())
            .filter_map(|u| u.host_str().map(str::to_owned))
            .collect();
        hosts.sort();
        hosts.dedup();
        if srcs.is_empty() {
            // Which of the four is true matters, and they are four different pages: one with no
            // pictures, one whose pictures hww will not open, one whose pictures were asked for
            // and refused, and one that is some of each. Counted rather than asked with `any`,
            // which answered "every image has already failed" for a page where most of them
            // were never requested at all.
            let declined = all.iter().filter(|s| is_declined(s)).count();
            self.flash(match (all.len(), declined) {
                (0, _) => "no images on this page".to_owned(),
                (n, d) if n == d => IMAGES_ARE_DECLINED.to_owned(),
                (_, 0) => IMAGES_ALL_FAILED.to_owned(),
                _ => IMAGES_ARE_DECLINED_OR_FAILED.to_owned(),
            });
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
                        // Exhaustive on purpose: a new `Rewrite` variant fails to compile here
                        // rather than reaching the user unreported.
                        let line = match &rewrite {
                            Rewrite::Applied { .. } | Rewrite::Searched { .. } => {
                                rewrite.to_string()
                            }
                        };
                        // One navigation can produce both, and each names a host the user did
                        // not type. Neither may quietly replace the other.
                        self.rewrite_notice = Some(match self.rewrite_notice.take() {
                            Some(prev) => format!("{prev} {line}"),
                            None => line,
                        });
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
                    self.fail(url, error);
                }
                Msg::Image {
                    page,
                    src,
                    cookies,
                    result,
                    kept,
                    ..
                } => {
                    // Before the staleness test, and the only thing here that is: a file was
                    // written on this machine whether or not the reader is still looking at the
                    // page that asked for it, and an index that did not learn about it would
                    // have the second open of a list in one session refetch every mark the
                    // first open had just kept.
                    //
                    // The archive is asked *again* here, and not because the permission was in
                    // doubt when it was granted. It was granted when the job was submitted, and
                    // between then and now the reader may have pressed `Ctrl+D`, a row's
                    // `remove`, or either forget button — `prune_icons` runs on all four, while
                    // this fetch is still in flight and its worker still about to call
                    // `iconcache::write`, which creates the directory again on its way past.
                    // Taking the file back is the only place that can be answered: the worker
                    // cannot see the archive, and the prune could not see a file that did not
                    // exist yet.
                    if let Kept::Written(at) = kept
                        && let Some(key) = archive::icon_address(&src)
                    {
                        if self.archive.marks_icon(&key) {
                            self.icons.note(&key, at, result.is_err());
                        } else {
                            self.icons.forget(&key);
                        }
                    }
                    // Against the page the request was made *from*, not the navigation in
                    // flight: see `Ready::req`. A picture whose page has been replaced is
                    // dropped, which is the whole reason the id is stamped there.
                    if !self.awaits(page, &src) {
                        continue;
                    }
                    // Counted before the result is looked at: a cookie the CDN tried to set is
                    // a thing that happened whether or not the picture ever decoded.
                    self.images.cookie_attempts += cookies;
                    match result {
                        Ok(decoded) => self.images.insert(ctx, &src, decoded, kept == Kept::Read),
                        Err(e) => {
                            let why = e.to_string();
                            self.images.fail(
                                &src,
                                if e.offers_retry() {
                                    Failure::transient(why)
                                } else {
                                    Failure::permanent(why)
                                },
                            );
                        }
                    }
                }
                Msg::ImageDropped { page, src } => {
                    // Same staleness test as `Msg::Image`, and it is nearly always false here:
                    // a job is dropped because its page stopped being current, so the page it
                    // names is usually already gone. It matters in the case that is left — a
                    // navigation that failed and put the reader back on nothing — where the
                    // placeholder has to go back to offering itself rather than sit on
                    // `Loading` for a request no worker still holds.
                    if !self.awaits(page, &src) {
                        continue;
                    }
                    // Takes back the entry and the disclosure the queued job had recorded.
                    self.images.forget(&src);
                    // Nothing was requested, so nothing was attempted: if this picture is
                    // still in the band the automatic policy may plan it again, with its full
                    // allowance intact. A site mark is handed back for the same reason — this
                    // arm is the navigation that failed and put the reader back where they
                    // were, and the bookmarks they are still looking at should draw its column.
                    self.auto_attempts.remove(&src);
                    self.icons_asked.remove(&src);
                }
                Msg::IconMissed { page, src } => {
                    // The index said this mark was on the disk and the worker found it gone,
                    // unreadable, or written for another address. Dropped from the index here,
                    // *before* anything else, so the retry below cannot come back to the same
                    // wrong answer and ping-pong at frame rate.
                    if let Some(key) = archive::icon_address(&src) {
                        self.icons.forget(&key);
                    }
                    if !self.awaits(page, &src) {
                        continue;
                    }
                    // Handed back exactly as a dropped job is: nothing was requested and nothing
                    // was disclosed, so the mark goes back to having no state and `note_icon`
                    // records it again next frame. What it reaches then is the ordinary door,
                    // which asks the policy and records the request — which is the whole reason
                    // a worker answers a miss instead of fetching one itself.
                    self.images.forget(&src);
                    self.icons_asked.remove(&src);
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
                        let url = self.loading_url().unwrap_or_else(|| {
                            Url::parse("about:blank").expect("a literal URL parses")
                        });
                        self.fail(
                            url,
                            LoadError::Fetch(crate::fetch::FetchError::Io(std::io::Error::other(
                                "a worker failed while handling this request",
                            ))),
                        );
                    }
                }
            }
        }
    }

    fn settle(&mut self, loaded: Loaded) {
        // A dead rewrite rule used to flash here for twelve seconds and then be unrecoverable.
        // It is a bar now (`notice::about_page`), which lasts as long as the page it is about:
        // "this may not be the page you asked for" is not a thing to say once and withdraw.
        let outline = outline::build(&loaded.doc.blocks);
        let masthead = title::masthead(&loaded.doc);
        let (text_len, rtl, threads) = derived(&loaded.doc);
        let req = self.current.unwrap_or_else(|| self.net.mint_page());
        self.install(Box::new(Ready {
            loaded,
            outline,
            masthead,
            text_len,
            rtl,
            threads,
            req,
            // Sound here and nowhere later: `drain` reaches `settle` only when
            // `self.current == Some(req)`, and `navigate` writes `current` and `last_opts`
            // in the same call, so the two cannot disagree about which navigation this is.
            opts: self.last_opts,
            fetched: Instant::now(),
            restored: false,
            // A document that came back from `Job::Load`, which is a fetch by construction.
            // The built views never reach `settle`; they are installed by `show_builtin`.
            built: false,
        }));
    }

    /// Put a page in the column. Both a fetch and a restore end here.
    ///
    /// Extracted from `settle` so the two cannot drift: a restore has to do everything an
    /// arriving fetch does except produce the document, in the same order and for the same
    /// reasons, and a reset added to one path and not the other is invisible until a reader
    /// finds it.
    fn install(&mut self, page: Box<Ready>) {
        // Before the page is installed, so `jump_to_fragment` below overrides the reset rather
        // than being overridden by it. Also where the outgoing page is filed away.
        self.commit();
        // The strip line lasts until the navigation ends. The page is up; the rewrite bar
        // owns the remark from here.
        self.rewrite_notice = None;
        // The history is recorded here, and the argument is `commit`'s own one line up: this is
        // the function every page that takes the column passes through, so a transition added
        // later cannot record half of them. It covers a restore from the page cache too, which
        // is right — Back onto a page is a page hww drew — and it deliberately does not cover an
        // error screen, which never reaches `install`.
        //
        // A built view records nothing: `hww:history` must not list itself, and a bookmarks list the
        // reader opens twice a day is not something they read.
        if !page.built {
            self.record_visit(&page);
        }
        // After `commit`, which cleared the outgoing page's index, and before the page is
        // installed, which is the last moment the document is not behind `self.page`.
        self.image_blocks_budget = self.settings.read.entry_summary_bytes;
        self.image_blocks = image_blocks(&page.loaded.doc, self.image_blocks_budget);
        self.page = Page::Ready(page);
        // Where this page was left, if it is a page the reader has been on: Back, Forward and
        // `r` all arrive here with the cursor on an entry that has been read before, and
        // `commit` has just reset the column to the top. A link followed, or a URL typed, mints
        // an entry at zero and takes the fragment arm instead.
        self.shown_entry = self.history.current_id();
        let restore = self
            .shown_entry
            .map_or(0.0, |id| self.history.offset_of(id));
        if restore > 0.0 {
            self.scroll_offset = Some(restore);
        } else if let Some(f) = self
            .history
            .current()
            .and_then(|u| u.fragment().map(str::to_owned))
        {
            // Read here rather than passed in. Both callers would compute it from
            // `self.history.current()` and `commit` does not touch the stack, so a parameter
            // would be the one input the two paths could still drift on — which is the thing
            // this function was extracted to prevent.
            self.jump_to_fragment(&f);
        }
        // The page changed, so the cached answer to "is this one kept?" is about the wrong page
        // until this runs. Once per navigation, against a menu bar that would otherwise ask it
        // once per frame.
        self.refresh_shown_kept();
    }

    /// Put an error screen in the column. **The one door**, for the reason `install` is one:
    /// three callers reached it — a failed load, a worker that panicked under a document, and
    /// `present` — and each had to remember the same three steps.
    ///
    /// The third of those steps is the one that was being forgotten. A failure replaces the page
    /// it was fetched over, so it is a commit like any other; it also *unshows* a page, and
    /// [`ReaderApp::shown_bookmarked`] is a cache about the page on screen. Without the refresh, a
    /// link that times out on a page the reader had kept leaves the File menu drawing that
    /// page's tick over an error screen.
    fn fail(&mut self, url: Url, error: LoadError) {
        self.commit();
        self.page = Page::Failed {
            url,
            error,
            opts: self.last_opts,
        };
        self.refresh_shown_kept();
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
        self.current = Some(self.net.mint_page());
        self.rewrite_notice = rewrite_notice;
        self.history = History::new();
        // For the same reason the stack is replaced rather than pushed onto. Ids are monotonic
        // for the process, so nothing here could collide; a cache carried between scenes would
        // make one scene's picture depend on how many ran ahead of it, which is a picture that
        // changes without the page changing.
        self.pages.clear();
        self.history.push(url.clone());
        match result {
            Ok(loaded) => self.settle(loaded),
            Err(error) => self.fail(url, error),
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
    /// Called from every place a page actually takes the column: `install`, on behalf of both
    /// an arriving fetch and a restore from memory; the `Msg::Failed` and `Msg::Panicked` arms
    /// of `drain`, where an error screen replaces the page it was fetched over; and `present`,
    /// so the screenshot tool cannot drift from the navigation path. `keep_shown` is the one
    /// transition that does not, because its page never leaves the column; it calls
    /// [`ReaderApp::prune_pages`] for the half of this that still applies.
    ///
    /// It also *keeps* the page being replaced, rather than only clearing the state derived from
    /// it. Here because this is the one function every page transition passes through, so a
    /// fourth transition added later cannot forget to file the page it displaced — the same
    /// argument that put the resets here. See `reader::pagecache` for what is kept and why.
    fn commit(&mut self) {
        // The page leaving the column, kept for the entry it belongs to rather than dropped.
        // Taken, never cloned: a `Ready` is a document, an outline and a masthead, and the one
        // copy of it moves from the column into the cache and back out again.
        //
        // `stash_under` is what keeps a reload from filing its own outgoing document under the
        // entry the reloaded one is about to occupy, and `holds` covers the entry that was
        // truncated away while its page was still on screen — `navigate` discards the branch
        // ahead at dispatch, so by the time a reply lands the outgoing page may name nothing.
        if let Some(id) = stash_under(self.shown_entry, self.history.current_id())
            && self.history.holds(id)
            && let Some(page) = self.page.take_shown()
        {
            // A built view is never kept, and that is a feature rather than a cost. The bookmarks
            // has to reflect what has been kept or forgotten since it was drawn, so Back onto it
            // rebuilds through `show_builtin` and shows a current list; a cached copy would show
            // the reader a page they have already changed. Its `chars` is zero, too, so it would
            // be a free entry that never expires under the cache's own budget.
            if !page.built {
                // `prov.chars` *is* `doc.text_len()`, computed once in `session::load`.
                // Re-walking the block tree on every navigation to learn what the fetch already
                // measured would be the same number for more work.
                let (cost, opts) = (page.loaded.prov.chars, page.opts);
                self.pages.insert(id, page, cost, opts);
            }
        }
        // Paired with the insert above rather than left to the caller, because they are one
        // rule: file the page leaving the column, drop every page the stack can no longer reach.
        self.prune_pages();
        // Textures and the disclosure counters both die with the page they belong to: the
        // page-info panel answers how the page on screen arrived, not what the session has
        // done since it started.
        self.images.clear_page();
        // Both are about the page being replaced: the blocks index and the attempt counts
        // describe the document that is going away, and an attempt spent on it must not count
        // against the same `src` on the page arriving.
        self.image_blocks.clear();
        self.auto_attempts.clear();
        self.icons_asked.clear();
        self.heights.clear();
        self.collapsed.clear();
        self.find_current = 0;
        self.find_total = 0;
        self.pending_link = None;
        self.pending_href = None;
        self.chrome.dismissed.clear();
        self.top_block = None;
        // The arriving page has no entry of its own until `settle` names one, and a failure
        // never gets one: an error screen is not a position worth remembering.
        self.shown_entry = None;
        self.scroll_offset = Some(0.0);
        // The other two scroll one-shots, or a `Shift+G` pressed on the very frame a page
        // commits is applied against the *outgoing* page's `max_scroll` and opens the new one
        // at an arbitrary offset.
        self.scroll_delta = 0.0;
        self.scroll_to_block = None;
    }

    /// Drop every kept page the history stack can no longer reach.
    ///
    /// A page kept for an entry that has been truncated away is memory nothing can spend:
    /// `navigate` discards the branch ahead the moment it dispatches, so the entries a Back
    /// filled the cache for can stop existing while their pages are still held. Called from
    /// `commit`, which pairs it with the insert, and from `keep_shown`, which is the one page
    /// transition that does not commit.
    fn prune_pages(&mut self) {
        self.pages.retain(|id| self.history.holds(id));
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

        // While a menu or a context menu is open, egui owns the keyboard. It closes a popup on
        // `Escape` in its own `popup.rs`, and it never sees the key because this function runs
        // before any panel is drawn and takes `Escape` unconditionally two lines below. Without
        // this guard an open menu could not be dismissed, and every bare letter below would fire
        // its page action underneath it: `d` would cycle the theme under an open View menu.
        //
        // This was already true of the link context menu before there was a menu bar. The bar is
        // what makes it constant rather than obscure.
        if egui::Popup::is_any_open(ctx) {
            return;
        }

        // Pointer side buttons: "typically corresponds to the Browser back/forward button".
        //
        // Ahead of the `typing` return below, because a thumb button is not text: a browser goes
        // back from a focused address bar, and reading these after that guard meant the mouse
        // stopped working the moment the reader clicked into the URL field.
        //
        // `button_pressed` and not `button_clicked`. egui only calls a release a click when the
        // pointer has moved less than `max_click_dist` (6 points) since the press and the button
        // was held for less than `max_click_duration` (0.8s). Both are written for a button under
        // a finger that is aiming at a widget; a side button is under the thumb of the hand
        // holding the mouse, so pressing it nudges the pointer, and the first press of a run is
        // the slow deliberate one. Either condition drops the whole click silently, which is the
        // "nothing happened, click it again" this fixes. Nothing here is aimed at a widget, so
        // neither condition buys anything: the press is the whole gesture.
        let (extra1, extra2) = ctx.input(|i| {
            (
                i.pointer.button_pressed(egui::PointerButton::Extra1),
                i.pointer.button_pressed(egui::PointerButton::Extra2),
            )
        });
        if extra1 {
            self.go_back();
        }
        if extra2 {
            self.go_forward();
        }

        // Minimize and Close, which on macOS are the Window menu's key equivalents rather than
        // anything a window handles for itself — and winit builds only the application menu:
        // About, Services, Hide, Hide Others, Show All, Quit. With no Window menu in the bar,
        // `Cmd+M` and `Cmd+W` reach the window like any other chord and nothing answered them,
        // so a Mac reader pressed the two keys every application on the system answers and hww
        // sat there.
        //
        // Bound here rather than by building an `NSMenu`, because hww's menu bar is its own and
        // is drawn in the window; and absent from `menu::keyspec` and the help card, because
        // these are the platform's keys and not hww's grammar. The tables spell what hww binds,
        // and no browser lists Minimize in its own help either — the same reason `keyspec`
        // describes `Cmd+H` as a key the system takes rather than as one hww offers.
        //
        // Above the typing guard, for the reason the side buttons are: a window key is not
        // text. Every other application on the system minimizes with the caret in a text field,
        // and hww is where it is *most* likely to be, since the address bar takes focus on an
        // empty start. `Cmd+W` closes rather than hiding, because hww is one window and the
        // window going away is the session ending — what `Cmd+W` does to a browser holding a
        // single tab.
        #[cfg(target_os = "macos")]
        {
            let mac = |mods, key| ctx.input_mut(|i| i.consume_key(mods, key));
            if mac(Modifiers::COMMAND, Key::M) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            }
            if mac(Modifiers::COMMAND, Key::W) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }

        let esc = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        if esc {
            // Never the status strip.
            self.chrome.url_bar = None;
            self.chrome.find = None;
            self.chrome.help_open = false;
            self.chrome.about_open = false;
            self.chrome.outline_open = false;
            self.chrome.info_open = false;
            self.chrome.settings_open = false;
            ctx.memory_mut(|m| m.surrender_focus(egui::Id::new(URL_BAR_ID)));
            return;
        }
        if typing {
            // One exception, and it is the find key over the find field. Every browser answers
            // it there by offering the last query back selected rather than by doing nothing,
            // which is the whole use of pressing it twice: search, read, press it again, type
            // over what you searched for. It is a chord and never text, so letting it through
            // costs the field no keystroke it wanted. Everything else while a field has focus
            // stays text, which is what this guard is for.
            if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::F)) {
                self.open_find();
            }
            return;
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
        // Before `COMMAND+R` below. `consume_key` ignores an *extra* Shift, so a bare reload
        // tested second would fire an ordinary one and the toast would never appear.
        if k(Modifiers::COMMAND | Modifiers::SHIFT, Key::R) {
            self.reload(LoadOptions::BARE);
            self.flash(notice::RELOADED_BARE.to_owned());
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
        // Before the unshifted `l` below, which opens the list: `consume_key` ignores extra
        // Shift, so `Shift+L` tested second would open the reading list instead of adding to it.
        if k(Modifiers::SHIFT, Key::L) {
            self.read_focused_link_later();
        }
        if k(Modifiers::SHIFT, Key::Space) || k(Modifiers::NONE, Key::PageUp) {
            self.scroll_delta += page;
        }
        // Before the unshifted `d` below, which cycles the theme: `consume_key` ignores extra
        // Shift and Alt but not the command modifier, and testing the modified binding first is
        // the rule this block already follows.
        if k(Modifiers::COMMAND, Key::D) {
            self.bookmark_page();
        }
        if k(Modifiers::COMMAND, Key::L) {
            self.open_url_bar();
        }
        if k(Modifiers::COMMAND, Key::F) {
            self.open_find();
        }
        if k(Modifiers::COMMAND, Key::R) {
            self.reload(self.opts);
        }
        if k(Modifiers::COMMAND | Modifiers::SHIFT, Key::B) {
            self.open_bookmarks(true);
        }
        // After its shifted twin above and not before it: `consume_key` ignores an *extra*
        // Shift, so a bare `Cmd+B` tested first would swallow `Cmd+Shift+B` and the key that
        // opens the bookmarks would toggle the sidebar instead. Through `run_command` rather
        // than flipping the field here, because unlike the outline this is a saved setting and
        // the arm there is what marks it dirty.
        if k(Modifiers::COMMAND, Key::B) {
            self.run_command(ctx, Command::ToggleBookmarkSidebar);
        }
        if k(Modifiers::COMMAND | Modifiers::SHIFT, Key::O) {
            self.chrome.outline_open = !self.chrome.outline_open;
        }
        if k(Modifiers::COMMAND, Key::I) {
            self.chrome.info_open = !self.chrome.info_open;
        }
        // `Cmd+H` hides the application on macOS: the system takes it and no window ever sees
        // it, so History is `Cmd+Y` there, as it is in Safari and Chrome. The one binding whose
        // *key* and not merely its modifier changes with the platform; `menu::HISTORY` carries
        // the same split for what the tables say, and its comment carries the argument.
        #[cfg(target_os = "macos")]
        let history = k(Modifiers::COMMAND, Key::Y);
        #[cfg(not(target_os = "macos"))]
        let history = k(Modifiers::COMMAND, Key::H);
        if history {
            self.open_history(true);
        }
        if k(Modifiers::COMMAND, Key::Q) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        // `Cmd+,` is the macOS convention for preferences and `Ctrl+,` is what everything with
        // a settings key binds elsewhere, which is one spec through `Modifiers::COMMAND`.
        if k(Modifiers::COMMAND, Key::Comma) {
            self.chrome.settings_open = !self.chrome.settings_open;
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
            self.toggle_help();
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
        if k(Modifiers::NONE, Key::L) {
            self.open_reading_list(true);
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
        // The GTK and Windows convention for reaching a menu bar. It reveals the bar first, so
        // hiding it is never a way to lose it.
        if k(Modifiers::NONE, Key::F10) {
            if !self.settings.show_menu_bar {
                self.settings.show_menu_bar = true;
                self.settings_dirty = true;
            }
            self.chrome.focus_menu_bar = true;
        }
        // Enter follows the focused link. egui's own Tab order does the cycling, because links
        // and comment toggles (and nothing else) are made focusable.
        //
        // The focus test comes **first**, and that ordering is the whole of it: `consume_key`
        // takes the event whether or not the arm that follows does anything, so testing it
        // before `focused_href` swallowed every Enter in the program. Nothing noticed while the
        // page held the only focusable widgets. It stopped being invisible when the menu bar
        // arrived: Enter is how egui activates a focused button, so `F10` then Tab reached a
        // menu title that could not be opened, and the same was true of every chrome button
        // Tab could reach. Consume it only when there is a link to follow.
        if self.focused_href.is_some()
            && k(Modifiers::NONE, Key::Enter)
            && let Some(h) = self.focused_href.clone()
        {
            self.follow_link(&h);
        }
    }

    /// Put the address bar away, because the address in it is about to stop being the address
    /// of the page in the column.
    ///
    /// Every browser does this, and hww did it only for the one navigation the bar starts
    /// itself. So the mouse's Back button — which reads *ahead* of the typing guard on purpose,
    /// since going back from a focused address bar is what a thumb button is for — left the
    /// reader on a new page with the old page's address in a field that still owned the
    /// keyboard, and so did the strip's arrows and the menu's Back.
    ///
    /// Nothing surrenders focus here. egui drops focus at the end of the pass in which the
    /// focused widget was not drawn, and this runs before any panel is drawn, so the field is
    /// gone from this frame's layout and its focus with it.
    fn close_url_bar(&mut self) {
        self.chrome.url_bar = None;
        self.focus_url_bar = false;
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

    fn open_find(&mut self) {
        self.chrome.find = Some(self.chrome.find.take().unwrap_or_default());
        self.focus_find = true;
    }

    fn collapse_focused(&mut self) {
        match self.focused_comment.clone() {
            Some(key) => self.toggle_collapsed(key),
            None => self.flash("no comment focused".to_owned()),
        }
    }

    fn toggle_collapsed(&mut self, key: CommentKey) {
        self.collapsed_changes += 1;
        if !self.collapsed.remove(&key) {
            self.collapsed.insert(key);
        }
    }

    fn collapse_all(&mut self) {
        let Some(ready) = self.shown() else {
            return;
        };
        // The trees the renderer draws, not a second set built here: two builds of one tree
        // are two places for a `CommentKey` to be derived, and a key that does not match the one
        // `thread_ui` asks about collapses nothing while reporting that it did.
        let mut keys = Vec::new();
        for tree in ready.threads.values() {
            for (i, node) in tree.nodes.iter().enumerate() {
                if !node.children.is_empty() && !tree.starts_collapsed(i) {
                    keys.push(node.key.clone());
                }
            }
        }
        if keys.is_empty() {
            self.flash("nothing to collapse".to_owned());
            return;
        }
        self.collapsed_changes += 1;
        for k in keys {
            self.collapsed.insert(k);
        }
    }
}

/// Every image `src` in a block list, in document order, repeats kept.
///
/// `budget` is `ReadOpts::entry_summary_bytes` and is why this takes an argument at all: an
/// entry's summary is cut at the draw, and a walk that read it whole would collect figures from
/// the tail, name their hosts in the load-images disclosure, count them in page info, and fetch
/// them — third-party requests for pictures the reader cannot see. `budget::lead_in` is the one
/// place that decides where the cut falls, and both callers ask it.
fn walk_blocks(blocks: &[ir::Block], budget: u16, out: &mut Vec<String>) {
    for b in blocks {
        match b {
            ir::Block::Figure { image, .. } => out.push(image.src.clone()),
            ir::Block::Paragraph(inlines) | ir::Block::Heading { inlines, .. } => {
                walk_inlines(inlines, out)
            }
            ir::Block::List { items, .. } => items.iter().for_each(|i| walk_blocks(i, budget, out)),
            ir::Block::Quote { blocks, .. } => walk_blocks(blocks, budget, out),
            ir::Block::Table { headers, rows } => {
                headers.iter().for_each(|c| walk_inlines(c, out));
                rows.iter().flatten().for_each(|c| walk_inlines(c, out));
            }
            ir::Block::Thread(cs) => cs.iter().for_each(|c| walk_blocks(&c.blocks, budget, out)),
            // Entry thumbnails draw once loaded (`blocks::entries_ui`), so `I` loads them
            // with the rest and the page-info panel counts them. The summary is walked only as
            // far as it is drawn: see this function's doc comment.
            ir::Block::Entries(es) => es.iter().for_each(|e| {
                out.extend(e.image.as_ref().map(|i| i.src.clone()));
                walk_inlines(&e.title, out);
                walk_blocks(
                    crate::reader::budget::lead_in(&e.summary, budget),
                    budget,
                    out,
                );
            }),
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

/// Every image `src` in one top-level block, in document order, repeats kept.
///
/// The unit the band works in: the reading column decides block by block what is near the
/// window, so what a block contains is what "near the window" can mean for a picture.
/// `autoload::plan` does the deduplicating, because a `src` repeated across two in-band blocks
/// has to collapse the same way a `src` repeated inside one does.
fn block_image_srcs(b: &ir::Block, budget: u16) -> Vec<String> {
    let mut out = Vec::new();
    walk_blocks(std::slice::from_ref(b), budget, &mut out);
    out
}

/// Which top-level blocks contain each image `src`.
///
/// Built once when a page commits and read when a picture resolves, so the height of the block
/// it landed in can be forgotten without the reader walking the document to find out which one
/// that was. A `src` maps to more than one block whenever a page reuses it, which a logo, a
/// sprite, or a repeated byline portrait routinely does.
///
/// This is *not* how the automatic policy decides what is near the window, and must not become
/// it: a feed is one `ir::Block::Entries` and a discussion is one `ir::Block::Thread`, so a
/// single block that touches the band can carry every picture on the page — which would make
/// the bound `reader::autoload` exists to keep no bound at all. `RenderCtx::autoload_srcs`
/// gathers those where each one draws instead.
///
/// The document favicon is absent. It draws in the masthead, which no remembered height covers.
/// So are the bookmarks' site marks, for the neighbouring reason: the column reserves the mark's
/// box whether or not the bytes are here, so an icon arriving re-sizes nothing and there is no
/// height to forget.
fn image_blocks(doc: &ir::Document, budget: u16) -> HashMap<String, Vec<usize>> {
    let mut map: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, b) in doc.blocks.iter().enumerate() {
        for src in block_image_srcs(b, budget) {
            let at = map.entry(src).or_default();
            // Repeats within one block are one entry; the same `src` in two blocks is two.
            if at.last() != Some(&i) {
                at.push(i);
            }
        }
    }
    map
}

fn collect_image_srcs(doc: &ir::Document, budget: u16) -> Vec<String> {
    let mut out = Vec::new();
    walk_blocks(&doc.blocks, budget, &mut out);
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
        let pal = theme::apply(&ctx, &self.settings.read, &mut self.style_installed);
        // Cheap, and the body size it is derived from can change under `[`/`]` and a settings
        // reload, so it is re-derived rather than trusted from startup.
        apply_scroll_speed(&ctx, &self.settings);
        self.drain(&ctx);
        // A link the desktop sent, which on macOS is the only way one arrives after launch.
        // Through `follow_link` and so through `session::classify_link`, because an address off
        // an Apple Event is exactly as much somebody else's string as an `href` on a page is:
        // the bundle claims `http` and `https`, and what actually arrives is whatever the
        // sender put in the descriptor.
        #[cfg(target_os = "macos")]
        if let Some(href) = crate::reader::mac_open::take() {
            self.follow_link(&href);
        }
        self.ensure_favicon();
        self.handle_keys(&ctx);

        // Focus and hover are discovered during render; start each frame not knowing. Hover
        // matters most: `ready_screen` is the only writer, so a value left standing survives
        // into `Loading` and onto the failure screen, where it pins the previous page's link
        // over hww's own words for as long as the fetch takes.
        self.focus_was_on_page = self.focused_href.is_some()
            || self.focused_image.is_some()
            || self.focused_comment.is_some()
            || self.focused_other;
        self.focus_block_was = self.focus_block.take();
        self.focus_href_was = self.focused_href.take();
        self.focus_text_was = self.focused_text.take();
        self.focused_image = None;
        self.focused_comment = None;
        self.focused_other = false;
        self.hover_href = None;

        // Read here rather than where it is used, so it can be held for the frame after as
        // well; see `tab_frames`. `handle_keys` has already run and does not consume Tab, and
        // egui's own focus handling reads it without taking it out of `events`.
        self.tab_frames = if ctx.input(|i| i.key_pressed(egui::Key::Tab)) {
            2
        } else {
            self.tab_frames.saturating_sub(1)
        };

        // Order matters twice over, and the two orders are the same list read for different
        // reasons. **Layout**: a panel takes its space from the order it is added in, so this
        // sequence is where each bar sits — menus, address, find, remarks, page, and the strip
        // holding the bottom line against everything variable above it. **Focus**: egui's Tab
        // cycle is registration order within the frame, with no way to sort it afterwards, so
        // whatever is drawn first is what the first Tab lands on.
        //
        // The menu bar therefore comes first: `File` is where a reader expects Tab to start,
        // and it used to take five presses to reach because the strip's five pointer controls
        // were drawn ahead of it. The strip still precedes every bar whose height the page can
        // change, which is what "never hides" was about; a menu bar is one row and hideable and
        // cannot squeeze it.
        self.menu_bar(ui, &pal);
        self.status_strip(ui, &pal);
        self.url_bar(ui, &pal);
        self.find_bar(ui, &pal);
        // Under the bars and above the page, docked: the one place a remark about the page
        // cannot scroll away and cannot be mistaken for the page.
        self.notice_bars(ui, &pal);
        // Outboard of the outline, so the two stack in a fixed order rather than swapping
        // places depending on which was opened first.
        self.bookmark_sidebar(ui, &pal);
        self.outline_panel(ui, &pal);
        self.central(ui, &pal);
        // *After* the page, because the page is what discovers the hover. An overlay takes no
        // space from anything, so nothing above depends on it having been drawn. See
        // `hover_strip`.
        self.hover_strip(ui, &pal);
        self.toast(&ctx, &pal);
        self.help_overlay(&ctx, &pal);
        self.about_overlay(&ctx, &pal);
        self.info_panel(&ctx, &pal);
        self.settings_panel(&ctx, &pal);

        if let Some(text) = self.pending_copy.take() {
            ctx.copy_text(text);
        }
        self.sync_window_title(&ctx);
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
    /// construction, so changing the measure and quitting inside the window discarded it
    /// silently. Quitting is exactly when a pending write has to land.
    fn on_exit(&mut self) {
        if self.settings_dirty {
            self.save_settings();
        }
        // The same argument one line up, about the other file: the debounce exists so a burst of
        // navigations is one write, and it must not be the reason the last page read is missing
        // from the history.
        if self.visits_dirty.is_some() {
            self.save_visits();
        }
    }
}

impl ReaderApp {
    /// How long `[`/`]` can be held down before a write lands, and how long a page waits to
    /// reach the history.
    const SAVE_DEBOUNCE: Duration = Duration::from_millis(1500);

    /// Name the window after the page in it.
    ///
    /// Read off the state at the end of the frame rather than sent from the transitions that
    /// change it, and that is the point: `install` is one door and `fail` is another, but a
    /// page also arrives by `restore` out of `reader::pagecache` and leaves by a navigation that
    /// has not answered yet, and a title sent from each of those is four places to forget one.
    /// Comparing against the name the window already carries is what makes reading it every
    /// frame cost nothing: winit is told only when the string actually changes.
    ///
    /// The page **on screen**, through `shown`, so the outgoing article keeps naming the window
    /// for the length of a fetch — the same rule the reading column follows, and the reason
    /// `Page::Loading` is not a case here. A failure names the address instead: there is no
    /// title, and an error screen that left the previous page's name in the task list would be
    /// the one place hww's chrome said the reader was somewhere they are not.
    fn sync_window_title(&mut self, ctx: &egui::Context) {
        let page = match &self.page {
            Page::Failed { url, .. } => Some(notice::host_and_path(url)),
            _ => self.shown().map(|r| {
                title::display(&r.loaded.doc)
                    .unwrap_or_else(|| notice::host_and_path(&r.loaded.prov.final_url))
            }),
        };
        let want = notice::window_title(page.as_deref());
        if want != self.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(want.clone()));
            self.window_title = want;
        }
    }

    /// Persist settings, but not on every keystroke of `[`/`]`, and the history, but not on the
    /// frame the page it records appears.
    fn persist(&mut self, ctx: &egui::Context) {
        let zoom = ctx.zoom_factor();
        if (zoom - self.settings.zoom_factor).abs() > f32::EPSILON {
            self.settings.zoom_factor = zoom;
            self.settings_dirty = true;
        }
        if self.settings_dirty {
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
        // Measured from the change rather than from the last write, unlike the settings above.
        // A held `[` is a burst that has to be let through every so often; a navigation is one
        // page, and what the delay is buying is that the write does not land on the frame that
        // page appears. See `ReaderApp::record_visit`.
        if let Some(since) = self.visits_dirty {
            let waited = since.elapsed();
            if waited >= Self::SAVE_DEBOUNCE {
                self.save_visits();
            } else {
                ctx.request_repaint_after(Self::SAVE_DEBOUNCE - waited);
            }
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

    /// The menu bar, and the one command it produced.
    ///
    /// Drawn from `reader::menu`, which holds the table; this supplies the live state the bar
    /// needs to grey an item that would fire on nothing and to tick the ones that reflect a
    /// setting.
    fn menu_bar(&mut self, ui: &mut Ui, pal: &theme::Palette) {
        if !self.settings.show_menu_bar {
            return;
        }
        let enable = menu_ui::Enable {
            page: self.shown().is_some(),
            // Narrower than `page`, and the bookmarks is the difference: once a built view is a
            // real `Page::Ready`, `Needs::Page` is satisfied on it and "keep this page" there
            // would offer to bookmark the bookmarks view.
            fetched_page: self.shown().is_some_and(|r| !r.built),
            focused_link: self.focus_href_was.is_some(),
            thread: self.shown().is_some_and(|r| {
                r.loaded
                    .doc
                    .blocks
                    .iter()
                    .any(|b| matches!(b, ir::Block::Thread(_)))
            }),
            back: self.history.can_go_back(),
            forward: self.history.can_go_forward(),
        };
        let checks = menu_ui::Checks {
            // Cached, not asked. `holds` parses every stored address, and this runs on every
            // frame the bar is drawn: with a full bookmarks a tick mark cost a few thousand URL
            // parses per frame, and the page-info panel asked the same question again.
            bookmarked: self.shown_bookmarked,
            link_addresses: self.settings.read.show_link_urls,
            outline: self.chrome.outline_open,
            page_info: self.chrome.info_open,
            menu_bar: self.settings.show_menu_bar,
            bookmark_sidebar: self.settings.show_bookmark_sidebar,
        };
        let bar = menu_ui::bar(ui, pal, &self.settings, enable, checks);
        panel_edge(ui, bar.rect, Side::Bottom, pal);

        // `F10` asked for focus and the id only exists now that the bar has been laid out.
        if self.chrome.focus_menu_bar {
            self.chrome.focus_menu_bar = false;
            if let Some(resp) = &bar.focus_first {
                resp.request_focus();
            }
        }
        if let Some(command) = bar.command {
            self.run_command(ui.ctx(), command);
        }
    }

    /// Apply one menu command. Every arm is something a key already does, so this is a second
    /// door onto the same rooms rather than a second implementation of any of them.
    ///
    /// Public for `bin/shot.rs`, on the same argument as [`Self::follow_link`]: a synthetic Tab
    /// does not reliably land focus inside an egui popup, so a command reachable *only* from
    /// the menu bar (`About hww` is the one) has no keyboard route the harness can drive and
    /// would otherwise be the one piece of chrome with no picture at all. It runs what the
    /// menu item runs, so the scene still photographs the real path.
    pub fn run_command(&mut self, ctx: &egui::Context, command: Command) {
        match command {
            Command::OpenLocation => self.open_url_bar(),
            Command::Reload => self.reload(self.opts),
            Command::ReloadBare => {
                self.reload(LoadOptions::BARE);
                self.flash(notice::RELOADED_BARE.to_owned());
            }
            Command::BookmarkPage => self.bookmark_page(),
            Command::OpenBookmarks => self.open_bookmarks(true),
            Command::ReadLater => self.read_focused_link_later(),
            Command::OpenReadingList => self.open_reading_list(true),
            Command::OpenHistory => self.open_history(true),
            Command::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Command::CopyPageUrl => {
                if let Some(u) = self
                    .shown()
                    .map(|r| r.loaded.prov.final_url.clone())
                    .or_else(|| self.history.current().cloned())
                {
                    self.copy(u.as_str());
                    self.flash("page URL copied".to_owned());
                }
            }
            Command::CopyFocusedLink => match self.focus_href_was.clone() {
                Some(h) => {
                    self.copy(&h);
                    self.flash(format!("copied {h}"));
                }
                None => self.flash("no link focused; Tab to one first".to_owned()),
            },
            Command::Find => self.open_find(),
            Command::OpenSettings => self.chrome.settings_open = true,
            Command::ZoomIn => {
                ctx.set_zoom_factor((ctx.zoom_factor() + 0.1).min(Settings::ZOOM_MAX))
            }
            Command::ZoomOut => {
                ctx.set_zoom_factor((ctx.zoom_factor() - 0.1).max(Settings::ZOOM_MIN))
            }
            Command::ZoomReset => ctx.set_zoom_factor(1.0),
            Command::Narrower => {
                self.settings.read.narrow();
                self.settings_dirty = true;
            }
            Command::Wider => {
                self.settings.read.widen();
                self.settings_dirty = true;
            }
            Command::SetTheme(i) => self.set_field(prefs::FieldId::Theme, i),
            Command::SetTypeface(i) => self.set_field(prefs::FieldId::Typeface, i),
            Command::SetImages(i) => self.set_field(prefs::FieldId::Images, i),
            Command::ToggleLinkAddresses => {
                self.settings.read.show_link_urls = !self.settings.read.show_link_urls;
                self.settings_dirty = true;
            }
            Command::ToggleOutline => self.chrome.outline_open = !self.chrome.outline_open,
            Command::TogglePageInfo => self.chrome.info_open = !self.chrome.info_open,
            Command::ToggleMenuBar => {
                self.settings.show_menu_bar = !self.settings.show_menu_bar;
                self.settings_dirty = true;
                if !self.settings.show_menu_bar {
                    self.flash("menu bar hidden; F10 brings it back".to_owned());
                }
            }
            Command::ToggleBookmarkSidebar => {
                self.settings.show_bookmark_sidebar = !self.settings.show_bookmark_sidebar;
                self.settings_dirty = true;
                // Only on the way *on*, and only when there is nothing to draw. The strip is an
                // icon column with no room for a sentence, so an empty one reads as a fault
                // rather than as an empty list; this is the one place with width for the answer.
                // `ToggleMenuBar` above flashes on the way off for the mirror-image reason.
                if self.settings.show_bookmark_sidebar && self.archive.is_empty() {
                    self.flash(notice::NO_BOOKMARKS_YET.to_owned());
                }
            }
            Command::CollapseAllReplies => self.collapse_all(),
            Command::Back => self.go_back(),
            Command::Forward => self.go_forward(),
            Command::ShowHelp => self.toggle_help(),
            Command::ShowAbout => {
                self.chrome.about_open = true;
                self.chrome.help_open = false;
            }
        }
    }

    /// Write one `prefs` field by index, and mark the settings dirty.
    fn set_field(&mut self, field: prefs::FieldId, index: usize) {
        prefs::set(&mut self.settings, field, prefs::Value::Index(index));
        self.settings_dirty = true;
    }

    /// The settings panel, and whatever it changed.
    fn settings_panel(&mut self, ctx: &egui::Context, pal: &theme::Palette) {
        if !self.chrome.settings_open {
            return;
        }
        let mut open = true;
        let events = prefs_ui::panel(ctx, pal, &mut self.settings, self.strip_height, &mut open);
        self.chrome.settings_open = open;
        for event in events {
            match event {
                prefs_ui::Event::Changed => self.settings_dirty = true,
                // Zoom is egui's, and `persist` copies the context's value *into* the settings
                // every frame. Writing the field alone would be overwritten before it took
                // effect, so the panel reports the number and this hands it to egui.
                prefs_ui::Event::ZoomSet(z) => ctx.set_zoom_factor(z),
                // The panel owns neither the store nor the file, and it is not the place that
                // knows whether the bookmarks is the page on screen. `rebuild_builtin_view` is a
                // no-op on any other page, so forgetting from a fetched page costs nothing and
                // forgetting while looking at the bookmarks redraws it rather than leaving rows
                // on screen for items that no longer exist.
                prefs_ui::Event::ForgetBookmarks => {
                    // Asked before the forget, because forgetting is what lifts it: either
                    // button is the reader saying they want this file replaced by what hww
                    // holds, which is the way out of an unreadable file that does not involve
                    // finding it by hand. A count would be the wrong sentence there — hww never
                    // read the file, so "already empty" is what it saw and not what happened.
                    let replaced = self.archive.refuses_to_be_written();
                    let n = self.archive.forget_all();
                    self.prune_icons();
                    if self.save_archive() {
                        self.flash(if replaced {
                            notice::UNREADABLE_FILE_REPLACED.to_owned()
                        } else {
                            notice::bookmarks_forgotten(n)
                        });
                    }
                    self.rebuild_builtin_view();
                }
                // The same three steps for the other two tenants, written out rather than folded
                // into one helper because each button says a different thing to the reader and
                // must go on being able to. The reading list's are a named function only because
                // its own view carries a second button that does the identical thing.
                prefs_ui::Event::ForgetReadingList => self.forget_reading_list(),
                prefs_ui::Event::ForgetHistory => {
                    let replaced = self.archive.refuses_to_be_written();
                    let n = self.archive.forget_visits();
                    if self.save_archive() {
                        self.flash(if replaced {
                            notice::UNREADABLE_FILE_REPLACED.to_owned()
                        } else {
                            notice::history_forgotten(n)
                        });
                    }
                    self.rebuild_builtin_view();
                }
            }
        }
    }

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
                    .inner_margin(egui::Margin::symmetric(10, 6)),
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
                    // Segmented: a `rule` hairline between the arrows, the address, and the
                    // right-hand group. `separator` in a horizontal is a vertical line in
                    // `noninteractive.bg_stroke`, which is `rule`: decoration, deliberately
                    // soft, because these divide one strip into parts rather than one thing
                    // from another.
                    // Pointer-reachable back/forward: a mouse-only reader must be able to
                    // return, and `Esc` may have hidden everything else. Arrows and not
                    // triangles: `◀`/`▶` are in none of the text faces `ui::fonts` embeds and
                    // would be drawn from the emoji face, in a different design from every
                    // label beside them; the arrows are in all of them.
                    // Every control on this strip is frameless, so every one of them takes
                    // its focus mark from `theme::focus_ring` rather than from egui's
                    // `active` visuals, which a frame is what paints.
                    let back = ui
                        .add_enabled(
                            self.history.can_go_back(),
                            egui::Button::new("←").frame(false),
                        )
                        .on_hover_text(menu::TIP_BACK);
                    theme::focus_ring(ui, &back, pal);
                    if back.clicked() {
                        action = Some(StripAction::Back);
                    }
                    let forward = ui
                        .add_enabled(
                            self.history.can_go_forward(),
                            egui::Button::new("→").frame(false),
                        )
                        .on_hover_text(menu::TIP_FORWARD);
                    theme::focus_ring(ui, &forward, pal);
                    if forward.clicked() {
                        action = Some(StripAction::Forward);
                    }
                    ui.separator();

                    // The URL itself summons the URL bar: the pointer's route to typing one.
                    let shown = self.status_url();
                    let address = ui
                        .add(egui::Button::new(RichText::new(shown).color(pal.fg)).frame(false))
                        .on_hover_text(menu::TIP_URL_BAR);
                    theme::focus_ring(ui, &address, pal);
                    if address.clicked() {
                        action = Some(StripAction::OpenUrlBar);
                    }

                    // Charter point 4: this line is on screen from before the request is sent
                    // until the navigation ends, whatever the fetch does.
                    if let Some(n) = &self.rewrite_notice {
                        ui.label(RichText::new(n).color(pal.notice_fg));
                    }
                    // The wrapper is what pins to the right edge; the icon alone would sit
                    // wherever the notices happened to end. What used to be here was seven
                    // facts `·`-joined into one dim truncated line, which crowded the URL and
                    // on a narrow window was not drawn at all. It is a panel now.
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if pageinfo_ui::icon(ui, pal, &self.settings.read).clicked() {
                            action = Some(StripAction::ToggleInfo);
                        }
                        ui.separator();
                        // The one hint the strip keeps: where the rest of the keys are.
                        let keys = ui
                            .add(
                                egui::Button::new(RichText::new("? keys").color(pal.dim))
                                    .frame(false),
                            )
                            .on_hover_text("Keyboard shortcuts (?)");
                        theme::focus_ring(ui, &keys, pal);
                        if keys.clicked() {
                            action = Some(StripAction::ToggleHelp);
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
            Some(StripAction::ToggleHelp) => self.toggle_help(),
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
        self.hover_height = 0.0;
        let Some(href) = self.hover_href.as_ref() else {
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
        let inner = egui::Area::new(egui::Id::new("hww-hover"))
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
        self.hover_height = inner.response.rect.height();
    }

    /// The transient status, as a pill above the strip, for `notice::TOAST_FOR` and not a frame
    /// longer: the repaint is requested for the moment it expires, so it leaves on its own
    /// rather than on the next input.
    fn toast(&mut self, ctx: &egui::Context, pal: &theme::Palette) {
        let Some((text, at)) = &self.status else {
            return;
        };
        let age = at.elapsed();
        if age >= notice::TOAST_FOR {
            self.status = None;
            return;
        }
        ctx.request_repaint_after(notice::TOAST_FOR - age);
        notice_ui::toast(
            ctx,
            pal,
            &self.settings.read,
            text,
            self.strip_height + self.hover_height,
        );
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
            Page::Idle => "no page".to_owned(),
        }
    }

    fn url_bar(&mut self, ui: &mut Ui, pal: &theme::Palette) {
        let Some(mut text) = self.chrome.url_bar.take() else {
            return;
        };
        let mut go = false;
        let mut keep = true;
        let bar =
            egui::Panel::top("hww-url")
                .show_separator_line(false)
                .frame(
                    egui::Frame::new()
                        .fill(pal.chrome_bg)
                        .inner_margin(egui::Margin::symmetric(10, 6)),
                )
                .show(ui, |ui| {
                    ui.style_mut().override_font_id = Some(theme::chrome_font(&self.settings.read));
                    ui.horizontal(|ui| {
                        let out = egui::TextEdit::singleline(&mut text)
                            .id(egui::Id::new(URL_BAR_ID))
                            .desired_width(f32::INFINITY)
                            .margin(egui::Margin::symmetric(8, 3))
                            .hint_text(notice::URL_BAR_HINT)
                            .show(ui);
                        let resp = out.response.response;
                        if self.focus_url_bar {
                            resp.request_focus();
                            self.focus_url_bar = false;
                            // Opening the bar offers the current address for replacement, the way
                            // every other browser does, so typing overwrites it and an arrow key
                            // still lands the cursor in it. The selection has to be written into
                            // stored state; `request_focus` alone leaves the caret where it was.
                            let mut state = out.state;
                            state.cursor.set_char_range(Some(
                                egui::text::CCursorRange::select_all(&out.galley),
                            ));
                            state.store(ui.ctx(), resp.id);
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
            // hww's own views round-trip through the bar, before anything is called a search.
            // `resolve_input` cannot recognise them and should not have to — `hww:bookmarks` is
            // not an http address, and `search` owns no view of hww's — while the bar is
            // prefilled with whatever the history holds, so without this the ordinary
            // open-the-bar-and-press-Enter gesture on a built view would ask somebody else's
            // search engine about hww's own internal address. The bar closes as it does on any
            // other success: `text` was taken at the top and only `keep` puts it back.
            if let Ok(url) = Url::parse(&typed)
                && builtin_view(&url).is_some()
            {
                self.navigate(url, self.opts, true);
                return;
            }
            // Words rather than an address are a search. `resolve_input` owns that decision and
            // the bare-host rule both, so this bar and `bin/hww`'s argument cannot drift apart.
            // The engine is named on screen before the request leaves; `navigate` does not
            // dispatch it here.
            let engine = crate::sites::engine_by_id(self.settings.search_engine);
            match crate::search::resolve_input(&typed, engine) {
                Ok(url) => self.navigate(url, self.opts, true),
                // Enter on a blank bar is a slip, and the bar opens blank whenever there is no
                // page. `resolve_input` refuses it, and there is nothing to name in a toast
                // about it, so the bar simply stays open.
                Err(_) if typed.is_empty() => keep = true,
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
        let bar =
            egui::Panel::top("hww-find")
                .show_separator_line(false)
                .frame(
                    egui::Frame::new()
                        .fill(pal.chrome_bg)
                        .inner_margin(egui::Margin::symmetric(10, 6)),
                )
                .show(ui, |ui| {
                    ui.style_mut().override_font_id = Some(theme::chrome_font(&self.settings.read));
                    ui.horizontal(|ui| {
                        ui.label("Find");
                        let out = egui::TextEdit::singleline(&mut query)
                            .id(egui::Id::new(FIND_ID))
                            .desired_width(240.0)
                            .margin(egui::Margin::symmetric(8, 3))
                            .show(ui);
                        let resp = out.response.response;
                        if self.focus_find {
                            resp.request_focus();
                            self.focus_find = false;
                            // The offer the URL bar makes, for the same reason: the find key reopens
                            // the bar over the last thing searched for, so hand it back selected.
                            // Typing then replaces it and Enter alone repeats the search, which is
                            // what every browser's find bar does. `.show` rather than `ui.add`
                            // because the selection has to be written into the stored state, and
                            // only `TextEditOutput` carries the state and the galley to write it
                            // from; `request_focus` alone leaves the caret where it was.
                            let mut state = out.state;
                            state.cursor.set_char_range(Some(
                                egui::text::CCursorRange::select_all(&out.galley),
                            ));
                            state.store(ui.ctx(), resp.id);
                        }
                        if resp.changed() {
                            next = Some(0);
                        }
                        // Bare Enter is TextEdit's return_key and surrenders focus; Shift+Enter is
                        // not, so previous has to fire while the field still holds it.
                        if total > 0
                            && ui.input(|i| i.key_pressed(Key::Enter))
                            && (resp.lost_focus() || resp.has_focus())
                        {
                            let prev = ui.input(|i| i.modifiers.shift);
                            next = Some(if prev {
                                (current + total - 1) % total
                            } else {
                                (current + 1) % total
                            });
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
                            RichText::new("Esc to dismiss")
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

    /// The bookmarks sidebar: one site mark per bookmark, down the left edge.
    ///
    /// Drawn from `archive.json` and not from the page, so unlike [`Self::outline_panel`] it
    /// needs no `shown()` and stays up across every navigation. That is also what makes its
    /// textures a different problem from the page's; see `ImageStore::clear_page`.
    ///
    /// **It precedes the page in the Tab cycle and there is no way around that**: egui takes a
    /// side panel's space before `CentralPanel`, and its focus order is registration order. What
    /// bounds the cost is that the strip is off until asked for and dismissed with one key,
    /// which is the same answer `show_menu_bar` gives to the same objection.
    ///
    /// It is not bounded by the band, and that was the first attempt: with a control only for the
    /// rows on screen a reader tabs past a screenful rather than past two thousand, and also
    /// cannot reach row thirty-one by any means, because a widget that is not drawn is not in
    /// egui's focus cycle and the strip has no scrollbar to say the rest are there. A pointer had
    /// the whole list and a keyboard had the top of it. So the band bounds what is *built* — the
    /// labels, the addresses, the requests — and `whole` widens the layout to every row on the
    /// frames a key is reaching through.
    ///
    /// **With no page on screen it draws what this machine already holds.** `OpenOn::Nothing`,
    /// and the frames of a launch before the first view commits, leave the reading column empty
    /// and the strip up, and a mark drawn then belongs to no page: `ReaderApp::mark_owner` gives
    /// it `net::ReqId::CHROME`, which no navigation makes stale, and `kept_mark` reads it off the
    /// disk. What it does *not* do there is fetch — see `mark_owner` for why a request with no
    /// page to report it in is one the reader could never read about — so a first run with an
    /// empty icon store still opens on empty squares, and fills them the moment there is a page
    /// to account for the requests.
    fn bookmark_sidebar(&mut self, ui: &mut Ui, pal: &theme::Palette) {
        if !self.settings.show_bookmark_sidebar {
            return;
        }
        let opts = self.settings.read.clone();
        // The same box the entries column uses; see `theme::mark_box`.
        let box_size = theme::mark_box(&opts);
        let pad = theme::snap(opts.base_size_pt * 0.5);
        // One row's height including the gap under it, which is what lets the window below be
        // arithmetic rather than a walk: every row in this strip is the same square.
        let step = box_size + pad;
        let count = self.archive.len();
        // The strip's own half of `lays_out_whole_page`, and it is here for that function's two
        // keyboard reasons rather than for its four page ones. **Tab**: egui moves focus among
        // the widgets of the frame the key arrived in, so a row the band left out is a row Tab
        // cannot reach — without this a reader with no pointer could walk the first screenful of
        // bookmarks and no further, with nothing on screen to say the rest were there.
        // **AccessKit**: the tree is built out of the widgets drawn this frame too, so a screen
        // reader was told the strip held a screenful whatever the archive said.
        //
        // The cost is bounded the way the page's is: it is the frames a key is reaching through,
        // not every frame, and what it builds is a button and a label per bookmark rather than a
        // document.
        let whole = self.tab_frames > 0 || accesskit_is_building(ui.ctx());
        let mut open = None;
        let mut hovered = None;
        let mut wanted: Vec<String> = Vec::new();
        let panel = egui::Panel::left("hww-bookmark-sidebar")
            // Not resizable, unlike the outline: that panel holds text a reader may want more
            // room for, and this one holds a square. There is one right width and dragging it
            // wider would only add margin.
            .resizable(false)
            .show_separator_line(false)
            .exact_size(box_size + pad * 2.0)
            .frame(
                egui::Frame::new()
                    .fill(pal.chrome_bg)
                    .inner_margin(egui::Margin::symmetric(pad as i8, pad as i8)),
            )
            .show(ui, |ui| {
                ui.style_mut().override_font_id = Some(theme::chrome_font(&opts));
                egui::ScrollArea::vertical()
                    .scroll_bar_visibility(
                        egui::containers::scroll_area::ScrollBarVisibility::AlwaysHidden,
                    )
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = pad;
                        // The window the strip will ask marks for, on the same terms the reading
                        // column asks for its own: touching the band is enough, and the margin
                        // either side is what makes a small scroll draw from what is already here.
                        let top = ui.next_widget_position().y;
                        let clip = ui.clip_rect();
                        let band = measure::Band::around(clip.top(), clip.bottom());
                        // Which rows those are, computed rather than discovered. The rows outside
                        // the window are not built at all — see `Archive::bookmark_marks` — so
                        // there is no row to ask `Band::skips` about, and every one of them is
                        // the same height, so there does not need to be.
                        let (first, last) = if whole {
                            (0, count)
                        } else {
                            band.rows(top, step, count)
                        };
                        // The space the rows above the window would have taken, in one
                        // allocation. Without it the strip is as long as its band and the
                        // scrollbar, the row positions, and the band itself all move as the
                        // reader scrolls.
                        if first > 0 {
                            ui.allocate_space(egui::vec2(box_size, first as f32 * step - pad));
                        }
                        for (url, label, icon) in self.archive.bookmark_marks(first, last - first) {
                            let resp = ui.add_sized(
                                egui::vec2(box_size, box_size),
                                egui::Button::new("").frame(false),
                            );
                            // The button carries no text, so this is the only name an assistive
                            // technology gets for it. The bookmark's own label, which is its
                            // title or its address; see `archive::Item::label`.
                            resp.widget_info(|| {
                                egui::WidgetInfo::labeled(egui::WidgetType::Link, true, &label)
                            });
                            // The strip scrolls itself to the row Tab just reached, exactly as
                            // the reading column does. `scroll_to_me` is answered by the scroll
                            // area the widget is inside, which here is the strip's own — the
                            // objection in `follow_focus` is about a chrome widget that is
                            // inside none and would have the *page* jump to a rect that is not
                            // on it. Without this a reader tabbing past the visible rows moves a
                            // focus ring they cannot see.
                            super::follow_focus(&resp);
                            theme::focus_ring(ui, &resp, pal);
                            let drawn = icon.as_deref().is_some_and(|src| {
                                super::images::paint_mark(ui, &self.images, src, resp.rect)
                            });
                            if !drawn {
                                // A slot with nothing in it yet. A dim outline rather than a
                                // glyph: the embedded faces carry no obvious placeholder mark,
                                // and an empty square still says "a row is here" to a pointer.
                                ui.painter().rect_stroke(
                                    resp.rect.shrink(1.0),
                                    theme::RADIUS,
                                    egui::Stroke::new(1.0, pal.guide),
                                    egui::StrokeKind::Inside,
                                );
                                // The same two questions `RenderCtx::note_icon` asks for the
                                // reading column's marks, in the same order and for the same
                                // reasons. **No state at all**, which is not what `drawn`
                                // answers: a mark that settled as `Failed` paints nothing, so
                                // asking `drawn` put it back into `wanted` and `icons_asked`
                                // is cleared on commit, which re-requested a host that had
                                // already said no once per navigation for as long as the strip
                                // was open. **Touching the band**, whatever `whole` widened the
                                // layout to: a Tab frame must not turn one keypress into a
                                // request for every mark in the archive.
                                if let Some(src) = icon
                                    && self.images.state(&src).is_none()
                                    && !band.skips(resp.rect.top(), box_size)
                                {
                                    // Moved, not cloned: `icon` is owned and is not read again.
                                    wanted.push(src);
                                }
                            }
                            if resp.clicked() {
                                open = Some(url.to_owned());
                            }
                            if resp.hovered() || resp.has_focus() {
                                hovered = Some(url.to_owned());
                            }
                        }
                        if last < count {
                            ui.allocate_space(egui::vec2(
                                box_size,
                                (count - last) as f32 * step - pad,
                            ));
                        }
                    });
            });
        panel_edge(ui, panel.response.rect, Side::Right, pal);
        // Written only when there is one, on the same terms `central` writes it: the strip is
        // the first surface of the frame to answer today, and a bare assignment would make it
        // the one that has to stay first.
        if hovered.is_some() {
            self.hover_href = hovered;
        }
        // The one door, so the strip gets the disk-first order, the policy gate, the
        // `icons_asked` dedupe and the throttle without restating any of them.
        self.load_site_icons(&wanted);
        if let Some(url) = open {
            self.follow_link(&url);
        }
    }

    fn outline_panel(&mut self, ui: &mut Ui, pal: &theme::Palette) {
        if !self.chrome.outline_open {
            return;
        }
        let Some(ready) = self.shown() else {
            return;
        };
        let entries: Vec<OutlineEntry> = ready.outline.clone();
        // The section the reader is in: the last heading at or above the block touching the
        // top of the window, which `ready_screen` found last frame.
        let current = self.top_block.and_then(|top| {
            entries
                .iter()
                .enumerate()
                .filter(|(_, e)| e.block <= top)
                .map(|(i, _)| i)
                .next_back()
        });
        let opts = self.settings.read.clone();
        let mut jump = None;
        let panel = egui::Panel::left("hww-outline")
            .resizable(true)
            // See `status_strip`. A resizable panel still shows its transient line while the
            // edge is hovered or dragged, which is the affordance rather than the separation.
            .show_separator_line(false)
            .default_size(240.0)
            .frame(
                egui::Frame::new()
                    .fill(pal.chrome_bg)
                    .inner_margin(egui::Margin::symmetric(8, 8)),
            )
            .show(ui, |ui| {
                ui.style_mut().override_font_id = Some(theme::chrome_font(&opts));
                ui.label(theme::label_job("Outline", &opts, pal.dim));
                ui.add_space(theme::snap(opts.base_size_pt * 0.3));
                if entries.is_empty() {
                    ui.label(RichText::new("no headings on this page").color(pal.dim));
                }
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for (i, e) in entries.iter().enumerate() {
                        let is_current = current == Some(i);
                        // The current entry is lifted onto the page ground and carries a 2px
                        // `guide` bar: the lift alone is 1.11:1 and would be missed.
                        let fill = if is_current {
                            pal.bg
                        } else {
                            egui::Color32::TRANSPARENT
                        };
                        let resp = egui::Frame::new()
                            .fill(fill)
                            .corner_radius(egui::CornerRadius {
                                nw: 0,
                                sw: 0,
                                ne: theme::RADIUS,
                                se: theme::RADIUS,
                            })
                            .inner_margin(egui::Margin::symmetric(8, 2))
                            .show(ui, |ui| {
                                ui.set_min_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    ui.add_space(f32::from(e.level - 1) * 12.0);
                                    let color = if is_current { pal.strong } else { pal.fg };
                                    let row = ui.add(
                                        egui::Button::new(RichText::new(&e.text).color(color))
                                            .frame(false),
                                    );
                                    theme::focus_ring(ui, &row, pal);
                                    if row.clicked() {
                                        jump = Some(e.block);
                                    }
                                });
                            })
                            .response;
                        if is_current {
                            let r = resp.rect;
                            ui.painter().rect_filled(
                                egui::Rect::from_min_max(
                                    r.left_top(),
                                    egui::pos2(r.left() + 2.0, r.bottom()),
                                ),
                                0.0,
                                pal.guide,
                            );
                        }
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
            Some(r) => pageinfo::rows(
                &r.loaded.prov,
                &self.images.counts(),
                r.rtl,
                r.arrival(),
                self.kept_state(),
            ),
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

    /// `?`, the menu item, and the strip's control all reach the help card, and all three go
    /// through here so the About card closes behind it.
    ///
    /// Both are `centred_card`, anchored `CENTER_CENTER`: opened together they stack, and the
    /// one underneath is unreachable rather than merely untidy.
    fn toggle_help(&mut self) {
        self.chrome.help_open = !self.chrome.help_open;
        if self.chrome.help_open {
            self.chrome.about_open = false;
        }
    }

    fn help_overlay(&mut self, ctx: &egui::Context, pal: &theme::Palette) {
        if !self.chrome.help_open {
            return;
        }
        let opts = self.settings.read.clone();
        centred_card(ctx, pal, &opts, "hww-help", |ui| {
            // The keys as keycaps (`notice_ui::keycap`), the one convention for
            // "press this" the reader has; the rest of the card is the list it was.
            egui::Grid::new("hww-help")
                .num_columns(2)
                .spacing([
                    theme::snap(opts.base_size_pt),
                    theme::snap(opts.base_size_pt * 0.45),
                ])
                .show(ui, |ui| {
                    for (keys, what) in menu::HELP {
                        ui.horizontal(|ui| notice_ui::keycaps(ui, pal, &opts, keys));
                        ui.label(*what);
                        ui.end_row();
                    }
                });
            ui.add_space(theme::snap(opts.base_size_pt * 0.3));
            ui.separator();
            // The sentence that used to open this — "Ctrl means Cmd on macOS" — is gone: the
            // specs above now say Cmd on a Mac, so there is nothing left to translate. What
            // remains is the one fact the table cannot carry, that the zoom is egui's own and
            // reaches further than the three keys listed. The wording is `menu::HELP_FOOTNOTE`
            // rather than a literal here, for the reason every other reader-facing string is
            // outside `ui/`.
            ui.label(
                RichText::new(menu::HELP_FOOTNOTE)
                    .color(pal.dim)
                    .font(theme::chrome_font(&opts)),
            );
        });
    }

    /// Help, About hww: [`menu::ABOUT`], all of it.
    ///
    /// It was a `flash` of `ABOUT[0]` alone, which put the one uncontroversial sentence in a
    /// six-second pill and left the two that say what the program actually refuses to do
    /// unreachable from the running reader. They are `pub const`, so nothing warned. A card
    /// rather than a longer toast, because this much is reading rather than status, and
    /// the reader already has a floating surface shaped for it.
    fn about_overlay(&mut self, ctx: &egui::Context, pal: &theme::Palette) {
        if !self.chrome.about_open {
            return;
        }
        let opts = self.settings.read.clone();
        centred_card(ctx, pal, &opts, "hww-about", |ui| {
            ui.set_max_width(theme::snap(opts.base_size_pt * 26.0));
            ui.label(theme::label_job("About hww", &opts, pal.dim));
            ui.add_space(theme::snap(opts.base_size_pt * 0.5));
            for line in menu::ABOUT {
                ui.label(
                    RichText::new(*line)
                        .color(pal.fg)
                        .font(theme::chrome_font(&opts)),
                );
                ui.add_space(theme::snap(opts.base_size_pt * 0.4));
            }
            ui.separator();
            ui.label(
                RichText::new("Escape closes this.")
                    .color(pal.dim)
                    .font(theme::chrome_font(&opts)),
            );
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
                    // One geometry, every consumer. Computing the measure twice is how the error
                    // page and the column would come to disagree about a left edge. The outline
                    // panel takes width from the column, so the measure is a ceiling rather
                    // than a promise; without the clamp inside `Column` the text is simply cut
                    // off on the right when the panel opens.
                    let column = notice_ui::Column::new(ui, wanted);

                    // The states with no page under them are the reader's: the splash, the
                    // loading line, the error page. Everything else is the page.
                    self.empty_states(ui, pal, column);

                    if self.shown().is_some() {
                        ui.horizontal_top(|ui| {
                            ui.add_space(column.gutter);
                            ui.vertical(|ui| {
                                ui.set_max_width(column.measure);
                                ui.set_min_width(column.measure);
                                ui.add_space(theme::snap(
                                    theme::line_height_px(&self.settings.read) * 1.5,
                                ));
                                self.ready_screen(ui, pal);
                            });
                        });
                    }
                });
                self.viewport_height = out.inner_rect.height().max(1.0);
                self.max_scroll = (out.content_size.y - out.inner_rect.height()).max(0.0);
                // Every frame, not once at dispatch: the outgoing page stays visible and
                // scrollable for the length of a fetch, so where it was left is not known until
                // the last frame it is on screen. `out.state` is what the scroll area settled
                // on after clamping, which is where the page is rather than where it was asked
                // to go.
                if let Some(id) = self.shown_entry {
                    self.history.set_offset(id, out.state.offset.y);
                }
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

    /// Every remark about the page on screen, as infobars docked under the top chrome.
    ///
    /// "On screen" rather than "current" is load-bearing during a navigation: the cautions belong
    /// to the article being read, so a truncated body or a dead rewrite rule must not blink out
    /// the moment a link is clicked and reappear when the next page lands. The rewrite bar comes
    /// first (it is about how the page was asked for), then the cautions loudest first, which is
    /// the order `about_page` chose, then the RTL caution.
    fn notice_bars(&mut self, ui: &mut Ui, pal: &theme::Palette) {
        // Owned, so the borrow of `self.page` ends here rather than running through the draw
        // loop and colliding with the `&mut self` that acting on a button needs.
        let notices: Vec<Notice> = match self.shown() {
            Some(r) => {
                let mut n: Vec<Notice> = notice::rewrite(&r.loaded.prov).into_iter().collect();
                n.extend(notice::about_page(&r.loaded.prov, r.text_len, r.arrival()));
                n.extend(r.rtl.then(notice::rtl_notice));
                n
            }
            None => Vec::new(),
        };
        let mut pressed = None;
        for (i, n) in notices.iter().enumerate() {
            if self.chrome.dismissed.contains(n.key()) {
                continue;
            }
            let event = notice_ui::bar(ui, pal, &self.settings.read, n, i);
            panel_edge(ui, event.rect, Side::Bottom, pal);
            if event.dismissed {
                self.chrome.dismissed.insert(n.key().to_owned());
            }
            if event.pressed.is_some() {
                pressed = event.pressed;
            }
        }
        // Acting is deferred past the borrow, the same shape `status_strip` uses for
        // `StripAction`: widgets record an intent, `app` acts on it once per frame. The only
        // action a bar over a loaded page offers is the rewrite bar's.
        match pressed {
            Some(Button::RetryWithoutRewrite) => self.reload(LoadOptions {
                rewrite: false,
                ..self.opts
            }),
            Some(Button::Retry) => self.reload(self.opts),
            Some(Button::OpenSettings) => self.chrome.settings_open = true,
            Some(Button::CopyUrl) | None => {}
        }
    }

    /// The states with no page under them: the splash, the loading line, and the error page.
    ///
    /// Drawn in the central panel's scroll area, in the reading column's geometry, and nowhere
    /// else: each owns the viewport, which is what tells it apart from a bar over a page.
    fn empty_states(&mut self, ui: &mut Ui, pal: &theme::Palette, column: notice_ui::Column) {
        let opts = self.settings.read.clone();
        let pressed = match &self.page {
            Page::Idle => {
                notice_ui::splash(ui, pal, &opts, column, &notice::idle());
                None
            }
            Page::Loading {
                url,
                started,
                from: None,
                ..
            } => {
                if let Some(line) = notice::loading(url, started.elapsed(), false) {
                    notice_ui::loading_line(ui, pal, &opts, &line);
                }
                None
            }
            Page::Failed { url, error, .. } => {
                let n = notice::failure(error, url);
                notice_ui::error_page(ui, pal, &opts, column, &n)
            }
            Page::Loading { from: Some(_), .. } | Page::Ready(_) => None,
        };
        let (Some(b), Page::Failed { url, opts, .. }) = (pressed, &self.page) else {
            return;
        };
        let (url, opts) = (url.to_string(), *opts);
        match b {
            Button::Retry => self.reload(opts),
            Button::RetryWithoutRewrite => self.reload(LoadOptions {
                rewrite: false,
                ..opts
            }),
            Button::OpenSettings => self.chrome.settings_open = true,
            Button::CopyUrl => {
                self.copy(&url);
                self.flash("page URL copied".to_owned());
            }
        }
    }

    /// A find query is being drawn: the bar is open and the query is not empty.
    fn find_open(&self) -> bool {
        self.chrome.find.as_deref().is_some_and(|q| !q.is_empty())
    }

    /// Whether this frame has to lay the page out whole, skipping nothing.
    ///
    /// Skipping a block off the window is invisible to the eye and is not invisible to
    /// everything that walks the page by *widget*. Each of these is a case where a block
    /// nowhere near the window still has to be one of this frame's widgets:
    ///
    /// - Find: `find_total` counts matches across the whole document, and `n` scrolls to one
    ///   that is by definition somewhere the reader is not looking.
    /// - Tab: egui moves focus among the widgets of the frame the key arrived in, so the link
    ///   Tab is reaching for has to be drawn on it, and on the frame after as well; see
    ///   `tab_frames`.
    /// - Focus already on the page: a focused widget that stops being drawn loses focus, and
    ///   `Enter`, `Y`, `i`, and `z` all read what Tab last landed on. `focus_was_on_page` and
    ///   not the four fields, which this is downstream of the clearing of.
    /// - A selection: egui builds the copy buffer out of the labels drawn this frame, so a
    ///   selection running off the window would put only the visible part on the clipboard.
    /// - A jump (`scroll_to_block`): it scrolls to a `Response`, which a block replaced by
    ///   empty space does not have.
    /// - A screen reader: egui builds the AccessKit tree out of the widgets drawn this frame
    ///   too, and a block replaced by empty space contributes no text to it. The eye loses
    ///   nothing to skipping and an assistive technology would lose the page, so the whole
    ///   optimization turns off for as long as one is attached.
    fn lays_out_whole_page(&self, ui: &Ui) -> bool {
        self.find_open()
            || self.scroll_to_block.is_some()
            // Focus on the page needs its block laid out, not the page: `focus_block_was`
            // exempts that one from skipping below. Only a focus whose block is not known
            // (a control outside the block loop) still costs the whole page.
            || (self.focus_was_on_page && self.focus_block_was.is_none())
            || self.tab_frames > 0
            || ui
                .ctx()
                .plugin_opt::<egui::text_selection::LabelSelectionState>()
                .is_some_and(|p| p.lock().has_selection())
            || accesskit_is_building(ui.ctx())
    }

    /// Draw the page the reading column is showing, which during a navigation is the outgoing
    /// one rather than the one being fetched.
    fn ready_screen(&mut self, ui: &mut Ui, pal: &theme::Palette) {
        // The page is behind `&mut self`, and rendering needs both it and the image store, so
        // take it for the duration and put it back. Cheaper than cloning a document.
        let Some(ready) = self.page.take_shown() else {
            unreachable!("checked by the caller")
        };
        // Both before `RenderCtx`, which borrows `self.images` for the rest of the function.
        let whole = self.lays_out_whole_page(ui);
        self.heights.under(&measure::Layout {
            opts: self.settings.read.clone(),
            measure: ui.max_rect().width(),
            pixels_per_point: ui.ctx().pixels_per_point(),
            find: self.find_open(),
            pending_link: self.pending_link.map(|id| id.value()),
            collapsed: self.collapsed_changes,
        });
        // Before the map is read, because the reader may have moved the lead-in budget since the
        // page committed and the blocks it names are then the wrong ones. Cheap to compare and
        // rare to rebuild: this is a walk of one document, on the frames a setting changed.
        if self.image_blocks_budget != self.settings.read.entry_summary_bytes {
            self.image_blocks_budget = self.settings.read.entry_summary_bytes;
            self.image_blocks = image_blocks(&ready.loaded.doc, self.image_blocks_budget);
        }
        // After `under`, which may have cleared the table wholesale, and before anything is
        // drawn from it. A picture that arrived, failed, was dropped, or was evicted since the
        // last frame re-sizes the block it sits in and leaves every other block alone; this is
        // what keeps that local, instead of making one arrival cost the whole page.
        for src in self.images.take_changed() {
            for i in self.image_blocks.get(&src).into_iter().flatten() {
                self.heights.forget(*i);
            }
        }
        // Only worth gathering under the policy that uses it. `ReaderApp::autoload` asks the
        // same question again and is the enforcement; this one skips the work.
        //
        // `current == req` is "nothing newer is loading". Planning pictures for the outgoing
        // page while a navigation is in flight would queue jobs the pool is about to discard,
        // and each discard hands the `src` back to be planned again on the next frame.
        let auto =
            self.settings.read.images.loads_automatically() && self.current == Some(ready.req);
        let base = ready.loaded.prov.final_url.clone();
        // A second handle, because `RenderCtx` takes ownership of the first and the automatic
        // policy needs the same base to resolve a `src` against after the column is drawn.
        let auto_base = base.clone();
        // Read before `base` is moved into the context, and through `builtin_view`, which matches
        // one exact address: the controls this turns on act on `reader::archive`, so no fetched
        // page may be drawn holding them.
        let reading_list = builtin_view(&base) == Some(View::ReadingList);
        let mut ctx = RenderCtx::new(
            // The palette `theme::apply` returned at the top of the frame and every other panel
            // was handed. Building a second one here asked the desktop for its appearance twice
            // a frame and filled in a second copy of twenty-odd colours to reach the same
            // answer.
            *pal,
            &self.settings.read,
            base,
            &mut self.images,
            &self.collapsed,
            &ready.threads,
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
        ctx.pending_href = self.pending_href.clone();
        ctx.comment_heights = self.heights.take_comments();
        ctx.entry_heights = self.heights.take_entries();
        ctx.reading_list = reading_list;

        let doc = &ready.loaded.doc;
        blocks::document_header(ui, doc, &mut ctx);
        let scroll_to = self.scroll_to_block.take();
        let clip_top = ui.clip_rect().top();
        let band = measure::Band::around(clip_top, ui.clip_rect().bottom());
        // Unconditionally, unlike `ctx.band`, which is cleared for any block laid out whole:
        // "may this block be skipped" and "is this picture near the window" are different
        // questions, and only the first one is allowed to have no answer.
        ctx.autoload_band = auto.then_some(band);
        // Unconditionally, unlike the line above: a site mark is fetched under every policy but
        // `NoRequests`, exactly as the masthead favicon is, and `RenderCtx::note_icon` is where
        // that question is asked.
        ctx.icon_band = Some(band);
        self.top_block = None;
        for (i, b) in doc.blocks.iter().enumerate() {
            // The article's <h1> is usually also its <title>, and its "By Name" is already
            // in the strip; rendering both shows each twice.
            if ready.masthead.skips(i) {
                continue;
            }
            // Empty space of the height this block measured last time, for as long as it is
            // clear of the window. `allocate_space` and not `add_space`, so the item spacing
            // either side of it is the spacing the block itself would have had.
            let may_skip = !whole && self.focus_block_was != Some(i);
            let y = ui.next_widget_position().y;
            if may_skip
                && let Some(h) = self.heights.get(i)
                && band.skips(y, h)
            {
                if self.top_block.is_none() && y + h >= clip_top {
                    self.top_block = Some(i);
                }
                ui.allocate_space(egui::vec2(0.0, h));
                continue;
            }
            ctx.block = Some(i);
            ctx.band = may_skip.then_some(band);
            let had_focus = ctx.focus_href.is_some()
                || ctx.focus_image.is_some()
                || ctx.focus_comment.is_some();
            let resp = ui.scope(|ui| blocks::block_ui(ui, b, &mut ctx)).response;
            if !had_focus
                && (ctx.focus_href.is_some()
                    || ctx.focus_image.is_some()
                    || ctx.focus_comment.is_some())
            {
                self.focus_block = Some(i);
            }
            self.heights.set(i, resp.rect.height());
            if self.top_block.is_none() && resp.rect.bottom() >= clip_top {
                self.top_block = Some(i);
            }
            if scroll_to == Some(i) {
                resp.scroll_to_me(Some(Align::TOP));
            }
        }
        ui.add_space(theme::snap(
            theme::line_height_px(&self.settings.read) * 4.0,
        ));

        self.heights
            .restore_comments(std::mem::take(&mut ctx.comment_heights));
        self.heights
            .restore_entries(std::mem::take(&mut ctx.entry_heights));
        // Gathered where each picture drew, at every nesting depth, so a feed card or a comment
        // is judged by where *it* landed and not by the one block that holds the lot.
        let in_band = std::mem::take(&mut ctx.autoload_srcs);
        let marks = std::mem::take(&mut ctx.icon_srcs);
        self.find_total = ctx.find_seen;
        self.find_scroll = false;
        self.focused_href = ctx.focus_href.clone();
        self.focused_text = ctx.focus_text.clone();
        self.focused_image = ctx.focus_image.clone();
        self.focused_comment = ctx.focus_comment.clone();
        self.focused_other = ctx.focus_other;
        // Assigned only when the page has one, so a chrome surface that drew earlier in the
        // frame keeps its own: see the field. The page still wins where both would answer,
        // which is a case the pointer cannot actually produce.
        if ctx.hover_href.is_some() {
            self.hover_href = ctx.hover_href.clone();
        }
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
        // Last, and only if this frame did not start a navigation: a click that is leaving the
        // page has just made every picture on it something nobody is going to look at.
        // Before the article images and under the same "not if this frame is leaving" rule:
        // the marks are what the reader is looking at while a list page settles, and a click
        // that navigates has just made every request queued for the old page pointless.
        if self.current == before && !marks.is_empty() {
            self.load_site_icons(&marks);
        }
        if auto && self.current == before && !in_band.is_empty() {
            self.autoload(ui.ctx(), &auto_base, &in_band);
        }
    }

    fn apply(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::Follow(href) => self.follow(&href, self.opts),
            Action::FollowBare(href) => self.follow(&href, LoadOptions::BARE),
            Action::Copy(text) => {
                self.copy(&text);
                self.flash(format!("copied {text}"));
            }
            Action::ReadLater { href, title } => self.read_later(&href, &title),
            Action::ForgetNext(href) => self.forget_next_link(&href),
            Action::ForgetAllNext => self.forget_reading_list(),
            Action::LoadImage(src) => self.load_image(ctx, &src),
            Action::GoToBlock(b) => self.scroll_to_block = Some(b),
            Action::ToggleComment(key) => self.toggle_collapsed(key),
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
/// Top chrome (the URL and find bars, the notice bars, the outline) takes `chrome_bg` and this
/// edge; the status strip stays on `bg`. A floating surface takes `chrome_bg`, a `guide` stroke,
/// and the radius, and is not drawn here.
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
    ToggleHelp,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The strip's marks are not dropped for a page it never belonged to.
    ///
    /// Both halves of one rule, and the reason it is a function rather than a line at three
    /// match arms. A reply for the page on screen is wanted and one for the page before it is
    /// not, which is what keeps a texture off a document nobody is reading. A reply the sidebar
    /// asked for outlives every page and is answered by the store instead: still pending means
    /// the strip is still waiting for it, and a commit — which drops every pending mark — is
    /// what makes it stale. Neither direction is visible in a screenshot: too strict is a column
    /// of empty squares that never fills, too loose is the same bytes inserted twice.
    #[test]
    fn a_reply_belongs_to_what_asked_for_it() {
        let page = ReqId::for_test(7);
        let older = ReqId::for_test(6);
        assert!(reply_is_wanted(page, Some(page), true));
        assert!(
            !reply_is_wanted(older, Some(page), true),
            "a picture for the page the reader left is not wanted, pending or not"
        );
        assert!(
            reply_is_wanted(ReqId::CHROME, None, true),
            "the sidebar draws with no page under it, which is the whole point of the owner"
        );
        assert!(
            reply_is_wanted(ReqId::CHROME, Some(page), true),
            "and keeps waiting for it once a page arrives"
        );
        assert!(
            !reply_is_wanted(ReqId::CHROME, Some(page), false),
            "a commit dropped the pending mark, so the row is asked for again with an owner"
        );
    }

    /// Only a list's mark may be drawn with no page, and `mark_owner` is where that is decided.
    ///
    /// `Mark::Read` is a page's own favicon and there is no page, so there is nothing to draw it
    /// beside; `Mark::No` is an article picture, which is a page's by definition. Reachable only
    /// through this function, so a later caller cannot invent a page-free owner for either.
    #[test]
    fn only_a_list_mark_is_owned_by_the_chrome() {
        assert_eq!(chrome_owner(Mark::Kept), Some(ReqId::CHROME));
        assert_eq!(
            chrome_owner(Mark::Read),
            None,
            "a page's favicon needs its page"
        );
        assert_eq!(
            chrome_owner(Mark::No),
            None,
            "an article picture is a page's"
        );
    }

    /// A site mark of either kind carries no Referer, whatever the reader's setting says.
    ///
    /// Under `ui/` on the rule's own terms: it needs no `egui::Context`, and its failure is
    /// invisible everywhere else. Nothing on screen changes, no test of the panel moves — the
    /// count and the row would agree with each other and both be one line about a header the
    /// reader never asked to send. What it would cost is a bookmarked host's icon server
    /// learning, on every navigation with the strip open, which page its reader is on.
    #[test]
    fn a_site_mark_never_carries_a_referer() {
        assert!(!sends_referer(true, Mark::Kept), "the strip must send none");
        assert!(
            !sends_referer(true, Mark::Read),
            "and a page's own favicon must be indistinguishable from one a bookmark names, or \
             the header that is missing tells the host which of its pages the reader kept"
        );
        assert!(
            sends_referer(true, Mark::No),
            "an article picture is unchanged"
        );
        for mark in [Mark::No, Mark::Read, Mark::Kept] {
            assert!(
                !sends_referer(false, mark),
                "the setting still turns it off"
            );
        }
    }

    /// This test belongs under `ui/` because [`APP_ID`] is handed directly to eframe. It needs
    /// no `egui::Context`, and its failure is otherwise invisible to Rust: Ubuntu matches a
    /// running Wayland window to its dock icon through these strings in the installed desktop
    /// entry. A typo compiles, installs, and leaves the application with a generic icon.
    #[test]
    fn desktop_entry_matches_the_application_id() {
        let desktop = include_str!("../../../packaging/hww.desktop");
        assert_eq!(desktop.lines().next(), Some("[Desktop Entry]"));

        let field = |key: &str| {
            let prefix = format!("{key}=");
            let values: Vec<_> = desktop
                .lines()
                .filter_map(|line| line.strip_prefix(&prefix))
                .collect();
            assert_eq!(values.len(), 1, "{key} must occur exactly once");
            values[0]
        };

        assert_eq!(field("Type"), "Application");
        assert_eq!(field("Icon"), APP_ID);
        assert_eq!(field("StartupWMClass"), APP_ID);
        assert_eq!(field("Exec").split_ascii_whitespace().next(), Some(APP_ID));

        // The door. Without this line hww appears in no other application's "Open with…" menu,
        // and the only ways in are a terminal and a blank window. `%u` in `Exec` above is the
        // other half: the handler is passed the URL as an argument.
        //
        // Schemes only, and no `text/html`. hww cannot open a local file yet, and a desktop
        // entry claiming a type the application fails on is the quiet lie the notice system
        // exists to refuse. It arrives with local Markdown.
        let types: Vec<&str> = field("MimeType")
            .split_terminator(';')
            .filter(|t| !t.is_empty())
            .collect();
        assert_eq!(types, ["x-scheme-handler/http", "x-scheme-handler/https"]);
        assert!(
            field("MimeType").ends_with(';'),
            "a desktop-entry list is terminated by its separator, not joined by it"
        );
    }

    /// The chokepoint, and the argument for testing it here rather than citing precedent.
    ///
    /// [`builtin_view`] takes a `&Url` and returns an `Option`, with no `egui` anywhere; it lives
    /// under `ui/` only because `navigate` does. Its failure is the kind that hides: a match too
    /// loose by one character would divert a real address from a real fetch and the reader would
    /// be shown their bookmarks instead of the page they asked for, with nothing anywhere saying
    /// why. A match too tight hands `hww:bookmarks` to reqwest, which is the Back-out-of-the-
    /// bookmarks bug this function exists to close.
    #[test]
    fn builtin_view_claims_one_address_and_diverts_no_fetch() {
        let u = |s: &str| Url::parse(s).expect("a fixture URL parses");
        assert_eq!(builtin_view(&u(archive::BOOKMARKS)), Some(View::Bookmarks));
        assert_eq!(builtin_view(&u(archive::HISTORY)), Some(View::History));
        assert_eq!(
            builtin_view(&u(archive::READING_LIST)),
            Some(View::ReadingList)
        );
        // Three addresses, three distinct strings. Two of them differing would send one view's
        // key to the other's page, and the `match` above would compile either way.
        assert_ne!(archive::BOOKMARKS, archive::HISTORY);
        assert_ne!(archive::BOOKMARKS, archive::READING_LIST);
        assert_ne!(archive::HISTORY, archive::READING_LIST);
        for other in [
            "https://example.com/",
            "https://example.com/bookmarks",
            "http://bookmarks/",
            "https://hww.example.com/bookmarks",
            // Near misses on the scheme's own side.
            "hww:bookmarks/",
            "hww:bookmark",
            "hww:bookmarksy",
            "hww:history/",
            "hww:histor",
            "hww:historyy",
            "hww:reading-list/",
            "hww:reading-lis",
            "hww:reading-listt",
            "hww:readinglist",
            "hww:reading_list",
            "hww:other",
        ] {
            assert_eq!(
                builtin_view(&u(other)),
                None,
                "{other} was diverted from a fetch"
            );
        }
    }

    /// This test belongs under `ui/` because the transition it checks lives here, but its
    /// argument stands on its own rather than relying on another test as precedent.
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
        let (text_len, rtl, threads) = derived(&doc);
        Box::new(Ready {
            masthead: title::masthead(&doc),
            outline: outline::build(&doc.blocks),
            text_len,
            rtl,
            threads,
            loaded: Loaded {
                doc,
                prov: crate::session::Provenance::fixture(url.as_str()),
            },
            req: ReqId::for_test(req),
            opts: LoadOptions::default(),
            fetched: Instant::now(),
            restored: false,
            built: false,
        })
    }

    /// The fourth, and the argument again rather than the precedent.
    ///
    /// [`image_blocks`] and [`block_image_srcs`] are `ir::Document` in and a map out, with no
    /// `egui` anywhere; they live here only because their caller does. Their failure is the
    /// kind that hides: the index feeds `Heights::forget`, so a `src` mapped to the wrong block
    /// forgets a block that did not change and keeps one that did, and what a reader sees is a
    /// page that shifts by a few points under them when a picture lands somewhere else. A
    /// screenshot of either frame is correct, and nothing else in this file would notice.
    #[test]
    fn every_picture_maps_to_the_blocks_that_contain_it() {
        let url = Url::parse("https://example.com/a").expect("a fixture URL parses");
        let doc = crate::html::extract(
            "<article><h1>Title</h1>\
             <p>One <img src=\"/logo.png\" alt=\"a\"> and <img src=\"/logo.png\" alt=\"b\">.</p>\
             <p>Plain words.</p>\
             <figure><img src=\"/photo.jpg\" alt=\"c\"></figure>\
             <p>A repeat of <img src=\"/logo.png\" alt=\"d\">.</p></article>",
            &url,
        );
        // Zero: this document is an article and carries no entries, so the budget has nothing
        // to cut, and pinning it makes that explicit rather than incidental.
        let map = image_blocks(&doc, 0);
        // The extractor absolutises every `src` against the page, which is why
        // `autoload::plan` resolves rather than assumes and why these are spelled out.
        let logo = map
            .get("https://example.com/logo.png")
            .expect("the logo is on the page");
        let photo = map
            .get("https://example.com/photo.jpg")
            .expect("the photo is on the page");
        assert_eq!(
            logo.len(),
            2,
            "twice in one block is one entry, in two is two"
        );
        assert_eq!(photo.len(), 1);
        assert_ne!(logo[0], photo[0], "a shared block index");
        // Every index names a block that really carries the `src`, which is the half a
        // hand-written map gets wrong.
        for (src, blocks) in &map {
            for i in blocks {
                assert!(
                    block_image_srcs(&doc.blocks[*i], 0).contains(src),
                    "block {i} does not contain {src}"
                );
            }
        }
        // And nothing on the page is missing from it.
        for src in collect_image_srcs(&doc, 0) {
            assert!(map.contains_key(&src), "{src} is in no block");
        }
    }

    /// The image walk stops where the draw stops.
    ///
    /// Same argument as the test above, sharpened: an entry's summary is cut at the draw, and a
    /// walk that read it whole would put every picture in the cut tail into the load-images
    /// disclosure, into the page-info count, and onto the wire. That is a third-party request
    /// for something the reader cannot see, and no screenshot shows it — the picture never
    /// draws, which is the whole point. `budget::lead_in` is the one place that decides, and
    /// `blocks::entries_ui` asks the same function.
    #[test]
    fn the_image_walk_stops_where_the_entry_is_cut() {
        let para = |n: usize| ir::Block::Paragraph(vec![ir::Inline::Text("x".repeat(n))]);
        let pic = |src: &str| ir::Block::Figure {
            image: ir::Image {
                src: src.to_owned(),
                alt: None,
            },
            caption: None,
        };
        let doc = ir::Document {
            url: "https://example.com/feed.xml".to_owned(),
            title: None,
            byline: None,
            published: None,
            site_name: None,
            favicon: None,
            lang: None,
            blocks: vec![ir::Block::Entries(vec![ir::Entry {
                title: vec![ir::Inline::Text("One".to_owned())],
                href: None,
                summary: vec![
                    pic("https://example.com/shown.png"),
                    para(500),
                    pic("https://example.com/cut.png"),
                    para(500),
                ],
                published: None,
                address: None,
                icon: None,
                image: None,
            }])],
        };
        // The budget is spent by the first paragraph, so the second picture never draws.
        let shown = collect_image_srcs(&doc, 100);
        assert_eq!(shown, ["https://example.com/shown.png"]);
        // With the budget off, both are on the page and both are offered.
        assert_eq!(collect_image_srcs(&doc, 0).len(), 2);
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
            opts: LoadOptions::BARE,
        };
        assert!(failed.take_shown().is_none());
    }
}
