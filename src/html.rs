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
//!
//! # Extraction order
//!
//! The merge in [`extract_traced`] is load-bearing:
//!
//! 1. [`crate::cards::detect`] identifies sibling groups of story cards. [`walk_blocks`]
//!    emits each accepted group as one [`Block::Entries`] run in place; an accepted card group
//!    is not reconsidered as a thread.
//! 2. Content-root candidates are scored and the winner is mapped to blocks. When class-name
//!    chrome hints consume most of a root, [`blocks_guarded`] walks it without those hints.
//!    Headers and footers inside an article or section remain content.
//! 3. [`crate::thread::extract_thread_traced`] runs independently. A longer thread replaces the
//!    article result; a thread over 200 characters is appended to a longer article result.
//! 4. [`crate::tweet::detect`] runs last of the three detectors, over the same census: a
//!    container carrying a name, a date, and two or more counted actions is a tweet — the shape
//!    X serves, and named for X rather than for tweets at large because no other social
//!    host has been measured; `tweet.rs` says why. It takes the document only where steps 2 and 3 came out under [`crate::ir::THIN_TEXT`], so it
//!    rescues a page nothing else could read rather than competing with one that reads. A page
//!    that installed tweets skips step 5.
//! 5. A result below [`crate::ir::THIN_TEXT`], or below 1,000 characters when `<body>` emits
//!    five times as much, falls back to `<body>` — unless step 4 produced a tweet. A tweet is
//!    short by nature, and every floor here reads that shortness as a failure to extract.
//!
//! Reordering these steps changes whether cards become comments, nested discussions disappear,
//! thin pages become blank, or a tweet that parsed is replaced by the navigation around it. Diagnose changes through `--why`, which is produced by this same
//! pass.

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
    // Token matching made `nav` stop matching `navigation`, `advert` `advertising`, and
    // `cookie` `cookies`. These are the whole-word forms the substring match used to cover,
    // and a page that falls back to `<body>` meets every one of them.
    "navigation",
    "navbar",
    "topnav",
    "subnav",
    "mainnav",
    "submenu",
    "megamenu",
    "advertising",
    "advertorial",
    "cookies",
    "sharing",
    "sharebar",
    "socials",
    "subscription",
    "subscriptions",
    // Token matching made `advert` stop matching `advertisement`; these are the forms seen.
    "advertisement",
    "ad",
    "ads",
    "sponsored",
    "sponsor",
    "native-ad",
    // Recirculation vendors and embedded widgets, the same classes on hundreds of hosts:
    // outbrain/taboola rows, a "listen to this article" player, a Google preferred-sources box.
    "taboola",
    "outbrain",
    "tts",
    "text-to-speech",
    "audio-player",
    "beyondwords",
    "preferred-sources",
    // Text a sighted reader never sees, there to label an icon button for a screen reader:
    // "Return to Homepage" on a close control. This reader shows no controls.
    "sr-only",
    "visually-hidden",
    "visuallyhidden",
    "screen-reader-text",
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
    "details",
    "summary",
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
    /// The chrome hints were switched off for this root because they were eating it; see
    /// `root_candidates`. `emitted` is then the hint-free figure.
    pub hints_off: bool,
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
    /// What the story-card detector saw.
    pub cards: crate::cards::Verdict,
    /// The text of the accepted cards outside chrome: what [`CARDS_RATIO`] weighed against
    /// the root. Less than the accepted groups' total when a card list sits in a sidebar.
    pub cards_text: usize,
    /// What the site profile did, if one applied. Its `host` is filled by `session`.
    pub profile: Option<crate::profile::ProfileReport>,
    /// What the search-shape reader did, if the page was a known engine's result page. Filled
    /// by `session`, which owns that decision; the extractor knows no engines. Without this
    /// line `--why` would rank root candidates for a page whose document came from somewhere
    /// else entirely, which is the one thing the flag exists to prevent.
    pub search: Option<String>,
    /// What the thread merge did: `"replaced the document"`, `"appended"`,
    /// `"dropped (thinner than the floor)"`, or `"no thread"`.
    pub thread_merge: &'static str,
    /// What the tweet detector saw.
    pub tweet: crate::tweet::Verdict,
    /// What the tweet merge did: `"became the document"`, `"no tweet"`, or
    /// `"dropped (the page already read)"`.
    pub tweet_merge: &'static str,
    /// The tweet run became the document. The same fact `tweet_merge` spells, as a bool, because
    /// `session::explain` sets `Provenance::tweets` from it and `session::load` sets the same
    /// field by looking for `Block::Tweets` in the document: two routes to one flag, and a
    /// string compare on the second would come apart from the first the day the wording moved.
    pub tweets: bool,
    /// `(field, source, value)`: where each metadata field came from, or `"none"`.
    pub meta: Vec<(&'static str, &'static str, Option<String>)>,
    /// The `<body>` fallback ran and its blocks replaced the result.
    pub body_fallback: bool,
    /// What walking `<body>` whole would emit, when that was measured (it is, whenever the
    /// root came out under [`SLIVER_TEXT`]); `None` when it was not needed.
    pub body_emitted: Option<usize>,
    /// The body walk, too, had to drop the chrome hints to say anything; see `blocks_guarded`.
    pub body_hints_off: bool,
    pub blocks: usize,
    pub text_len: usize,
}

/// Extract, and say why. Headless `--why` prints this.
pub fn explain(source: &str, url: &Url) -> Explanation {
    extract_traced(source, url, &crate::profile::Profile::NONE).1
}

/// [`explain`], under a site profile.
pub fn explain_with_profile(
    source: &str,
    url: &Url,
    profile: &crate::profile::Profile,
) -> Explanation {
    extract_traced(source, url, profile).1
}

impl std::fmt::Display for Explanation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "why: {}", self.url)?;
        if let Some(p) = &self.profile {
            writeln!(
                f,
                "profile: {}",
                p.fields
                    .iter()
                    .map(|fr| format!("{} {}", fr.field, fr.outcome))
                    .collect::<Vec<_>>()
                    .join(", ")
            )?;
        }
        if let Some(sr) = &self.search {
            writeln!(f, "search: {sr}")?;
        }
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
            let note = if c.hints_off {
                " (chrome hints off)"
            } else {
                ""
            };
            writeln!(
                f,
                "  {mark} {:<11} <{}{id}{class}> raw={} links={:.2} score={:.0} emitted={}{note}",
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
        writeln!(
            f,
            "tweet: {}; {} weighed, {} kept, {} listed",
            self.tweet_merge,
            self.tweet.weighed,
            self.tweet.kept,
            self.tweet.candidates.len()
        )?;
        for c in &self.tweet.candidates {
            let mark = if c.rejected.is_none() { '*' } else { ' ' };
            writeln!(f, "  {mark} {c}")?;
        }
        let accepted = self.cards.accepted().count();
        writeln!(
            f,
            "cards: {accepted} group(s) look like story cards; {} chars outside chrome",
            self.cards_text
        )?;
        for g in &self.cards.groups {
            let mark = if g.rejected.is_none() { '*' } else { ' ' };
            writeln!(f, "  {mark} {g}")?;
        }
        writeln!(f, "meta:")?;
        for (field, source, value) in &self.meta {
            match value {
                Some(v) => writeln!(f, "    {field:<9} <- {source}: {v:?}")?,
                None => writeln!(f, "    {field:<9} <- {source}")?,
            }
        }
        let body = self
            .body_emitted
            .map(|n| {
                let note = if self.body_hints_off {
                    ", chrome hints off"
                } else {
                    ""
                };
                format!(" (body would emit {n}{note})")
            })
            .unwrap_or_default();
        writeln!(
            f,
            "body fallback: {}{body}",
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
    extract_traced(source, url, &crate::profile::Profile::NONE).0
}

/// [`extract`] under a site profile, with the account of what the profile did.
pub fn extract_with_profile(
    source: &str,
    url: &Url,
    profile: &crate::profile::Profile,
) -> Extraction {
    let (doc, why) = extract_traced(source, url, profile);
    Extraction {
        doc,
        profile: why.profile,
    }
}

/// What [`extract_with_profile`] hands back: the document, and what the profile did to it.
pub struct Extraction {
    pub doc: Document,
    /// `None` when the profile was `Profile::NONE`.
    pub profile: Option<crate::profile::ProfileReport>,
}

/// The one extraction pass. [`extract`] keeps the document and [`explain`] keeps the account;
/// neither re-enacts the other's decisions.
fn extract_traced(
    source: &str,
    url: &Url,
    profile: &crate::profile::Profile,
) -> (Document, Explanation) {
    let mut html = Html::parse_document(source);
    let profile_report = apply_profile(&mut html, profile);
    let (title, title_src) = first_of([
        ("og:title", meta_prop(&html, "og:title")),
        ("<title>", title_element(&html)),
    ]);
    let (byline, byline_src) = first_of([
        (
            "meta[name=author]",
            meta_attr(&html, "author").filter(|s| is_name(s)),
        ),
        (
            "meta[property=article:author]",
            meta_prop(&html, "article:author").filter(|s| is_name(s)),
        ),
        ("[rel=author]", visible_byline(&html, "[rel=author]")),
        ("[class~=byline]", visible_byline(&html, "[class~=byline]")),
        ("[class~=author]", visible_byline(&html, "[class~=author]")),
    ]);
    let (published, published_src) = first_of([
        (
            "meta[property=article:published_time]",
            meta_prop(&html, "article:published_time"),
        ),
        ("meta[name=date]", meta_attr(&html, "date")),
    ]);
    let (favicon, favicon_src) = favicon_of(&html, url);
    let mut doc = Document {
        url: url.to_string(),
        title,
        byline,
        published,
        site_name: meta_prop(&html, "og:site_name"),
        favicon,
        lang: html.root_element().value().attr("lang").map(str::to_owned),
        blocks: Vec::new(),
    };
    // One sibling-group census, lent to both detectors. They ask the same question of the same
    // tree — which runs of same-signature siblings does this page have — and each used to walk
    // every element and build a signature for it. See `thread::extract_thread_in`.
    let groups = crate::thread::sibling_groups(&html);
    let (card_map, cards) = crate::cards::detect_in(&groups);
    let mut why = Explanation {
        url: url.to_string(),
        candidates: Vec::new(),
        winner: None,
        thread: Default::default(),
        cards,
        cards_text: 0,
        profile: profile_report,
        search: None,
        thread_merge: "no thread",
        tweet: Default::default(),
        tweet_merge: "no tweet",
        tweets: false,
        meta: vec![
            ("title", title_src, doc.title.clone()),
            ("byline", byline_src, doc.byline.clone()),
            ("published", published_src, doc.published.clone()),
            ("favicon", favicon_src, doc.favicon.clone()),
        ],
        body_fallback: false,
        body_emitted: None,
        body_hints_off: false,
        blocks: 0,
        text_len: 0,
    };
    // Measure every node's text once, after profile stripping has settled the tree. Scoring
    // then reads a map instead of re-walking overlapping subtrees (see `TextMetrics`).
    let metrics = build_text_metrics(&html);
    let ranked = root_candidates(&html, &card_map, &metrics);
    why.winner = choose_root(&ranked);
    why.candidates = ranked.iter().map(|(_, c)| c.clone()).collect();
    if let Some(i) = why.winner {
        let walk = if ranked[i].1.hints_off {
            Walk::without_hints(url)
        } else {
            Walk::bare(url)
        };
        doc.blocks = blocks_from(ranked[i].0, &walk.cards(&card_map));
    }
    // A discussion is not a subtree, so it gets its own algorithm. When the thread dominates
    // the page it *is* the document; when it merely accompanies an article, it is appended.
    let (thread, verdict) = crate::thread::extract_thread_in(&html, url, &card_map, &groups);
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

    // A tweet is the third shape and the only one whose length is not its completeness. It runs
    // last so the other two keep what they win, and it is a rescue rather than a competitor: it
    // takes the document only where the article and thread paths came out under the floor. A
    // forum whose posts each carry a like count stays a thread, because the thread carried text.
    let (tweets, verdict) = crate::tweet::detect_in(&html, url, &groups);
    why.tweet = verdict;
    if let Some(tweets) = tweets {
        if doc.text_len() < crate::ir::THIN_TEXT {
            doc.blocks = vec![Block::Tweets(tweets)];
            why.tweet_merge = "became the document";
            why.tweets = true;
        } else {
            why.tweet_merge = "dropped (the page already read)";
        }
    }

    // Scoring finds nothing on pages built entirely from unlabelled divs (product grids,
    // listing pages). Falling back to <body> bleeds navigation, but a thin page beats a
    // blank one, and the renderer can still be read.
    //
    // A root that is a sliver of the page gets the same treatment: on two front pages the
    // scorer's best was a 240-char text box and a 471-char legal block, each clearing the
    // floor and each a fraction of the page. Until front pages have a shape of their own, the
    // body with its navigation beats a legal notice with nothing.
    // And a root the page's own story cards dwarf: one portal keeps its sixty-card stream
    // beside `<main>` rather than in it, and `<main>` was a 1,233-char root.
    let cards_text = cards_outside_chrome(&html, &card_map);
    why.cards_text = cards_text;
    let have = doc.text_len();
    // `!why.tweets` is load-bearing: a tweet is under every floor here having parsed perfectly, so
    // the fallback would trade the message for the navigation around it. See the header.
    if !why.tweets
        && (have < SLIVER_TEXT || cards_text > CARDS_RATIO * have)
        && let Some(body) = html.select(&sel::BODY).next()
    {
        let (fallback, hints_off) = blocks_guarded(body, url, &card_map);
        let got = block_text(&fallback);
        why.body_emitted = Some(got);
        why.body_hints_off = hints_off;
        if got > have
            && (have < crate::ir::THIN_TEXT
                || got > SLIVER_RATIO * have
                || cards_text > CARDS_RATIO * have)
        {
            doc.blocks = fallback;
            why.body_fallback = true;
        }
    }
    trim_tail(&mut doc.blocks);
    why.blocks = doc.blocks.len();
    why.text_len = doc.text_len();
    #[cfg(test)]
    snapshot::record(source, &why);
    (doc, why)
}

/// A scaffold for proving an extraction change moved nothing.
///
/// Every fixture in this crate is an inline `#[cfg(test)]` string, so there is no corpus a
/// binary can be pointed at: `--why` takes a URL and fetches, and nothing may assert on that
/// binary's output anyway. This records the [`Explanation`] of every extraction any test
/// performs, keyed by the source, so `HWW_WHY_DUMP=path cargo test` before and after a change
/// yields two files that must be identical. Compiled only under `cfg(test)` and inert unless
/// the variable is set.
#[cfg(test)]
mod snapshot {
    use super::Explanation;
    use std::io::Write;

    pub fn record(source: &str, why: &Explanation) {
        // Read once. `std::env::var` takes a lock and allocates, and this sits in the pass every
        // extraction test runs — asking it per document made the `html` tests 18% slower than
        // the change they exist to measure.
        static PATH: std::sync::LazyLock<Option<String>> =
            std::sync::LazyLock::new(|| std::env::var("HWW_WHY_DUMP").ok());
        let Some(path) = PATH.as_ref() else {
            return;
        };
        // Tests run in parallel and each appends a multi-line record, so the write is taken
        // under a lock of this process's own rather than trusting a single `write` to be
        // atomic. Keyed by the source and sorted at read time, so thread order does not show.
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let mut key = 0xcbf2_9ce4_8422_2325u64;
        for b in source.as_bytes() {
            key = (key ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
        let _held = LOCK.lock();
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = write!(f, "\u{1}{key:016x}\n{why}\n");
        }
    }
}

/// The text of every accepted story card whose list is not inside chrome: what
/// [`CARDS_RATIO`] weighs against the chosen root.
///
/// The card detector reads the raw tree and never asks [`in_chrome`], so a forty-card
/// "Latest" rail in a `div.sidebar` is as accepted as a front page's stream. Summing every
/// accepted group made a correctly rooted 1,369-char article take the body walk, rail and
/// newsletter box included, because the rail out-weighed it three to one. The walker would
/// drop that rail on the way past, so the ratio does not count it either. Measured from the
/// map rather than the reports so a card that joined a list late (`cards::detect`'s lone-card
/// join) is counted with the list it joined.
fn cards_outside_chrome(html: &Html, cards: &crate::cards::CardMap) -> usize {
    cards
        .iter()
        .filter(|(parent, _)| {
            html.tree
                .get(**parent)
                .and_then(ElementRef::wrap)
                .is_some_and(|p| !in_chrome(&p))
        })
        .flat_map(|(_, members)| members.iter())
        .filter_map(|id| html.tree.get(*id))
        .map(crate::thread::text_len)
        .sum()
}

/// An article ends in prose. What trails the last prose block is link rubble: a tag list, a
/// "See all" or "Read more", a "Most viewed" heading whose list the extractor already dropped,
/// a rule. Measured on forty articles across fifty-seven hosts: ten ended this way. Prose is
/// never trimmed, however short; only link-only runs, link-only lists, headings with nothing
/// after them, and rules.
fn trim_tail(blocks: &mut Vec<Block>) {
    let mut total = crate::ir::blocks_text_len(blocks);
    if total < crate::ir::THIN_TEXT {
        return;
    }
    while let Some(last) = blocks.last() {
        if !is_rubble(last) {
            break;
        }
        // Never under the floor. A page that is one list of linked titles (a blog archive,
        // which only the body fallback reaches) is rubble item by item and the page by the
        // sum; popping the list and then the heading over it left a document of nothing.
        let rest = total - crate::ir::blocks_text_len(std::slice::from_ref(last));
        if rest < crate::ir::THIN_TEXT {
            break;
        }
        total = rest;
        blocks.pop();
    }
}

/// A closing line is rubble when it is links and nothing else, and short.
const RUBBLE_TEXT: usize = 80;

fn is_rubble(b: &Block) -> bool {
    match b {
        Block::Rule => true,
        // A closing image with no caption and a short alt is a badge, a logo, or a tracking
        // pixel ("Google News", "Publicidad", "Imagen de seguimiento"); a closing photograph
        // carries a caption.
        Block::Figure {
            caption: None,
            image,
        } => image.alt.as_deref().is_none_or(|a| a.len() < 40),
        // The same badges set inline: a run that is nothing but short-alt images.
        Block::Paragraph(inl) if images_only(inl) => true,
        // `plain_text` and not `ir::inlines_text_len`, which is the cheaper currency: a heading
        // may hold an `Inline::Image`, and a badge whose alt text is the only thing in it should
        // be measured by that text rather than counted as empty and trimmed away.
        Block::Heading { inlines, .. } => crate::ir::plain_text(inlines).len() < RUBBLE_TEXT,
        Block::Paragraph(inl) => links_only(inl),
        Block::List { items, .. } => items.iter().all(|item| {
            item.iter()
                .all(|b| matches!(b, Block::Paragraph(inl) if links_only(inl)))
        }),
        _ => false,
    }
}

fn images_only(inl: &[Inline]) -> bool {
    let mut any = false;
    for i in inl {
        match i {
            Inline::Text(t) if t.trim().is_empty() => {}
            Inline::Image(img) if img.alt.as_deref().is_none_or(|a| a.len() < 40) => any = true,
            Inline::Link { inlines, .. } => {
                if !images_only(inlines) {
                    return false;
                }
                any = true;
            }
            _ => return false,
        }
    }
    any
}

fn links_only(inl: &[Inline]) -> bool {
    fn walk(inl: &[Inline], any: &mut bool) -> bool {
        for i in inl {
            match i {
                // Separators between tags ("Canada, Donald Trump, trade war", "Categories:")
                // are punctuation, not prose.
                Inline::Text(t)
                    if t.chars()
                        .all(|c| c.is_whitespace() || c.is_ascii_punctuation() || c == '·') => {}
                Inline::Link { .. } => *any = true,
                // "*Subscribe to the newsletter*": emphasis around a link is still a link.
                Inline::Emph(v) | Inline::Strong(v) => {
                    if !walk(v, any) {
                        return false;
                    }
                }
                _ => return false,
            }
        }
        true
    }
    let mut any = false;
    // `ir::inlines_text_len` and not `ir::plain_text(..).len()`: the walk above has already
    // returned `false` for every variant the two count differently (an `Image`'s alt text and a
    // `Break`'s newline), so this is the same number without the string.
    walk(inl, &mut any) && any && crate::ir::inlines_text_len(inl) < RUBBLE_TEXT
}

/// A byline names a person. Two of the top-ten US news sites put a URL in
/// `article:author` (a Facebook page, an author index), which is a link and not a name.
fn is_name(s: &str) -> bool {
    let t = s.trim();
    !t.is_empty() && t.len() < 120 && !t.starts_with("http://") && !t.starts_with("https://")
}

/// The text of the first element matching `sel`, as a byline: short, non-empty, the leading
/// "By" dropped. The visible fallback behind the meta tags.
fn visible_byline(html: &Html, sel: &str) -> Option<String> {
    let selector = Selector::parse(sel).ok()?;
    html.select(&selector)
        .map(|e| inner_text(*e))
        .map(|t| {
            let t = t.trim();
            let t = t
                .strip_prefix("By ")
                .or_else(|| t.strip_prefix("by "))
                .or_else(|| t.strip_prefix("BY "))
                .unwrap_or(t);
            t.to_owned()
        })
        .find(|t| is_name(t))
}

/// A root under this many characters that the `<body>` out-emits [`SLIVER_RATIO`] to one is a
/// sliver of its page, and the page is taken instead. Measured: 236 and 471 on two front pages
/// whose bodies held five and eleven thousand.
const SLIVER_TEXT: usize = 1000;
const SLIVER_RATIO: usize = 5;
/// Accepted story-card lists carrying this many times the chosen root's text mean the root
/// is not the page. Measured: 14,891 against 1,233.
const CARDS_RATIO: usize = 3;

/// A `strip` that would remove more than this share of the page's text is refused: a profile
/// is a candidate with a floor, and a stale selector that now matches the article body must
/// not be allowed to delete it silently.
const STRIP_CEILING: f64 = 0.5;

/// Apply a profile to the parsed page, before anything reads it. `strip` detaches matched
/// subtrees from the tree, so scoring, walking, metadata, and the thread and card detectors
/// all see a page without them. One `FieldReport` per set field, whatever happened.
fn apply_profile(
    html: &mut Html,
    profile: &crate::profile::Profile,
) -> Option<crate::profile::ProfileReport> {
    use crate::profile::{FieldReport, Outcome, ProfileReport};
    if profile.is_none() {
        return None;
    }
    let mut fields = Vec::new();
    if let Some(sel) = &profile.strip {
        let outcome = match Selector::parse(sel) {
            Err(_) => Outcome::Rejected("selector does not parse"),
            Ok(selector) => {
                let before = text_len(html.tree.root());
                let ids: Vec<ego_tree::NodeId> = html.select(&selector).map(|e| e.id()).collect();
                if ids.is_empty() {
                    Outcome::Dead
                } else {
                    // Outermost matches only: a container and its child both matching the
                    // selector is one removal, not two, against the ceiling.
                    let set: std::collections::HashSet<ego_tree::NodeId> =
                        ids.iter().copied().collect();
                    let removed: usize = ids
                        .iter()
                        .filter_map(|id| html.tree.get(*id))
                        .filter(|n| !n.ancestors().any(|a| set.contains(&a.id())))
                        .map(text_len)
                        .sum();
                    if before > 0 && removed as f64 > STRIP_CEILING * before as f64 {
                        Outcome::Rejected("would remove most of the page")
                    } else {
                        for id in &ids {
                            if let Some(mut n) = html.tree.get_mut(*id) {
                                n.detach();
                            }
                        }
                        Outcome::Matched(ids.len())
                    }
                }
            }
        };
        fields.push(FieldReport {
            field: "strip",
            outcome,
        });
    }
    Some(ProfileReport {
        host: String::new(),
        fields,
    })
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

/// One of the three shared text-length counters. This used to build a
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
    crate::ir::normalize_ws(&out)
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
fn root_candidates<'h>(
    html: &'h Html,
    cards: &crate::cards::CardMap,
    metrics: &TextMetrics,
) -> Vec<(ElementRef<'h>, Candidate)> {
    // Emitted length is measured against a dummy base: hrefs do not change how much text a
    // root yields, and the real base is not known here. See `DUMMY_BASE`.
    let base = &*DUMMY_BASE;
    let mut candidates: Vec<(ElementRef<'_>, &'static str)> = Vec::new();
    for (name, selector) in [
        ("article", &*sel::ARTICLE),
        ("main", &*sel::MAIN),
        ("[role=main]", &*sel::ROLE_MAIN),
    ] {
        candidates.extend(html.select(selector).map(|el| (el, name)));
    }
    candidates.extend(score_best(html, &Walk::bare(base), metrics).map(|el| (el, "scored")));
    let mut ranked: Vec<(ElementRef<'_>, Candidate)> = candidates
        .into_iter()
        .map(|(el, origin)| {
            let raw_text = metrics.text_len(el.id());
            let link_density = metrics.link_density(el.id());
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
                hints_off: false,
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
    for (el, c) in &mut ranked {
        let (blocks, hints_off) = blocks_guarded(*el, base, cards);
        c.emitted = block_text(&blocks);
        c.hints_off = hints_off;
    }
    ranked
}

/// Walk `root` with the chrome hints in force, and without them if that is what it takes.
///
/// A hint that deletes most of a root is not describing chrome. One wire service names every
/// story card `PagePromo`, and `promo` is a hint: its front page emitted 2,371 of 16,673
/// chars. Another wraps its whole main column in `region-content-sidebar-secondary`, and
/// `sidebar` is a hint: its `<body>` emitted 283 chars. When the hinted subtrees are more than
/// half the root's text, the root is walked without them, and the Explanation says so.
fn blocks_guarded(
    root: ElementRef<'_>,
    base: &Url,
    cards: &crate::cards::CardMap,
) -> (Vec<Block>, bool) {
    // Every candidate root is walked twice and the body fallback twice more, up to a dozen full
    // IR builds of one page. The two walks differ on exactly one line — the hint arm at the end
    // of `is_noise`, which is the only reader of `Walk::noise_hints` — so a hinted walk that
    // dropped nothing *on hint grounds* has already produced what the unhinted walk would, and
    // the second is a re-derivation of a result in hand. The comparison agrees: with both
    // lengths equal, `without > 2 * with` is false for any length including zero, so the
    // shortcut returns the arm the full form would have.
    //
    // Counted rather than compared, and drops rather than dropped text: a zero is a proof, and
    // anything else falls through to the two-walk answer unchanged. It rarely fires on a real
    // page — `nav`, `footer`, and `share` are hints and most sites carry one — which is the
    // point of making it cost a counter rather than a heuristic.
    let drops = std::cell::Cell::new(0usize);
    let with = blocks_from(root, &Walk::bare(base).cards(cards).counting_hints(&drops));
    if drops.get() == 0 {
        return (with, false);
    }
    let without = blocks_from(root, &Walk::without_hints(base).cards(cards));
    if block_text(&without) > 2 * block_text(&with) {
        (without, true)
    } else {
        (with, false)
    }
}

/// Index of the candidate with the most emitted text. Ties go to the later one, which is what
/// `max_by_key` did when this was inline.
fn choose_root(ranked: &[(ElementRef<'_>, Candidate)]) -> Option<usize> {
    // Equal emitted text, prefer the tighter container: when `<main>` and the article body
    // inside it yield the same blocks, the body is the one without the video card and the
    // app-download banner that sit beside it in `<main>` and get flattened into its run.
    ranked
        .iter()
        .enumerate()
        .max_by(|(_, (_, a)), (_, (_, b))| {
            a.emitted
                .cmp(&b.emitted)
                .then_with(|| b.raw_text.cmp(&a.raw_text))
        })
        .map(|(i, _)| i)
}

/// Readability-style: score leaf text blocks, propagate to ancestors, and discount link density.
fn score_best<'h>(
    html: &'h Html,
    walk: &Walk<'_>,
    metrics: &TextMetrics,
) -> Option<ElementRef<'h>> {
    let mut scores: HashMap<ego_tree::NodeId, f64> = HashMap::new();
    // `div`/`section` are included because most modern pages never emit a `<p>`. A container
    // only counts when the prose is its *own* (see `own_text`).
    for el in html.select(&sel::SCORED) {
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
                && !is_noise(&e, walk)
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
            Some((el, s * (1.0 - metrics.link_density(el.id()))))
        })
        .filter(|(_, s)| *s > 0.0)
        // **The tie-break is what makes this function deterministic.** `scores` is a `HashMap`
        // with a per-process seed, and `max_by` keeps the last maximum it sees, so without a
        // second key a page whose containers score equally chose a different root on every run:
        // the 6,000-deep chain in `deeply_nested_text_extracts_in_linear_time` extracted 53k,
        // 107k, and 168k characters on three consecutive runs of the same binary. A linear chain
        // of wrappers is exactly the shape that ties, because each one holds the same prose.
        //
        // The earlier node wins, which in an `ego_tree` index is the outer container. That is
        // the one `choose_root` would rank first anyway, since it emits at least as much as
        // anything nested inside it, so the two tie-breaks agree rather than fight. `NodeId` is
        // an index into the tree's own vector, so this is document order and needs no key built
        // for it.
        .max_by(|a, b| {
            a.1.partial_cmp(&b.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.0.id().cmp(&a.0.id()))
        })
        .map(|(el, _)| el)
}

fn is_noise(el: &ElementRef, walk: &Walk<'_>) -> bool {
    let v = el.value();
    let name = v.name();
    // A page's `<header>` and `<footer>` are chrome. A card's or an article's are not: one
    // front page sets every headline in `<article><header><h4>`, and dropping the tag there
    // dropped the page. Inside an `<article>` or `<section>` the two are structure.
    if matches!(name, "header" | "footer") {
        return !el
            .ancestors()
            .filter_map(ElementRef::wrap)
            .any(|a| matches!(a.value().name(), "article" | "section"));
    }
    if NOISE.contains(&name) {
        return true;
    }
    if !walk.noise_hints {
        return false;
    }
    // Token match, not substring, so `navigator`, `header-image`, and `shareholder` are no
    // longer chrome. See `hint`. A token that *is* a hint on a content container (one wire
    // service names its story cards `PagePromo`) is the ratio guard's job, in
    // `root_candidates`. The two attributes go in as they are: this is asked of every element of
    // every walk, and building one string to hold them was an allocation per element per walk.
    let hinted = crate::hint::matches_in(
        &[v.id().unwrap_or(""), v.attr("class").unwrap_or("")],
        NOISE_HINT,
    );
    // Counted here and nowhere else, because this is the only line the two walks disagree on.
    if hinted && let Some(drops) = walk.hint_drops {
        drops.set(drops.get() + 1);
    }
    hinted
}

/// Is this element chrome, or inside chrome, by the same tags and hints the walker drops?
/// The thread and card detectors read the raw tree, so a "related stories" block under a
/// `related` container looked like four dated posts to them until they asked.
pub(crate) fn in_chrome(el: &ElementRef) -> bool {
    let walk = Walk::bare(&DUMMY_BASE);
    std::iter::once(*el)
        .chain(el.ancestors().filter_map(ElementRef::wrap))
        .any(|e| is_noise(&e, &walk))
}

// ---------- text measurement ----------

fn inner_text(node: NodeRef<'_, Node>) -> String {
    let mut out = String::new();
    collect_text(node, &mut out);
    crate::ir::normalize_ws(&out)
}

/// Iterative, like [`crate::reader::thread_tree`]'s traversal and for the same reason: the
/// input is untrusted, a recursive walk costs a stack frame per DOM level, and a stack
/// overflow is an abort that no guard in the reader can contain. See [`MAX_DEPTH`] for the
/// walkers that cannot be flattened this cheaply.
fn collect_text(node: NodeRef<'_, Node>, out: &mut String) {
    enum Item<'a> {
        Node(NodeRef<'a, Node>),
        Space,
    }
    let mut stack = vec![Item::Node(node)];
    while let Some(item) = stack.pop() {
        let n = match item {
            Item::Space => {
                out.push(' ');
                continue;
            }
            Item::Node(n) => n,
        };
        match n.value() {
            Node::Text(t) => out.push_str(t),
            Node::Element(e) if NOISE.contains(&e.name.local.as_ref()) => {}
            Node::Element(e) => {
                // A block-level element starts and ends on a word boundary, as it does on
                // the page: `<div>21 hours ago</div><div>Joe Inwood</div>` is two things.
                let block = BLOCK_LEVEL.contains(&e.name.local.as_ref()) || &*e.name.local == "br";
                if block {
                    stack.push(Item::Space);
                }
                stack.extend(n.children().rev().map(Item::Node));
                if block {
                    stack.push(Item::Space);
                }
            }
            _ => stack.extend(n.children().rev().map(Item::Node)),
        }
    }
}

fn text_len(node: NodeRef<'_, Node>) -> usize {
    inner_text(node).len()
}

/// One node's contribution to a whitespace-normalized text length, composable across siblings.
///
/// `inner_text` collapses whitespace runs to a single space and trims the ends, so a node's
/// length is not the sum of its children's: two children that meet without whitespace between
/// them share a word. This carries just enough to fold children left-to-right and land on the
/// same length `inner_text(node).len()` would: the byte count across tokens (`nonws`), the
/// token count (`words`), and whether the run starts and ends on a word so an adjacent sibling
/// knows whether to merge.
#[derive(Clone, Copy, Default)]
pub(crate) struct WsSummary {
    nonws: usize,
    words: usize,
    first_is_word: bool,
    last_is_word: bool,
    nonempty: bool,
}

impl WsSummary {
    /// `collect_text` puts a bare space either side of a block element; this is that space.
    const fn ws() -> Self {
        Self {
            nonws: 0,
            words: 0,
            first_is_word: false,
            last_is_word: false,
            nonempty: true,
        }
    }

    pub(crate) fn text(t: &str) -> Self {
        let mut nonws = 0;
        let mut words = 0;
        for w in t.split_whitespace() {
            nonws += w.len();
            words += 1;
        }
        Self {
            nonws,
            words,
            first_is_word: t.chars().next().is_some_and(|c| !c.is_whitespace()),
            last_is_word: t.chars().next_back().is_some_and(|c| !c.is_whitespace()),
            nonempty: !t.is_empty(),
        }
    }

    /// `a` then `b`. Their touching tokens merge into one word iff neither side has whitespace
    /// at the seam. The default value is the empty string and is the identity here.
    pub(crate) fn concat(a: Self, b: Self) -> Self {
        let merge = a.last_is_word && b.first_is_word;
        Self {
            nonws: a.nonws + b.nonws,
            words: a.words + b.words - usize::from(merge),
            first_is_word: if a.nonempty {
                a.first_is_word
            } else {
                b.first_is_word
            },
            last_is_word: if b.nonempty {
                b.last_is_word
            } else {
                a.last_is_word
            },
            nonempty: a.nonempty || b.nonempty,
        }
    }

    /// Normalized length: tokens joined by single spaces, trimmed.
    pub(crate) fn len(self) -> usize {
        if self.words == 0 {
            0
        } else {
            self.nonws + self.words - 1
        }
    }
}

/// Per-node text measurements, computed once so scoring is linear rather than O(n²).
///
/// `text_len` and `link_density` each walk the whole subtree below a node. Called per
/// candidate ([`root_candidates`]) and per scored node ([`score_best`]), on a deeply nested
/// page that is O(n²): a linear chain of scored ancestors, each re-measuring the chain below
/// it. Untrusted HTML reaches this directly, and it is not bounded by [`MAX_DEPTH`], which
/// guards the IR walkers downstream of scoring. One post-order pass makes every measurement a
/// map lookup.
///
/// `text_len` reproduces `inner_text(node).len()` exactly: `collect_text` skips [`NOISE`]
/// subtrees and spaces a block element on both sides, and normalization collapses and trims.
/// `anchor_len` reproduces `link_density`'s old numerator, `el.select("a").map(text_len).sum()`,
/// which descends *into* `NOISE` (a `<nav>` full of links counts) and double-counts an anchor
/// nested in another.
struct TextMetrics {
    by_node: std::collections::HashMap<ego_tree::NodeId, (WsSummary, usize)>,
}

impl TextMetrics {
    fn text_len(&self, id: ego_tree::NodeId) -> usize {
        self.by_node.get(&id).map_or(0, |&(s, _)| s.len())
    }

    /// Fraction of an element's text that sits inside anchors. High density means navigation.
    fn link_density(&self, id: ego_tree::NodeId) -> f64 {
        let Some(&(summary, anchor)) = self.by_node.get(&id) else {
            return 0.0;
        };
        let total = summary.len();
        if total == 0 {
            return 0.0;
        }
        (anchor as f64 / total as f64).min(1.0)
    }
}

/// The one post-order pass. Iterative for the same reason as [`collect_text`]: a stack frame
/// per DOM level on untrusted input is a stack overflow waiting for a deep enough page.
fn build_text_metrics(html: &Html) -> TextMetrics {
    let mut by_node: std::collections::HashMap<ego_tree::NodeId, (WsSummary, usize)> =
        std::collections::HashMap::new();
    let mut stack: Vec<(NodeRef<'_, Node>, bool)> = vec![(html.tree.root(), false)];
    while let Some((n, done)) = stack.pop() {
        if !done {
            stack.push((n, true));
            stack.extend(n.children().map(|c| (c, false)));
            continue;
        }
        // Anchors below this node, matching `select("a")`: descend everywhere, including into
        // NOISE, and count a nested anchor once at each level (the double-count `select` makes).
        let anchor: usize = n
            .children()
            .map(|c| {
                let (cs, ca) = by_node[&c.id()];
                let is_a = matches!(c.value(), Node::Element(e) if e.name.local.as_ref() == "a");
                usize::from(is_a) * cs.len() + ca
            })
            .sum();
        let summary = match n.value() {
            Node::Text(t) => WsSummary::text(t),
            Node::Element(e) if NOISE.contains(&e.name.local.as_ref()) => WsSummary::default(),
            Node::Element(e) => {
                let inner = n.children().fold(WsSummary::default(), |acc, c| {
                    WsSummary::concat(acc, by_node[&c.id()].0)
                });
                let name = e.name.local.as_ref();
                if BLOCK_LEVEL.contains(&name) || name == "br" {
                    WsSummary::concat(WsSummary::concat(WsSummary::ws(), inner), WsSummary::ws())
                } else {
                    inner
                }
            }
            // Document, fragment, comment, doctype: `collect_text` descends without a space.
            _ => n.children().fold(WsSummary::default(), |acc, c| {
                WsSummary::concat(acc, by_node[&c.id()].0)
            }),
        };
        by_node.insert(n.id(), (summary, anchor));
    }
    TextMetrics { by_node }
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

/// The selectors this module parses from literals, parsed once.
///
/// `Selector::parse` is a CSS parse and several allocations, and most of these sat inside
/// per-element code: the `li` selector ran once per list and the three in [`entry_from`] once
/// per accepted story card, across up to thirteen block walks of one page. A selector built
/// from a profile, a search shape, or a `meta` name is not here — those are not literals and
/// are parsed where they are read.
///
/// `pub(crate)` because `cards` and `thread` parse the same three literals per group and per
/// member; one table is the point.
pub(crate) mod sel {
    use scraper::Selector;
    use std::sync::LazyLock;

    /// `Selector` is `Sync` — a `servo_arc` of interned atoms, with the `nth`-index cache made
    /// per match rather than held — but nothing in this crate proved it before these statics
    /// existed. A failed assumption here should be a compile error, not a shared cache.
    const _: fn() = || {
        fn assert_sync<T: Sync>() {}
        assert_sync::<Selector>();
    };

    macro_rules! sel {
        ($($name:ident = $css:literal;)*) => {$(
            pub(crate) static $name: LazyLock<Selector> =
                LazyLock::new(|| Selector::parse($css).expect($css));
        )*};
    }

    sel! {
        A = "a";
        IMG_PICTURE = "img, picture";
        BODY = "body";
        SCORED = "p, td, pre, blockquote, li, div, section, dd";
        LI = "li";
        IMG = "img";
        FIGCAPTION = "figcaption";
        SOURCE_SRC = "source[src]";
        SOURCE_SRCSET = "source[srcset]";
        A_HREF = "a[href]";
        HEADING = "h1, h2, h3, h4, h5, h6";
        TR = "tr";
        CELL = "th, td";
        TITLE = "title";
        LINK_REL = "link[rel][href]";
        ARTICLE = "article";
        MAIN = "main";
        ROLE_MAIN = "[role=main]";
    }
}

/// The base a root candidate's blocks are measured under, and the one [`in_chrome`] walks with.
///
/// Neither reads it: emitted length does not depend on where a href resolves to, and chrome is a
/// question about tags and class names. It is parsed once because it was being parsed per card
/// parent and per sibling group.
static DUMMY_BASE: std::sync::LazyLock<Url> =
    std::sync::LazyLock::new(|| Url::parse("https://x.invalid/").expect("a literal URL"));

/// Accepted cards inside accepted cards, past which the walk stops making entries. See
/// `Walk::card_depth`.
const MAX_CARD_NESTING: u8 = 3;

/// The block walk with the class-name chrome hints off: how a feed entry's payload is read.
///
/// The hints exist to drop nav, promo, and footer subtrees out of a **whole page**, where a
/// `share`, `promo`, or `related` class names something wrapped *around* the content. A feed
/// summary is not a page: the publisher already chose what went in it, so a post body that
/// arrives inside a promo-classed wrapper has the article deleted with nothing on screen saying
/// why. That is the `PagePromo` failure `docs/findings.md` records, one level in, and it is
/// silent — the entry simply comes out short.
///
/// `a_promo_class_in_a_summary_survives` in `feed` is what holds this call in place.
pub fn blocks_from_public_unhinted(root: ElementRef<'_>, base: &Url) -> Vec<Block> {
    blocks_from(root, &Walk::without_hints(base))
}

/// The block walk that does not enter the subtrees in `skip`: how a post's body is read when
/// its replies nest inside it.
pub fn blocks_from_skipping(
    root: ElementRef<'_>,
    base: &Url,
    skip: &std::collections::HashSet<ego_tree::NodeId>,
) -> Vec<Block> {
    let walk = Walk {
        skip: Some(skip),
        ..Walk::bare(base)
    };
    blocks_from(root, &walk)
}

/// [`blocks_from_skipping`] with the loose-text floor lifted: how a tweet's message is
/// read once `tweet::detect` has taken the attribution and the counts out of the container.
///
/// The floor exists to stop a stray label on a *page* becoming a paragraph. A tweet is not a
/// page. "Well well well" is fourteen characters and is the entire message, and dropping it is
/// how a reader ends up being told a page it has in hand needs JavaScript. `tweet.rs` has
/// already established that this container is one message, which is the claim the floor was
/// standing in for.
pub fn blocks_from_tweet(
    root: ElementRef<'_>,
    base: &Url,
    skip: &std::collections::HashSet<ego_tree::NodeId>,
) -> Vec<Block> {
    let walk = Walk {
        skip: Some(skip),
        short_runs: true,
        ..Walk::bare(base)
    };
    blocks_from(root, &walk)
}

/// What every walker is handed, and all it is allowed to know: where hrefs resolve, and
/// whether the class-name chrome hints are in force. Never a hostname.
///
/// `noise_hints` is off for a root whose hinted subtrees turned out to be most of its text
/// (see `root_candidates`): a wire service names its story cards `PagePromo`, and a hint that
/// deletes the page is not describing chrome.
#[derive(Clone, Copy)]
pub struct Walk<'a> {
    pub base: &'a Url,
    noise_hints: bool,
    /// How many accepted cards this walk is inside. A lead package nests a card list in a
    /// card, which is one level; past [`MAX_CARD_NESTING`] the walk carries no `cards`, so
    /// a page of nested triplets is walked as plain containers, whose frames the
    /// [`MAX_DEPTH`] budget was measured for. `entry_from` costs a frame of its own per
    /// level, and 256 of those on top of the walker's did not fit the worker stack.
    card_depth: u8,
    /// Which children of which containers are story cards, from `cards::detect`. `None` is
    /// "no cards anywhere", which is what a thread body or a test fixture wants.
    cards: Option<&'a crate::cards::CardMap>,
    /// Subtrees not to enter: a post's body stops at the replies nested inside it, which are
    /// posts of their own.
    skip: Option<&'a std::collections::HashSet<ego_tree::NodeId>>,
    /// Keep a gathered run that clears no length floor. Off everywhere but a tweet body, where
    /// the container is already known to be one message: `MIN_LOOSE_TEXT` is a guess about
    /// *page* text, and "Well well well" is a whole tweet. Same argument as
    /// [`blocks_from_public_unhinted`] makes for a feed summary, one floor further down.
    short_runs: bool,
    /// How many subtrees this walk dropped *because a hint named them*, as opposed to because
    /// of their tag. [`blocks_guarded`] reads it to decide whether the second, unhinted walk can
    /// tell it anything; see there. A shared `Cell` rather than a field, so `Walk` stays `Copy`
    /// and the counter survives the `..*walk` copies the walkers make of it.
    hint_drops: Option<&'a std::cell::Cell<usize>>,
}

impl<'a> Walk<'a> {
    pub fn bare(base: &'a Url) -> Self {
        Self {
            base,
            noise_hints: true,
            card_depth: 0,
            cards: None,
            skip: None,
            short_runs: false,
            hint_drops: None,
        }
    }
    fn counting_hints(self, drops: &'a std::cell::Cell<usize>) -> Self {
        Self {
            hint_drops: Some(drops),
            ..self
        }
    }
    fn without_hints(base: &'a Url) -> Self {
        Self {
            noise_hints: false,
            ..Self::bare(base)
        }
    }
    fn cards(self, cards: &'a crate::cards::CardMap) -> Self {
        Self {
            cards: Some(cards),
            ..self
        }
    }
    /// Is this node a subtree the walk must not enter? Every arm that reaches past a
    /// container's direct children (`<li>`s under a list, `<dd>`s under a `<dl>`) asks this
    /// too: a reply nested as `<ul class="children"><li class="comment">` is a post of its
    /// own, and a guard on the children loop alone walked it into its parent's body.
    fn skips(&self, id: ego_tree::NodeId) -> bool {
        self.skip.is_some_and(|k| k.contains(&id))
    }
}

fn blocks_from(root: ElementRef<'_>, walk: &Walk<'_>) -> Vec<Block> {
    let mut out = Vec::new();
    walk_blocks(*root, walk, &mut out, 0);
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

fn walk_blocks(node: NodeRef<'_, Node>, walk: &Walk<'_>, out: &mut Vec<Block>, depth: usize) {
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
    // Story cards among this container's children become one `Entries` run, in place, so a
    // front page keeps its section headings between its card lists. Only the members the
    // detector accepted; a stray child between two cards is walked as itself.
    let members = walk.cards.and_then(|m| m.get(&node.id()));
    let mut entries: Vec<crate::ir::Entry> = Vec::new();
    for child in node.children() {
        let Some(el) = ElementRef::wrap(child) else {
            if matches!(child.value(), Node::Text(_)) {
                inline_child(child, walk, depth + 1, &mut pending);
            }
            continue;
        };
        if walk.skips(child.id()) {
            continue;
        }
        // Media before the noise check: `iframe` is a noise tag (nothing inside one is
        // readable), but the element itself is a placeholder the reader may act on.
        if matches!(el.value().name(), "video" | "audio" | "iframe") {
            if let Some(b) = embed_from(&el, walk) {
                flush_pending(&mut pending, out, walk);
                flush_entries(&mut entries, out);
                out.push(b);
            }
            continue;
        }
        if is_noise(&el, walk) {
            continue;
        }
        if members.is_some_and(|m| m.contains(&child.id()))
            && let Some(e) = entry_from(&el, walk, depth + 1)
        {
            flush_pending(&mut pending, out, walk);
            entries.push(e);
            continue;
        }
        let name = el.value().name();
        let mut made: Vec<Block> = Vec::new();
        match name {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let level = name.as_bytes()[1] - b'0';
                let inl = trim_inlines(inlines_from(child, walk, depth + 1));
                if !inl.is_empty() {
                    made.push(Block::Heading {
                        level,
                        inlines: inl,
                    });
                }
            }
            "p" => {
                let inl = trim_inlines(inlines_from(child, walk, depth + 1));
                if !inl.is_empty() {
                    made.push(Block::Paragraph(inl));
                }
            }
            // A list whose items are story cards is a card list, not a list: walked as a
            // container, so its members go through the entries path above.
            "ul" | "ol" if walk.cards.is_some_and(|m| m.contains_key(&child.id())) => {
                walk_blocks(child, walk, &mut made, depth + 1);
            }
            "ul" | "ol" => {
                let ordered = name == "ol";
                let li = &*sel::LI;
                let items: Vec<Vec<Block>> = el
                    .select(li)
                    .filter(|l| l.parent().map(|p| p.id()) == Some(child.id()))
                    .filter(|l| !walk.skips(l.id()))
                    .map(|l| {
                        let mut b = Vec::new();
                        walk_blocks(*l, walk, &mut b, depth + 2);
                        if b.is_empty() {
                            let inl = inlines_from(*l, walk, depth + 2);
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
                walk_blocks(child, walk, &mut b, depth + 1);
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
            // A definition list is a list whose items lead with their term. `<dt>` is short by
            // nature and used to fall under the loose-text floor, which dropped every term.
            "dl" => {
                if let Some(b) = dl_from(&el, walk, depth) {
                    made.push(b);
                }
            }
            // `<details>` reads open: its summary is a bold line, whatever its length, and
            // its body follows as ordinary blocks (the container arm below).
            "summary" => {
                let inl = trim_inlines(inlines_from(child, walk, depth + 1));
                if !inl.is_empty() {
                    made.push(Block::Paragraph(vec![Inline::Strong(inl)]));
                }
            }
            "img" => {
                if let Some(img) = image_from(&el, walk.base) {
                    made.push(Block::Figure {
                        image: img,
                        caption: None,
                    });
                }
            }
            "figure" => {
                let img_sel = &*sel::IMG;
                let cap_sel = &*sel::FIGCAPTION;
                if let Some(img) = el
                    .select(img_sel)
                    .next()
                    .and_then(|i| image_from(&i, walk.base))
                {
                    let caption = el
                        .select(cap_sel)
                        .next()
                        .map(|c| inlines_from(*c, walk, depth + 2))
                        .filter(|c| !c.is_empty());
                    made.push(Block::Figure {
                        image: img,
                        caption,
                    });
                } else {
                    // A figure around a table, a code listing, or a quote is a container
                    // like any other. Falling to the inline flatten here turned a hero
                    // caption into a stray paragraph and a figure-wrapped table into a run
                    // of words.
                    walk_blocks(child, walk, &mut made, depth + 1);
                }
            }
            "table" => {
                if let Some(t) = table_from(&el, walk, depth) {
                    made.push(t);
                }
            }
            // Containers: recurse, but only into something that actually holds blocks.
            // Inline content joins the paragraph its siblings are building rather than
            // becoming one of its own, which is what keeps `<a>headline</a> (<a>host</a>)`
            // one line instead of a linked headline and a discarded remainder.
            _ if !BLOCK_LEVEL.contains(&name) && !holds_blocks(&el) => {
                inline_child(child, walk, depth + 1, &mut pending);
            }
            _ => {
                walk_blocks(child, walk, &mut made, depth + 1);
                if made.is_empty() {
                    inline_child(child, walk, depth + 1, &mut pending);
                } else if name == "a"
                    && let Some(href) = el.value().attr("href").and_then(|h| walk.base.join(h).ok())
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
            flush_pending(&mut pending, out, walk);
            flush_entries(&mut entries, out);
            out.append(&mut made);
        }
    }
    flush_pending(&mut pending, out, walk);
    flush_entries(&mut entries, out);
}

fn flush_entries(entries: &mut Vec<crate::ir::Entry>, out: &mut Vec<Block>) {
    if !entries.is_empty() {
        out.push(Block::Entries(std::mem::take(entries)));
    }
}

/// A definition list as a list whose items lead with their term. Out of `walk_blocks` so the
/// recursive frame stays small: 256 frames of it have to fit a 2 MiB thread.
fn dl_from(el: &ElementRef, walk: &Walk<'_>, depth: usize) -> Option<Block> {
    let mut items: Vec<Vec<Block>> = Vec::new();
    let mut terms: Vec<Inline> = Vec::new();
    for c in el.children() {
        let Some(ce) = ElementRef::wrap(c) else {
            continue;
        };
        if walk.skips(c.id()) {
            continue;
        }
        match ce.value().name() {
            "dt" => {
                let t = trim_inlines(inlines_from(c, walk, depth + 2));
                if !t.is_empty() {
                    if !terms.is_empty() {
                        push_space(&mut terms);
                    }
                    terms.extend(t);
                }
            }
            "dd" => {
                let mut item: Vec<Block> = Vec::new();
                if !terms.is_empty() {
                    item.push(Block::Paragraph(vec![Inline::Strong(std::mem::take(
                        &mut terms,
                    ))]));
                }
                walk_blocks(c, walk, &mut item, depth + 2);
                if !item.is_empty() {
                    items.push(item);
                }
            }
            _ => {}
        }
    }
    if !terms.is_empty() {
        items.push(vec![Block::Paragraph(vec![Inline::Strong(terms)])]);
    }
    (!items.is_empty()).then_some(Block::List {
        ordered: false,
        items,
    })
}

/// A `<video>`, `<audio>`, or `<iframe>` as the placeholder the reader may act on. Never
/// loaded here; the source is taken from `src` or the first `<source>`, and a media element
/// with neither (a player that fills itself in by script) is nothing the reader can say.
fn embed_from(el: &ElementRef, walk: &Walk<'_>) -> Option<Block> {
    let kind = match el.value().name() {
        "video" => crate::ir::EmbedKind::Video,
        "audio" => crate::ir::EmbedKind::Audio,
        _ => crate::ir::EmbedKind::Iframe,
    };
    let source = &*sel::SOURCE_SRC;
    let raw = el
        .value()
        .attr("src")
        .or_else(|| el.select(source).next().and_then(|s| s.value().attr("src")))
        .filter(|s| !s.trim().is_empty() && !s.starts_with("data:") && !s.starts_with("about:"))?;
    let url = walk.base.join(raw).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    Some(Block::Embed {
        kind,
        url: url.to_string(),
    })
}

/// One story card as an `Entry`: the headline is the card's heading if it has one and its
/// longest link otherwise; the href is the longest link's, or the card's own when the card is
/// the anchor; the summary is the rest of the card, minus the headline, the thumbnail, and
/// anything shorter than a headline (a kicker, a read time, a date, which `published` keeps).
fn entry_from(card: &ElementRef, walk: &Walk<'_>, depth: usize) -> Option<crate::ir::Entry> {
    let a = &*sel::A_HREF;
    let h = &*sel::HEADING;
    let img = &*sel::IMG;
    let longest = card
        .select(a)
        .map(|l| (text_len(*l), l))
        .max_by_key(|(n, _)| *n);
    let own_href = (card.value().name() == "a")
        .then(|| card.value().attr("href"))
        .flatten();
    let href = longest
        .and_then(|(_, l)| l.value().attr("href"))
        .or(own_href)
        .and_then(|raw| walk.base.join(raw).ok())
        .map(|u| u.to_string());
    let heading = card
        .select(h)
        .find(|e| text_len(**e) >= crate::cards::MIN_HEADLINE);
    // A card that is itself the anchor, with no link inside it, titles itself.
    let title_el = heading
        .or(longest.map(|(_, l)| l))
        .or(own_href.map(|_| *card))?;
    let title = trim_inlines(unimage(unlink(inlines_from(*title_el, walk, depth + 1))));
    let title_text = crate::ir::plain_text(&title);
    if title_text.trim().len() < crate::cards::MIN_HEADLINE {
        return None;
    }
    let published = crate::thread::find_hint(card, crate::thread::TIME_HINT);
    let image = card.select(img).find_map(|i| image_from(&i, walk.base));
    // The card's own blocks, at this depth and not from zero: an accepted card list can sit
    // inside an accepted card (a lead package), and restarting the count at each one let a
    // page of nested triplets walk past `MAX_DEPTH` and off the end of the stack. One level
    // of nesting is a real shape; past `MAX_CARD_NESTING` the body is walked without cards.
    let inner = Walk {
        card_depth: walk.card_depth + 1,
        cards: walk
            .cards
            .filter(|_| walk.card_depth + 1 < MAX_CARD_NESTING),
        ..*walk
    };
    let mut body = Vec::new();
    walk_blocks(**card, &inner, &mut body, depth);
    let summary = summary_of(body, &title_text, href.as_deref(), published.as_deref());
    Some(crate::ir::Entry {
        title,
        href,
        summary,
        published,
        // A card says where it goes in its own words. See `ir::Entry::address`.
        address: None,
        // And every card on a front page leads to the same site, so a mark per row would say
        // one thing many times. See `ir::Entry::icon`.
        icon: None,
        image,
    })
}

/// What a card said beyond its headline: its paragraphs and lists, minus the headline
/// itself, the thumbnail and its alt, the time, and anything shorter than a headline (a
/// kicker, a read time, a byline). Recursive through lists, because a lead package nests its
/// secondary stories as one.
fn summary_of(
    blocks: Vec<Block>,
    title_text: &str,
    href: Option<&str>,
    published: Option<&str>,
) -> Vec<Block> {
    let keep_text = |inlines: &[Inline]| {
        let t = crate::ir::plain_text(inlines);
        let t = t.trim();
        let repeats = t.is_empty()
            || title_text.contains(t)
            || t.contains(title_text.trim())
            || published.is_some_and(|p| p.contains(t));
        !repeats && t.len() >= SUMMARY_TEXT
    };
    let mut after_image = false;
    blocks
        .into_iter()
        .filter_map(|b| match b {
            Block::Figure { .. } => {
                after_image = true;
                None
            }
            Block::Heading { inlines, .. } | Block::Paragraph(inlines) => {
                // A short run carrying the thumbnail, or directly under it, is its credit
                // line: the photographer and the agency, not the dek.
                let had_image = has_image(&inlines);
                let credit_slot = had_image || std::mem::take(&mut after_image);
                let inlines = unimage(unlink_to(inlines, href));
                let text_len = crate::ir::plain_text(&inlines).trim().len();
                if credit_slot && text_len < CREDIT_TEXT {
                    return None;
                }
                keep_text(&inlines).then_some(Block::Paragraph(inlines))
            }
            Block::List { ordered, items } => {
                after_image = false;
                let items: Vec<Vec<Block>> = items
                    .into_iter()
                    .map(|item| summary_of(item, title_text, href, published))
                    .filter(|item| !item.is_empty())
                    .collect();
                (!items.is_empty()).then_some(Block::List { ordered, items })
            }
            other => Some(other),
        })
        .collect()
}

/// Text left beside a thumbnail under this length is a photo credit, not a dek.
const CREDIT_TEXT: usize = 60;
/// A summary line shorter than this is a kicker, a read time, or a comment count. A dek is a
/// sentence.
const SUMMARY_TEXT: usize = 25;

/// Links to `href` replaced by their text, other links kept: a dek that links to its own
/// story again is noise, a lead package's secondary stories are not.
fn unlink_to(inlines: Vec<Inline>, href: Option<&str>) -> Vec<Inline> {
    inlines
        .into_iter()
        .flat_map(|i| match i {
            Inline::Link { href: h, inlines } if Some(h.as_str()) == href => {
                unlink_to(inlines, href)
            }
            Inline::Link { href: h, inlines } => vec![Inline::Link {
                href: h,
                inlines: unlink_to(inlines, href),
            }],
            Inline::Emph(v) => vec![Inline::Emph(unlink_to(v, href))],
            Inline::Strong(v) => vec![Inline::Strong(unlink_to(v, href))],
            other => vec![other],
        })
        .collect()
}

fn has_image(inlines: &[Inline]) -> bool {
    inlines.iter().any(|i| match i {
        Inline::Image(_) => true,
        Inline::Emph(v) | Inline::Strong(v) | Inline::Link { inlines: v, .. } => has_image(v),
        _ => false,
    })
}

/// Inline images dropped: a thumbnail's alt is a description of a picture that is not shown.
fn unimage(inlines: Vec<Inline>) -> Vec<Inline> {
    inlines
        .into_iter()
        .filter_map(|i| match i {
            Inline::Image(_) => None,
            Inline::Emph(v) => Some(Inline::Emph(unimage(v))),
            Inline::Strong(v) => Some(Inline::Strong(unimage(v))),
            Inline::Link { href, inlines } => Some(Inline::Link {
                href,
                inlines: unimage(inlines),
            }),
            other => Some(other),
        })
        .collect()
}

/// Links replaced by their text: an entry's title is one link already, and a dek that links
/// to the same story three times is noise in a renderer that underlines.
fn unlink(inlines: Vec<Inline>) -> Vec<Inline> {
    inlines
        .into_iter()
        .flat_map(|i| match i {
            Inline::Link { inlines, .. } => unlink(inlines),
            Inline::Emph(v) => vec![Inline::Emph(unlink(v))],
            Inline::Strong(v) => vec![Inline::Strong(unlink(v))],
            other => vec![other],
        })
        .collect()
}

/// Emit the gathered inline run as a paragraph.
///
/// A run with no link has to clear [`MIN_LOOSE_TEXT`]. A run carrying one is kept whatever
/// its length: a headline is often shorter than the floor, and the floor was dropping the
/// only route to the page behind it.
fn flush_pending(pending: &mut Vec<Inline>, out: &mut Vec<Block>, walk: &Walk<'_>) {
    let inl = trim_inlines(std::mem::take(pending));
    if inl.iter().all(is_blank) {
        return;
    }
    // A run that is one image, linked or not, is a picture: a figure, not a line of alt
    // text set in the link face. Front pages wrap every thumbnail this way.
    if let Some(img) = lone_image(&inl) {
        out.push(Block::Figure {
            image: img,
            caption: None,
        });
        return;
    }
    // A short run of links into the page itself ("Skip to content", a "close" button with
    // `href="#"`) is a control, not prose. A table of contents is a list, and list items
    // do not come through here.
    if crate::ir::inlines_text_len(&inl) <= MIN_LOOSE_TEXT
        && all_same_document_links(&inl, walk.base)
    {
        return;
    }
    if walk.short_runs
        || crate::ir::inlines_text_len(&inl) > MIN_LOOSE_TEXT
        || inl.iter().any(has_link)
    {
        out.push(Block::Paragraph(inl));
    }
}

/// The one image a run consists of, through a link if there is one; `None` if the run holds
/// anything else.
fn lone_image(inl: &[Inline]) -> Option<Image> {
    let mut image = None;
    for i in inl {
        match i {
            Inline::Text(t) if t.trim().is_empty() => {}
            Inline::Image(img) if image.is_none() => image = Some(img.clone()),
            Inline::Link { inlines, .. } => {
                for j in inlines {
                    match j {
                        Inline::Text(t) if t.trim().is_empty() => {}
                        Inline::Image(img) if image.is_none() => image = Some(img.clone()),
                        _ => return None,
                    }
                }
            }
            _ => return None,
        }
    }
    image
}

/// Every non-blank inline is a link whose target is this page plus a fragment.
fn all_same_document_links(inl: &[Inline], base: &Url) -> bool {
    use url::Position;
    let here = &base[..Position::AfterQuery];
    let mut any = false;
    for i in inl {
        match i {
            Inline::Text(t) if t.trim().is_empty() => {}
            Inline::Link { href, .. } => {
                let Ok(u) = Url::parse(href) else {
                    return false;
                };
                if u.fragment().is_none() || &u[..Position::AfterQuery] != here {
                    return false;
                }
                any = true;
            }
            _ => return false,
        }
    }
    any
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
        | Block::Thread(_)
        | Block::Entries(_)
        | Block::Tweets(_) => {}
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

fn inlines_from(node: NodeRef<'_, Node>, walk: &Walk<'_>, depth: usize) -> Vec<Inline> {
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
        inline_child(child, walk, depth, &mut out);
    }
    out
}

/// One child node as inline content. Split out of [`inlines_from`] because [`walk_blocks`]
/// needs the same mapping for a single child: an element that turned out to hold no blocks
/// is inline, and reaching it through `inlines_from` on its *parent* would re-walk siblings
/// that have already been emitted.
fn inline_child(child: NodeRef<'_, Node>, walk: &Walk<'_>, depth: usize, out: &mut Vec<Inline>) {
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
            // A container that emitted no blocks falls to this flatten, and the one whose
            // every block was a nested post is exactly that container: without the check
            // the post the block walk stepped around came back as inline text.
            if walk.skips(child.id()) || is_noise(&el, walk) {
                return;
            }
            let name = e.name.local.as_ref();
            match name {
                // An emphasis element holding nothing but a space (`representing<strong>
                // </strong>the`, which one CMS emits) is the space; wrapping it welds the
                // words either side.
                "em" | "i" | "strong" | "b" => {
                    let inl = inlines_from(child, walk, depth + 1);
                    if inl.iter().all(is_blank) {
                        let held_text = child
                            .descendants()
                            .any(|n| matches!(n.value(), Node::Text(t) if !t.is_empty()));
                        if held_text {
                            push_space(out);
                        }
                    } else if name == "em" || name == "i" {
                        out.push(Inline::Emph(inl));
                    } else {
                        out.push(Inline::Strong(inl));
                    }
                }
                "code" | "kbd" | "samp" => out.push(Inline::Code(inner_text(child))),
                "br" => out.push(Inline::Break),
                "img" => {
                    if let Some(i) = image_from(&el, walk.base) {
                        out.push(Inline::Image(i));
                    }
                }
                "a" => {
                    let inl = without_repeated_alt(inlines_from(child, walk, depth + 1));
                    match el.value().attr("href").and_then(|h| walk.base.join(h).ok()) {
                        Some(href) if !inl.is_empty() => out.push(Inline::Link {
                            href: href.to_string(),
                            inlines: inl,
                        }),
                        _ => out.extend(inl),
                    }
                }
                // A block-level element flattened into a run (a byline built from sibling
                // `<div>`s) starts and ends on a word boundary, as it would on a page; an
                // inline one (`<span>`) does not, as it would not.
                _ if BLOCK_LEVEL.contains(&name) => {
                    push_space(out);
                    out.extend(inlines_from(child, walk, depth + 1));
                    push_space(out);
                }
                _ => out.extend(inlines_from(child, walk, depth + 1)),
            }
        }
        _ => {}
    }
}

/// An image inside a link whose alt repeats the link's own text is the headline drawn twice:
/// once as a picture nobody has loaded and once as words. The words stay.
fn without_repeated_alt(inl: Vec<Inline>) -> Vec<Inline> {
    let words: String = inl
        .iter()
        .filter(|i| !matches!(i, Inline::Image(_)))
        .map(|i| crate::ir::plain_text(std::slice::from_ref(i)))
        .collect::<String>()
        .to_lowercase();
    let words = words.trim();
    if words.len() < 10 {
        return inl;
    }
    inl.into_iter()
        .filter(|i| match i {
            Inline::Image(img) => {
                let alt = img.alt.as_deref().unwrap_or("").trim().to_lowercase();
                alt.len() < 10 || !(alt.contains(words) || words.contains(alt.as_str()))
            }
            _ => true,
        })
        .collect()
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

pub(crate) fn image_from(el: &ElementRef, base: &Url) -> Option<Image> {
    let v = el.value();
    let raw = v
        .attr("src")
        .filter(|s| !s.starts_with("data:"))
        .or_else(|| v.attr("data-src"))
        .or_else(|| v.attr("data-original"))
        .or_else(|| v.attr("srcset").and_then(first_of_srcset))
        .or_else(|| v.attr("data-srcset").and_then(first_of_srcset))
        .or_else(|| {
            // `<picture><source srcset><img></picture>` with an `<img>` that carries nothing
            // but a placeholder: the first source's first candidate is the image.
            let parent = el.parent().and_then(ElementRef::wrap)?;
            if parent.value().name() != "picture" {
                return None;
            }
            let source = &*sel::SOURCE_SRCSET;
            parent
                .select(source)
                .find_map(|s| s.value().attr("srcset").and_then(first_of_srcset))
        })?;
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

/// The first URL in a `srcset`: `url 1x, url 2x` or `url 256w, url 512w`. The smallest
/// candidate is listed first by convention, and this reader decodes at column width anyway.
///
/// Not `split(',')`: a URL may carry commas (an image CDN's transform segment,
/// `/upload/w_300,h_200,c_fill/photo.jpg`), and the HTML grammar separates candidates on a
/// comma only where it ends a URL token. A URL runs to the first whitespace, loses any commas
/// it ends in, and the descriptors after it run to the next comma.
fn first_of_srcset(srcset: &str) -> Option<&str> {
    let mut rest = srcset;
    loop {
        rest = rest.trim_start_matches(|c: char| c.is_whitespace() || c == ',');
        if rest.is_empty() {
            return None;
        }
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let token = &rest[..end];
        let url = token.trim_end_matches(',');
        if !url.is_empty() && !url.starts_with("data:") {
            return Some(url);
        }
        // A token that ended in a comma had no descriptors; otherwise they run to the comma.
        rest = if url.len() < token.len() {
            &rest[end..]
        } else {
            match rest[end..].find(',') {
                Some(i) => &rest[end + i + 1..],
                None => "",
            }
        };
    }
}

/// A row is a *header* row only when every cell it owns is a `<th>`.
///
/// Splitting on "does this row contain any `<th>`" instead loses data both ways on the
/// row-header tables that infoboxes and spec sheets are built from: the first
/// `<tr><th>Name</th><td>Alice</td></tr>` contributed `Name` as the only header and dropped
/// `Alice`, and every row after it contributed its `<td>`s and dropped its `<th>`. Cells are
/// collected in document order with one selector so a mixed row keeps its shape.
fn table_from(el: &ElementRef, walk: &Walk<'_>, depth: usize) -> Option<Block> {
    let tr = &*sel::TR;
    let cell = &*sel::CELL;
    let mut headers: Vec<Vec<Inline>> = Vec::new();
    let mut rows: Vec<Vec<Vec<Inline>>> = Vec::new();
    // Descendant selectors reach *through* nested tables, so every cell of an inner table
    // would be emitted once for each enclosing table. Keep only cells this table owns.
    for row in el.select(tr).filter(|r| owning_table(**r) == Some(el.id())) {
        let cells: Vec<(bool, Vec<Inline>)> = row
            .select(cell)
            .filter(|c| owning_row(**c) == Some(row.id()))
            .map(|c| (c.value().name() == "th", inlines_from(*c, walk, depth + 1)))
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
    let sel = &*sel::TITLE;
    html.select(sel)
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

/// The page's favicon URL, and which source named it.
///
/// Prefers a `<link rel=icon>` (and the apple-touch variants) over the well-known
/// `/favicon.ico` fallback. SVG is skipped when a raster alternative exists: this crate
/// deliberately does not decode SVG. `data:` hrefs are skipped the same way image `src` is.
fn favicon_of(html: &Html, base: &Url) -> (Option<String>, &'static str) {
    let mut best: Option<(i32, u32, String)> = None;
    for el in html.select(&sel::LINK_REL) {
        let rel = el.value().attr("rel").unwrap_or("");
        if !is_icon_rel(rel) {
            continue;
        }
        let href = el.value().attr("href").unwrap_or("");
        if href.is_empty() || href.starts_with("data:") {
            continue;
        }
        let Ok(url) = base.join(href) else {
            continue;
        };
        let typ = el.value().attr("type");
        let sizes = el.value().attr("sizes");
        let rank = icon_rank(url.as_str(), typ, sizes);
        // SVG ranks at 0: keep it only when nothing better turned up.
        if rank.0 == 0 && best.as_ref().is_some_and(|(r, ..)| *r > 0) {
            continue;
        }
        match &best {
            Some((r, s, _)) if (*r, *s) >= rank => {}
            _ => best = Some((rank.0, rank.1, url.to_string())),
        }
    }
    if let Some((rank, _, url)) = best
        && rank > 0
    {
        return (Some(url), "<link rel=icon>");
    }
    // Well-known fallback. Browsers request this when markup is silent; so do we.
    // An SVG-only `<link>` loses to the fallback: this crate does not decode SVG.
    match base.join("/favicon.ico") {
        Ok(u) => (Some(u.to_string()), "/favicon.ico"),
        Err(_) => (None, "none"),
    }
}

fn is_icon_rel(rel: &str) -> bool {
    rel.split_whitespace().any(|t| {
        let t = t.to_ascii_lowercase();
        matches!(
            t.as_str(),
            "icon" | "apple-touch-icon" | "apple-touch-icon-precomposed"
        )
    })
}

/// `(format_rank, size_px)`. Higher wins. SVG is 0 so a raster link always beats it.
fn icon_rank(href: &str, typ: Option<&str>, sizes: Option<&str>) -> (i32, u32) {
    let path = href.split('?').next().unwrap_or(href).to_ascii_lowercase();
    let typ = typ.map(|t| t.to_ascii_lowercase()).unwrap_or_default();
    let format = if typ.contains("svg") || path.ends_with(".svg") {
        0
    } else if typ.contains("png") || path.ends_with(".png") {
        4
    } else if typ.contains("icon") || path.ends_with(".ico") {
        3
    } else if typ.contains("jpeg")
        || typ.contains("jpg")
        || path.ends_with(".jpg")
        || path.ends_with(".jpeg")
        || typ.contains("webp")
        || path.ends_with(".webp")
        || typ.contains("gif")
        || path.ends_with(".gif")
    {
        2
    } else {
        1
    };
    let size = sizes
        .map(|s| {
            s.split_whitespace()
                .filter_map(|tok| {
                    let tok = tok.to_ascii_lowercase();
                    if tok == "any" {
                        return None;
                    }
                    let (w, h) = tok.split_once('x')?;
                    Some(w.parse::<u32>().ok()?.max(h.parse().ok()?))
                })
                .max()
                .unwrap_or(0)
        })
        .unwrap_or(0);
    (format, size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{Block, Inline};

    fn doc(html: &str) -> Document {
        extract(html, &Url::parse("https://example.test/a/b").unwrap())
    }

    /// A tweet fixture in the chain a status permalink actually serves: `main > div > article`,
    /// with the login prompt the page puts beside it.
    fn tweet_page(words: &str, prompt: &str) -> String {
        format!(
            r#"<html><body><div><main><div><article>
                 <div><a href="/writer"><img alt="@writer" src="/pfp.jpg"></a>
                   <a href="https://example.test/writer">A Writer</a>
                   <a href="https://example.test/writer">@writer</a></div>
                 <div dir="auto"><span>{words}</span></div>
                 <div><span><a href="/a/b">10:47 PM · Jul 5, 2026</a></span>
                      <span><a href="/a/b"><span>108.2M</span><span>Views</span></a></span></div>
                 <div><a aria-label="Reply" href="/i/1"><span>56K</span></a>
                      <button aria-label="Repost"><span>166K</span></button>
                      <button aria-label="Like"><span>2.6M</span></button></div>
               </article></div></main></div><div>{prompt}</div></body></html>"#
        )
    }

    /// The whole point. Before `tweet.rs`, this page scored under every floor, fell to `<body>`,
    /// and drew the "may need JavaScript" caution over content that was entirely present.
    #[test]
    fn a_permalinked_tweet_becomes_a_tweet_and_not_a_body_fallback() {
        let src = tweet_page("Well well well", "Log in or sign up");
        let d = doc(&src);
        let [Block::Tweets(tweets)] = d.blocks.as_slice() else {
            panic!("expected one run of tweets, got {:?}", d.blocks);
        };
        assert_eq!(tweets.len(), 1);
        assert_eq!(tweets[0].handle.as_deref(), Some("@writer"));
        assert_eq!(tweets[0].stats.len(), 4);
        let why = explain(&src, &Url::parse("https://example.test/a/b").unwrap());
        assert_eq!(why.tweet_merge, "became the document");
        assert!(
            why.tweets,
            "the flag `session::explain` reads must agree with the document"
        );
        assert!(!why.body_fallback, "the tweet must survive step 5");
    }

    /// `Explanation::tweets` is the one fact `session::explain` has, and `session::load` reads
    /// the document instead. Both routes must answer alike on both kinds of page, or `--why`
    /// and the reader disagree about whether the thin-text caution belongs on it.
    #[test]
    fn the_tweets_flag_agrees_with_the_document_either_way() {
        let base = Url::parse("https://example.test/a/b").unwrap();
        for src in [
            tweet_page("Well well well", "Log in or sign up"),
            r#"<html><body><article><h1>A headline</h1>
               <p>Body prose long enough to clear the floor, and then some more of it, and more
               still, so that the article path wins and no tweet is installed anywhere.</p>
               </article></body></html>"#
                .to_owned(),
        ] {
            let d = doc(&src);
            let in_doc = d.blocks.iter().any(|b| matches!(b, Block::Tweets(_)));
            let why = explain(&src, &base);
            assert_eq!(why.tweets, in_doc, "{why}");
            assert_eq!(
                why.tweets,
                why.tweet_merge == "became the document",
                "{why}"
            );
        }
    }

    /// A tweet is under every floor in this file having parsed perfectly, so the `<body>`
    /// fallback would trade the message for the navigation around it. `is_post` is the guard.
    #[test]
    fn the_body_fallback_does_not_overwrite_a_tweet() {
        // Chrome that `<body>` would emit several times over, and that no root candidate can
        // win with: all links, so `raw_text * (1 - link_density)` stays under the floor.
        let prompt = r#"<a href="https://example.test/login">See what is happening on here and join the conversation today</a>"#.repeat(12);
        let src = tweet_page("Well well well", &prompt);
        let why = explain(&src, &Url::parse("https://example.test/a/b").unwrap());
        assert_eq!(why.tweet_merge, "became the document", "{why}");
        let d = doc(&src);
        assert!(
            matches!(d.blocks.as_slice(), [Block::Tweets(_)]),
            "the body walk won: {:?}",
            d.blocks
        );
    }

    /// The rescue must not become a competitor. An article that carries counted controls is
    /// still an article, because the article path cleared the floor and the tweet path only
    /// takes a page nothing else could read.
    #[test]
    fn an_article_that_reads_is_never_replaced_by_a_tweet() {
        let prose = "Body prose long enough to clear the floor, and then some more of it. ";
        let src = format!(
            r#"<html><body><article>
                 <h1>A headline about a thing</h1>
                 <div><a href="https://example.test/writer">A Writer</a>
                      <a href="https://example.test/writer">@writer</a></div>
                 <p>{prose}</p><p>{prose}</p><p>{prose}</p>
                 <div><span><a href="/a/b">Jul 5, 2026</a></span></div>
                 <div><button aria-label="Like"><span>1.2K</span></button>
                      <button aria-label="Bookmark"><span>340</span></button></div>
               </article></body></html>"#
        );
        let d = doc(&src);
        assert!(
            !d.blocks.iter().any(|b| matches!(b, Block::Tweets(_))),
            "an article that reads was redrawn as a tweet: {:?}",
            d.blocks
        );
        let why = explain(&src, &Url::parse("https://example.test/a/b").unwrap());
        assert_eq!(why.tweet_merge, "dropped (the page already read)");
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
        // No <link rel=icon>: the well-known path, resolved against the page URL.
        assert_eq!(
            d.favicon.as_deref(),
            Some("https://example.test/favicon.ico")
        );
    }

    #[test]
    fn a_link_rel_icon_beats_the_well_known_fallback() {
        let d = doc(r#"<html><head>
              <link rel="icon" href="/static/mark.png" sizes="32x32">
              <link rel="apple-touch-icon" href="/static/touch.png" sizes="180x180">
            </head><body><article><p>Enough prose that this page has a body worth extracting as an article page for the favicon test.</p></article></body></html>"#);
        // Larger PNG wins over the smaller one; both beat /favicon.ico.
        assert_eq!(
            d.favicon.as_deref(),
            Some("https://example.test/static/touch.png")
        );
    }

    #[test]
    fn a_raster_icon_beats_an_svg_one() {
        let d = doc(r#"<html><head>
              <link rel="icon" href="/mark.svg" type="image/svg+xml">
              <link rel="icon" href="/mark.ico">
            </head><body><article><p>Enough prose that this page has a body worth extracting as an article page for the favicon test.</p></article></body></html>"#);
        assert_eq!(d.favicon.as_deref(), Some("https://example.test/mark.ico"));
    }

    #[test]
    fn an_svg_only_icon_falls_back_to_favicon_ico() {
        let d = doc(r#"<html><head>
              <link rel="icon" href="/mark.svg" type="image/svg+xml" sizes="any">
            </head><body><article><p>Enough prose that this page has a body worth extracting as an article page for the favicon test.</p></article></body></html>"#);
        assert_eq!(
            d.favicon.as_deref(),
            Some("https://example.test/favicon.ico")
        );
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
    /// and timestamps on Hacker News. See docs/findings.md.
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
                Block::Entries(es) => es.iter().for_each(|e| {
                    out.extend(e.href.clone());
                    e.summary.iter().for_each(|b| walk(b, out));
                }),
                Block::Thread(cs) => cs.iter().flat_map(|c| &c.blocks).for_each(|b| walk(b, out)),
                Block::Tweets(ps) => ps.iter().for_each(|p| {
                    out.extend(p.permalink.clone());
                    p.blocks.iter().for_each(|b| walk(b, out));
                }),
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
    /// Capped at 5,000 rather than higher because `content_root`'s ancestor scoring is quadratic
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

    /// The memo must agree with the walk it replaces on every node, or scoring shifts. The
    /// fixture mixes the cases that make composition subtle: adjacent inline elements with and
    /// without a whitespace node between them, a block that spaces its neighbours, a `<br>`, an
    /// anchor, and links inside a `<nav>` (NOISE for text, but `select("a")` still counts them,
    /// so `text_len` and the `link_density` numerator disagree about that subtree by design).
    #[test]
    fn text_metrics_match_the_reference_walk() {
        let src = "<html><body><article>\
            <p>One <b>two</b>three <a href=\"x\">four five</a> six.</p>\
            <div><span>a</span><span>b</span> <span>c</span></div>\
            <p>line<br>break</p>\
            <nav><a href=\"y\">menu link one</a> <a href=\"z\">menu link two</a></nav>\
            <ul><li>alpha beta</li><li>gamma</li></ul>\
            <pre>code   spaced</pre>\
            </article></body></html>";
        let html = Html::parse_document(src);
        let metrics = build_text_metrics(&html);
        let a_sel = Selector::parse("a").unwrap();
        for el in html.select(&Selector::parse("*").unwrap()) {
            let id = el.id();
            let total = text_len(*el);
            assert_eq!(
                metrics.text_len(id),
                total,
                "text_len at <{}>",
                el.value().name()
            );
            let want = if total == 0 {
                0.0
            } else {
                let linked: usize = el.select(&a_sel).map(|a| text_len(*a)).sum();
                (linked as f64 / total as f64).min(1.0)
            };
            assert_eq!(
                metrics.link_density(id),
                want,
                "link_density at <{}>",
                el.value().name()
            );
        }
    }

    /// Text at *every* level makes each `<div>` a scored candidate, which is what turned root
    /// selection O(n²): a scored node re-measured the whole chain below it. With one shared
    /// measurement pass it is linear. The ceiling is generous enough for a debug build and far
    /// under what a quadratic regression at this size would take (minutes).
    #[test]
    fn deeply_nested_text_extracts_in_linear_time() {
        const DEPTH: usize = 6_000;
        let mut src = String::from("<html><body>");
        for _ in 0..DEPTH {
            src.push_str("<div>twenty five plus characters here ");
        }
        src.push_str(&"</div>".repeat(DEPTH));
        src.push_str("</body></html>");

        let elapsed = std::thread::Builder::new()
            .stack_size(2 * 1024 * 1024)
            .spawn(move || {
                let start = std::time::Instant::now();
                let doc = extract(&src, &Url::parse("https://example.com/a").unwrap());
                assert!(doc.text_len() > 0);
                start.elapsed()
            })
            .unwrap()
            .join()
            .expect("extraction unwound");
        assert!(elapsed.as_secs() < 10, "extraction took {elapsed:?}");
    }

    /// The skipped second walk would have produced what the first one did.
    ///
    /// `blocks_guarded` returns the hinted result unread when no hint dropped anything, on the
    /// argument that the two walks differ on one line. That is only true if the counter sees
    /// every place that line is reached, and the walker reaches it twice: once for a block
    /// child and once for an inline one. A page whose only hinted element sits inside a
    /// paragraph is the case a counter wired to `walk_blocks` alone would get wrong — it would
    /// report zero drops, take the shortcut, and hand back a document with the hinted span still
    /// in it.
    ///
    /// Each fixture is compared against the two-walk answer computed here, so this pins the
    /// equivalence rather than a remembered output.
    #[test]
    fn skipping_the_unhinted_walk_changes_nothing() {
        let prose = "Twenty five plus characters of prose, enough to clear the floors that the \
                     walkers apply to a loose run of text.";
        let cases = [
            // Nothing hinted: the shortcut fires.
            format!("<div id=root><p>{prose}</p><p>{prose}</p></div>"),
            // A hinted block child: the shortcut must not fire.
            format!("<div id=root><p>{prose}</p><aside class=share>{prose}</aside></div>"),
            // A hinted *inline* child, reached through the flatten rather than through
            // `walk_blocks`. This is the one that catches a half-wired counter.
            format!("<div id=root><p>{prose} <span class=share>skip me</span></p></div>"),
            // A hint that eats most of the root, which is what the guard exists for.
            format!("<div id=root><div class=promo><p>{prose}</p><p>{prose}</p></div></div>"),
            // Chrome by tag rather than by hint: dropped in both walks, counted in neither.
            format!("<div id=root><p>{prose}</p><nav><p>{prose}</p></nav></div>"),
        ];
        let base = Url::parse("https://example.com/a").unwrap();
        let cards = crate::cards::CardMap::new();
        for body in cases {
            let html = Html::parse_document(&format!("<html><body>{body}</body></html>"));
            let root = html
                .select(&Selector::parse("#root").unwrap())
                .next()
                .expect("the fixture has a root");

            let with = blocks_from(root, &Walk::bare(&base).cards(&cards));
            let without = blocks_from(root, &Walk::without_hints(&base).cards(&cards));
            let want = if block_text(&without) > 2 * block_text(&with) {
                (without, true)
            } else {
                (with, false)
            };

            let got = blocks_guarded(root, &base, &cards);
            assert_eq!(got.1, want.1, "hints_off differs on {body}");
            assert_eq!(
                crate::render::to_text(
                    &Document {
                        url: String::new(),
                        title: None,
                        byline: None,
                        published: None,
                        site_name: None,
                        favicon: None,
                        lang: None,
                        blocks: got.0,
                    },
                    &crate::render::TextOpts::default()
                ),
                crate::render::to_text(
                    &Document {
                        url: String::new(),
                        title: None,
                        byline: None,
                        published: None,
                        site_name: None,
                        favicon: None,
                        lang: None,
                        blocks: want.0,
                    },
                    &crate::render::TextOpts::default()
                ),
                "blocks differ on {body}"
            );
        }
    }

    /// Root selection has to answer the same way twice.
    ///
    /// `score_best` reads its scores out of a `HashMap`, whose iteration order is seeded per
    /// process, and `max_by` keeps the last maximum it sees. A page whose containers score
    /// equally therefore chose its root by hash order: the chain above extracted 53k, 107k, and
    /// 168k characters on three consecutive runs of one binary, and a reader reloading a page
    /// could be handed a different article each time. The tie-break in `score_best` is what
    /// closes that, and this pins which way it goes — the outer container, which is the one
    /// `choose_root` prefers on the emitted-text ranking that follows.
    ///
    /// One process cannot observe the randomness (a map with the same contents iterates the same
    /// way all run), so this asserts the *rule* rather than trying to catch the flake.
    #[test]
    fn a_score_tie_is_broken_toward_the_outer_container() {
        // A chain of wrappers each carrying its own prose, which is the shape that ties: every
        // `<div>` is a scored element in its own right, and each credits its parent fully, its
        // grandparent at half, and its great-grandparent at a quarter. `a`, `b`, and the chain's
        // outermost wrapper all collect 1.75 shares; `c` collects 1.5 and `d` one.
        let prose = "Twenty five plus characters of prose, enough that this wrapper clears the \
                     scoring floor on its own text.";
        let mut src = String::from("<html><body><div id=top>");
        for id in ["a", "b", "c", "d"] {
            src.push_str(&format!("<div id={id}>{prose}"));
        }
        src.push_str(&"</div>".repeat(4));
        src.push_str("</div></body></html>");

        let why = explain(&src, &Url::parse("https://example.com/a").unwrap());
        let winner = why.winner.expect("a root was chosen");
        assert_eq!(
            why.candidates[winner].id.as_deref(),
            Some("top"),
            "the outermost of the tied containers wins: {why}"
        );
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

    /// Regression: one wire service classes every story card `PagePromo`, `promo` is a
    /// chrome hint, and its front page emitted 2,371 of 16,673 characters. A hint that
    /// deletes most of a root is not describing chrome; the root is walked without hints and
    /// the explanation says so.
    #[test]
    fn a_noise_hint_that_eats_the_root_is_ignored() {
        let cards = (0..8)
            .map(|i| {
                format!(
                    "<div class=\"PagePromo\"><h3 class=\"PagePromo-title\"><a href=\"/s{i}\">Headline \
                     {i} about something that happened in the world today</a></h3><p class=\"PagePromo-description\">A \
                     summary sentence for story {i}, long enough to be worth reading and scoring.</p></div>"
                )
            })
            .collect::<String>();
        let src = format!(
            "<html><body><main><p>Top stories, a short intro.</p>{cards}</main></body></html>"
        );
        let url = Url::parse("https://example.test/").unwrap();
        let why = explain(&src, &url);
        let d = extract(&src, &url);
        let w = why.winner.expect("a root");
        assert!(why.candidates[w].hints_off, "{why}");
        assert!(
            d.text_len() > 600,
            "cards were dropped as chrome: {} chars\n{why}",
            d.text_len()
        );
        assert!(why.to_string().contains("(chrome hints off)"));
    }

    /// The control for the guard: a real sidebar under the root is still dropped, because it
    /// is not most of the root's text.
    #[test]
    fn a_related_sidebar_is_still_chrome() {
        let body = "Body prose long enough to clear the two-hundred-character floor on its own, \
                    repeated so the article is the obvious root of this page and nothing else. "
            .repeat(6);
        let src = format!(
            "<html><body><main><article><p>{body}</p></article>\
             <div class=\"related\"><p>Related: a story you did not ask for, with a sentence.</p>\
             <p>Related: another one, also with a sentence of its own.</p></div></main></body></html>"
        );
        let d = doc(&src);
        let text = crate::render::to_text(&d, &crate::render::TextOpts::default());
        assert!(!text.contains("Related:"), "sidebar bled in:\n{text}");
    }

    /// Regression: two of the top-ten US news sites put a URL in `article:author`. A link is
    /// not a name; the visible byline is.
    #[test]
    fn a_url_is_not_a_byline_and_the_visible_one_is_used() {
        let body =
            "Prose long enough to clear the floor, repeated a few times over for size. ".repeat(4);
        let src = format!(
            "<html><head><meta property=\"article:author\" content=\"https://www.facebook.com/example\">\
             </head><body><article><div class=\"byline\">By Jane Writer</div><p>{body}</p></article></body></html>"
        );
        let d = doc(&src);
        assert_eq!(d.byline.as_deref(), Some("Jane Writer"));
        let why = explain(&src, &Url::parse("https://example.test/a").unwrap());
        assert_eq!(why.meta[1].1, "[class~=byline]");
    }

    /// Regression: `<main>` and `div.article-body` emitted the same 5,774 characters and the
    /// tie went to `<main>`, which is the one with the "close" button and the app banner.
    #[test]
    fn an_emitted_tie_goes_to_the_tighter_container() {
        let body = "Body prose long enough to clear the two-hundred-character floor on its own, \
                    repeated so the article is the obvious root of this page and nothing else. "
            .repeat(4);
        let src = format!(
            "<html><body><main><nav>a b c d e f g h i j k l m n o p q r s t u v w x y z</nav>\
             <article><p>{body}</p><p>{body}</p></article></main></body></html>"
        );
        let why = explain(&src, &Url::parse("https://example.test/a").unwrap());
        let w = why.winner.unwrap();
        assert_eq!(why.candidates[w].tag, "article", "{why}");
    }

    /// Regression: a hero image served as `<picture><source srcset><img src="data:...">`
    /// found no `src` worth the name, the figure arm emitted nothing, and the caption fell
    /// through as a bare paragraph above the body.
    #[test]
    fn a_picture_element_and_a_srcset_are_images() {
        let body =
            "Prose long enough to clear the floor, repeated a few times over for size. ".repeat(4);
        let src = format!(
            "<html><body><article>\
             <figure><picture><source srcset=\"/a-256.jpg 256w, /a-512.jpg 512w\">\
             <img src=\"data:image/gif;base64,R0lGOD\" alt=\"A hero\"></picture>\
             <figcaption>The hero, captioned.</figcaption></figure>\
             <p>{body}</p><img srcset=\"/b-1x.jpg 1x, /b-2x.jpg 2x\" alt=\"Second\"><p>{body}</p></article></body></html>"
        );
        let d = doc(&src);
        let figs: Vec<_> = d
            .blocks
            .iter()
            .filter_map(|b| match b {
                Block::Figure { image, .. } => Some(image.src.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            figs,
            vec![
                "https://example.test/a-256.jpg",
                "https://example.test/b-1x.jpg"
            ],
            "{:?}",
            d.blocks
        );
    }

    /// A `<figure>` around a table is a container, not a run of words.
    #[test]
    fn a_figure_without_an_image_keeps_its_content() {
        let body =
            "Prose long enough to clear the floor, repeated a few times over for size. ".repeat(4);
        let src = format!(
            "<html><body><article><p>{body}</p>\
             <figure><table><tr><th>Year</th><th>Value</th></tr><tr><td>2024</td><td>1</td></tr></table>\
             <figcaption>Values by year.</figcaption></figure></article></body></html>"
        );
        let d = doc(&src);
        assert!(
            d.blocks.iter().any(|b| matches!(b, Block::Table { .. })),
            "table lost: {:?}",
            d.blocks
        );
    }

    /// Regression: two front pages' best root was a 240-char text box and a 471-char legal
    /// block, each clearing the 200 floor and each a sliver of the page.
    #[test]
    fn a_root_that_is_a_sliver_of_the_page_yields_to_the_body() {
        // Cards too short for the scorer to credit (`own_text` under 25), so the legal
        // block is the only candidate, as on the measured pages.
        let cards = (0..80)
            .map(|i| {
                format!("<div class=\"zz\"><a href=\"/s{i}\">Story {i} headline text</a></div>")
            })
            .collect::<String>();
        let legal = "Legal notice text, long enough to clear the two hundred character floor \
                     on its own and be chosen as the best-scoring prose on the page. "
            .repeat(2);
        let src = format!(
            "<html><body><div>{cards}</div><div class=\"legal\"><p>{legal}</p></div></body></html>"
        );
        let url = Url::parse("https://example.test/").unwrap();
        let why = explain(&src, &url);
        assert!(why.body_fallback, "{why}");
        assert!(why.text_len > 500, "{why}");
    }

    /// Regression: a front page set every headline in `<article><header><h4>`, and `header`
    /// is a chrome tag, so the body emitted 2,337 of 11,600 characters. A header inside an
    /// article is the article's, not the page's; the page's own stays chrome.
    #[test]
    fn a_header_inside_an_article_is_content_and_the_page_header_is_not() {
        let body =
            "Prose long enough to clear the floor, repeated a few times over for size. ".repeat(4);
        let src = format!(
            "<html><body><header><a href=\"/\">Site Name</a> <a href=\"/x\">Section</a></header>\
             <article><header><h1>The Headline</h1><p>By A Writer</p></header><p>{body}</p>\
             <footer><p>Filed under things.</p></footer></article></body></html>"
        );
        let d = doc(&src);
        let text = crate::render::to_text(&d, &crate::render::TextOpts::default());
        assert!(text.contains("The Headline"), "{text}");
        assert!(text.contains("Filed under"), "{text}");
        assert!(!text.contains("Site Name"), "{text}");
    }

    /// The shape of a front page: cards under section headings become `Entries` runs in
    /// place, with the headline as title, the story as href, the dek as summary, and the
    /// thumbnail and time carried, while the headings between the runs stay where they were.
    #[test]
    fn a_front_page_of_cards_becomes_entries_in_place() {
        let cards = |n: usize, tag: &str| {
            (0..n)
                .map(|i| {
                    format!(
                        "<div class=\"{tag}\"><a href=\"/{tag}-{i}\"><img src=\"/t{i}.jpg\" alt=\"Thumb {i}\"></a>\
                         <h3><a href=\"/{tag}-{i}\">Headline {i} about a thing that happened today</a></h3>\
                         <p class=\"dek\">A dek sentence for story {i}, with a little more.</p>\
                         <span class=\"timestamp\">2 hours ago</span></div>"
                    )
                })
                .collect::<String>()
        };
        // The second list is a `<ul>` of `<li>` cards: the list arm must not swallow them.
        let more = cards(4, "more").replace("<div class=\"more\">", "<li class=\"more\">");
        let more = more.replace("</span></div>", "</span></li>");
        let src = format!(
            "<html><body><main><h2>Top stories</h2>{}<h2>More news</h2><ul>{more}</ul></main></body></html>",
            cards(5, "lead"),
        );
        let url = Url::parse("https://example.test/").unwrap();
        let d = extract(&src, &url);
        let shape: Vec<String> = d
            .blocks
            .iter()
            .map(|b| match b {
                Block::Heading { .. } => "h".to_owned(),
                Block::Entries(es) => format!("e{}", es.len()),
                other => format!("{other:?}"),
            })
            .collect();
        assert_eq!(shape, ["h", "e5", "h", "e4"], "{:?}", d.blocks);
        let Block::Entries(es) = &d.blocks[1] else {
            unreachable!()
        };
        let e = &es[0];
        assert_eq!(
            crate::ir::plain_text(&e.title),
            "Headline 0 about a thing that happened today"
        );
        assert_eq!(e.href.as_deref(), Some("https://example.test/lead-0"));
        assert_eq!(e.published.as_deref(), Some("2 hours ago"));
        assert_eq!(
            e.image.as_ref().map(|i| i.src.as_str()),
            Some("https://example.test/t0.jpg")
        );
        let summary: Vec<String> = e
            .summary
            .iter()
            .map(|b| match b {
                Block::Paragraph(i) => crate::ir::plain_text(i),
                other => format!("{other:?}"),
            })
            .collect();
        assert_eq!(summary, ["A dek sentence for story 0, with a little more."]);
        // And `--why` says the same thing.
        let why = explain(&src, &url);
        assert_eq!(why.cards.accepted().count(), 2, "{why}");
    }

    /// Regression: a portal keeps its sixty-card stream beside `<main>`, and `<main>` won the
    /// root ranking with 1,233 chars. The cards are the page.
    #[test]
    fn a_card_stream_outside_the_root_brings_the_body_in() {
        let intro = "A short welcome paragraph in main, long enough to clear the two hundred \
                     character floor and to be the best prose on the page by far, twice. "
            .repeat(2);
        let cards = (0..20)
            .map(|i| {
                format!(
                    "<li class=\"story\"><h3><a href=\"/s{i}\">Headline {i} about a thing that \
                     happened today</a></h3><p>A dek sentence for story {i}, with a little more \
                     to say.</p></li>"
                )
            })
            .collect::<String>();
        let src = format!("<html><body><main><p>{intro}</p></main><ul>{cards}</ul></body></html>");
        // Two routes to the same place: the card list wins the root ranking outright, or the
        // body is brought in over a prose root the cards dwarf. What is pinned is the outcome.
        let url = Url::parse("https://example.test/").unwrap();
        let d = extract(&src, &url);
        assert!(
            d.blocks
                .iter()
                .any(|b| matches!(b, Block::Entries(es) if es.len() == 20)),
            "{:?}\n{}",
            d.blocks,
            explain(&src, &url)
        );
    }

    /// Regression: `representing<strong> </strong>the only person` came out as
    /// "representingthe only person". A space wrapped in emphasis is a space.
    #[test]
    fn an_emphasis_holding_only_a_space_is_a_space() {
        let body =
            "Prose long enough to clear the floor, repeated a few times over for size. ".repeat(3);
        let d = doc(&format!(
            "<html><body><article><p>{body}</p><p>The lawyer, representing<strong> </strong>the \
             only person ever charged, spoke at<em> </em>length about the case.</p></article></body></html>"
        ));
        let text = crate::render::to_text(&d, &crate::render::TextOpts::default());
        assert!(text.contains("representing the only"), "{text}");
        assert!(text.contains("spoke at length"), "{text}");
    }

    /// Regression: a byline built as `<div>21 hours ago</div><div>Joe Inwood</div>` with no
    /// whitespace between the divs came out as "21 hours agoJoe Inwood". Block-level siblings
    /// flattened into one run keep the boundary a page would draw between them.
    #[test]
    fn block_level_siblings_flattened_into_a_run_keep_their_boundary() {
        let body =
            "Prose long enough to clear the floor, repeated a few times over for size. ".repeat(3);
        let d = doc(&format!(
            "<html><body><article><div class=\"meta\"><div>21 hours ago</div><div>Joe Inwood</div>\
             <div>World news correspondent</div>\
             <span>World</span><span>wide</span></div><p>{body}</p></article></body></html>"
        ));
        let text = crate::render::to_text(&d, &crate::render::TextOpts::default());
        assert!(
            text.contains("21 hours ago Joe Inwood World news correspondent"),
            "{text}"
        );
        // Inline siblings stay welded, as a browser renders them.
        assert!(text.contains("Worldwide"), "{text}");
        // The same boundary in the plain-text measure, which visible bylines are read with.
        let html = Html::parse_document("<div><div>21 hours ago</div><div>Joe Inwood</div></div>");
        assert_eq!(inner_text(*html.root_element()), "21 hours ago Joe Inwood");
    }

    /// "Skip to content" and a "close" button with `href="#"` are controls that point at the
    /// page itself; a link to another page is not, and a list of fragment links (a table of
    /// contents) is a list, which never passes through the loose-run floor.
    #[test]
    fn a_short_run_of_links_into_the_page_itself_is_not_a_paragraph() {
        let body =
            "Prose long enough to clear the floor, repeated a few times over for size. ".repeat(3);
        let d = doc(&format!(
            "<html><body><a href=\"#main\">Skip to content</a><article><a href=\"#\">close</a>\
             <ul><li><a href=\"#one\">Part one</a></li><li><a href=\"#two\">Part two</a></li>\
             <li><a href=\"#three\">Part three</a></li></ul>\
             <a href=\"/next\">Next story</a><p>{body}</p></article></body></html>"
        ));
        let text = crate::render::to_text(&d, &crate::render::TextOpts::default());
        assert!(!text.contains("Skip to content"), "{text}");
        assert!(!text.contains("close"), "{text}");
        assert!(text.contains("Next story"), "{text}");
        assert!(text.contains("Part two"), "{text}");
    }

    /// A thumbnail wrapped in a link is a picture, not a line of alt text set as a link.
    #[test]
    fn an_image_only_link_is_a_figure() {
        let body =
            "Prose long enough to clear the floor, repeated a few times over for size. ".repeat(3);
        let d = doc(&format!(
            "<html><body><article><p>{body}</p>\
             <a href=\"/story\"><img src=\"/t.jpg\" alt=\"A thumbnail described at length here\"></a>\
             <p>{body}</p></article></body></html>"
        ));
        assert!(
            d.blocks
                .iter()
                .any(|b| matches!(b, Block::Figure { image, .. } if image.src.ends_with("/t.jpg"))),
            "{:?}",
            d.blocks
        );
    }

    /// Regression: one front page wraps `<img alt="Headline - Site">` and the headline text
    /// in the same anchor, and the alt was read out before the headline as link text.
    #[test]
    fn an_alt_that_repeats_its_link_text_is_dropped() {
        let body =
            "Prose long enough to clear the floor, repeated a few times over for size. ".repeat(3);
        let d = doc(&format!(
            "<html><body><article><p>{body}</p>\
             <p><a href=\"/s\"><img src=\"/t.jpg\" alt=\"Boy finds tooth on beach - Site\">Boy finds \
             tooth on beach</a></p>\
             <p><a href=\"/u\"><img src=\"/v.jpg\" alt=\"A photo of the sea at dusk\">Boy finds tooth \
             on beach</a></p><p>{body}</p></article></body></html>"
        ));
        let text = crate::render::to_text(&d, &crate::render::TextOpts::default());
        assert!(!text.contains("tooth on beach - Site"), "{text}");
        assert!(text.contains("[A photo of the sea at dusk]"), "{text}");
    }

    fn body_with_banner() -> String {
        let body =
            "Prose long enough to clear the floor, repeated a few times over for size. ".repeat(4);
        format!(
            "<html><body><article><div class=\"listen-banner\"><span>NEW</span>You can now listen \
             to articles!</div><p>{body}</p><p>{body}</p></article></body></html>"
        )
    }

    #[test]
    fn extract_with_the_none_profile_is_extract() {
        let src = body_with_banner();
        let url = Url::parse("https://example.test/a").unwrap();
        let plain = extract(&src, &url);
        let x = extract_with_profile(&src, &url, &crate::profile::Profile::NONE);
        assert!(x.profile.is_none());
        assert_eq!(
            serde_json::to_string(&plain).unwrap(),
            serde_json::to_string(&x.doc).unwrap()
        );
    }

    #[test]
    fn strip_removes_the_subtree_and_reports_the_count() {
        let src = body_with_banner();
        let url = Url::parse("https://example.test/a").unwrap();
        let p = crate::profile::Profile {
            strip: crate::profile::sel(".listen-banner"),
        };
        let x = extract_with_profile(&src, &url, &p);
        let text = crate::render::to_text(&x.doc, &crate::render::TextOpts::default());
        assert!(!text.contains("You can now listen"), "{text}");
        assert!(text.contains("Prose long enough"), "{text}");
        let r = x.profile.expect("a report");
        assert_eq!(r.fields.len(), 1);
        assert_eq!(r.fields[0].outcome, crate::profile::Outcome::Matched(1));
        // And the same through `--why`.
        let why = explain_with_profile(&src, &url, &p);
        assert!(
            why.to_string().contains("profile: strip matched 1"),
            "{why}"
        );
    }

    #[test]
    fn a_dead_strip_is_reported_and_changes_nothing() {
        let src = body_with_banner();
        let url = Url::parse("https://example.test/a").unwrap();
        let p = crate::profile::Profile {
            strip: crate::profile::sel(".no-such-thing"),
        };
        let x = extract_with_profile(&src, &url, &p);
        assert_eq!(
            x.profile.unwrap().fields[0].outcome,
            crate::profile::Outcome::Dead
        );
        assert_eq!(
            serde_json::to_string(&x.doc).unwrap(),
            serde_json::to_string(&extract(&src, &url)).unwrap()
        );
    }

    /// A stale selector that has come to match the article is refused, not obeyed: the
    /// profile is a candidate with a floor.
    #[test]
    fn a_strip_that_would_remove_most_of_the_page_is_refused() {
        let src = body_with_banner();
        let url = Url::parse("https://example.test/a").unwrap();
        let p = crate::profile::Profile {
            strip: crate::profile::sel("article"),
        };
        let x = extract_with_profile(&src, &url, &p);
        assert_eq!(
            x.profile.unwrap().fields[0].outcome,
            crate::profile::Outcome::Rejected("would remove most of the page")
        );
        assert!(x.doc.text_len() > 500);
    }

    #[test]
    fn a_selector_that_does_not_parse_is_rejected_not_fatal() {
        let src = body_with_banner();
        let url = Url::parse("https://example.test/a").unwrap();
        let p = crate::profile::Profile {
            strip: crate::profile::sel("div[["),
        };
        let x = extract_with_profile(&src, &url, &p);
        assert_eq!(
            x.profile.unwrap().fields[0].outcome,
            crate::profile::Outcome::Rejected("selector does not parse")
        );
    }

    /// Regression: "Return to Homepage" on a portal's close button, in a `span.sr-only`, read
    /// out as the first line of every article. A label for a control a sighted reader never
    /// sees is not prose.
    #[test]
    fn screen_reader_only_labels_are_chrome() {
        let body =
            "Prose long enough to clear the floor, repeated a few times over for size. ".repeat(4);
        let d = doc(&format!(
            "<html><body><a href=\"/\"><svg></svg><span class=\"sr-only\">Return to Homepage</span></a>\
             <article><p>{body}</p></article></body></html>"
        ));
        let text = crate::render::to_text(&d, &crate::render::TextOpts::default());
        assert!(!text.contains("Return to Homepage"), "{text}");
    }

    /// Ten of forty articles across fifty-seven hosts ended in link rubble: a tag list, "See
    /// all", "HOME", a "Most viewed" heading. The last paragraph of prose stays, however short.
    #[test]
    fn link_rubble_after_the_last_prose_is_trimmed_and_prose_is_not() {
        let body =
            "Prose long enough to clear the floor, repeated a few times over for size. ".repeat(4);
        let d = doc(&format!(
            "<html><body><article><p>{body}</p><p>We'll see.</p>\
             <ul><li><a href=\"/t/a\">Tag A</a></li><li><a href=\"/t/b\">Tag B</a></li><li><a href=\"/t/c\">Tag C</a></li></ul>\
             <h2>Most viewed</h2><p><a href=\"/\">HOME</a></p><hr></article></body></html>"
        ));
        let text = crate::render::to_text(&d, &crate::render::TextOpts::default());
        assert!(text.trim_end().ends_with("We'll see."), "{text}");
        assert!(
            !text.contains("Tag A") && !text.contains("Most viewed"),
            "{text}"
        );
        // Comma-separated tag links, a bold newsletter link, and an uncaptioned badge image
        // at the foot are rubble too; the short closing sentence is not.
        let d = doc(&format!(
            "<html><body><article><p>{body}</p><p>We'll see.</p>\
             <p><a href=\"/t/x\">Canada</a>, <a href=\"/t/y\">Trade war</a></p>\
             <p><strong><a href=\"/newsletter\">Subscribe to the newsletter</a></strong></p>\
             <img src=\"/badge.png\" alt=\"Google News\"></article></body></html>"
        ));
        let text = crate::render::to_text(&d, &crate::render::TextOpts::default());
        assert!(text.trim_end().ends_with("We'll see."), "{text}");
        assert!(!text.contains("Trade war"), "{text}");
    }

    /// Regression: a "related stories" block of dated summaries under a `related` container
    /// became a four-post thread, because the thread detector reads the raw tree and had never
    /// asked whether the group sat in chrome.
    #[test]
    fn a_dated_related_list_inside_chrome_is_not_a_thread() {
        let body =
            "Prose long enough to clear the floor, repeated a few times over for size. ".repeat(4);
        let related = (0..4)
            .map(|i| {
                format!(
                    "<div class=\"related-summary\"><span class=\"story-date\">Aug. {i}, 2023</span> \
                     A summary of a related story, long enough to be counted as a post body by the detector.</div>"
                )
            })
            .collect::<String>();
        let d = doc(&format!(
            "<html><body><article><p>{body}</p></article><div class=\"related\">{related}</div></body></html>"
        ));
        assert!(
            !d.blocks.iter().any(|b| matches!(b, Block::Thread(_))),
            "{:?}",
            d.blocks
        );
    }

    /// Regression: token matching made `advert` stop matching `advertisement`, and a
    /// recirculation row classed `advertisement ad-below-entry-recirc` came back.
    #[test]
    fn advertisement_and_recirculation_vendor_classes_are_chrome() {
        let body =
            "Prose long enough to clear the floor, repeated a few times over for size. ".repeat(4);
        let d = doc(&format!(
            "<html><body><article><p>{body}</p>\
             <div class=\"advertisement\"><p>Buy a thing, a sentence of sales copy long enough to pass.</p></div>\
             <div class=\"taboola-container\"><p>From our partner, a sentence of sales copy long enough to pass.</p></div>\
             <div class=\"trinity-tts-pb\"><strong>Getting your audio player ready, a sentence long enough.</strong></div>\
             </article></body></html>"
        ));
        let text = crate::render::to_text(&d, &crate::render::TextOpts::default());
        assert!(
            !text.contains("Buy a thing") && !text.contains("From our partner"),
            "{text}"
        );
        assert!(!text.contains("audio player ready"), "{text}");
    }

    /// `<dt>` is short by nature and fell under the loose-text floor, which dropped every
    /// term of every definition list.
    #[test]
    fn a_definition_list_keeps_its_short_terms() {
        let body =
            "Prose long enough to clear the floor, repeated a few times over for size. ".repeat(3);
        let d = doc(&format!(
            "<html><body><article><p>{body}</p><dl><dt>Term</dt><dd>The definition of the term, in a \
             sentence.</dd><dt>Other</dt><dt>Alias</dt><dd>Two terms, one definition, in a sentence.</dd></dl>\
             <p>{body}</p></article></body></html>"
        ));
        let text = crate::render::to_text(&d, &crate::render::TextOpts::default());
        assert!(text.contains("*Term*"), "{text}");
        assert!(text.contains("*Other Alias*"), "{text}");
        assert!(text.contains("Two terms, one definition"), "{text}");
    }

    #[test]
    fn details_and_summary_are_read_open() {
        let body =
            "Prose long enough to clear the floor, repeated a few times over for size. ".repeat(3);
        let d = doc(&format!(
            "<html><body><article><p>{body}</p><details><summary>Show the workings</summary>\
             <p>The workings, in a paragraph long enough to be kept on its own merits.</p></details>\
             <p>{body}</p></article></body></html>"
        ));
        let text = crate::render::to_text(&d, &crate::render::TextOpts::default());
        assert!(text.contains("*Show the workings*"), "{text}");
        assert!(text.contains("The workings, in a paragraph"), "{text}");
    }

    /// `Block::Embed` existed and nothing constructed it: video, audio, and iframes vanished.
    #[test]
    fn media_elements_become_embed_placeholders() {
        let body =
            "Prose long enough to clear the floor, repeated a few times over for size. ".repeat(3);
        let d = doc(&format!(
            "<html><body><article><p>{body}</p>\
             <video controls><source src=\"/clip.mp4\" type=\"video/mp4\"></video>\
             <audio src=\"/talk.mp3\"></audio>\
             <iframe src=\"https://player.example.test/embed/1\"></iframe>\
             <iframe src=\"about:blank\"></iframe>\
             <p>{body}</p></article></body></html>"
        ));
        let kinds: Vec<_> = d
            .blocks
            .iter()
            .filter_map(|b| match b {
                Block::Embed { kind, url } => Some((*kind, url.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(
            kinds,
            vec![
                (
                    crate::ir::EmbedKind::Video,
                    "https://example.test/clip.mp4".to_owned()
                ),
                (
                    crate::ir::EmbedKind::Audio,
                    "https://example.test/talk.mp3".to_owned()
                ),
                (
                    crate::ir::EmbedKind::Iframe,
                    "https://player.example.test/embed/1".to_owned()
                ),
            ],
            "{:?}",
            d.blocks
        );
    }

    /// Accepted card lists nest (a lead package holds a list of its own), and `entry_from`
    /// used to walk a card's body from depth zero: a page of nested triplets never reached
    /// `MAX_DEPTH` and walked off the end of the stack. The harness is the one
    /// `a_pathologically_nested_page_extracts_instead_of_aborting` uses.
    #[test]
    fn nested_story_cards_do_not_walk_off_the_stack() {
        // Overflowed at 200 before the fix, and not at 100. Capped at the smaller number that
        // did, because the detector's cost is quadratic in this depth.
        const DEPTH: usize = 200;
        let open = "<div class=\"card\"><a href=\"/s\">A headline long enough to be one</a> \
                    and a dek of forty characters or more beside it. ";
        let leaf = "<div class=\"card\"><a href=\"/s\">A headline long enough to be one</a> \
                    and a dek of forty characters or more beside it.</div>";
        let mut src = String::from("<html><body><main>");
        src.push_str(&open.repeat(DEPTH));
        src.push_str(&leaf.repeat(3));
        src.push_str(&format!("{leaf}{leaf}</div>").repeat(DEPTH));
        src.push_str(&leaf.repeat(2));
        src.push_str("</main></body></html>");
        let out = std::thread::Builder::new()
            .stack_size(2 * 1024 * 1024)
            .spawn(move || extract(&src, &Url::parse("https://example.com/a").unwrap()))
            .unwrap()
            .join()
            .expect("extraction unwound");
        assert!(
            out.text_len() > 0,
            "the nested card page extracted nothing at all"
        );
        assert!(
            out.blocks.iter().any(|b| matches!(b, Block::Entries(_))),
            "not a card page: {:?}",
            out.blocks.first()
        );
    }

    /// A blog archive is one list of linked titles under a heading. Only the body fallback
    /// reaches it, and `trim_tail` then saw a link-only list and a short heading: rubble by
    /// the block, the whole page by the sum, and a document of nothing after it.
    #[test]
    fn a_body_fallback_page_that_is_one_link_list_keeps_the_list() {
        let items: String = (0..40)
            .map(|i| format!("<li><a href=\"/p/{i}\">Post number {i} title</a></li>"))
            .collect();
        let d = doc(&format!("<main><h1>Archive</h1><ul>{items}</ul></main>"));
        assert!(d.text_len() > 0, "trimmed to nothing");
        assert!(
            d.blocks.iter().any(|b| matches!(b, Block::List { .. })),
            "the list is the page: {:?}",
            d.blocks
        );
    }

    /// Token matching dropped the chrome classes the substring match reached by accident:
    /// `navigation` no longer contained `nav`. These are the whole-word forms, and a root
    /// that is `<main>` or `<body>` meets every one of them.
    #[test]
    fn whole_word_chrome_classes_are_still_chrome() {
        let d = doc("<main>\
             <div class=\"main-navigation\"><ul><li><a href=\"/a\">Home</a></li>\
             <li><a href=\"/b\">World</a></li></ul></div>\
             <div class=\"navbar\"><a href=\"/login\">Login</a> <a href=\"/reg\">Register</a></div>\
             <div class=\"advertising\"><p>Buy the thing advertised here today, on offer.</p></div>\
             <div class=\"cookies\"><p>We use cookies to improve your experience on this site.</p></div>\
             <div class=\"content\"><p>The article itself, a paragraph of prose long enough to be \
             the content of the page and to clear every floor the extractor applies to a root \
             candidate, which it is.</p></div></main>");
        let text = d
            .blocks
            .iter()
            .map(|b| match b {
                Block::Paragraph(i) | Block::Heading { inlines: i, .. } => crate::ir::plain_text(i),
                Block::List { items, .. } => format!("{items:?}"),
                _ => String::new(),
            })
            .collect::<Vec<_>>()
            .join(" ");
        assert!(text.contains("The article itself"), "{text}");
        for leaked in ["Login", "World", "advertised", "cookies to improve"] {
            assert!(!text.contains(leaked), "{leaked:?} leaked: {text}");
        }
    }

    /// `Walk.skip` was checked in the children loop and nowhere else, and the list arm
    /// reaches its `<li>`s by selector: a reply nested as `<ul class="children"><li
    /// class="comment">` was walked into its parent's body as a bullet list.
    #[test]
    fn a_reply_nested_as_a_list_item_is_skipped_like_any_other() {
        let html = Html::parse_document(
            "<html><body><ul><li class=\"comment\" id=\"p\"><p>The parent post, which says \
             enough to be a paragraph of its own here.</p>\
             <ul class=\"children\"><li class=\"comment\" id=\"r\"><p>The reply, which is a \
             post of its own and not part of the parent.</p></li></ul></li></ul></body></html>",
        );
        let sel = |s: &str| html.select(&Selector::parse(s).unwrap()).next().unwrap();
        let skip: std::collections::HashSet<_> = [sel("#r").id()].into_iter().collect();
        let body = blocks_from_skipping(
            sel("#p"),
            &Url::parse("https://example.test/").unwrap(),
            &skip,
        );
        let text = format!("{body:?}");
        assert!(text.contains("The parent post"), "{text}");
        assert!(
            !text.contains("The reply"),
            "reply walked into the parent: {text}"
        );
    }

    /// A srcset URL may carry commas (an image CDN's transform segment); the grammar splits
    /// candidates only on a comma that ends a URL token.
    #[test]
    fn first_of_srcset_keeps_a_comma_inside_a_url() {
        assert_eq!(
            first_of_srcset(
                "https://res.example.com/image/upload/w_300,h_200,c_fill/photo.jpg 300w, \
                 https://res.example.com/image/upload/w_600,h_400,c_fill/photo.jpg 600w"
            ),
            Some("https://res.example.com/image/upload/w_300,h_200,c_fill/photo.jpg")
        );
        assert_eq!(first_of_srcset("a.jpg 1x, b.jpg 2x"), Some("a.jpg"));
        assert_eq!(first_of_srcset("a.jpg 1x,b.jpg 2x"), Some("a.jpg"));
        assert_eq!(first_of_srcset("a.jpg, b.jpg"), Some("a.jpg"));
        // No whitespace after the comma: one URL, as the grammar and browsers read it.
        assert_eq!(first_of_srcset("a.jpg,b.jpg"), Some("a.jpg,b.jpg"));
        assert_eq!(first_of_srcset(" , a.jpg"), Some("a.jpg"));
        assert_eq!(
            first_of_srcset("data:image/gif;base64,R0lGOD 1x, b.jpg 2x"),
            Some("b.jpg")
        );
        assert_eq!(first_of_srcset("data:image/gif;base64,R0lGOD 1x"), None);
        assert_eq!(first_of_srcset(""), None);
    }

    /// A forty-card rail in a class-hinted sidebar is accepted as cards and dropped by the
    /// walker; counted against the root it made a correctly rooted article take the body walk.
    #[test]
    fn a_card_rail_in_a_sidebar_does_not_count_against_the_root() {
        let prose: String = (0..8)
            .map(|i| {
                format!(
                    "<p>Paragraph {i} of the article, carrying enough prose that the article \
                     is the root of the page by a comfortable margin and clears the sliver floor \
                     on its own, as a real article of modest length does.</p>"
                )
            })
            .collect();
        let rail: String = (0..40)
            .map(|i| {
                format!(
                    "<div class=\"card\"><h3><a href=\"/s{i}\">Latest headline number {i} about \
                     a thing</a></h3><p>A dek sentence for the latest story {i}, long enough.</p>\
                     </div>"
                )
            })
            .collect();
        let src = format!(
            "<html><body><nav><a href=\"/\">Home</a></nav><article>{prose}</article>\
             <div class=\"sidebar\"><h2>Latest</h2>{rail}</div></body></html>"
        );
        let why = explain(&src, &Url::parse("https://example.test/a").unwrap());
        assert_eq!(why.cards.accepted().count(), 1, "{why}");
        assert_eq!(why.cards_text, 0, "{why}");
        assert!(!why.body_fallback, "{why}");
        let d = doc(&src);
        assert!(
            !d.blocks.iter().any(|b| matches!(b, Block::Entries(_))),
            "the rail reached the page: {:?}",
            d.blocks
        );
    }
}
