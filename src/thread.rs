//! Thread extraction — the second content algorithm.
//!
//! Article extraction picks the single best subtree. A discussion has no single best
//! subtree: the content is N sibling subtrees, and scoring will happily return one comment
//! out of eighty-eight. Phase 0 measured 8/8 forum pages broken and 5.6% of content
//! recovered under article extraction alone. See docs/phase0-findings.md.
//!
//! The approach here is structural rather than heuristic: find the largest set of sibling
//! elements sharing a class signature, and treat each as a post.

use crate::ir::{Block, Comment, Inline};
use ego_tree::NodeRef;
use scraper::{ElementRef, Html, Node};
use std::collections::HashMap;
use url::Url;

/// Minimum sibling repetitions before a group is considered a comment list.
const MIN_SIBLINGS: usize = 3;
/// Minimum median text per post, to reject navigation and tag-cloud repetition.
const MIN_MEDIAN_TEXT: usize = 40;

const AUTHOR_HINT: &[&str] = &["hnuser", "author", "username", "user-name", "byline", "poster"];
const TIME_HINT: &[&str] = &["age", "date", "time", "timestamp", "created"];
const BODY_HINT: &[&str] = &["commtext", "comment-body", "usertext-body", "post-body", "md", "entry-content", "message"];

pub fn extract_thread(html: &Html, base: &Url) -> Option<Vec<Comment>> {
    let group = best_group(html)?;
    let mut comments: Vec<Comment> = Vec::new();
    for el in &group {
        let body = body_of(el, base);
        if crate::html_inlines_len(&body) == 0 && body.is_empty() {
            continue;
        }
        comments.push(Comment {
            author: find_hint(el, AUTHOR_HINT),
            timestamp: find_hint(el, TIME_HINT),
            depth: depth_of(el, &group),
            id: el.value().id().map(str::to_owned),
            blocks: body,
        });
    }
    (comments.len() >= MIN_SIBLINGS).then_some(comments)
}

/// Normalise a class list into a stable signature, dropping per-item noise
/// (`c00`, `depth-3`, hashed module names) that would otherwise split a group.
fn signature(el: &ElementRef) -> String {
    let mut classes: Vec<&str> = el
        .value()
        .attr("class")
        .unwrap_or("")
        .split_whitespace()
        .filter(|c| {
            c.len() > 1
                && !c.chars().any(|ch| ch.is_ascii_digit())
                && c.len() < 30
        })
        .collect();
    classes.sort_unstable();
    classes.dedup();
    format!("{}.{}", el.value().name(), classes.join("."))
}

/// The largest set of same-signature siblings that looks like posts rather than chrome.
fn best_group<'a>(html: &'a Html) -> Option<Vec<ElementRef<'a>>> {
    let mut groups: HashMap<(ego_tree::NodeId, String), Vec<ElementRef<'a>>> = HashMap::new();
    for node in html.tree.root().descendants() {
        let Some(el) = ElementRef::wrap(node) else { continue };
        let Some(parent) = node.parent() else { continue };
        let sig = signature(&el);
        // A bare tag with no classes is too weak a signal — every <li> on the page would match.
        if sig.ends_with('.') {
            continue;
        }
        groups.entry((parent.id(), sig)).or_default().push(el);
    }

    groups
        .into_values()
        .filter(|g| g.len() >= MIN_SIBLINGS)
        .filter_map(|g| {
            let mut lens: Vec<usize> = g.iter().map(|e| text_len(**e)).collect();
            lens.sort_unstable();
            let median = lens[lens.len() / 2];
            if median < MIN_MEDIAN_TEXT {
                return None;
            }
            // Decisive test: posts carry an author or a timestamp; article paragraphs do not.
            // Without this, a run of sibling <p> tags in a blog post is mistaken for a thread
            // and the article is rendered as a list of comments by "anon".
            let attributed = g
                .iter()
                .filter(|e| find_hint(e, AUTHOR_HINT).is_some() || find_hint(e, TIME_HINT).is_some())
                .count();
            if attributed * 2 < g.len() {
                return None;
            }
            let total: usize = lens.iter().sum();
            Some((g, total))
        })
        .max_by_key(|(_, total)| *total)
        .map(|(g, _)| g)
}

/// Reply nesting. HN encodes depth in an `indent` attribute on a spacer cell; nested markup
/// (Discourse, Reddit) encodes it by containment. Support both.
fn depth_of(el: &ElementRef, group: &[ElementRef]) -> u16 {
    for node in el.descendants() {
        if let Some(e) = ElementRef::wrap(node) {
            if let Some(v) = e.value().attr("indent").and_then(|v| v.parse::<u16>().ok()) {
                return v;
            }
        }
    }
    let ids: Vec<_> = group.iter().map(|g| g.id()).collect();
    let mut depth = 0u16;
    let mut cur = el.parent();
    while let Some(n) = cur {
        if ids.contains(&n.id()) {
            depth += 1;
        }
        cur = n.parent();
    }
    depth
}

fn find_hint(el: &ElementRef, hints: &[&str]) -> Option<String> {
    for node in el.descendants() {
        let Some(e) = ElementRef::wrap(node) else { continue };
        let attrs = format!(
            "{} {} {}",
            e.value().attr("class").unwrap_or(""),
            e.value().id().unwrap_or(""),
            e.value().name()
        )
        .to_lowercase();
        if hints.iter().any(|h| attrs.contains(h)) {
            let t = text_of(node);
            if !t.is_empty() && t.len() < 120 {
                return Some(t);
            }
        }
    }
    None
}

/// The post body: a hinted container if present, else the largest text-bearing descendant.
fn body_of(el: &ElementRef, base: &Url) -> Vec<Block> {
    for node in el.descendants() {
        let Some(e) = ElementRef::wrap(node) else { continue };
        let class = e.value().attr("class").unwrap_or("").to_lowercase();
        if BODY_HINT.iter().any(|h| class.split_whitespace().any(|c| c == *h)) {
            return crate::html::blocks_from_public(e, base);
        }
    }
    // No hint: take the element itself, minus the metadata line.
    crate::html::blocks_from_public(*el, base)
}

fn text_of(node: NodeRef<'_, Node>) -> String {
    let mut s = String::new();
    collect(node, &mut s);
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn text_len(node: NodeRef<'_, Node>) -> usize {
    text_of(node).len()
}

fn collect(node: NodeRef<'_, Node>, out: &mut String) {
    match node.value() {
        Node::Text(t) => out.push_str(t),
        Node::Element(e) if matches!(e.name.local.as_ref(), "script" | "style" | "svg") => {}
        _ => node.children().for_each(|c| collect(c, out)),
    }
}

/// Sum of text in a block list, for deciding thread-vs-article.
pub fn comments_text_len(cs: &[Comment]) -> usize {
    cs.iter()
        .map(|c| {
            crate::ir::Document {
                url: String::new(), title: None, byline: None, published: None,
                site_name: None, lang: None, blocks: c.blocks.clone(),
            }
            .text_len()
        })
        .sum()
}

#[allow(dead_code)]
fn unused(_: Inline) {}
