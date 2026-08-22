//! HTML -> [`Document`]. Parse at full fidelity, then map down to the IR.
//!
//! The subset lives *here*, downstream of the parser, not in the parser. html5ever
//! (via `scraper`) handles the spec's error recovery, foster parenting, and adoption agency;
//! this module then discards everything outside the IR's vocabulary.
//!
//! Scoring is Readability-shaped, with trafilatura's bias toward recall. Phase 0 measured
//! trafilatura beating readability-lxml on 21 of 49 contested pages, so it is the reference.
//!
//! **A block walk has to gather inline children, not dispatch on them one at a time.** The
//! IR has blocks and it has inlines, and HTML hands you both as siblings under one container.
//! Emitting a block per inline child splits a sentence around its own link; dropping the ones
//! that produced no block loses the link entirely. [`walk_blocks`] therefore accumulates
//! inline children and flushes them as one paragraph when a block child interrupts them, and
//! [`holds_blocks`] decides which a child is by what it contains rather than by its tag name,
//! because `<a>` and `<span>` are each used both ways. That is what makes a front page
//! readable: every headline on one is an anchor, either wrapped around the card's blocks or
//! standing as a run of inline text, and both shapes used to arrive with the href discarded.

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

/// One content-root candidate, as `--why` reports it. Plain data: it outlives the parse.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    /// `"article"`, `"main"`, `"[role=main]"`, or `"scored"` for the density winner.
    pub origin: &'static str,
    pub tag: String,
    pub id: Option<String>,
    pub class: Option<String>,
    /// Visible text under the element, before the walker drops anything.
    pub raw_text: usize,
    pub link_density: f64,
    /// `raw_text * (1 - link_density)`: the first-pass ranking.
    pub score: f64,
    /// Text of the blocks the walker actually emits from it: the ranking that decides.
    pub emitted: usize,
}

/// What the extractor did to one page and why. Built by the same pass that builds the
/// `Document`, so it cannot describe a decision the extractor did not take.
#[derive(Debug, Clone, PartialEq)]
pub struct Explanation {
    pub url: String,
    /// Ranked by `score`, floor applied, at most five: what `content_root` chose among.
    pub candidates: Vec<Candidate>,
    /// Index into `candidates` of the one whose blocks became the document.
    pub winner: Option<usize>,
    pub thread: crate::thread::Verdict,
    /// What the thread merge did: `"replaced the document"`, `"appended"`,
    /// `"dropped (thinner than the floor)"`, or `"no thread"`.
    pub thread_merge: &'static str,
    /// `(field, source, value)`: where each metadata field came from, or `"none"`.
    pub meta: Vec<(&'static str, &'static str, Option<String>)>,
    /// The `<body>` fallback ran and its blocks replaced the result.
    pub body_fallback: bool,
    pub blocks: usize,
    pub text_len: usize,
}

/// Extract, and say why. The CLI's `--why` and `bin/dbg` print this.
pub fn explain(source: &str, url: &Url) -> Explanation {
    extract_traced(source, url).1
}

impl std::fmt::Display for Explanation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "why: {}", self.url)?;
        writeln!(f, "root candidates (ranked by emitted text; * chose):")?;
        if self.candidates.is_empty() {
            writeln!(f, "    none cleared the floor")?;
        }
        for (i, c) in self.candidates.iter().enumerate() {
            let mark = if self.winner == Some(i) { '*' } else { ' ' };
            let id = c.id.as_deref().map(|s| format!("#{s}")).unwrap_or_default();
            let class = c
                .class
                .as_deref()
                .map(|s| format!(".{}", s.split_whitespace().collect::<Vec<_>>().join(".")))
                .unwrap_or_default();
            writeln!(
                f,
                "  {mark} {:<11} <{}{id}{class}> raw={} links={:.2} score={:.0} emitted={}",
                c.origin, c.tag, c.raw_text, c.link_density, c.score, c.emitted
            )?;
        }
        writeln!(f, "thread: {}", self.thread_merge)?;
        for (i, g) in self.thread.groups.iter().enumerate() {
            let mark = if self.thread.chosen == Some(i) {
                '*'
            } else {
                ' '
            };
            writeln!(f, "  {mark} {g}")?;
        }
        writeln!(f, "meta:")?;
        for (field, source, value) in &self.meta {
            match value {
                Some(v) => writeln!(f, "    {field:<9} <- {source}: {v:?}")?,
                None => writeln!(f, "    {field:<9} <- {source}")?,
            }
        }
        writeln!(
            f,
            "body fallback: {}",
            if self.body_fallback {
                "fired"
            } else {
                "not needed"
            }
        )?;
        writeln!(f, "result: {} blocks, {} chars", self.blocks, self.text_len)
    }
}

pub fn extract(source: &str, url: &Url) -> Document {
    extract_traced(source, url).0
}

/// The one extraction pass. [`extract`] keeps the document and [`explain`] keeps the account;
/// neither re-enacts the other's decisions.
fn extract_traced(source: &str, url: &Url) -> (Document, Explanation) {
    let html = Html::parse_document(source);
    let (title, title_src) = first_of([
        ("og:title", meta_prop(&html, "og:title")),
        ("<title>", title_element(&html)),
    ]);
    let (byline, byline_src) = first_of([
        ("meta[name=author]", meta_attr(&html, "author")),
        (
            "meta[property=article:author]",
            meta_prop(&html, "article:author"),
        ),
    ]);
    let (published, published_src) = first_of([
        (
            "meta[property=article:published_time]",
            meta_prop(&html, "article:published_time"),
        ),
        ("meta[name=date]", meta_attr(&html, "date")),
    ]);
    let mut doc = Document {
        url: url.to_string(),
        title,
        byline,
        published,
        site_name: meta_prop(&html, "og:site_name"),
        lang: html.root_element().value().attr("lang").map(str::to_owned),
        blocks: Vec::new(),
    };
    let mut why = Explanation {
        url: url.to_string(),
        candidates: Vec::new(),
        winner: None,
        thread: Default::default(),
        thread_merge: "no thread",
        meta: vec![
            ("title", title_src, doc.title.clone()),
            ("byline", byline_src, doc.byline.clone()),
            ("published", published_src, doc.published.clone()),
        ],
        body_fallback: false,
        blocks: 0,
        text_len: 0,
    };
    let ranked = root_candidates(&html);
    why.winner = choose_root(&ranked);
    why.candidates = ranked.iter().map(|(_, c)| c.clone()).collect();
    if let Some(i) = why.winner {
        doc.blocks = blocks_from(ranked[i].0, url);
    }
    // A discussion is not a subtree, so it gets its own algorithm. When the thread dominates
    // the page it *is* the document; when it merely accompanies an article, it is appended.
    let (thread, verdict) = crate::thread::extract_thread_traced(&html, url);
    why.thread = verdict;
    if let Some(comments) = thread {
        let tlen = crate::thread::comments_text_len(&comments);
        if tlen > doc.text_len() {
            doc.blocks = vec![Block::Thread(comments)];
            why.thread_merge = "replaced the document";
        } else if tlen > crate::ir::THIN_TEXT {
            doc.blocks.push(Block::Thread(comments));
            why.thread_merge = "appended";
        } else {
            why.thread_merge = "dropped (thinner than the floor)";
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
            why.body_fallback = true;
        }
    }
    why.blocks = doc.blocks.len();
    why.text_len = doc.text_len();
    (doc, why)
}

/// The first source that produced a value, and its name; `"none"` when nothing did.
fn first_of<const N: usize>(
    sources: [(&'static str, Option<String>); N],
) -> (Option<String>, &'static str) {
    for (name, value) in sources {
        if value.is_some() {
            return (value, name);
        }
    }
    (None, "none")
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
///
/// The content-root candidates, ranked by first-pass score with the floor applied, each
/// carrying the emitted length that [`choose_root`] decides on.
fn root_candidates(html: &Html) -> Vec<(ElementRef<'_>, Candidate)> {
    let mut candidates: Vec<(ElementRef<'_>, &'static str)> = Vec::new();
    for sel in ["article", "main", "[role=main]"] {
        let Ok(selector) = Selector::parse(sel) else {
            continue;
        };
        candidates.extend(html.select(&selector).map(|el| (el, sel)));
    }
    candidates.extend(score_best(html).map(|el| (el, "scored")));
    let mut ranked: Vec<(ElementRef<'_>, Candidate)> = candidates
        .into_iter()
        .map(|(el, origin)| {
            let raw_text = text_len(*el);
            let link_density = link_density(&el);
            let v = el.value();
            let c = Candidate {
                origin,
                tag: v.name().to_owned(),
                id: v.id().map(str::to_owned),
                class: v.attr("class").map(str::to_owned),
                raw_text,
                link_density,
                score: raw_text as f64 * (1.0 - link_density),
                emitted: 0,
            };
            (el, c)
        })
        .filter(|(_, c)| c.score > 200.0)
        .collect();
    ranked.sort_by(|a, b| {
        b.1.score
            .partial_cmp(&a.1.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    ranked.dedup_by_key(|(el, _)| el.id());
    ranked.truncate(5);
    // Rank by what the block walker actually *emits*, not by raw text. A container can be
    // dense with text yet yield almost nothing once chrome and empty wrappers are dropped,
    // which is how a newsletter post lost 90% of its body to a higher-scoring ancestor.
    let base = Url::parse("https://x.invalid/").unwrap();
    for (el, c) in &mut ranked {
        c.emitted = block_text(&blocks_from(*el, &base));
    }
    ranked
}

/// Index of the candidate with the most emitted text. Ties go to the later one, which is what
/// `max_by_key` did when this was inline.
fn choose_root(ranked: &[(ElementRef<'_>, Candidate)]) -> Option<usize> {
    ranked
        .iter()
        .enumerate()
        .max_by_key(|(_, (_, c))| c.emitted)
        .map(|(i, _)| i)
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

/// The floor a gathered inline run has to clear to become a paragraph of its own.
///
/// Inherited from the bare-text branch this gathering replaced. It keeps a stray label, a
/// lone bullet glyph, or a "1 of 12" counter from each becoming a paragraph, at the cost of
/// dropping a genuinely short line that no block element claimed. A run carrying a link is
/// exempt: see [`flush_pending`].
const MIN_LOOSE_TEXT: usize = 40;

/// Does this element hold block structure, or is it inline content?
///
/// The tag name cannot answer it for the case that matters. A news front page builds one
/// card as `<a href><div><h2>..</h2></div></a>` and the next as `<li><a>headline</a></li>`;
/// the first has to be descended into so the href can be carried down to the heading, the
/// second is a run of inline content that belongs in its parent's paragraph. So the content
/// decides. `<span>` wrappers, which pages use interchangeably with `<div>`, resolve the
/// same way.
///
/// `any` stops at the first block-level descendant, so the container case is cheap; the case
/// that scans a whole subtree is the inline one, whose subtree is small by definition.
fn holds_blocks(el: &ElementRef) -> bool {
    el.descendants()
        .skip(1)
        .filter_map(ElementRef::wrap)
        .any(|d| BLOCK_LEVEL.contains(&d.value().name()))
}

fn walk_blocks(node: NodeRef<'_, Node>, base: &Url, out: &mut Vec<Block>, depth: usize) {
    if depth >= MAX_DEPTH {
        // Keep the text rather than the structure; see `MAX_DEPTH`.
        let text = inner_text(node);
        if text.len() > MIN_LOOSE_TEXT {
            out.push(Block::Paragraph(vec![Inline::Text(text)]));
        }
        return;
    }
    // Inline children gather here and flush as one paragraph when a block child interrupts
    // them or the container ends. Deciding child by child instead dropped every inline
    // sibling of a block: in `<div><p>..</p><a>more</a></div>` the anchor emitted no block,
    // so it fell to a fallback that only ran when the *whole* container had emitted nothing.
    let mut pending: Vec<Inline> = Vec::new();
    for child in node.children() {
        let Some(el) = ElementRef::wrap(child) else {
            if matches!(child.value(), Node::Text(_)) {
                inline_child(child, base, depth + 1, &mut pending);
            }
            continue;
        };
        if is_noise(&el) {
            continue;
        }
        let name = el.value().name();
        let mut made: Vec<Block> = Vec::new();
        match name {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let level = name.as_bytes()[1] - b'0';
                let inl = trim_inlines(inlines_from(child, base, depth + 1));
                if !inl.is_empty() {
                    made.push(Block::Heading {
                        level,
                        inlines: inl,
                    });
                }
            }
            "p" => {
                let inl = trim_inlines(inlines_from(child, base, depth + 1));
                if !inl.is_empty() {
                    made.push(Block::Paragraph(inl));
                }
            }
            "ul" | "ol" => {
                let ordered = name == "ol";
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
                    made.push(Block::List { ordered, items });
                }
            }
            "blockquote" => {
                let mut b = Vec::new();
                walk_blocks(child, base, &mut b, depth + 1);
                if !b.is_empty() {
                    made.push(Block::Quote {
                        blocks: b,
                        cite: el.value().attr("cite").map(str::to_owned),
                    });
                }
            }
            "pre" => {
                let text = inner_text_raw(child);
                if !text.trim().is_empty() {
                    made.push(Block::Code {
                        lang: code_lang(&el),
                        text,
                    });
                }
            }
            "hr" => made.push(Block::Rule),
            "img" => {
                if let Some(img) = image_from(&el, base) {
                    made.push(Block::Figure {
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
                    made.push(Block::Figure {
                        image: img,
                        caption,
                    });
                }
            }
            "table" => {
                if let Some(t) = table_from(&el, base, depth) {
                    made.push(t);
                }
            }
            // Containers: recurse, but only into something that actually holds blocks.
            // Inline content joins the paragraph its siblings are building rather than
            // becoming one of its own, which is what keeps `<a>headline</a> (<a>host</a>)`
            // one line instead of a linked headline and a discarded remainder.
            _ if !BLOCK_LEVEL.contains(&name) && !holds_blocks(&el) => {
                inline_child(child, base, depth + 1, &mut pending);
            }
            _ => {
                walk_blocks(child, base, &mut made, depth + 1);
                if made.is_empty() {
                    inline_child(child, base, depth + 1, &mut pending);
                } else if name == "a"
                    && let Some(href) = el.value().attr("href").and_then(|h| base.join(h).ok())
                {
                    // A card on a news front page is `<a href><div><h2>..</h2></div></a>`,
                    // and descending into it as a plain container left the headline as
                    // unlinked prose. The href belongs on the text it wraps.
                    let href = href.to_string();
                    for b in &mut made {
                        link_block(b, &href);
                    }
                }
            }
        }
        if !made.is_empty() {
            flush_pending(&mut pending, out);
            out.append(&mut made);
        }
    }
    flush_pending(&mut pending, out);
}

/// Emit the gathered inline run as a paragraph.
///
/// A run with no link has to clear [`MIN_LOOSE_TEXT`]. A run carrying one is kept whatever
/// its length: a headline is often shorter than the floor, and the floor was dropping the
/// only route to the page behind it.
fn flush_pending(pending: &mut Vec<Inline>, out: &mut Vec<Block>) {
    let inl = trim_inlines(std::mem::take(pending));
    if inl.iter().all(is_blank) {
        return;
    }
    if inlines_len(&inl) > MIN_LOOSE_TEXT || inl.iter().any(has_link) {
        out.push(Block::Paragraph(inl));
    }
}

fn is_blank(i: &Inline) -> bool {
    matches!(i, Inline::Text(t) if t.trim().is_empty())
}

/// Drop the indentation whitespace that sits at either end of a run. Interior spacing is
/// left exactly as `push_space` built it: it is word spacing, not padding.
fn trim_inlines(mut v: Vec<Inline>) -> Vec<Inline> {
    while v.first().is_some_and(is_blank) {
        v.remove(0);
    }
    while v.last().is_some_and(is_blank) {
        v.pop();
    }
    if let Some(i) = v.first_mut() {
        trim_edge(i, true);
    }
    if let Some(i) = v.last_mut() {
        trim_edge(i, false);
    }
    v
}

/// Trim one outer edge, descending through the wrappers that can sit on a block boundary.
/// A link that keeps the markup's padding draws its underline out past its own words, which
/// is how a column of headlines gives itself away.
fn trim_edge(i: &mut Inline, start: bool) {
    match i {
        Inline::Text(t) => {
            let s = if start { t.trim_start() } else { t.trim_end() }.to_owned();
            *t = s;
        }
        Inline::Emph(v) | Inline::Strong(v) | Inline::Link { inlines: v, .. } => {
            if let Some(x) = if start { v.first_mut() } else { v.last_mut() } {
                trim_edge(x, start);
            }
        }
        Inline::Code(_) | Inline::Image(_) | Inline::Break => {}
    }
}

/// Carry a block-level anchor's href down onto the text it wraps.
///
/// The IR has no linked block and must not grow one: a link is an inline, and what this
/// recovers is a card whose heading and summary point at the same story. A block that
/// already carries a link of its own is left alone, because that href is the more specific
/// one and wrapping it would put it inside an outer link that no renderer can reach.
fn link_block(b: &mut Block, href: &str) {
    match b {
        Block::Heading { inlines, .. } | Block::Paragraph(inlines) => link_inlines(inlines, href),
        Block::List { items, .. } => items.iter_mut().flatten().for_each(|b| link_block(b, href)),
        Block::Quote { blocks, .. } => blocks.iter_mut().for_each(|b| link_block(b, href)),
        Block::Figure {
            caption: Some(c), ..
        } => link_inlines(c, href),
        Block::Table { headers, rows } => {
            headers.iter_mut().for_each(|c| link_inlines(c, href));
            rows.iter_mut()
                .flatten()
                .for_each(|c| link_inlines(c, href));
        }
        Block::Figure { .. }
        | Block::Code { .. }
        | Block::Rule
        | Block::Embed { .. }
        | Block::Thread(_) => {}
    }
}

fn link_inlines(v: &mut Vec<Inline>, href: &str) {
    if v.iter().all(is_blank) || v.iter().any(has_link) {
        return;
    }
    let inlines = std::mem::take(v);
    v.push(Inline::Link {
        href: href.to_owned(),
        inlines,
    });
}

fn has_link(i: &Inline) -> bool {
    match i {
        Inline::Link { .. } => true,
        Inline::Emph(v) | Inline::Strong(v) => v.iter().any(has_link),
        _ => false,
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
        inline_child(child, base, depth, &mut out);
    }
    out
}

/// One child node as inline content. Split out of [`inlines_from`] because [`walk_blocks`]
/// needs the same mapping for a single child: an element that turned out to hold no blocks
/// is inline, and reaching it through `inlines_from` on its *parent* would re-walk siblings
/// that have already been emitted.
fn inline_child(child: NodeRef<'_, Node>, base: &Url, depth: usize, out: &mut Vec<Inline>) {
    match child.value() {
        Node::Text(t) => {
            let s = squeeze(&t.replace(['\n', '\t'], " "));
            if s.trim().is_empty() {
                // A whitespace-only node *between* two inline elements is a word
                // boundary, and dropping it welds them together:
                // `<a>written language</a> <a>legible</a>` became "written
                // languagelegible". Markup indentation produces these constantly.
                if !s.is_empty() {
                    push_space(out);
                }
            } else {
                out.push(Inline::Text(s));
            }
        }
        Node::Element(e) => {
            let Some(el) = ElementRef::wrap(child) else {
                return;
            };
            if is_noise(&el) {
                return;
            }
            match e.name.local.as_ref() {
                "em" | "i" => out.push(Inline::Emph(inlines_from(child, base, depth + 1))),
                "strong" | "b" => out.push(Inline::Strong(inlines_from(child, base, depth + 1))),
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

/// The `<title>` element's text, which is the fallback behind `og:title`.
fn title_element(html: &Html) -> Option<String> {
    let sel = Selector::parse("title").ok()?;
    html.select(&sel)
        .next()
        .map(|e| inner_text(*e))
        .filter(|s| !s.is_empty())
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
    use crate::ir::{Block, Inline};

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

    /// Collect every href the document carries, in order.
    fn hrefs(d: &Document) -> Vec<String> {
        fn walk(b: &Block, out: &mut Vec<String>) {
            match b {
                Block::Heading { inlines, .. } | Block::Paragraph(inlines) => {
                    push_hrefs(inlines, out)
                }
                Block::List { items, .. } => items.iter().flatten().for_each(|b| walk(b, out)),
                Block::Quote { blocks, .. } => blocks.iter().for_each(|b| walk(b, out)),
                Block::Figure { caption, .. } => {
                    if let Some(c) = caption {
                        push_hrefs(c, out)
                    }
                }
                Block::Table { headers, rows } => {
                    headers.iter().for_each(|c| push_hrefs(c, out));
                    rows.iter().flatten().for_each(|c| push_hrefs(c, out));
                }
                Block::Thread(cs) => cs.iter().flat_map(|c| &c.blocks).for_each(|b| walk(b, out)),
                Block::Code { .. } | Block::Rule | Block::Embed { .. } => {}
            }
        }
        fn push_hrefs(v: &[Inline], out: &mut Vec<String>) {
            for i in v {
                match i {
                    Inline::Link { href, inlines } => {
                        out.push(href.clone());
                        push_hrefs(inlines, out);
                    }
                    Inline::Emph(x) | Inline::Strong(x) => push_hrefs(x, out),
                    _ => {}
                }
            }
        }
        let mut out = Vec::new();
        d.blocks.iter().for_each(|b| walk(b, &mut out));
        out
    }

    /// Every news front page builds a card as an anchor wrapped *around* block content, and
    /// descending into it as a plain container dropped the href: the headline rendered as
    /// unlinked prose with no route to the story. Measured on two front pages at once.
    #[test]
    fn a_block_level_anchor_keeps_its_href() {
        let d = doc(
            r#"<main><a href="/story/one"><picture><img src="/p.jpg" alt="pic"></picture>
            <div><h2>A headline that leads to the story behind the card</h2></div></a>
            <a href="/story/two"><div><p>A second headline, marked up as a paragraph</p></div></a>
            </main>"#,
        );
        assert_eq!(
            hrefs(&d),
            [
                "https://example.test/story/one",
                "https://example.test/story/two"
            ]
        );
        let heading = d
            .blocks
            .iter()
            .find(|b| matches!(b, Block::Heading { .. }))
            .expect("the heading inside the anchor should still be a heading");
        assert!(
            matches!(heading, Block::Heading { inlines, .. } if matches!(inlines.as_slice(), [Inline::Link { .. }])),
            "got: {heading:?}"
        );
    }

    /// The href does not displace one that is already there: an inner link is the more
    /// specific of the two, and wrapping it would put it inside an outer link that
    /// `reader::inline::flatten` deliberately flattens away.
    #[test]
    fn a_block_level_anchor_does_not_swallow_an_inner_link() {
        let d = doc(
            r#"<main><a href="/outer"><div><h2>A headline for the card</h2>
            <p>Summary text long enough to survive, with <a href="/inner">its own link</a>.</p>
            </div></a></main>"#,
        );
        let h = hrefs(&d);
        assert!(
            h.contains(&"https://example.test/outer".to_owned()),
            "{h:?}"
        );
        assert!(
            h.contains(&"https://example.test/inner".to_owned()),
            "{h:?}"
        );
    }

    /// The bug that made "some links show but not many": an anchor emitting no block fell to
    /// a fallback that only ran when the *whole* container had emitted nothing, so whether a
    /// link survived came down to whether its text cleared 40 characters. Hacker News shipped
    /// both outcomes in identical markup on one page, decided by title length alone.
    #[test]
    fn an_inline_anchor_keeps_its_href_at_any_length() {
        let short = doc(
            r#"<main><div><span class="titleline"><a href="https://a.test/x">Short</a><span
            class="sitebit"> (<a href="/from?site=a.test">a.test</a>)</span></span></div></main>"#,
        );
        let long = doc(
            r#"<main><div><span class="titleline"><a href="https://a.test/x">A title comfortably
            past forty characters long</a><span
            class="sitebit"> (<a href="/from?site=a.test">a.test</a>)</span></span></div></main>"#,
        );
        for d in [&short, &long] {
            assert_eq!(
                hrefs(d),
                ["https://a.test/x", "https://example.test/from?site=a.test"],
                "got: {:?}",
                d.blocks
            );
        }
        // And both stay one line: the title must not be split from the host beside it.
        for d in [&short, &long] {
            assert_eq!(
                d.blocks
                    .iter()
                    .filter(|b| matches!(b, Block::Paragraph(_)))
                    .count(),
                1,
                "got: {:?}",
                d.blocks
            );
        }
    }

    /// Inline content standing next to a block used to be dropped whole, which is how an
    /// article lost the opening clause of a footnote and how a paragraph split mid-sentence
    /// around a link.
    #[test]
    fn inline_siblings_of_a_block_are_not_dropped() {
        let d = doc(
            r#"<article><div><p>A first paragraph with plenty of text to clear the floor.</p>
            though, as we discussed <a href="/there">in that post</a>, the rest of the sentence
            carries on past the link and must not be thrown away with it.</div></article>"#,
        );
        let text = crate::ir::plain_text(&match &d.blocks[1] {
            Block::Paragraph(v) => v.clone(),
            b => panic!("expected a second paragraph, got {b:?}"),
        });
        assert!(text.starts_with("though, as we discussed"), "got: {text}");
        assert!(text.ends_with("thrown away with it."), "got: {text}");
        assert_eq!(hrefs(&d), ["https://example.test/there"]);
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

    /// `explain` is built by the pass that builds the document, so the two cannot disagree;
    /// this pins that the winner it names is the one whose blocks the document carries.
    #[test]
    fn explain_lists_every_candidate_and_names_the_winner() {
        let body = "Body prose long enough to clear the two-hundred-character floor on its own, \
                    repeated so the article is the obvious root of this page and nothing else. "
            .repeat(4);
        let aside = "An author card with a couple of lines of biography, not the article.";
        let src = format!(
            "<html><body><main><article><p>{body}</p><p>{body}</p></article>\
             <div class=\"bio\"><p>{aside}</p></div></main></body></html>"
        );
        let url = Url::parse("https://example.test/a").unwrap();
        let why = explain(&src, &url);
        let d = extract(&src, &url);
        assert!(
            why.candidates.len() >= 2,
            "expected main and article at least: {:?}",
            why.candidates
        );
        let w = why.winner.expect("a winner");
        assert_eq!(why.candidates[w].emitted, d.text_len());
        assert_eq!(why.text_len, d.text_len());
        assert_eq!(why.blocks, d.blocks.len());
        assert_eq!(why.meta[0], ("title", "none", None));
        assert_eq!(why.thread_merge, "no thread");
        // The printed form names the winner and every candidate's origin.
        let text = why.to_string();
        assert!(text.contains("* "), "no winner marked:\n{text}");
        assert!(text.contains("main") && text.contains("article"), "{text}");
    }

    /// The `repeated_paragraphs_are_not_a_thread` fixture, seen from the other side: the
    /// verdict says what the group was and why it was turned down.
    #[test]
    fn explain_reports_why_a_thread_group_was_rejected() {
        let ps = (0..6)
            .map(|i| {
                format!(
                    "<p class=\"wp-block-paragraph\">Paragraph {i} of an ordinary article, with \
             plenty of text so it clears every length filter in the detector.</p>"
                )
            })
            .collect::<String>();
        let why = explain(
            &format!("<html><body><article>{ps}</article></body></html>"),
            &Url::parse("https://example.test/a").unwrap(),
        );
        assert_eq!(why.thread.chosen, None);
        let g = why
            .thread
            .groups
            .iter()
            .find(|g| g.signature == "p.wp-block-paragraph")
            .unwrap_or_else(|| panic!("group not reported: {:?}", why.thread));
        assert_eq!(g.count, 6);
        assert_eq!(g.attributed, 0);
        assert_eq!(g.rejected, Some("attribution under a majority"));
    }

    /// The `div_soup` fixture: scoring finds nothing, the `<body>` fallback fires, and the
    /// explanation says so rather than claiming a root it did not have.
    #[test]
    fn explain_and_extract_agree_on_the_body_fallback() {
        let items = (0..8)
            .map(|i| {
                format!(
                    "<div class=\"item-cell\"><div class=\"item-title\">Item {i} in a listing grid \
             with a description long enough to be worth reading and scoring.</div></div>"
                )
            })
            .collect::<String>();
        let src = format!("<html><body><div class=\"grid\">{items}</div></body></html>");
        let url = Url::parse("https://example.test/a").unwrap();
        let why = explain(&src, &url);
        let d = extract(&src, &url);
        assert_eq!(why.text_len, d.text_len());
        // Either a candidate cleared the floor and won, or the fallback fired. What is pinned
        // is that the account matches the outcome, whichever way the scorer went.
        if why.winner.is_none() {
            assert!(why.body_fallback, "no root and no fallback: {why}");
        }
        assert!(why.to_string().contains("body fallback:"));
    }

    /// The thread path through `explain`: the table-layout fixture's group is the chosen one,
    /// and the merge is reported as the replacement it was.
    #[test]
    fn explain_names_the_chosen_thread_group_and_the_merge() {
        let mut rows = String::new();
        for i in 0..4 {
            rows.push_str(&format!(
                "<tr class=\"athing comtr\" id=\"c{i}\"><td><table><tr>\
                 <td class=\"ind\" indent=\"0\"></td>\
                 <td><span class=\"hnuser\">user{i}</span> <span class=\"age\">1 hour ago</span>\
                 <div class=\"commtext\">A comment with enough words in it to clear the median \
                 text floor and be counted as a post in its own right.</div></td></tr></table></td></tr>"
            ));
        }
        let src = format!("<html><body><table class=\"comment-tree\">{rows}</table></body></html>");
        let why = explain(&src, &Url::parse("https://example.test/t").unwrap());
        let c = why.thread.chosen.expect("a chosen group");
        assert_eq!(why.thread.groups[c].count, 4);
        assert_eq!(why.thread.groups[c].rejected, None);
        assert_eq!(why.thread_merge, "replaced the document");
        let text = why.to_string();
        assert!(text.contains("* tr.athing.comtr x4"), "{text}");
    }
}
