//! A discussion, rendered as a collapsible tree.
//!
//! Traversal is **iterative**, matching `reader::thread_tree::dfs`: a pathological thread is
//! untrusted input, and a recursive walk over one is a crash rather than a bad render.
//!
//! Indent is capped at [`ReadOpts::max_thread_indent`] (a 40-deep thread would otherwise
//! render two pixels wide), and deep or very long subtrees start collapsed, so a huge thread
//! opens navigable instead of as forty screens of nesting.

use crate::ir;
use crate::reader::thread_tree::{self, ThreadTree};
use crate::reader::ui::blocks::blocks_ui;
use crate::reader::ui::{Action, RenderCtx};
use eframe::egui::{self, RichText, Ui};

pub fn thread_ui(ui: &mut Ui, comments: &[ir::Comment], ctx: &mut RenderCtx<'_>) {
    let tree = thread_tree::build(comments);
    if tree.is_empty() {
        return;
    }
    // Iterative, with an explicit stack: `(node, needs_no_further_children)`.
    let mut stack: Vec<usize> = tree.roots.iter().rev().copied().collect();
    while let Some(n) = stack.pop() {
        let node = &tree.nodes[n];
        let comment = &comments[node.comment];
        let collapsed = is_collapsed(&tree, n, ctx);

        ui.horizontal_top(|ui| {
            let indent = ctx.opts.indent_for(node.depth);
            // A hairline guide per level, so a reply four deep can be traced back to what it
            // answers without counting pixels. Reserved and filled in after the reply is laid
            // out, for the reason `blocks::Quote` gives: `available_height` is the rest of the
            // viewport, not the height of the thing being marked.
            let guide = (indent > 0.0).then(|| {
                ui.add_space(indent);
                let shape = ui.painter().add(egui::Shape::Noop);
                let (slot, _) = ui.allocate_exact_size(egui::vec2(1.0, 0.0), egui::Sense::hover());
                ui.add_space(super::theme::snap(ctx.opts.base_size_pt * 0.5));
                (shape, slot)
            });
            let reply = ui
                .vertical(|ui| {
                    header(ui, comment, &tree, n, collapsed, ctx);
                    if !collapsed {
                        blocks_ui(ui, &comment.blocks, ctx, None);
                    }
                })
                .response
                .rect;
            if let Some((shape, slot)) = guide {
                ui.painter().set(
                    shape,
                    egui::Shape::rect_filled(
                        egui::Rect::from_min_max(
                            egui::pos2(slot.left(), reply.top()),
                            egui::pos2(slot.left() + 1.0, reply.bottom()),
                        ),
                        0.0,
                        ctx.pal.guide,
                    ),
                );
            }
        });

        if !collapsed {
            stack.extend(node.children.iter().rev().copied());
        }
    }
}

/// The collapse control, painted rather than typeset.
///
/// A glyph would be simpler, but egui's embedded fonts are not guaranteed to carry any
/// particular triangle (`▸` and `▾` both come out as tofu), and a reader that renders a box
/// where its only structural control should be is broken in a way no test would catch. A shape
/// is font-independent and scales with the reading size.
fn disclosure(ui: &mut Ui, collapsed: bool, size: f32, color: egui::Color32) -> egui::Response {
    let side = size * 0.62;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::click());
    let c = rect.center();
    let r = side * 0.34;
    let points = if collapsed {
        vec![
            egui::pos2(c.x - r * 0.7, c.y - r),
            egui::pos2(c.x - r * 0.7, c.y + r),
            egui::pos2(c.x + r, c.y),
        ]
    } else {
        vec![
            egui::pos2(c.x - r, c.y - r * 0.7),
            egui::pos2(c.x + r, c.y - r * 0.7),
            egui::pos2(c.x, c.y + r),
        ]
    };
    let color = if resp.hovered() {
        color.gamma_multiply(1.6)
    } else {
        color
    };
    ui.painter().add(egui::Shape::convex_polygon(
        points,
        color,
        egui::Stroke::NONE,
    ));
    resp.on_hover_text(if collapsed {
        "Expand (z)"
    } else {
        "Collapse (z)"
    })
}

fn is_collapsed(tree: &ThreadTree, n: usize, ctx: &RenderCtx<'_>) -> bool {
    let key = &tree.nodes[n].key;
    // The set holds *toggles*, so a node that auto-collapses is expanded by being in it and a
    // node that does not is collapsed by being in it. One set, both directions, and no need to
    // seed it on every navigation.
    ctx.collapsed.contains(key) != tree.starts_collapsed(n)
}

fn header(
    ui: &mut Ui,
    comment: &ir::Comment,
    tree: &ThreadTree,
    n: usize,
    collapsed: bool,
    ctx: &mut RenderCtx<'_>,
) {
    let node = &tree.nodes[n];
    let pal = ctx.pal;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = super::theme::snap(ctx.opts.base_size_pt * 0.35);
        if !node.children.is_empty() {
            let resp = disclosure(ui, collapsed, ctx.opts.base_size_pt, pal.dim);
            if resp.clicked() {
                ctx.act(Action::ToggleComment(node.key.clone()));
            }
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if resp.has_focus() {
                ctx.focus_comment = Some(node.key.clone());
            }
        }
        ui.label(
            RichText::new(comment.author.as_deref().unwrap_or("anon"))
                .color(pal.strong)
                .small(),
        );
        if let Some(ts) = comment
            .timestamp
            .as_deref()
            .filter(|t| !t.trim().is_empty())
        {
            ui.label(RichText::new(ts).color(pal.dim).small());
        }
        if collapsed && node.subtree_count > 1 {
            let hidden = node.subtree_count - 1;
            let word = if hidden == 1 { "reply" } else { "replies" };
            ui.label(
                RichText::new(format!("({hidden} {word} hidden)"))
                    .color(pal.dim)
                    .small(),
            );
        }
    });
}
