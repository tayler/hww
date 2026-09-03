//! A tweet, and the profile over a timeline of them, rendered as cards.
//!
//! The shape the page itself uses, because it is the shape that makes the parts legible: the
//! name over the message, the message in the reading face, the date under it, a hairline, and
//! the counts along the bottom. Without the card a tweet is four unlabelled numbers after a
//! paragraph, which is what `Block::Tweets` exists so a renderer never has to draw.
//!
//! **The counts are words, not icons.** `fonts/` carries text faces and Noto Emoji, and
//! `prefs::no_reader_facing_string_needs_a_missing_glyph` already pins that even `⏵ ▸ ☰ ⚙ ⓘ` are
//! absent; a speech bubble, a retweet arrow, a heart and a bookmark are four hand-painted marks
//! for something the page labels in words anyway. `56K replies` says what a glyph would, out
//! loud, in a screen reader, and on a monochrome display. The only non-ASCII mark here is `·`,
//! which `blocks::document_header` already draws.
//!
//! **The avatar waits.** It is a third-party picture on a fetched page, so it is `ImagePolicy`'s
//! like every other picture, and it is not the site-mark exception: a mark identifies a site on
//! one of hww's own surfaces, and this is a person on somebody else's. The square is allocated
//! whether or not the bytes are here, or the name would slide sideways when they arrive.
//!
//! A tweet is one top-level block, so the block-level window rule in `ReaderApp::ready_screen`
//! already covers it. It needs no height cache of its own, unlike `entries_ui` and `thread_ui`,
//! which draw thousands of rows under one block index.

use crate::ir;
use crate::reader::face::Face;
use crate::reader::ui::blocks::blocks_ui;
use crate::reader::ui::{RenderCtx, fonts, theme};
use eframe::egui::{self, RichText, Ui};

pub fn tweets_ui(ui: &mut Ui, tweets: &[ir::Tweet], ctx: &mut RenderCtx<'_>) {
    for tweet in tweets {
        let indent = theme::snap(ctx.opts.base_size_pt * 1.4) * f32::from(tweet.depth.min(4));
        ui.horizontal_top(|ui| {
            if indent > 0.0 {
                ui.add_space(indent);
            }
            ui.vertical(|ui| {
                ui.set_max_width((ui.available_width() - indent).max(0.0));
                card(ui, tweet, ctx);
            });
        });
        ui.add_space(theme::snap(ctx.opts.base_size_pt * 0.5));
    }
}

fn card(ui: &mut Ui, tweet: &ir::Tweet, ctx: &mut RenderCtx<'_>) {
    // `i8` because `egui::Margin` is; the clamp keeps the cast in range, as `theme::bar_pad`'s.
    let pad = (ctx.opts.base_size_pt * 0.6).round().clamp(6.0, 20.0) as i8;
    // `guide` and not `rule`: `Palette::guide` is the role for a mark that identifies structure
    // rather than decorating it, and a tweet's border is the whole of what says this is one tweet
    // and not the page. It is already held to 3:1 by the contrast suite, so nothing there moves.
    let stroke = egui::Stroke::new(1.0, ctx.pal.guide);
    egui::Frame::new()
        .stroke(stroke)
        .corner_radius(egui::CornerRadius::same(theme::RADIUS))
        .inner_margin(egui::Margin::same(pad))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            byline(ui, tweet, ctx);
            blocks_ui(ui, &tweet.blocks, ctx, None);
            if let Some(t) = tweet.timestamp.as_deref().filter(|t| !t.trim().is_empty()) {
                ui.add_space(theme::snap(ctx.opts.base_size_pt * 0.3));
                ui.label(
                    RichText::new(t)
                        .color(ctx.pal.dim)
                        .font(theme::small_font(ctx.opts)),
                );
            }
            if !tweet.stats.is_empty() {
                ui.add_space(theme::snap(ctx.opts.base_size_pt * 0.3));
                ui.separator();
                stats_row(ui, &tweet.stats, ctx);
            }
        });
}

/// The account over a timeline: the banner, the face beside the name, the bio, the link, the
/// joined date, and the counts. Laid out as a tweet's card is, because its parts are the same
/// kinds of thing — a name, some words, a row of counts — with a wide picture over them, and
/// drawn without the card's border, because it is the page's header and not an item in it.
///
/// The banner is drawn through [`super::images::placeholder`] as a figure is, and not through
/// a mark's door: it is a third-party picture on a fetched page and answers `ImagePolicy` like
/// the face under it. The face is [`super::images::avatar`]'s square at twice a tweet's size,
/// because this is the one place on the page where the person is the subject.
pub fn profile_ui(ui: &mut Ui, p: &ir::Profile, ctx: &mut RenderCtx<'_>) {
    // The same margins as a tweet's card and no border: bordered, the account read as the
    // first post on its own timeline. The header is the page's, not one item in the column,
    // and the hairline over the counts is the one rule it draws.
    let pad = (ctx.opts.base_size_pt * 0.6).round().clamp(6.0, 20.0) as i8;
    egui::Frame::new()
        .inner_margin(egui::Margin::same(pad))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            if let Some(b) = &p.banner {
                super::images::placeholder(ui, b, ctx);
                ui.add_space(theme::snap(ctx.opts.base_size_pt * 0.4));
            }
            let small = theme::small_font(ctx.opts);
            let bold = egui::FontId::new(
                theme::body_font(ctx.opts).size * 1.15,
                fonts::family(Face::new(ctx.opts.family, true, false)),
            );
            ui.horizontal_top(|ui| {
                if let Some(a) = &p.avatar {
                    super::images::avatar(ui, a, theme::avatar_box(ctx.opts) * 2.0, ctx);
                    ui.add_space(theme::snap(ctx.opts.base_size_pt * 0.5));
                }
                ui.vertical(|ui| {
                    if let Some(name) = p.name.as_deref().filter(|n| !n.trim().is_empty()) {
                        ui.label(RichText::new(name).color(ctx.pal.strong).font(bold.clone()));
                    }
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = theme::snap(ctx.opts.base_size_pt * 0.35);
                        if let Some(h) = p.handle.as_deref().filter(|h| !h.trim().is_empty()) {
                            ui.label(RichText::new(h).color(ctx.pal.dim).font(small.clone()));
                        }
                        if let Some(a) = &p.avatar {
                            super::images::load_control(ui, a, ctx);
                        }
                    });
                });
            });
            if !p.bio.is_empty() {
                ui.add_space(theme::snap(ctx.opts.base_size_pt * 0.4));
                blocks_ui(ui, &p.bio, ctx, None);
            }
            if !p.website.is_empty() {
                // One paragraph built per frame so the link is drawn, focused, and followed
                // exactly as a link in the page is. It is one inline; the clone is a few bytes.
                let line = [ir::Block::Paragraph(p.website.clone())];
                blocks_ui(ui, &line, ctx, None);
            }
            if let Some(j) = p.joined.as_deref().filter(|j| !j.trim().is_empty()) {
                ui.label(RichText::new(j).color(ctx.pal.dim).font(small.clone()));
            }
            if !p.stats.is_empty() {
                ui.add_space(theme::snap(ctx.opts.base_size_pt * 0.3));
                ui.separator();
                stats_row(ui, &p.stats, ctx);
            }
        });
    ui.add_space(theme::snap(ctx.opts.base_size_pt * 0.6));
}

/// The name over the tweet: the face, the display name, and the at-name under it.
fn byline(ui: &mut Ui, tweet: &ir::Tweet, ctx: &mut RenderCtx<'_>) {
    if tweet.author.is_none() && tweet.handle.is_none() && tweet.avatar.is_none() {
        return;
    }
    let small = theme::small_font(ctx.opts);
    let bold = egui::FontId::new(
        theme::body_font(ctx.opts).size,
        fonts::family(Face::new(ctx.opts.family, true, false)),
    );
    ui.horizontal_top(|ui| {
        if let Some(a) = &tweet.avatar {
            super::images::avatar(ui, a, theme::avatar_box(ctx.opts), ctx);
            ui.add_space(theme::snap(ctx.opts.base_size_pt * 0.4));
        }
        ui.vertical(|ui| {
            if let Some(name) = tweet.author.as_deref().filter(|n| !n.trim().is_empty()) {
                ui.label(RichText::new(name).color(ctx.pal.strong).font(bold.clone()));
            }
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = theme::snap(ctx.opts.base_size_pt * 0.35);
                if let Some(h) = tweet.handle.as_deref().filter(|h| !h.trim().is_empty()) {
                    ui.label(RichText::new(h).color(ctx.pal.dim).font(small.clone()));
                }
                // The one offer for the face, beside the name it belongs to. The message's own
                // pictures carry theirs in the column, as they do on any other page.
                if let Some(a) = &tweet.avatar {
                    super::images::load_control(ui, a, ctx);
                }
            });
        });
    });
    ui.add_space(theme::snap(ctx.opts.base_size_pt * 0.35));
}

/// `56K replies · 166K reposts · 2.6M likes`, wrapped to the measure.
///
/// Wrapped and not a fixed row because the counts are the page's own strings and a label is a
/// word in whatever language the page speaks.
fn stats_row(ui: &mut Ui, stats: &[ir::Stat], ctx: &mut RenderCtx<'_>) {
    let small = theme::small_font(ctx.opts);
    let bold = egui::FontId::new(
        small.size,
        fonts::family(Face::new(ctx.opts.family, true, false)),
    );
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = theme::snap(ctx.opts.base_size_pt * 0.3);
        for (i, s) in stats.iter().enumerate() {
            if i > 0 {
                ui.label(RichText::new("·").color(ctx.pal.dim).font(small.clone()));
            }
            ui.label(
                RichText::new(&s.count)
                    .color(ctx.pal.strong)
                    .font(bold.clone()),
            );
            ui.label(
                RichText::new(s.kind.label())
                    .color(ctx.pal.dim)
                    .font(small.clone()),
            );
        }
    });
}
