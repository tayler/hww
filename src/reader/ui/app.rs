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
use crate::reader::notice::{self, Button, Notice};
use crate::reader::outline::{self, OutlineEntry};
use crate::reader::settings::{self, Settings};
use crate::reader::thread_tree::CommentKey;
use crate::reader::title;
use crate::reader::ui::images::ImageStore;
use crate::reader::ui::{Action, Launch, RenderCtx, blocks, net, notice_ui, theme};
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
    /// Nothing asked for yet: the URL bar is open and focused.
    Idle,
    Loading {
        url: Url,
        started: Instant,
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

struct Ready {
    loaded: Loaded,
    outline: Vec<OutlineEntry>,
    /// `Some(i)` => `blocks[i]` repeats `doc.title` and is skipped.
    skip_block: Option<usize>,
}

#[derive(Default)]
struct Chrome {
    url_bar: Option<String>,
    outline_open: bool,
    find: Option<String>,
    help_open: bool,
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
    /// Toggles against the auto-collapse default (see `thread_ui::is_collapsed`).
    collapsed: HashSet<CommentKey>,
    find_current: usize,
    find_total: usize,
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
    /// Deferred to the end of the frame: `ctx.copy_text` inside a closure that already borrows
    /// the app would be a second mutable borrow.
    pending_copy: Option<String>,
    /// What Tab last landed on, read back by `Enter`, `Y`, `i`, and `z`. Set during render.
    focused_href: Option<String>,
    focused_image: Option<String>,
    focused_comment: Option<CommentKey>,
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

impl ReaderApp {
    fn new(cc: &eframe::CreationContext<'_>, launch: Launch) -> Result<Self, LoadError> {
        let net = Net::new(cc.egui_ctx.clone())?;
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
            collapsed: HashSet::new(),
            find_current: 0,
            find_total: 0,
            rewrite: launch.rewrite,
            last_rewrite: launch.rewrite,
            scroll_to_block: None,
            scroll_delta: 0.0,
            scroll_offset: None,
            viewport_height: 600.0,
            pending_copy: None,
            focused_href: None,
            focused_image: None,
            focused_comment: None,
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

    fn navigate(&mut self, url: Url, rewrite: bool, push: bool) {
        let req = self.net.mint();
        self.last_rewrite = rewrite;
        self.current = Some(req);
        self.rewrite_notice = None;
        // Textures die with the page they were loaded for; the disclosure counters do not.
        self.images.clear_textures();
        self.collapsed.clear();
        self.find_current = 0;
        self.find_total = 0;
        self.scroll_offset = Some(0.0);
        if push {
            self.history.push(url.clone());
        }
        self.page = Page::Loading {
            url: url.clone(),
            started: Instant::now(),
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

    fn same_page(&self, url: &Url) -> bool {
        let Page::Ready(ready) = &self.page else {
            return false;
        };
        let mut a = url.clone();
        a.set_fragment(None);
        let mut b = ready.loaded.prov.final_url.clone();
        b.set_fragment(None);
        a == b
    }

    fn jump_to_fragment(&mut self, fragment: &str) {
        let Page::Ready(ready) = &self.page else {
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
        let (Page::Ready(ready), Some(page_req)) = (&self.page, self.current) else {
            return;
        };
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
        let Page::Ready(ready) = &self.page else {
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
                    self.page = Page::Failed {
                        url,
                        error,
                        rewrite: self.last_rewrite,
                    };
                }
                Msg::Image {
                    page, src, result, ..
                } => {
                    if self.current != Some(page) {
                        continue;
                    }
                    match result {
                        Ok(decoded) => self.images.insert(ctx, &src, decoded),
                        Err(e) => self.images.fail(&src, e.to_string()),
                    }
                }
                Msg::Panicked { page, .. } => {
                    if self.current == Some(page) {
                        self.flash("a worker failed while handling that request".to_owned());
                    }
                }
            }
        }
    }

    fn settle(&mut self, loaded: Loaded) {
        // The rewrite landed somewhere other than the rule aimed at. Not an error, but the
        // rule is dead, and that is the whole early-warning system.
        if loaded.prov.rule_appears_dead {
            self.flash(format!(
                "{} redirected away; that rewrite rule appears dead",
                loaded
                    .prov
                    .rewritten_to
                    .as_ref()
                    .and_then(Url::host_str)
                    .unwrap_or("?")
            ));
        }
        let outline = outline::build(&loaded.doc.blocks);
        let skip_block = title::dedupe(&loaded.doc);
        let fragment = self
            .history
            .current()
            .and_then(|u| u.fragment().map(str::to_owned));
        self.page = Page::Ready(Box::new(Ready {
            loaded,
            outline,
            skip_block,
        }));
        if let Some(f) = fragment {
            self.jump_to_fragment(&f);
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
            self.scroll_offset = Some(f32::INFINITY);
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
        if k(Modifiers::NONE, Key::Slash) {
            self.chrome.find = Some(self.chrome.find.take().unwrap_or_default());
            self.focus_find = true;
        }
        if self.find_total > 0 && k(Modifiers::NONE, Key::N) {
            self.find_current = (self.find_current + 1) % self.find_total;
        }
        if k(Modifiers::NONE, Key::I) {
            match self.focused_image.clone() {
                Some(src) => self.load_image(ctx, &src),
                None => self
                    .flash("no image focused; Tab to a placeholder, or Shift+I for all".to_owned()),
            }
        }
        if k(Modifiers::NONE, Key::Y)
            && let Some(u) = self.history.current().cloned()
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
        let Page::Ready(ready) = &self.page else {
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
        self.focused_href = None;
        self.focused_image = None;
        self.focused_comment = None;
        self.hover_href = None;

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

        if let Some(text) = self.pending_copy.take() {
            ctx.copy_text(text);
        }
        self.persist(&ctx);

        // While anything is in flight, keep repainting so the elapsed-time indicator animates
        // without a busy loop. Workers also repaint on every message, so this is the *floor*
        // on responsiveness, not the mechanism.
        if matches!(self.page, Page::Loading { .. }) || self.images.any_pending() {
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
            .frame(
                egui::Frame::new()
                    .fill(pal.chrome_bg)
                    .inner_margin(egui::Margin::symmetric(8, 4)),
            )
            .show(ui, |ui| {
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

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let summary = self.provenance_summary();
                        let resp = ui.label(RichText::new(&summary).color(pal.dim).small());
                        if !self.images.hosts.is_empty() {
                            let mut hosts: Vec<&str> =
                                self.images.hosts.iter().map(String::as_str).collect();
                            hosts.sort_unstable();
                            resp.on_hover_text(format!("image hosts: {}", hosts.join(", ")));
                        }
                    });
                });
            });
        self.strip_height = strip.response.rect.height();
        match action {
            Some(StripAction::Back) => self.go_back(),
            Some(StripAction::Forward) => self.go_forward(),
            Some(StripAction::OpenUrlBar) => self.open_url_bar(),
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
                        // An `Area` offers unbounded width, so wrapping needs an explicit one.
                        ui.set_max_width(width);
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                        ui.label(RichText::new(shown).color(pal.link));
                    });
            });
    }

    fn status_url(&self) -> String {
        match &self.page {
            Page::Loading { url, started } => {
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

    fn provenance_summary(&self) -> String {
        let Page::Ready(r) = &self.page else {
            return String::new();
        };
        let p = &r.loaded.prov;
        let mut parts = vec![
            format!("{} {}", p.status, p.encoding),
            format!("{} -> {} chars", human_bytes(p.bytes), p.chars),
        ];
        if !p.hops.is_empty() {
            parts.push(format!("{} redirect(s)", p.hops.len()));
        }
        let cookies = p.cookie_attempts + self.images.cookie_attempts;
        if cookies > 0 {
            parts.push(format!("{cookies} cookie attempt(s) discarded"));
        }
        if let Some(t) = truncation_note(p.truncation) {
            parts.push(t);
        }
        if self.images.loaded > 0 {
            parts.push(format!(
                "{} image(s), {} host(s)",
                self.images.loaded,
                self.images.hosts.len()
            ));
        }
        if self.images.referer_requests > 0 {
            parts.push(format!(
                "{} origin-only referer(s)",
                self.images.referer_requests
            ));
        }
        parts.join(" · ")
    }

    fn url_bar(&mut self, ui: &mut Ui, pal: &theme::Palette) {
        let Some(mut text) = self.chrome.url_bar.take() else {
            return;
        };
        let mut go = false;
        let mut keep = true;
        egui::Panel::top("hww-url")
            .frame(
                egui::Frame::new()
                    .fill(pal.chrome_bg)
                    .inner_margin(egui::Margin::symmetric(8, 4)),
            )
            .show(ui, |ui| {
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
        egui::Panel::top("hww-find")
            .frame(
                egui::Frame::new()
                    .fill(pal.chrome_bg)
                    .inner_margin(egui::Margin::symmetric(8, 4)),
            )
            .show(ui, |ui| {
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
                    ui.label(RichText::new("n / N to step · Esc to dismiss").small());
                });
            });
        if let Some(n) = next {
            self.find_current = n;
        }
        self.chrome.find = Some(query);
    }

    fn outline_panel(&mut self, ui: &mut Ui, pal: &theme::Palette) {
        if !self.chrome.outline_open {
            return;
        }
        let Page::Ready(ready) = &self.page else {
            return;
        };
        let entries: Vec<OutlineEntry> = ready.outline.clone();
        let mut jump = None;
        egui::Panel::left("hww-outline")
            .resizable(true)
            .default_size(240.0)
            .frame(
                egui::Frame::new()
                    .fill(pal.chrome_bg)
                    .inner_margin(egui::Margin::symmetric(8, 6)),
            )
            .show(ui, |ui| {
                ui.label(RichText::new("Outline").color(pal.dim).small());
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
        if let Some(b) = jump {
            self.scroll_to_block = Some(b);
        }
    }

    fn help_overlay(&mut self, ctx: &egui::Context, pal: &theme::Palette) {
        if !self.chrome.help_open {
            return;
        }
        egui::Window::new("Keys")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
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
                    .small(),
                );
            });
    }

    // ---------------------------------------------------------------- the page

    fn central(&mut self, ui: &mut Ui, pal: &theme::Palette) {
        // Rounded to whole *pixels*, not points: at a fractional zoom a column on a half-pixel
        // boundary makes every glyph in it resample, which reads as soft text rather than as a
        // bug, and egui's debug build paints an "Unaligned" warning over the page to say so.
        let wanted = theme::measure_px(ui.ctx(), &self.settings.read);
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

                    if matches!(self.page, Page::Ready(_)) {
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
            });
    }

    /// Every notice for the current page, in the one place they are drawn.
    ///
    /// What this replaces is each screen drawing its own banner where it happened, which put
    /// the reader's words inside the reading column at the prose's own left edge and left
    /// colour as the only thing telling them apart.
    fn notices(&mut self, ui: &mut Ui, pal: &theme::Palette, column: notice_ui::Column) {
        // Owned, so the borrow of `self.page` ends here rather than running through the draw
        // loop and colliding with the `&mut self` that acting on a button needs.
        let notices: Vec<Notice> = match &self.page {
            Page::Idle => vec![notice::idle()],
            Page::Loading { url, started } => vec![notice::loading(url, started.elapsed())],
            Page::Failed { url, error, .. } => vec![notice::failure(error, url)],
            Page::Ready(r) => notice::about_page(r.loaded.prov.status, r.loaded.doc.text_len()),
        };
        let mut pressed = None;
        for notice in &notices {
            if let Some(b) = notice_ui::band(ui, pal, &self.settings.read, column, notice) {
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

    fn ready_screen(&mut self, ui: &mut Ui) {
        // The page is behind `&mut self`, and rendering needs both it and the image store, so
        // take it for the duration and put it back. Cheaper than cloning a document.
        let Page::Ready(ready) = std::mem::replace(&mut self.page, Page::Idle) else {
            unreachable!("checked by the caller")
        };
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

        let doc = &ready.loaded.doc;
        blocks::document_header(ui, doc, &mut ctx);
        let scroll_to = self.scroll_to_block.take();
        for (i, b) in doc.blocks.iter().enumerate() {
            // The article's <h1> is usually also its <title>; rendering both shows the
            // headline twice.
            if ready.skip_block == Some(i) {
                continue;
            }
            let resp = ui.scope(|ui| blocks::block_ui(ui, b, &mut ctx)).response;
            if scroll_to == Some(i) {
                resp.scroll_to_me(Some(Align::TOP));
            }
        }
        ui.add_space(theme::snap(
            theme::line_height_px(&self.settings.read) * 4.0,
        ));

        self.find_total = ctx.find_seen;
        self.focused_href = ctx.focus_href.clone();
        self.focused_image = ctx.focus_image.clone();
        self.focused_comment = ctx.focus_comment.clone();
        self.hover_href = ctx.hover_href.clone();
        let action = ctx.action.take();
        self.page = Page::Ready(ready);
        if let Some(a) = action {
            self.apply(ui.ctx(), a);
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

enum StripAction {
    Back,
    Forward,
    OpenUrlBar,
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
    ("/ then n / N", "find in page · next / previous match"),
    ("z / Z", "collapse focused reply · collapse all"),
    ("y / Y", "copy page URL · copy focused link"),
    ("[ / ]", "narrow / widen the reading measure"),
    ("d", "cycle theme"),
    ("? / Esc / q", "this help · dismiss chrome · quit"),
];

fn human_bytes(n: usize) -> String {
    if n >= 1_000_000 {
        format!("{:.1} MB", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{} kB", n / 1_000)
    } else {
        format!("{n} B")
    }
}

fn truncation_note(t: crate::fetch::Truncation) -> Option<String> {
    use crate::fetch::Truncation;
    match t {
        Truncation::Complete => None,
        Truncation::AtCap(n) => Some(format!("truncated at {} MB", n / 1_000_000)),
        Truncation::MaybeAtCap(n) => Some(format!("may be truncated at {} MB", n / 1_000_000)),
        Truncation::Timeout(d) => Some(format!("truncated: timed out after {}s", d.as_secs())),
        Truncation::Incomplete(n) => Some(format!("truncated: connection dropped at {n} bytes")),
    }
}
