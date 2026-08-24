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
//! Article images live on CDN hosts constantly, and "no third-party requests" is in the first
//! paragraph of both README and AGENTS.md. Loading one is allowed on exactly the reasoning
//! `rewrite` uses: the placeholder **names the host before the click**, the click is a
//! deliberate user action, the load is counted in the provenance strip, and the capability
//! lives behind the `gui` feature, so the `hww` CLI retains compile-time absence of it.
//!
//! Article images are never auto-loaded, prefetched, or loaded on hover. The one exception is
//! the page favicon beside the masthead eyebrow: that is chrome identity, fetched as soon as
//! the document names a URL (or the well-known `/favicon.ico` fallback), counted the same way,
//! and omitted from the column until the bytes decode.

use crate::ir;
use crate::reader::image_decode::Decoded;
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

pub enum State {
    Offered,
    Loading,
    Ready(Ready),
    Failed(String),
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
    /// Distinct hosts contacted for images this session, for the status strip.
    pub hosts: HashSet<String>,
    pub loaded: usize,
    /// Bumped by every change to `entries`. The reading column caches a height per block
    /// (`reader::measure`), and an image that arrives, fails, or is evicted re-sizes the block
    /// it sits in, including blocks nowhere near the window when `I` loads a whole page.
    changes: u64,
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

    /// How many times `entries` has changed. See the field.
    pub fn changes(&self) -> u64 {
        self.changes
    }

    pub fn begin(&mut self, src: &str) {
        self.set(src, State::Loading);
        self.touch(src);
        self.evict();
    }

    /// The only writer of `entries`, so the change counter cannot be forgotten at a new call
    /// site: the field doc promises every change bumps it, and this is where that is kept.
    fn set(&mut self, src: &str, state: State) {
        self.changes += 1;
        self.entries.insert(src.to_owned(), state);
    }

    pub fn is_pending(&self, src: &str) -> bool {
        matches!(self.entries.get(src), Some(State::Loading))
    }

    pub fn any_pending(&self) -> bool {
        self.entries.values().any(|s| matches!(s, State::Loading))
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
        self.touch(src);
        self.evict();
    }

    /// The disclosure counters, for the page-info panel.
    ///
    /// Assembled here so the field list lives beside the fields it reads: `pageinfo::Counts`
    /// is one directory up, where egui cannot reach, and a second hand-written copy of these
    /// four names is one that goes stale the next time a counter is added.
    pub fn counts(&self) -> Counts {
        let mut hosts: Vec<String> = self.hosts.iter().cloned().collect();
        hosts.sort_unstable();
        Counts {
            images: self.loaded,
            hosts,
            cookie_attempts: self.cookie_attempts,
            referer_requests: self.referer_requests,
        }
    }

    pub fn fail(&mut self, src: &str, why: String) {
        self.set(src, State::Failed(why));
        self.touch(src);
    }

    /// Dropped when a new page commits, not when one is asked for: the outgoing page stays on
    /// screen for the length of the fetch and its pictures have to stay with it. Dimensions and
    /// the disclosure counters survive even that: the first is layout memory, and the second is a
    /// running account of what this session disclosed, which resetting would quietly understate.
    pub fn clear_textures(&mut self) {
        self.changes += 1;
        self.entries.clear();
        self.order.clear();
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
        let mut i = 0;
        while self.order.len() > LRU_CAPACITY && i < self.order.len() {
            if self.is_pending(&self.order[i]) {
                i += 1;
                continue;
            }
            let oldest = self.order.remove(i);
            self.changes += 1;
            self.entries.remove(&oldest);
        }
    }
}

enum Snapshot {
    Offered,
    Loading,
    Ready,
    Failed(String),
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
        Some(State::Failed(why)) => Snapshot::Failed(why.clone()),
        Some(State::Ready(_)) => Snapshot::Ready,
        None | Some(State::Offered) => Snapshot::Offered,
    };
    if let Snapshot::Ready = state {
        show_ready(ui, img, ctx);
        return;
    }
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
                    Snapshot::Failed(why) => {
                        ui.label(RichText::new(format!("[image] {why}")).color(pal.notice_fg));
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&host).color(pal.dim));
                            let resp = ui.small_button("retry");
                            act = resp.clicked();
                            focused = resp.has_focus();
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

/// The one control an entry's thumbnail gets: a small dim `[image]` beside the time, which
/// loads that one picture, and nothing once it is loaded. `Never` shows nothing at all.
pub fn load_control(ui: &mut Ui, img: &ir::Image, ctx: &mut RenderCtx<'_>) {
    if !ctx.opts.images.offers_loading()
        || matches!(ctx.images.state(&img.src), Some(State::Ready(_)))
    {
        return;
    }
    let pal = ctx.pal;
    let resp =
        ui.add(egui::Button::new(RichText::new("[image]").color(pal.dim).small()).frame(false));
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
        _ => {
            let resp = ui.add(
                egui::Button::new(RichText::new(format!("[{alt}]")).color(pal.dim)).frame(false),
            );
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
            referer_requests: 1,
            ..ImageStore::default()
        };
        store.hosts.insert("img.example.org".to_owned());
        store.hosts.insert("cdn.example.net".to_owned());

        // The decode failed. Nothing about what the request disclosed is undone by that.
        store.fail("a.png", "not an image".to_owned());

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
}
