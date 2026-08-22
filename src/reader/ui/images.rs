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
//! Nothing is auto-loaded, prefetched, or loaded on hover.

use crate::ir;
use crate::reader::image_decode::Decoded;
use crate::reader::opts::ImagePolicy;
use crate::reader::pageinfo::Counts;
use crate::reader::ui::{Action, RenderCtx};
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

/// The compact strip a figure or an inline image shows before it is loaded.
///
/// One line, because no dimensions live in `ir::Image`, so a tall placeholder would reserve
/// the wrong box and shift the page when it filled. One line shifts by almost nothing, and a
/// revisit reserves the right box from [`ImageStore::reserved`].
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

pub fn placeholder(ui: &mut Ui, img: &ir::Image, ctx: &mut RenderCtx<'_>) {
    let alt_full = img.alt.as_deref().unwrap_or("").trim();
    let alt_short = short_alt(alt_full);
    let alt = alt_short.as_str();
    let host = super::inline_ui::image_host(img, &ctx.base);
    let pal = ctx.pal;

    if ctx.opts.images == ImagePolicy::Never {
        // Silently dropping the image is how a reader lies about a page: say it was there.
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
    match state {
        Snapshot::Loading => {
            ui.label(
                RichText::new(format!("[image] loading from {host}…"))
                    .color(pal.dim)
                    .font(super::theme::chrome_font(ctx.opts)),
            );
        }
        Snapshot::Failed(why) => {
            let mut retry = false;
            let mut focused = false;
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(format!("[image] {why}: {host}")).color(pal.notice_fg));
                let resp = ui.small_button("retry");
                retry = resp.clicked();
                focused = resp.has_focus();
            });
            // Reported like the offer's button: `i` then retries, and the block stays laid
            // out while Tab rests here.
            if focused {
                ctx.focus_image = Some(img.src.clone());
            }
            if retry {
                ctx.act(Action::LoadImage(img.src.clone()));
            }
        }
        Snapshot::Ready => show_ready(ui, img, ctx),
        Snapshot::Offered => {
            // The host is named *before* the click, which is the whole argument for allowing
            // the request at all.
            let text = if alt.is_empty() {
                format!("[image] load from {host}")
            } else {
                format!("[image] {alt} · load from {host}")
            };
            let (w, h) = ctx.images.reserved(&img.src).unwrap_or((0, 0));
            // Smaller than the prose it interrupts: a placeholder is an offer, not a
            // sentence, and at body size a photo-heavy page becomes a list of offers.
            let resp = ui.add(
                egui::Button::new(
                    RichText::new(text)
                        .color(pal.link)
                        .font(super::theme::chrome_font(ctx.opts)),
                )
                .frame(false),
            );
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if resp.has_focus() {
                ctx.focus_image = Some(img.src.clone());
            }
            if resp.clicked() {
                ctx.act(Action::LoadImage(img.src.clone()));
            }
            // A revisit knows the box; reserve it so the page does not jump when it fills.
            if w > 0 && h > 0 {
                let width = ui.available_width().min(w as f32);
                ui.add_space(width * h as f32 / w as f32);
            }
        }
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
            .corner_radius(0.0),
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
        _ => {
            let resp = ui.add(
                egui::Button::new(RichText::new(format!("[{alt}]")).color(pal.dim)).frame(false),
            );
            if resp.has_focus() {
                ctx.focus_image = Some(img.src.clone());
            }
            if resp.clicked() && ctx.opts.images != ImagePolicy::Never {
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
