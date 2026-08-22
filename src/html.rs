//! HTML -> [`Document`]. Parse at full fidelity, then map down to the IR.
//!
//! The subset lives *here*, downstream of the parser, not in the parser. html5ever
//! (via `scraper`) handles the spec's error recovery, foster parenting, and adoption agency;
//! this module then discards everything outside the IR's vocabulary.
//!
//! Scoring is Readability-shaped, with trafilatura's bias toward recall. Phase 0 measured
//! trafilatura beating readability-lxml on 21 of 49 contested pages, so it is the reference.

use crate::ir::{Block, Document, Image, Inline};
use scraper::{ElementRef, Html, Node, Selector};
use std::collections::HashMap;
use url::Url;

use ego_tree::NodeRef;

/// Elements that never contain reading material.
const NOISE: &[&str] = &[
    "script",
    "style",
    "noscript",
    "svg",
    "template",
    "head",
    "nav",
    "footer",
    "header",
    "aside",
    "form",
    "button",
    "select",
    "textarea",
    "iframe",
    "canvas",
    "figure_ad",
];

/// Class/id substrings that mark chrome rather than content.
///
/// Deliberately **not** included: `"comment"`. Readability strips it to clean up articles,
/// but on a thread page `<div class="comment">` *is* the content; stripping it is why both
/// trafilatura and the first cut of this extractor returned pure usernames and timestamps.
/// Comment-stripping belongs to the article path only, never to the parser.
const NOISE_HINT: &[&str] = &[
    "nav",
    "menu",
    "sidebar",
    "footer",
    "header",
    "share",
    "social",
    "related",
    "promo",
    "advert",
    "subscribe",
    "newsletter",
    "cookie",
    "banner",
    "breadcrumb",
    "pagination",
];

/// Elements that open a new block context. Used to decide whether a `<div>` is a leaf of
/// prose (a paragraph candidate) or merely a container of other blocks.
const BLOCK_LEVEL: &[&str] = &[
    "p",
    "div",
    "section",
    "article",
    "ul",
    "ol",
    "li",
    "table",
    "tr",
    "td",
    "th",
    "pre",
    "blockquote",
    "figure",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "footer",
    "nav",
    "aside",
    "main",
    "form",
    "dl",
    "dd",
    "dt",
];

/// Debug aid: which element did content selection choose, and how big is it?
pub fn debug_root(source: &str) -> String {
    let html = Html::parse_document(source);
    match content_root(&html) {
        None => "NONE".to_string(),
        Some(el) => format!(
            "<{} id={:?} class={:?}> text={} link_density={:.2} children={}",
            el.value().name(),
            el.value().id().unwrap_or("-"),
            el.value().attr("class").unwrap_or("-"),
            text_len(*el),
            link_density(&el),
            el.children().count()
        ),
    }
}

pub fn extract(source: &str, url: &Url) -> Document {
    let html = Html::parse_document(source);
    let mut doc = Document {
        url: url.to_string(),
        title: meta_title(&html),
        byline: meta_attr(&html, "author").or_else(|| meta_prop(&html, "article:author")),
        published: meta_prop(&html, "article:published_time").or_else(|| meta_attr(&html, "date")),
        site_name: meta_prop(&html, "og:site_name"),
        lang: html.root_element().value().attr("lang").map(str::to_owned),
        blocks: Vec::new(),
    };
    if let Some(root) = content_root(&html) {
        doc.blocks = blocks_from(root, url);
    }
    // A discussion is not a subtree, so it gets its own algorithm. When the thread dominates
    // the page it *is* the document; when it merely accompanies an article, it is appended.
    if let Some(comments) = crate::thread::extract_thread(&html, url) {
        let tlen = crate::thread::comments_text_len(&comments);
        if tlen > doc.text_len() {
            doc.blocks = vec![Block::Thread(comments)];
        } else if tlen > crate::ir::THIN_TEXT {
            doc.blocks.push(Block::Thread(comments));
        }
    }

    // Scoring finds nothing on pages built entirely from unlabelled divs (product grids,
    // listing pages). Falling back to <body> bleeds navigation, but a thin page beats a
    // blank one, and the renderer can still be read.
    if doc.text_len() < crate::ir::THIN_TEXT
        && let Ok(sel) = Selector::parse("body")
        && let Some(body) = html.select(&sel).next()
    {
        let fallback = blocks_from(body, url);
        if block_text(&fallback) > doc.text_len() {
            doc.blocks = fallback;
        }
    }
    doc
}

/// One of the three shared text-length counters (see `AGENTS.md`). This used to build a
/// throwaway `Document` around a cloned block list, once per `content_root` candidate and
/// again on the `<body>` fallback: up to six copies of a whole document per page.
fn block_text(blocks: &[Block]) -> usize {
    crate::ir::blocks_text_len(blocks)
}

/// Text belonging to this element directly, stopping at nested block containers.
/// Distinguishes `<div>Some prose</div>` (a paragraph) from `<div><p>..</p><p>..</p></div>`
/// (a container). This is the equivalent of Readability's div-to-p promotion.
fn own_text(node: NodeRef<'_, Node>) -> String {
    let mut out = String::new();
    own_text_into(node, &mut out, true);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Iterative for the same reason as [`collect_text`]. The `root` flag rides on the stack.
fn own_text_into(node: NodeRef<'_, Node>, out: &mut String, root: bool) {
    let mut stack = vec![(node, root)];
    while let Some((n, is_root)) = stack.pop() {
        match n.value() {
            Node::Text(t) => out.push_str(t),
            Node::Element(e) => {
                let name = e.name.local.as_ref();
                if NOISE.contains(&name) {
                    continue;
                }
                if !is_root && BLOCK_LEVEL.contains(&name) {
                    continue; // a nested block owns its own text
                }
                stack.extend(n.children().rev().map(|c| (c, false)));
            }
            _ => {}
        }
    }
}

// ---------- content selection ----------

/// Semantic containers *compete with* density scoring rather than overriding it.
///
/// Trusting `<article>` blindly picks the author-bio card on a blog whose real body sits in a
/// `<section>`, and in one measured case that section additionally contained a complete nested
/// `<!DOCTYPE html><html><body>`, which html5ever recovers from per spec. Whichever candidate
/// carries more non-link text wins.
fn content_root(html: &Html) -> Option<ElementRef<'_>> {
    let mut candidates: Vec<ElementRef<'_>> = Vec::new();
    for sel in ["article", "main", "[role=main]"] {
        let Ok(selector) = Selector::parse(sel) else {
            continue;
        };
        candidates.extend(html.select(&selector));
    }
    candidates.extend(score_best(html));
    let mut ranked: Vec<(ElementRef<'_>, f64)> = candidates
        .into_iter()
        .map(|el| (el, text_len(*el) as f64 * (1.0 - link_density(&el))))
        .filter(|(_, s)| *s > 200.0)
        .collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    ranked.dedup_by_key(|(el, _)| el.id());
    ranked.truncate(5);
    // Rank by what the block walker actually *emits*, not by raw text. A container can be
    // dense with text yet yield almost nothing once chrome and empty wrappers are dropped,
    // which is how a newsletter post lost 90% of its body to a higher-scoring ancestor.
    ranked
        .into_iter()
        .max_by_key(|(el, _)| {
            let base = Url::parse("https://x.invalid/").unwrap();
            block_text(&blocks_from(*el, &base))
        })
        .map(|(el, _)| el)
}

/// Readability-style: score leaf text blocks, propagate to ancestors, and discount link density.
fn score_best(html: &Html) -> Option<ElementRef<'_>> {
    let mut scores: HashMap<ego_tree::NodeId, f64> = HashMap::new();
    // `div`/`section` are included because most modern pages never emit a `<p>`. A container
    // only counts when the prose is its *own* (see `own_text`).
    let sel = Selector::parse("p, td, pre, blockquote, li, div, section, dd").ok()?;

    for el in html.select(&sel) {
        let text = own_text(*el);
        if text.len() < 25 {
            continue;
        }
        let base = 1.0 + text.matches(',').count() as f64 + (text.len() as f64 / 100.0).min(3.0);
        // Credit the parent fully and the grandparent at half: content lives in containers,
        // not in the paragraphs themselves.
        let mut node = el.parent();
        let mut decay = 1.0;
        for _ in 0..3 {
            let Some(n) = node else { break };
            if let Some(e) = ElementRef::wrap(n)
                && !is_noise(&e)
            {
                *scores.entry(n.id()).or_insert(0.0) += base * decay;
            }
            decay /= 2.0;
            node = n.parent();
        }
    }

    scores
        .into_iter()
        .filter_map(|(id, s)| {
            let node = html.tree.get(id)?;
            let el = ElementRef::wrap(node)?;
            Some((el, s * (1.0 - link_density(&el))))
        })
        .filter(|(_, s)| *s > 0.0)
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(el, _)| el)
}

fn is_noise(el: &ElementRef) -> bool {
    let v = el.value();
    if NOISE.contains(&v.name()) {
        return true;
    }
    let hint = format!("{} {}", v.id().unwrap_or(""), v.attr("class").unwrap_or("")).to_lowercase();
    !hint.trim().is_empty() && NOISE_HINT.iter().any(|h| hint.contains(h))
}

// ---------- text measurement ----------

fn inner_text(node: NodeRef<'_, Node>) -> String {
    let mut out = String::new();
    collect_text(node, &mut out);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Iterative, like [`crate::reader::thread_tree`]'s traversal and for the same reason: the
/// input is untrusted, a recursive walk costs a stack frame per DOM level, and a stack
/// overflow is an abort that no guard in the reader can contain. See [`MAX_DEPTH`] for the
/// walkers that cannot be flattened this cheaply.
fn collect_text(node: NodeRef<'_, Node>, out: &mut String) {
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        match n.value() {
            Node::Text(t) => out.push_str(t),
            Node::Element(e) if NOISE.contains(&e.name.local.as_ref()) => {}
            _ => stack.extend(n.children().rev()),
        }
    }
}

fn text_len(node: NodeRef<'_, Node>) -> usize {
    inner_text(node).len()
}

/// Fraction of an element's text that sits inside anchors. High density means navigation.
fn link_density(el: &ElementRef) -> f64 {
    let total = text_len(**el);
    if total == 0 {
        return 0.0;
    }
    let Ok(sel) = Selector::parse("a") else {
        return 0.0;
    };
    let linked: usize = el.select(&sel).map(|a| text_len(*a)).sum();
    (linked as f64 / total as f64).min(1.0)
}

// ---------- IR construction ----------

/// How far [`walk_blocks`] and [`inlines_from`] will descend.
///
/// Both build a *tree*, so neither flattens into an explicit stack the way [`collect_text`]
/// does, and both cost a stack frame per DOM level. Untrusted input reaches them directly:
/// roughly 2,000 nested `<div>`s (about 22 KB, three orders of magnitude under the 5 MB
/// transfer cap) overflowed the 2 MiB stack of a `reader::ui::net` worker and aborted the
/// process. An abort is not a panic, so `ReplyGuard` cannot turn it into a failed tab.
///
/// 256 is far past any real document (deep-but-sane pages measure under 100) and leaves the
/// worker stack an order of magnitude of headroom even when the two walkers interleave.
/// Content below the limit is not dropped (it is flattened to text), so the cap degrades a
/// pathological page rather than blanking it.
const MAX_DEPTH: usize = 256;

/// Public entry point for the thread extractor, which builds blocks for one post.
pub fn blocks_from_public(root: ElementRef<'_>, base: &Url) -> Vec<Block> {
    blocks_from(root, base)
}

fn blocks_from(root: ElementRef<'_>, base: &Url) -> Vec<Block> {
    let mut out = Vec::new();
    walk_blocks(*root, base, &mut out, 0);
    out
}

fn walk_blocks(node: NodeRef<'_, Node>, base: &Url, out: &mut Vec<Block>, depth: usize) {
    if depth >= MAX_DEPTH {
        // Keep the text rather than the structure; see `MAX_DEPTH`.
        let text = inner_text(node);
        if text.len() > 40 {
            out.push(Block::Paragraph(vec![Inline::Text(text)]));
        }
        return;
    }
    for child in node.children() {
        let Some(el) = ElementRef::wrap(child) else {
            // Bare text directly under a container still counts as a paragraph.
            if let Node::Text(t) = child.value() {
                let s = t.trim();
                if s.len() > 40 {
                    out.push(Block::Paragraph(vec![Inline::Text(s.to_string())]));
                }
            }
            continue;
        };
        if is_noise(&el) {
            continue;
        }
        match el.value().name() {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let level = el.value().name().as_bytes()[1] - b'0';
                let inl = inlines_from(child, base, depth + 1);
                if !inl.is_empty() {
                    out.push(Block::Heading {
                        level,
                        inlines: inl,
                    });
                }
            }
            "p" => {
                let inl = inlines_from(child, base, depth + 1);
                if !inl.is_empty() {
                    out.push(Block::Paragraph(inl));
                }
            }
            "ul" | "ol" => {
                let ordered = el.value().name() == "ol";
                let li = Selector::parse("li").unwrap();
                let items: Vec<Vec<Block>> = el
                    .select(&li)
                    .filter(|l| l.parent().map(|p| p.id()) == Some(child.id()))
                    .map(|l| {
                        let mut b = Vec::new();
                        walk_blocks(*l, base, &mut b, depth + 2);
                        if b.is_empty() {
                            let inl = inlines_from(*l, base, depth + 2);
                            if !inl.is_empty() {
                                b.push(Block::Paragraph(inl));
                            }
                        }
                        b
                    })
                    .filter(|b| !b.is_empty())
                    .collect();
                if !items.is_empty() {
                    out.push(Block::List { ordered, items });
                }
            }
            "blockquote" => {
                let mut b = Vec::new();
                walk_blocks(child, base, &mut b, depth + 1);
                if !b.is_empty() {
                    out.push(Block::Quote {
                        blocks: b,
                        cite: el.value().attr("cite").map(str::to_owned),
                    });
                }
            }
            "pre" => {
                let text = inner_text_raw(child);
                if !text.trim().is_empty() {
                    out.push(Block::Code {
                        lang: code_lang(&el),
                        text,
                    });
                }
            }
            "hr" => out.push(Block::Rule),
            "img" => {
                if let Some(img) = image_from(&el, base) {
                    out.push(Block::Figure {
                        image: img,
                        caption: None,
                    });
                }
            }
            "figure" => {
                let img_sel = Selector::parse("img").unwrap();
                let cap_sel = Selector::parse("figcaption").unwrap();
                if let Some(img) = el
                    .select(&img_sel)
                    .next()
                    .and_then(|i| image_from(&i, base))
                {
                    let caption = el
                        .select(&cap_sel)
                        .next()
                        .map(|c| inlines_from(*c, base, depth + 2))
                        .filter(|c| !c.is_empty());
                    out.push(Block::Figure {
                        image: img,
                        caption,
                    });
                }
            }
            "table" => {
                if let Some(t) = table_from(&el, base, depth) {
                    out.push(t);
                }
            }
            // Containers: recurse. Inline-level leftovers get gathered as a paragraph.
            _ => {
                let before = out.len();
                walk_blocks(child, base, out, depth + 1);
                if out.len() == before {
                    let inl = inlines_from(child, base, depth + 1);
                    if inlines_len(&inl) > 40 {
                        out.push(Block::Paragraph(inl));
                    }
                }
            }
        }
    }
}

fn inlines_len(v: &[Inline]) -> usize {
    v.iter()
        .map(|i| match i {
            Inline::Text(s) | Inline::Code(s) => s.len(),
            Inline::Emph(x) | Inline::Strong(x) => inlines_len(x),
            Inline::Link { inlines, .. } => inlines_len(inlines),
            _ => 0,
        })
        .sum()
}

fn inlines_from(node: NodeRef<'_, Node>, base: &Url, depth: usize) -> Vec<Inline> {
    let mut out = Vec::new();
    if depth >= MAX_DEPTH {
        // Keep the text rather than the styling; see `MAX_DEPTH`.
        let text = inner_text(node);
        if !text.is_empty() {
            out.push(Inline::Text(text));
        }
        return out;
    }
    for child in node.children() {
        match child.value() {
            Node::Text(t) => {
                let s = squeeze(&t.replace(['\n', '\t'], " "));
                if s.trim().is_empty() {
                    // A whitespace-only node *between* two inline elements is a word
                    // boundary, and dropping it welds them together:
                    // `<a>written language</a> <a>legible</a>` became "written
                    // languagelegible". Markup indentation produces these constantly.
                    if !s.is_empty() {
                        push_space(&mut out);
                    }
                } else {
                    out.push(Inline::Text(s));
                }
            }
            Node::Element(e) => {
                let Some(el) = ElementRef::wrap(child) else {
                    continue;
                };
                if is_noise(&el) {
                    continue;
                }
                match e.name.local.as_ref() {
                    "em" | "i" => out.push(Inline::Emph(inlines_from(child, base, depth + 1))),
                    "strong" | "b" => {
                        out.push(Inline::Strong(inlines_from(child, base, depth + 1)))
                    }
                    "code" | "kbd" | "samp" => out.push(Inline::Code(inner_text(child))),
                    "br" => out.push(Inline::Break),
                    "img" => {
                        if let Some(i) = image_from(&el, base) {
                            out.push(Inline::Image(i));
                        }
                    }
                    "a" => {
                        let inl = inlines_from(child, base, depth + 1);
                        match el.value().attr("href").and_then(|h| base.join(h).ok()) {
                            Some(href) if !inl.is_empty() => out.push(Inline::Link {
                                href: href.to_string(),
                                inlines: inl,
                            }),
                            _ => out.extend(inl),
                        }
                    }
                    _ => out.extend(inlines_from(child, base, depth + 1)),
                }
            }
            _ => {}
        }
    }
    out
}

/// Add a separating space, unless we are at the start or the previous inline already ends in
/// one. Never two spaces, and never a leading one.
fn push_space(out: &mut Vec<Inline>) {
    match out.last_mut() {
        None => {}
        Some(Inline::Text(prev)) => {
            if !prev.ends_with(' ') {
                prev.push(' ');
            }
        }
        Some(_) => out.push(Inline::Text(" ".to_owned())),
    }
}

fn squeeze(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for c in s.chars() {
        let is_space = c.is_whitespace();
        if !(is_space && prev_space) {
            out.push(if is_space { ' ' } else { c });
        }
        prev_space = is_space;
    }
    out
}

/// `<pre>` must keep its newlines: that is the entire point of `<pre>`.
fn inner_text_raw(node: NodeRef<'_, Node>) -> String {
    let mut s = String::new();
    collect_text(node, &mut s);
    s
}

/// Language hint for a `<pre>`. Highlighters put the class on the inner `<code>`
/// (`<pre><code class="language-rust">`), so check descendants, not just the `<pre>` itself.
fn code_lang(el: &ElementRef) -> Option<String> {
    el.descendants()
        .filter_map(ElementRef::wrap)
        .filter_map(|e| e.value().attr("class").map(str::to_owned))
        .find_map(|class| {
            class.split_whitespace().find_map(|c| {
                c.strip_prefix("language-")
                    .or_else(|| c.strip_prefix("lang-"))
                    .map(str::to_owned)
            })
        })
}

fn image_from(el: &ElementRef, base: &Url) -> Option<Image> {
    let raw = el
        .value()
        .attr("src")
        .or_else(|| el.value().attr("data-src"))
        .or_else(|| el.value().attr("data-original"))?;
    let src = base.join(raw).ok()?;
    Some(Image {
        src: src.to_string(),
        alt: el
            .value()
            .attr("alt")
            .filter(|a| !a.trim().is_empty())
            .map(str::to_owned),
    })
}

/// A row is a *header* row only when every cell it owns is a `<th>`.
///
/// Splitting on "does this row contain any `<th>`" instead loses data both ways on the
/// row-header tables that infoboxes and spec sheets are built from: the first
/// `<tr><th>Name</th><td>Alice</td></tr>` contributed `Name` as the only header and dropped
/// `Alice`, and every row after it contributed its `<td>`s and dropped its `<th>`. Cells are
/// collected in document order with one selector so a mixed row keeps its shape.
fn table_from(el: &ElementRef, base: &Url, depth: usize) -> Option<Block> {
    let tr = Selector::parse("tr").ok()?;
    let cell = Selector::parse("th, td").ok()?;
    let mut headers: Vec<Vec<Inline>> = Vec::new();
    let mut rows: Vec<Vec<Vec<Inline>>> = Vec::new();
    // Descendant selectors reach *through* nested tables, so every cell of an inner table
    // would be emitted once for each enclosing table. Keep only cells this table owns.
    for row in el
        .select(&tr)
        .filter(|r| owning_table(**r) == Some(el.id()))
    {
        let cells: Vec<(bool, Vec<Inline>)> = row
            .select(&cell)
            .filter(|c| owning_row(**c) == Some(row.id()))
            .map(|c| (c.value().name() == "th", inlines_from(*c, base, depth + 1)))
            .collect();
        if cells.is_empty() {
            continue;
        }
        // Headers come first or not at all: an all-`<th>` row further down is a repeated
        // header or a footer, and keeping it as data is what stops it disappearing.
        if headers.is_empty() && rows.is_empty() && cells.iter().all(|(is_th, _)| *is_th) {
            headers = cells.into_iter().map(|(_, c)| c).collect();
            continue;
        }
        rows.push(cells.into_iter().map(|(_, c)| c).collect());
    }
    (!rows.is_empty()).then_some(Block::Table { headers, rows })
}

/// Id of the nearest ancestor `<table>`, or None.
fn owning_table(node: NodeRef<'_, Node>) -> Option<ego_tree::NodeId> {
    nearest_ancestor(node, &["table"])
}

/// Id of the nearest ancestor `<tr>`, or None.
fn owning_row(node: NodeRef<'_, Node>) -> Option<ego_tree::NodeId> {
    nearest_ancestor(node, &["tr"])
}

fn nearest_ancestor(node: NodeRef<'_, Node>, names: &[&str]) -> Option<ego_tree::NodeId> {
    let mut cur = node.parent();
    while let Some(n) = cur {
        if let Node::Element(e) = n.value()
            && names.contains(&e.name.local.as_ref())
        {
            return Some(n.id());
        }
        cur = n.parent();
    }
    None
}

// ---------- metadata ----------

fn meta_title(html: &Html) -> Option<String> {
    meta_prop(html, "og:title").or_else(|| {
        let sel = Selector::parse("title").ok()?;
        html.select(&sel)
            .next()
            .map(|e| inner_text(*e))
            .filter(|s| !s.is_empty())
    })
}

fn meta_prop(html: &Html, prop: &str) -> Option<String> {
    let sel = Selector::parse(&format!(r#"meta[property="{prop}"]"#)).ok()?;
    html.select(&sel)
        .next()?
        .value()
        .attr("content")
        .map(str::to_owned)
}

fn meta_attr(html: &Html, name: &str) -> Option<String> {
    let sel = Selector::parse(&format!(r#"meta[name="{name}"]"#)).ok()?;
    html.select(&sel)
        .next()?
        .value()
        .attr("content")
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::Block;

    fn doc(html: &str) -> Document {
        extract(html, &Url::parse("https://example.test/a/b").unwrap())
    }

    #[test]
    fn maps_basic_prose_to_ir() {
        let d = doc(r#"<html><body><article>
            <h2>Title</h2><p>Hello <em>there</em> and <strong>welcome</strong> to the page today.</p>
            <ul><li>one item here</li><li>two items here</li></ul>
            <pre><code class="language-rust">fn main() {}</code></pre>
        </article></body></html>"#);
        assert!(matches!(d.blocks[0], Block::Heading { level: 2, .. }));
        assert!(matches!(d.blocks[1], Block::Paragraph(_)));
        match &d.blocks[2] {
            Block::List { ordered, items } => {
                assert!(!ordered);
                assert_eq!(items.len(), 2)
            }
            b => panic!("expected list, got {b:?}"),
        }
        match &d.blocks[3] {
            Block::Code { lang, text } => {
                assert_eq!(lang.as_deref(), Some("rust"));
                assert!(text.contains("fn main"));
            }
            b => panic!("expected code, got {b:?}"),
        }
    }

    #[test]
    fn resolves_relative_urls_against_base() {
        let d = doc(
            r#"<article><p>text long enough to survive the length filter here</p>
                       <img src="../img/x.png" alt="pic"></article>"#,
        );
        let found = d.blocks.iter().any(|b| {
            matches!(b,
            Block::Figure { image, .. } if image.src == "https://example.test/img/x.png")
        });
        assert!(
            found,
            "relative image src should resolve against the document URL"
        );
    }

    /// Regression: `comment` was in NOISE_HINT, which deleted every comment body on thread
    /// pages. Both trafilatura and the first cut of this extractor returned only usernames
    /// and timestamps on Hacker News. See docs/phase0-findings.md.
    /// Markup indentation puts a whitespace-only text node between adjacent inline elements
    /// constantly, and dropping it welds two words into one.
    #[test]
    fn whitespace_between_inline_elements_is_a_word_boundary() {
        let html = r#"<html><body><article><p>make <a href="/a">written language</a>
            <a href="/b">legible</a>, <em>readable</em> <strong>and</strong> appealing when
            displayed on a screen that is wide enough to need a paragraph of prose.</p>
            </article></body></html>"#;
        let doc = extract(html, &Url::parse("https://example.com/").unwrap());
        let text = crate::render::to_text(&doc, &crate::render::TextOpts::default());
        assert!(text.contains("written language legible"), "got: {text}");
        assert!(text.contains("_readable_ *and* appealing"), "got: {text}");
        // And no doubled or leading spaces from the indentation itself.
        assert!(!text.contains("  "), "got: {text}");
    }

    #[test]
    fn comment_class_is_not_treated_as_chrome() {
        let d = doc(
            r#"<article><div class="comment"><p>This is the substance of a reply that
            must survive extraction intact, because on a thread page it is the content.</p>
            </div></article>"#,
        );
        assert!(
            d.text_len() > 50,
            "comment body was stripped as chrome: {:?}",
            d.blocks
        );
    }

    /// html5ever earns its place here: this is malformed in three ways at once.
    #[test]
    fn survives_malformed_markup() {
        let d = doc(
            r#"<html><body><article><p>First paragraph with enough text to count.
            <p>Second paragraph, previous never closed, still needs to be recovered properly.
            <table><tr><td>cell</td></table>
            <b><i>misnested</b></i>
            </article></body></html>"#,
        );
        assert!(
            d.text_len() > 80,
            "error recovery lost content: {:?}",
            d.blocks
        );
    }

    #[test]
    fn nested_document_does_not_lose_body() {
        // One measured site embeds a whole <!DOCTYPE html><html><body> inside a <section>.
        let d = doc(
            r#"<body><section class="post__content"><!DOCTYPE html><html><body>
            <p>The real article body lives inside a nested document and must still be found.</p>
            </body></html></section></body>"#,
        );
        assert!(
            d.text_len() > 40,
            "nested document swallowed the body: {:?}",
            d.blocks
        );
    }

    /// A table-layout discussion page: reply depth in an `indent` attribute on a spacer cell,
    /// author and timestamp in a header span, body in a hinted div. Article extraction returns
    /// one post out of N here; thread extraction must return all of them with depths intact.
    #[test]
    fn table_layout_thread_extracts_all_posts_with_depth() {
        let mut rows = String::new();
        for (i, (who, depth, body)) in [
            (
                "alice",
                0,
                "The opening post of the discussion, written at enough length that it \
             comfortably clears every median and total length filter in the detector.",
            ),
            (
                "bob",
                1,
                "A reply to the opening post, also written at length so that the thread \
             as a whole carries more text than the surrounding page chrome does.",
            ),
            (
                "carol",
                2,
                "A nested reply sitting one level deeper than the reply above it, and \
             again long enough that it will not be discarded as incidental text.",
            ),
        ]
        .iter()
        .enumerate()
        {
            rows.push_str(&format!(
                r#"<tr class="athing comtr" id="p{i}"><td><table><tr>
                     <td class="ind" indent="{depth}"><img src="s.gif"></td>
                     <td class="default">
                       <div><span class="comhead"><a class="hnuser">{who}</a>
                            <span class="age">2 hours ago</span></span></div>
                       <div class="comment"><div class="commtext">{body}</div></div>
                     </td></tr></table></td></tr>"#
            ));
        }
        let d = doc(&format!("<html><body><table>{rows}</table></body></html>"));
        let Some(Block::Thread(comments)) = d.blocks.iter().find(|b| matches!(b, Block::Thread(_)))
        else {
            panic!("no thread extracted: {:?}", d.blocks)
        };
        assert_eq!(
            comments.len(),
            3,
            "expected every post, got {}",
            comments.len()
        );
        assert_eq!(
            comments.iter().map(|c| c.depth).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(comments[0].author.as_deref(), Some("alice"));
        assert!(
            d.text_len() > 400,
            "post bodies were dropped: {}",
            d.text_len()
        );
    }

    /// Regression: sibling `<p>` tags in an article were mistaken for a thread and rendered as
    /// posts by "anon". It *raised* the benchmark score while destroying the document: posts
    /// carry an author or timestamp, article paragraphs do not.
    #[test]
    fn repeated_paragraphs_are_not_a_thread() {
        let ps = (0..6)
            .map(|i| {
                format!(
                    "<p class=\"wp-block-paragraph\">Paragraph {i} of an ordinary article, with \
             plenty of text so it clears every length filter in the detector.</p>"
                )
            })
            .collect::<String>();
        let d = doc(&format!(
            "<html><body><article>{ps}</article></body></html>"
        ));
        assert!(
            !d.blocks.iter().any(|b| matches!(b, Block::Thread(_))),
            "article paragraphs misdetected as a discussion: {:?}",
            d.blocks
        );
    }

    /// Listing and product pages emit no `<p>` at all. Scoring that only considers `p, td, li`
    /// returns nothing on them; a `<div>` holding prose directly must count as a candidate.
    #[test]
    fn div_soup_without_paragraphs_still_extracts() {
        let items = (0..8)
            .map(|i| {
                format!(
                    "<div class=\"item-cell\"><div class=\"item-title\">Item {i} in a listing grid \
             with a description long enough to be worth reading and scoring.</div></div>"
                )
            })
            .collect::<String>();
        let d = doc(&format!(
            "<html><body><div class=\"grid\">{items}</div></body></html>"
        ));
        assert!(
            d.text_len() > 300,
            "div-soup page extracted nothing: {}",
            d.text_len()
        );
    }

    #[test]
    fn ir_carries_no_styling() {
        let d = doc(
            r#"<article><p style="color:red;font-size:40px" class="huge">
            Styling must be discarded at the IR boundary, not carried through it.</p></article>"#,
        );
        let json = serde_json::to_string(&d).unwrap();
        assert!(!json.contains("color"), "style leaked into the IR");
        assert!(!json.contains("font-size"), "style leaked into the IR");
    }

    /// A stack overflow is an abort, not a panic, so no guard in the reader can contain it;
    /// one hostile page would take the whole GUI process with it. Roughly 2,000 nested divs
    /// (about 22 KB, three orders of magnitude under the transfer cap) used to be enough on a
    /// worker's 2 MiB stack, and this is comfortably past that.
    ///
    /// Kept at 5,000 rather than higher because `content_root`'s ancestor scoring is quadratic
    /// in depth: 20,000 takes 15s where 5,000 takes one. That cost is a separate problem from
    /// the abort, and it is bounded by this cap rather than fixed by it.
    #[test]
    fn a_pathologically_nested_page_extracts_instead_of_aborting() {
        const DEPTH: usize = 5_000;
        let mut src = String::from("<html><body><article>");
        src.push_str(&"<div>".repeat(DEPTH));
        src.push_str("the sentence at the bottom of five thousand divs, long enough to keep");
        src.push_str(&"</div>".repeat(DEPTH));
        src.push_str("</article></body></html>");

        // 2 MiB is what `std::thread` gives a `reader::ui::net` worker, which is where
        // extraction actually runs in the GUI.
        let out = std::thread::Builder::new()
            .stack_size(2 * 1024 * 1024)
            .spawn(move || {
                let doc = extract(&src, &Url::parse("https://example.com/a").unwrap());
                doc.text_len()
            })
            .unwrap()
            .join()
            .expect("extraction unwound");
        // Capped, not dropped: the text below the limit is flattened, not discarded.
        assert!(out > 0, "the deep page extracted nothing at all");
    }

    /// Row-header tables (infoboxes, spec sheets) put a `<th>` and a `<td>` in the same row.
    /// Treating "has any `<th>`" as "is the header row" dropped a cell from every line.
    #[test]
    fn a_row_header_table_keeps_every_cell() {
        let src = "<html><body><article>\
            <p>A lead paragraph carrying enough prose that this subtree wins content selection \
               against the bare body fallback, which it must for the table to be reached.</p>\
            <table>\
              <tr><th>Name</th><td>Alice</td></tr>\
              <tr><th>Role</th><td>Engineer</td></tr>\
            </table></article></body></html>";
        let doc = extract(src, &Url::parse("https://example.com/a").unwrap());
        let table = doc
            .blocks
            .iter()
            .find_map(|b| match b {
                Block::Table { headers, rows } => Some((headers, rows)),
                _ => None,
            })
            .expect("no table extracted");
        let (headers, rows) = table;
        let cells: Vec<String> = headers
            .iter()
            .chain(rows.iter().flatten())
            .map(|c| crate::ir::plain_text(c))
            .collect();
        for want in ["Name", "Alice", "Role", "Engineer"] {
            assert!(
                cells.iter().any(|c| c == want),
                "table lost {want:?}: {cells:?}"
            );
        }
        // A mixed row is data, not a header row, so nothing is promoted out of the grid.
        assert!(headers.is_empty(), "mixed rows should not yield headers");
        assert_eq!(rows.len(), 2);
    }

    /// The all-`<th>` case still works, which is the reason the split existed.
    #[test]
    fn an_all_header_first_row_is_still_the_header_row() {
        let src = "<html><body><article>\
            <p>A lead paragraph carrying enough prose that this subtree wins content selection \
               against the bare body fallback, which it must for the table to be reached.</p>\
            <table>\
              <tr><th>Name</th><th>Role</th></tr>\
              <tr><td>Alice</td><td>Engineer</td></tr>\
            </table></article></body></html>";
        let doc = extract(src, &Url::parse("https://example.com/a").unwrap());
        let (headers, rows) = doc
            .blocks
            .iter()
            .find_map(|b| match b {
                Block::Table { headers, rows } => Some((headers, rows)),
                _ => None,
            })
            .expect("no table extracted");
        assert_eq!(headers.len(), 2);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].len(), 2);
    }
}
