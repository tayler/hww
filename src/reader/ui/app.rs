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
//! page-info panel now (`p`, or the italic `i`, `reader::ui::pageinfo_ui`). The two that could
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

use crate::ir;
use crate::reader::autoload;
use crate::reader::desktop;
use crate::reader::history::{EntryId, History};
use crate::reader::image_decode;
use crate::reader::measure::{self, Heights};
use crate::reader::menu::{self, Command};
use crate::reader::notice::{
    self, Button, IMAGES_ALL_FAILED, IMAGES_ARE_DECLINED, IMAGES_ARE_DECLINED_OR_FAILED,
    IMAGES_ARE_OFF, Notice,
};
use crate::reader::outline::{self, OutlineEntry};
use crate::reader::pagecache::{PageCache, stash_under};
use crate::reader::pageinfo;
use crate::reader::prefs;
use crate::reader::settings::{self, Settings};
use crate::reader::thread_tree::CommentKey;
use crate::reader::title;
use crate::reader::ui::images::{Failure, ImageStore};
use crate::reader::ui::{
    Action, Launch, RenderCtx, blocks, fonts, menu_ui, net, notice_ui, pageinfo_ui, prefs_ui, theme,
};
use crate::session::{self, LoadError, LoadOptions, Loaded, Rewrite, Target};
use eframe::egui::{self, Align, Key, Layout, Modifiers, RichText, Ui};
use net::{Job, Msg, Net, ReqId};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use url::Url;

const URL_BAR_ID: &str = "hww-url-bar";
const FIND_ID: &str = "hww-find";
const APP_ID: &str = "hww";

pub fn run(launch: Launch) -> eframe::Result {
    let icon =
        eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/logo/hww-256.png"))
            .expect("embedded hww icon is valid PNG");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("hww")
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
    /// that would put a `Shift+R` page under the default options and hand it back on an ordinary
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
}

impl Ready {
    /// How this page got into the column, for `pageinfo::rows`.
    ///
    /// Here rather than at the panel so the two fields above are read in one place: the age is
    /// the *fetch*'s, so a page put back twice still reports how old the document is.
    fn arrival(&self) -> pageinfo::Arrival {
        if self.restored {
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
    hover_href: Option<String>,
    /// Last frame's status-strip height, so the hover overlay can sit on its top edge.
    strip_height: f32,
    /// This frame's hover-URL overlay height, measured in [`Self::hover_strip`] before the
    /// toast is drawn, so a wrapping address lifts the pill by what was actually painted.
    hover_height: f32,
    chrome: Chrome,
    images: ImageStore,
    /// Which blocks of the page on screen contain each image `src`. Built once when it
    /// commits; see [`image_blocks`].
    image_blocks: HashMap<String, Vec<usize>>,
    /// How many times `ImagePolicy::Auto` has asked for each `src` on the page on screen.
    ///
    /// A count and not a set, because the LRU can drop a picture that is still on the page:
    /// one refill is right and an unbounded number is a request loop. `autoload::MAX_ATTEMPTS`
    /// holds the reasoning, and `autoload::plan` the other half of the pair.
    auto_attempts: HashMap<String, u32>,
    /// How tall each block was the last time it was laid out, so the blocks off the window can
    /// be skipped. See `reader::measure` for why, and `ready_screen` for when it is ignored.
    heights: Heights,
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
    /// The options of the navigation in flight, which `Shift+R` makes differ from the default.
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
    /// One-shot: the same for the find field. Focusing it from the keymap instead would hand
    /// the field the `/` that opened it: `consume_key` takes the key event, but the text event
    /// that arrives with it is a separate event, and a field focused earlier in the same frame
    /// still reads it.
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
        let mut app = Self {
            net,
            settings: launch.settings,
            settings_dirty: false,
            last_saved: Instant::now(),
            history: History::new(),
            pages: PageCache::new(),
            page: Page::Idle,
            current: None,
            rewrite_notice: None,
            status: launch.settings_note.map(|n| (n, Instant::now())),
            hover_href: None,
            strip_height: 0.0,
            hover_height: 0.0,
            chrome: Chrome::default(),
            images: ImageStore::default(),
            image_blocks: HashMap::new(),
            auto_attempts: HashMap::new(),
            heights: Heights::default(),
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
            focused_image: None,
            focused_comment: None,
            focused_other: false,
            focus_was_on_page: false,
            focus_href_was: None,
            focus_block: None,
            focus_block_was: None,
            tab_frames: 0,
            pending_link: None,
            pending_href: None,
            focus_url_bar: false,
            focus_find: false,
            top_block: None,
            shown_entry: None,
            desktop_theme: None,
        };
        match launch.start {
            Some(url) => app.navigate(url, app.opts, true),
            // With nothing to read, the URL bar is the only sensible thing on screen, so it is
            // also the only sensible thing to be typing into.
            None => {
                app.chrome.url_bar = Some(String::new());
                app.focus_url_bar = true;
            }
        }
        Ok(app)
    }

    // ---------------------------------------------------------------- navigation

    /// Dispatch a navigation. **Nothing on screen changes here except the chrome.**
    ///
    /// Back and Forward reach this only when they have to: [`ReaderApp::arrive`] tries the page
    /// cache first and comes here on a miss. Every other navigation — a link, a typed URL,
    /// `reload`, `Shift+R`, Retry — arrives here directly and always asks the site.
    ///
    /// The resets that belong to the page being *fetched* are not done here; they are
    /// [`ReaderApp::commit`]'s, and they run when a document actually takes the column. Doing
    /// them at dispatch is what used to clear the textures, collapse state, and scroll offset of
    /// the page the reader was still looking at.
    fn navigate(&mut self, url: Url, opts: LoadOptions, push: bool) {
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
    /// out of it: `reload`, `Shift+R`, `Retry`, a link, and a typed URL all reach `navigate`
    /// directly. A check inside `navigate` would have needed a flag argument at every call site
    /// and would have been the wrong shape.
    fn arrive(&mut self) {
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
        self.request_image(src, self.column_texture_width(ctx), true, false);
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
            // Not only "the store has an answer": also "this address will never be
            // requested", so a declined picture is not requested and failed once per frame.
            &|src| {
                self.images.state(src).is_some()
                    || base
                        .join(src)
                        .is_ok_and(|u| image_decode::declined_by_url(&u).is_some())
            },
        );
        let max_width = self.column_texture_width(ctx);
        for src in srcs {
            // Counted whether or not the request survives: `request_image` declines a `src`
            // already in flight, and an entry that never resolves must not be planned again on
            // the next frame. `Msg::ImageDropped` is the one thing that takes a count back.
            *self.auto_attempts.entry(src.clone()).or_default() += 1;
            self.request_image(&src, max_width, false, true);
        }
    }

    /// The masthead favicon: same path as an article image, but quiet and small. Kicked from
    /// [`Self::ensure_favicon`] once a page is on screen; a toast would fire on every
    /// navigation, and a column-width decode is wasted on a 16-px mark.
    fn load_favicon(&mut self, src: &str) {
        self.request_image(src, 64, false, false);
    }

    /// Start the favicon fetch for the page on screen, once. Failures stay silent: the eyebrow
    /// label does not need a retry control for a missing mark.
    ///
    /// This is the one image request the reader makes without being asked, which is why it is
    /// the one `ImagePolicy::NoRequests` exists to stop. `Never` deliberately does not: it turns
    /// off *article* images, and it meant that before the third policy arrived. A reader who
    /// wants nothing contacted now has a way to say so, and until this guard was here there was
    /// no such way at all — the mark went out on every page whatever the setting said.
    fn ensure_favicon(&mut self) {
        if !self.settings.read.images.allows_any_request() {
            return;
        }
        let Some(src) = self.shown().and_then(|r| r.loaded.doc.favicon.clone()) else {
            return;
        };
        if self.images.state(&src).is_some() {
            return;
        }
        self.load_favicon(&src);
    }

    fn request_image(&mut self, src: &str, max_width: u32, announce: bool, automatic: bool) {
        // The visible page, and *its* request id rather than `self.current`: see `Ready::req`.
        let Some(ready) = self.shown() else {
            return;
        };
        let page_req = ready.req;
        if self.images.is_pending(src) {
            return;
        }
        // Beside `is_pending`, and for the same reason: this is the one door, and the paths
        // that reach it without drawing a control — "load all", and a click on a placeholder
        // the LRU evicted and redrew as an offer — would otherwise re-issue a request whose
        // answer is already known.
        if self.images.refuses_retry(src) {
            return;
        }
        let base = ready.loaded.prov.final_url.clone();
        let Ok(url) = base.join(src) else {
            self.images
                .fail(src, Failure::permanent("unusable image address".to_owned()));
            return;
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
        if let Some(name) = image_decode::declined_by_url(&url) {
            let why = image_decode::DecodeError::Unsupported(name).to_string();
            self.images.fail(src, Failure::permanent(why));
            return;
        }
        let host = url.host_str().unwrap_or("?").to_owned();
        self.images.begin(src);
        // Recorded here, where the host is known and where the reader is told it, and taken
        // back by `ImageStore::forget` if the pool answers `Msg::ImageDropped` for it. A
        // counter moved for a request that never left is the panel claiming a disclosure that
        // did not happen.
        self.images
            .record_request(src, &host, self.settings.send_image_referer);
        // Decode straight to the reading column: without this, "load images" on a photo-heavy
        // page is a GPU-memory denial of service driven by untrusted input.
        self.net.submit(Job::Image {
            req: self.net.mint(),
            page: page_req,
            src: src.to_owned(),
            url,
            referrer: base,
            max_width,
            send_referer: self.settings.send_image_referer,
            automatic,
        });
        if announce {
            self.flash(format!("loading one image from {host}"));
        }
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
        let all = collect_image_srcs(&ready.loaded.doc);
        let base = ready.loaded.prov.final_url.clone();
        // Declined, and *only* declined. Testing the address the other way round dropped every
        // `src` that fails to join too, so a page whose images all had unusable addresses
        // reported them as a format hww does not display — a second wrong claim about the page,
        // in place of the one this removes. An unusable address still belongs in the list;
        // `request_image` has the honest message for it. Asked twice below: once to decide what
        // to request, and once to decide what to say when that comes to nothing.
        let is_declined = |s: &str| {
            base.join(s)
                .is_ok_and(|u| image_decode::declined_by_url(&u).is_some())
        };
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
                    // A failure replaces the page it was fetched over, so this is a commit too.
                    self.commit();
                    self.page = Page::Failed {
                        url,
                        error,
                        opts: self.last_opts,
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
                    if self.shown().map(|r| r.req) != Some(page) {
                        continue;
                    }
                    // Takes back the entry and the disclosure the queued job had recorded.
                    self.images.forget(&src);
                    // Nothing was requested, so nothing was attempted: if this picture is
                    // still in the band the automatic policy may plan it again, with its full
                    // allowance intact.
                    self.auto_attempts.remove(&src);
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
                            opts: self.last_opts,
                        };
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
        let req = self.current.unwrap_or_else(|| self.net.mint_page());
        self.install(Box::new(Ready {
            loaded,
            outline,
            masthead,
            req,
            // Sound here and nowhere later: `drain` reaches `settle` only when
            // `self.current == Some(req)`, and `navigate` writes `current` and `last_opts`
            // in the same call, so the two cannot disagree about which navigation this is.
            opts: self.last_opts,
            fetched: Instant::now(),
            restored: false,
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
        // After `commit`, which cleared the outgoing page's index, and before the page is
        // installed, which is the last moment the document is not behind `self.page`.
        self.image_blocks = image_blocks(&page.loaded.doc);
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
            Err(error) => {
                self.commit();
                self.page = Page::Failed {
                    url,
                    error,
                    opts: self.last_opts,
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
            // `prov.chars` *is* `doc.text_len()`, computed once in `session::load`. Re-walking
            // the block tree on every navigation to learn what the fetch already measured
            // would be the same number for more work.
            let (cost, opts) = (page.loaded.prov.chars, page.opts);
            self.pages.insert(id, page, cost, opts);
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
        // its page action underneath it: `q` would quit out from under an open File menu.
        //
        // This was already true of the link context menu before there was a menu bar. The bar is
        // what makes it constant rather than obscure.
        if egui::Popup::is_any_open(ctx) {
            return;
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
        if k(Modifiers::COMMAND, Key::F) {
            self.open_find();
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
        if k(Modifiers::NONE, Key::O) {
            self.open_url_bar();
        }
        if k(Modifiers::NONE, Key::Backspace) {
            self.go_back();
        }
        if k(Modifiers::NONE, Key::R) {
            self.reload(self.opts);
        }
        if k(Modifiers::NONE, Key::T) {
            self.chrome.outline_open = !self.chrome.outline_open;
        }
        if k(Modifiers::NONE, Key::P) {
            self.chrome.info_open = !self.chrome.info_open;
        }
        if k(Modifiers::NONE, Key::Slash) {
            self.open_find();
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
        // `,` is the settings key everywhere else, and `Cmd+,` is the macOS convention, so both
        // are bound rather than picking one.
        if k(Modifiers::NONE, Key::Comma) || k(Modifiers::COMMAND, Key::Comma) {
            self.chrome.settings_open = !self.chrome.settings_open;
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
        self.collapsed_changes += 1;
        for k in keys {
            self.collapsed.insert(k);
        }
    }
}

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
            // Entry thumbnails draw once loaded (`blocks::entries_ui`), so `I` loads them
            // with the rest and the page-info panel counts them.
            ir::Block::Entries(es) => es.iter().for_each(|e| {
                out.extend(e.image.as_ref().map(|i| i.src.clone()));
                walk_inlines(&e.title, out);
                walk_blocks(&e.summary, out);
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
fn block_image_srcs(b: &ir::Block) -> Vec<String> {
    let mut out = Vec::new();
    walk_blocks(std::slice::from_ref(b), &mut out);
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
fn image_blocks(doc: &ir::Document) -> HashMap<String, Vec<usize>> {
    let mut map: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, b) in doc.blocks.iter().enumerate() {
        for src in block_image_srcs(b) {
            let at = map.entry(src).or_default();
            // Repeats within one block are one entry; the same `src` in two blocks is two.
            if at.last() != Some(&i) {
                at.push(i);
            }
        }
    }
    map
}

fn collect_image_srcs(doc: &ir::Document) -> Vec<String> {
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

        // Order matters: the strip is laid out first so it keeps its line at the bottom no
        // matter what the rest of the chrome does. It is the one thing that never hides.
        self.status_strip(ui, &pal);
        // Above the URL bar, so the window reads top to bottom as menus, address, page. A top
        // panel takes its place from the order it is added in, so this line's position is the
        // layout.
        self.menu_bar(ui, &pal);
        self.url_bar(ui, &pal);
        self.find_bar(ui, &pal);
        // Under the bars and above the page, docked: the one place a remark about the page
        // cannot scroll away and cannot be mistaken for the page.
        self.notice_bars(ui, &pal);
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
            link_addresses: self.settings.read.show_link_urls,
            outline: self.chrome.outline_open,
            page_info: self.chrome.info_open,
            menu_bar: self.settings.show_menu_bar,
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
                    if ui
                        .add_enabled(
                            self.history.can_go_back(),
                            egui::Button::new("←").frame(false),
                        )
                        .on_hover_text("Back (Alt+Left)")
                        .clicked()
                    {
                        action = Some(StripAction::Back);
                    }
                    if ui
                        .add_enabled(
                            self.history.can_go_forward(),
                            egui::Button::new("→").frame(false),
                        )
                        .on_hover_text("Forward (Alt+Right)")
                        .clicked()
                    {
                        action = Some(StripAction::Forward);
                    }
                    ui.separator();

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
                        if ui
                            .add(
                                egui::Button::new(RichText::new("? keys").color(pal.dim))
                                    .frame(false),
                            )
                            .on_hover_text("Keyboard shortcuts (?)")
                            .clicked()
                        {
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
        let bar = egui::Panel::top("hww-find")
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
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut query)
                            .id(egui::Id::new(FIND_ID))
                            .desired_width(240.0)
                            .margin(egui::Margin::symmetric(8, 3)),
                    );
                    if self.focus_find {
                        resp.request_focus();
                        self.focus_find = false;
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
                                    if ui
                                        .add(
                                            egui::Button::new(RichText::new(&e.text).color(color))
                                                .frame(false),
                                        )
                                        .clicked()
                                    {
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
                notice::carries_rtl(&r.loaded.doc),
                r.arrival(),
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
            ui.label(
                RichText::new(
                    "Ctrl means Cmd on macOS. Zoom is egui's: Ctrl +/-/0, Ctrl+wheel, pinch.",
                )
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
    /// rather than a longer toast, because three sentences is reading rather than status, and
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
                                self.ready_screen(ui);
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
                n.extend(notice::about_page(&r.loaded.prov, r.loaded.doc.text_len()));
                n.extend(notice::rtl(&r.loaded.doc));
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
    fn ready_screen(&mut self, ui: &mut Ui) {
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
        ctx.pending_href = self.pending_href.clone();
        ctx.comment_heights = self.heights.take_comments();

        let doc = &ready.loaded.doc;
        blocks::document_header(ui, doc, &mut ctx);
        let scroll_to = self.scroll_to_block.take();
        let clip_top = ui.clip_rect().top();
        let band = measure::Band::around(clip_top, ui.clip_rect().bottom());
        // Unconditionally, unlike `ctx.band`, which is cleared for any block laid out whole:
        // "may this block be skipped" and "is this picture near the window" are different
        // questions, and only the first one is allowed to have no answer.
        ctx.autoload_band = auto.then_some(band);
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
            ctx.block = i;
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
        // Gathered where each picture drew, at every nesting depth, so a feed card or a comment
        // is judged by where *it* landed and not by the one block that holds the lot.
        let in_band = std::mem::take(&mut ctx.autoload_srcs);
        self.find_total = ctx.find_seen;
        self.find_scroll = false;
        self.focused_href = ctx.focus_href.clone();
        self.focused_image = ctx.focus_image.clone();
        self.focused_comment = ctx.focus_comment.clone();
        self.focused_other = ctx.focus_other;
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
        // Last, and only if this frame did not start a navigation: a click that is leaving the
        // page has just made every picture on it something nobody is going to look at.
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
        Box::new(Ready {
            masthead: title::masthead(&doc),
            outline: outline::build(&doc.blocks),
            loaded: Loaded {
                doc,
                prov: crate::session::Provenance::fixture(url.as_str()),
            },
            req: ReqId::for_test(req),
            opts: LoadOptions::default(),
            fetched: Instant::now(),
            restored: false,
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
        let map = image_blocks(&doc);
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
                    block_image_srcs(&doc.blocks[*i]).contains(src),
                    "block {i} does not contain {src}"
                );
            }
        }
        // And nothing on the page is missing from it.
        for src in collect_image_srcs(&doc) {
            assert!(map.contains_key(&src), "{src} is in no block");
        }
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
