//! RSS 2.0 and Atom: an XML document becomes [`ir::Block::Entries`].
//!
//! A feed is not refused today — `session::is_web_page` passes `application/rss+xml` and its
//! neighbours — it is handed to `html::extract`, which reads XML as HTML soup. That is the
//! confident wrong diagnosis the content-type gate exists to prevent, one level deeper: the
//! page comes out near-empty and the reader is told it may need JavaScript.
//!
//! Structural only. No hostname appears here or in this module's tests; `sites.rs` remains the
//! only file in the crate that names one. The public surface is modelled on
//! [`crate::search`], which is this crate's precedent for a non-article path to a
//! [`ir::Document`].
//!
//! # Why an XML parser rather than the html5ever already in the crate
//!
//! Measured 2026-08-28 over six feeds chosen for format and payload-encoding coverage, then a
//! wider sample of 28 for the parse rate. **html5ever cannot read feeds.** It parses
//! `<![CDATA[…]]>` as a bogus comment, which runs to the first `>` in the payload and swallows
//! the prose into it: on one full-text WordPress feed's first item `scraper` returned 0
//! characters for `<description>` where `roxmltree` returned 3,251, and 2,451 of the 3,251 in
//! the `content:encoded` beside it. How much is lost depends on where that first `>` happens to
//! fall, so the failure is silent and data-dependent. Over that whole feed the two recover
//! 13,735 characters and 63,480 from the same bytes. Two further divergences: html5ever treats
//! `<link>` as void, so an RSS item's URL is a loose sibling text node rather than the element's
//! text, and an unknown self-closing element such as `<media:thumbnail/>` nests its siblings
//! underneath itself.
//!
//! `roxmltree` read **26 of 28** real feeds across WordPress, Ghost, Hugo, Jekyll, Blogger and
//! hand-rolled generators, both formats, 1 to 71 items, and neither miss was the parser's: one
//! feed exceeds the 5 MB transfer cap and arrives cut mid-element, and one is served as
//! `application/octet-stream` and never reaches here. It recovers CDATA and entity-escaped
//! payloads alike and resolves `content:encoded` and `xml:base` by namespace. One package added
//! to the lockfile, no `unsafe`, no advisories.
//!
//! Its strictness is real: `&nbsp;`, an undeclared namespace prefix, a raw `&`, or a control
//! character kills the whole document. None occurred in the sample, because generators emit
//! well-formed XML — every other feed reader would break too. The answer is disclosure when it
//! happens ([`Outcome::Unreadable`]), not leniency.
//!
//! `allow_dtd` stays at roxmltree's default of **false**, and both measured consequences are
//! wanted: a billion-laughs entity bomb is refused at parse rather than expanded, and an HTML
//! page is refused for its `<!DOCTYPE html>` before any element is inspected. Turning it on
//! re-opens entity expansion on untrusted input.
//!
//! # What this module does not do
//!
//! It emits the **whole** summary. An entry shows a lead-in, and where that lead-in ends is a
//! reading decision that belongs to a renderer — `reader::title` rules the alternative out in
//! as many words, and a budget applied here would also delete text from the plain renderer,
//! which has no width pressure. See `reader::budget`.

use crate::ir::{self, Block, Document, Entry, Inline};
use roxmltree::Node;
use scraper::Html;
use url::Url;

/// The Atom namespace. An element is Atom's because it says so, not because of its prefix:
/// `<feed>` under `xmlns=` and `<atom:feed>` under a prefix are the same document.
const ATOM: &str = "http://www.w3.org/2005/Atom";
/// RSS's `content:encoded`, which is where a full-text RSS feed keeps the post body.
/// `<description>` beside it is then a teaser, so this one wins when both are present.
const CONTENT: &str = "http://purl.org/rss/1.0/modules/content/";
/// The one namespace every XML document has without declaring it: `xml:base`, `xml:lang`.
const XML: &str = "http://www.w3.org/XML/1998/namespace";

/// Which of the two formats answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Rss,
    Atom,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Rss => "RSS",
            Kind::Atom => "Atom",
        }
    }
}

/// What reading the feed did, for the disclosure. Carried on [`crate::session::Provenance`]
/// beside `profile`, so page info, `--why`, and `notice::about_page` all read one field rather
/// than three descriptions of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub kind: Kind,
    pub entries: usize,
}

impl std::fmt::Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let n = self.entries;
        write!(
            f,
            "{} feed, {n} {}",
            self.kind.label(),
            if n == 1 { "entry" } else { "entries" }
        )
    }
}

/// What [`read`] made of a response.
///
/// Deliberately not `PartialEq`, for the reason [`crate::search::Outcome`] is not: `ir::Entry`
/// is not, and deriving it there to serve a test here would put an ordering promise on the IR
/// that no renderer wants.
#[derive(Debug, Clone)]
pub enum Outcome {
    Feed(Document, Report),
    /// A feed that parsed and carried no items. Its title, because the reading column holds
    /// page content only and this has to be said as a page remark; naming the publication is
    /// the difference between that remark and a shrug.
    Empty {
        title: String,
    },
    /// It said it was a feed and would not parse. Carries the parser's message and position.
    ///
    /// Falling through silently instead would hand the reader a near-blank page under a caution
    /// blaming JavaScript, because the whole premise of choosing an XML parser is that the HTML
    /// extractor gets nothing out of these documents.
    Unreadable(String),
    /// Not a feed. Falls through to the generic extractor, which is what keeps an RSS 1.0 / RDF
    /// feed off a blank screen.
    NotAFeed,
}

/// How long a parser message may be before it is cut.
///
/// The message is **publisher-controlled**: it embeds the offending entity name or namespace
/// prefix, and it reaches both a terminal and a page remark. `bin/hww.rs` already routes
/// everything it prints through `render::sanitize_for_terminal`, so an escape sequence in a
/// feed does not become one on a terminal; this bounds the other half.
const MAX_MESSAGE: usize = 160;

/// How long a feed's own title may be before it is cut.
///
/// The same argument as [`MAX_MESSAGE`] and the same kind of text: `<channel><title>` is
/// publisher-controlled, and [`Outcome::Empty`] carries it straight into a page remark. Shorter
/// than a parser message because this one is a name rather than a diagnosis.
const MAX_TITLE: usize = 120;

/// How much of a summary stands in for a headline an item never carried. Characters, not
/// bytes: this one is cut for the eye rather than measured against a budget.
const FALLBACK_TITLE: usize = 90;

/// The headline an entry gets when it carried nothing at all to build one from. See [`entry`].
const UNTITLED: &str = "Untitled";

/// How deep an Atom `xhtml` payload is serialized before the rest of it is flattened to text.
///
/// The same argument as `html::MAX_DEPTH` and the same untrusted input: the serializer below
/// builds a tree and costs a stack frame per level, and a reader worker's stack is 2 MiB.
/// roxmltree itself parses into a flat arena, so the depth arrives here intact.
const MAX_DEPTH: usize = 256;

/// The root elements that name a feed. Matched at a tag boundary, so `<feedback>` is not one.
const FEED_ROOTS: [&str; 3] = ["<rss", "<feed", "<atom:feed"];

/// How many prolog items are stepped over while looking for the root element.
///
/// A declaration, a stylesheet instruction and a comment or two is the realistic worst case.
/// The bound is what stops a document that is nothing but comments from being walked at all.
const MAX_PROLOG_ITEMS: usize = 16;

/// What the document's first bytes claimed, which is what decides whether a parse failure is
/// disclosed or swallowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lead {
    /// XML whose root element is not a feed's. Could be any XML at all, so a failure here is
    /// [`Outcome::NotAFeed`].
    Declared,
    /// The root element is `<rss`, `<feed`, or `<atom:feed`, whether or not a declaration
    /// stands in front of it. It said what it was, so a failure is a remark.
    Named,
}

/// Read a response as a feed.
///
/// Two steps, and the order matters. The **guard** is the leading tag: after an optional BOM and
/// whitespace the source must open `<?xml`, `<rss`, `<feed`, or `<atom:feed`. An HTML page opens
/// `<!DOCTYPE` or `<html` and never reaches the parser, and a page that merely contains the word
/// `feed` does not either. Then **parse**, and the root decides: `rss`, or `feed` in Atom's
/// namespace. Any other root — `rdf:RDF` included — is [`Outcome::NotAFeed`].
pub fn read(source: &str, base: &Url) -> Outcome {
    let Some(lead) = lead(source) else {
        return Outcome::NotAFeed;
    };
    let xml = match roxmltree::Document::parse(source) {
        Ok(x) => x,
        Err(e) => {
            return match lead {
                Lead::Named => Outcome::Unreadable(clamp(&e.to_string())),
                // It only ever claimed to be XML, and plenty of XML is not a feed. Blaming a
                // publisher for a document hww was not asked to read would be the same wrong
                // confidence in the other direction.
                Lead::Declared => Outcome::NotAFeed,
            };
        }
    };
    let root = xml.root_element();
    let name = root.tag_name();
    let kind = if name.name() == "rss" && name.namespace().is_none() {
        Kind::Rss
    } else if name.name() == "feed" && matches!(name.namespace(), Some(ATOM) | None) {
        Kind::Atom
    } else {
        return Outcome::NotAFeed;
    };

    let (doc, entries) = match kind {
        // `<channel>` and not `<rss>`: an RSS document's metadata and items are all one level
        // in, and a feed with no channel has nothing to read.
        Kind::Rss => match child(root, root.tag_name().namespace(), "channel") {
            Some(channel) => rss(channel, base),
            None => return Outcome::NotAFeed,
        },
        Kind::Atom => atom(root, base),
    };
    if entries == 0 {
        return Outcome::Empty {
            // Capped, for the reason the parser's message is: it is the publisher's string and
            // it reaches a page remark whole.
            title: clamp_to(&doc.title.unwrap_or_default(), MAX_TITLE),
        };
    }
    Outcome::Feed(doc, Report { kind, entries })
}

/// What this document claims to be, or `None` for something that is not XML at all.
fn lead(source: &str) -> Option<Lead> {
    let s = source.trim_start_matches('\u{feff}').trim_start();
    if names_a_feed(s) {
        return Some(Lead::Named);
    }
    s.starts_with("<?xml").then_some(Lead::Declared)
}

/// Whether the **root element** is a feed's, looked for past any declaration, stylesheet
/// instruction, or comment standing in front of it.
///
/// The root and not byte zero. Nearly every generator emits `<?xml version="1.0"?>` first, so
/// reading the declaration alone as the whole claim made [`Outcome::Unreadable`] unreachable
/// for the documents it was written for: a raw `&nbsp;` from a WordPress export — the module
/// doc's own example of the commonest way a feed breaks — took the fallthrough and landed on a
/// near-blank page under a caution blaming JavaScript, which is the confident wrong diagnosis
/// this module exists to prevent.
///
/// A `<!DOCTYPE` answers `false` whatever follows it, and that is deliberate rather than a gap.
/// `allow_dtd` is false, so such a document is refused for carrying a DTD and not for being
/// malformed; the refusal is hww's policy, and reporting it as the publisher's broken markup
/// would be the same misplaced blame one level along.
fn names_a_feed(s: &str) -> bool {
    let mut rest = s;
    for _ in 0..MAX_PROLOG_ITEMS {
        rest = rest.trim_start();
        rest = if let Some(r) = rest.strip_prefix("<?") {
            match r.find("?>") {
                Some(i) => &r[i + 2..],
                None => return false,
            }
        } else if let Some(r) = rest.strip_prefix("<!--") {
            match r.find("-->") {
                Some(i) => &r[i + 3..],
                None => return false,
            }
        } else if rest.starts_with("<!") {
            return false;
        } else {
            return FEED_ROOTS.iter().any(|t| opens_with(rest, t));
        };
    }
    false
}

fn opens_with(s: &str, tag: &str) -> bool {
    s.strip_prefix(tag)
        .is_some_and(|rest| rest.starts_with(['>', '/']) || rest.starts_with(char::is_whitespace))
}

/// A parser message as it is allowed to reach a screen. See [`MAX_MESSAGE`].
fn clamp(msg: &str) -> String {
    clamp_to(msg, MAX_MESSAGE)
}

/// Trim to `max` characters, marking the cut. Characters and not bytes, so a multi-byte
/// codepoint is never halved.
fn clamp_to(s: &str, max: usize) -> String {
    let s = s.trim();
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_owned(),
    }
}

// ---------------------------------------------------------------- the two formats

/// An RSS `<channel>` as a document, and how many items it had.
fn rss(channel: Node<'_, '_>, base: &Url) -> (Document, usize) {
    let ns = channel.tag_name().namespace();
    let items: Vec<Entry> = children(channel, ns, "item")
        .map(|item| rss_item(item, ns, base))
        .collect();
    let n = items.len();
    let doc = document(
        base,
        child_text(channel, ns, "title"),
        // `lastBuildDate` is when the channel last changed, which is what a masthead wants;
        // `pubDate` is the fallback publishers who set only one of them set.
        child_text(channel, ns, "lastBuildDate").or_else(|| child_text(channel, ns, "pubDate")),
        child_text(channel, ns, "language").or_else(|| lang_at(channel)),
        items,
    );
    (doc, n)
}

fn rss_item(item: Node<'_, '_>, ns: Option<&str>, base: &Url) -> Entry {
    // An RSS `<link>` holds the address as its text. `<guid isPermaLink="true">` is the
    // fallback the spec sanctions and a fair number of generators rely on; `isPermaLink`
    // defaults to true, so the test is against the one value that opts out.
    //
    // The guid has to be an absolute web address before it is used as one. A guid is an
    // *identifier*, and generators put counters and `tag:` URIs in it without setting the
    // attribute; `base.join("12345")` would turn a counter into a confident link to a page that
    // does not exist, which is worse than the entry having no link at all.
    //
    // Each candidate is resolved before the next is considered, rather than the first element
    // that exists winning and only then being resolved. `<link></link>` and a `javascript:`
    // link are both present and both yield nothing, and taking either as the answer threw away
    // the usable permalink sitting in the `<guid>` beside it.
    let link = child(item, ns, "link")
        .and_then(|n| destination(n, &text_of(n), base))
        .or_else(|| {
            let g = child(item, ns, "guid")?;
            let raw = text_of(g);
            let usable = g.attribute("isPermaLink") != Some("false")
                && Url::parse(raw.trim()).is_ok_and(|u| matches!(u.scheme(), "http" | "https"));
            if usable {
                destination(g, &raw, base)
            } else {
                None
            }
        });
    // `content:encoded` is the post; `<description>` beside it is a teaser for the same post.
    let payload = child(item, Some(CONTENT), "encoded")
        .or_else(|| child(item, ns, "description"))
        .map(source_payload);
    entry(
        // RSS titles are text by the spec and entity-escaped HTML in practice, and the two are
        // indistinguishable after the XML parser has resolved the entities. Reading every one
        // as a source string is what stops `&amp;lt;em&amp;gt;` reaching the screen as tags.
        child(item, ns, "title").map(source_payload),
        link,
        payload,
        child_text(item, ns, "pubDate"),
        base,
    )
}

/// An Atom `<feed>` as a document, and how many entries it had.
fn atom(root: Node<'_, '_>, base: &Url) -> (Document, usize) {
    // The root's own namespace, exactly as `rss` takes the channel's, and not a hardcoded
    // [`ATOM`]. `read` admits a `<feed>` whose namespace is `None` — a generator that dropped
    // its `xmlns` — and asking for Atom's inside such a document matched nothing: every lookup
    // missed, the count came out zero, and a feed full of posts was reported to the reader as
    // one that listed none, with no fallthrough to the extractor behind it.
    let ns = root.tag_name().namespace();
    let items: Vec<Entry> = children(root, ns, "entry")
        .map(|e| atom_entry(e, ns, base))
        .collect();
    let n = items.len();
    let doc = document(
        base,
        child(root, ns, "title")
            .map(|t| plain_of(&payload_of(t)))
            .filter(|t| !t.is_empty()),
        child_text(root, ns, "updated"),
        lang_at(root),
        items,
    );
    (doc, n)
}

fn atom_entry(entry_node: Node<'_, '_>, ns: Option<&str>, base: &Url) -> Entry {
    let link = children(entry_node, ns, "link")
        // `rel` defaults to `alternate`, so selecting on the attribute alone drops every
        // conformant entry that leaves it out. `self`, `enclosure`, `replies` and the rest name
        // something other than the post.
        .find(|n| matches!(n.attribute("rel"), None | Some("alternate")))
        .and_then(|n| n.attribute("href").map(|h| (n, h.to_owned())))
        .and_then(|(n, raw)| destination(n, &raw, base));
    let payload = child(entry_node, ns, "content")
        .or_else(|| child(entry_node, ns, "summary"))
        .map(payload_of);
    entry(
        child(entry_node, ns, "title").map(payload_of),
        link,
        payload,
        // What the post says about itself first, and when it was last touched second.
        child_text(entry_node, ns, "published").or_else(|| child_text(entry_node, ns, "updated")),
        base,
    )
}

/// One entry, from the four things both formats have.
fn entry(
    title: Option<Payload<'_, '_>>,
    href: Option<String>,
    payload: Option<Payload<'_, '_>>,
    published: Option<String>,
    base: &Url,
) -> Entry {
    // Relative links inside a payload resolve against its own `xml:base` where one is in force,
    // else against the post the entry points at, else against the feed.
    let inner = payload.as_ref().map_or_else(
        || base.clone(),
        |p| {
            let declared = base_at(p.node, base);
            if declared != *base {
                declared
            } else {
                href.as_deref()
                    .and_then(|h| Url::parse(h).ok())
                    .unwrap_or_else(|| base.clone())
            }
        },
    );
    let summary = payload.map(|p| blocks_of(&p, &inner)).unwrap_or_default();
    let mut title = payload_title(title.as_ref());
    if title.is_empty() {
        // RSS permits an item carrying only a description, and link blogs use it.
        // `ir::Entry::title` is a `Vec<Inline>` rather than an `Option` and `entries_ui` builds
        // its link from that vec unconditionally, so an empty one draws an empty headline with
        // nothing to click. The line then appears twice, once as the headline and once at the
        // top of the dek; `html::summary_of` de-duplicates exactly this for cards but is
        // private, and the repetition costs less than widening that surface.
        title = lead_in(&summary);
    }
    if title.is_empty() {
        // And the chain runs out here, on an item carrying neither a title nor any text: a bare
        // `<link>` and `<pubDate>`, or a `<title>` holding markup and nothing else. The reason
        // one branch up is why it cannot be left empty at this one either — `entries_ui` wraps
        // whatever is here in a link unconditionally, so an empty string draws a headline with
        // no width and hands Tab a stop with nothing at it. The last resorts are the two things
        // such an item still has, and a word before a blank line.
        title = href
            .as_deref()
            .and_then(last_segment)
            .or_else(|| {
                published
                    .as_deref()
                    .map(str::trim)
                    .filter(|p| !p.is_empty())
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| UNTITLED.to_owned());
    }
    Entry {
        title: vec![Inline::Text(title)],
        href,
        summary,
        // The publisher's raw string, as every other producer in this crate leaves it.
        // Shortening is `reader::title::short_date`'s job, and the plain renderer wants the
        // stamp whole.
        published: published.filter(|p| !p.trim().is_empty()),
        // A feed row is a headline and a date; there is no second address to print under it.
        address: None,
        // Nor a site mark: every row in one feed leads to the same site, so a column of one
        // identical mark per row would be a request per row to say one thing. See
        // `ir::Entry::icon`.
        icon: None,
        // v1. A payload's own `<img>` already arrives as a `Block::Figure` inside the summary,
        // and enclosures can be added later without rework. `search::parse` carries `None` too,
        // for a different reason: there the picture would be the engine's, here there is not one.
        image: None,
    }
}

/// The last non-empty path segment of an address, as a headline of last resort. `None` for an
/// address that is all host, which has no segment to name the post by.
fn last_segment(href: &str) -> Option<String> {
    let url = Url::parse(href).ok()?;
    let seg = url.path_segments()?.rfind(|s| !s.is_empty())?;
    (!seg.is_empty()).then(|| seg.to_owned())
}

/// A headline, whitespace collapsed. Empty for an item that carried no title element, or one
/// holding only markup.
fn payload_title(p: Option<&Payload<'_, '_>>) -> String {
    crate::ir::normalize_ws(&p.map(plain_of).unwrap_or_default())
}

/// A feed as a document.
///
/// `site_name` stays `None` deliberately. `blocks::document_header` prints `site_name` as an
/// eyebrow above the `<h1>` it makes from `title`, so filling both prints one name twice; left
/// empty, the eyebrow falls back to the host, which is what a feed's masthead wants.
fn document(
    base: &Url,
    title: Option<String>,
    published: Option<String>,
    lang: Option<String>,
    entries: Vec<Entry>,
) -> Document {
    Document {
        url: base.to_string(),
        title: title.filter(|t| !t.trim().is_empty()),
        byline: None,
        published: published.filter(|p| !p.trim().is_empty()),
        site_name: None,
        // The feed's **own** origin, never the host in the channel's `<link>`. The favicon is
        // the one request hww makes without being asked, so deriving it from an address written
        // inside the document would let a feed point that request at a host the reader never
        // contacted.
        favicon: base.join("/favicon.ico").ok().map(String::from),
        lang: lang.filter(|l| !l.trim().is_empty()),
        // Document order, always. Feeds are conventionally newest-first and the spec does not
        // require it, so sorting by date would reorder a publisher who meant something by the
        // order. No de-duplication either: `search::parse` dedupes by href because engines
        // repeat results across their own columns, whereas a repeat in a feed is the
        // publisher's statement about their own `guid`.
        blocks: vec![Block::Entries(entries)],
    }
}

// ---------------------------------------------------------------- payloads

/// One element's content, and which of Atom's three `type` cases it turned out to be.
struct Payload<'a, 'i> {
    /// The element it came from, for the `xml:base` chain.
    node: Node<'a, 'i>,
    body: Body,
}

enum Body {
    /// An HTML source string: `type="html"`, a CDATA section, an entity-escaped payload, or no
    /// `type` at all. The XML parser has already resolved CDATA and entities, so all four
    /// arrive here as the same thing and are re-parsed with `Html::parse_fragment`.
    Source(String),
    /// `type="text"`. Plain prose, and handing it to a fragment parser would eat any `<` in it.
    Text(String),
}

/// An element read under Atom's `type` attribute.
///
/// Three cases and only one of them is a source string. `xhtml` is the trap: the payload is a
/// real child `<div>` element rather than an escaped string, so `text_of` returns the words with
/// every tag gone and the entry silently looks broken. Serializing the subtree is what makes it
/// readable again.
fn payload_of<'a, 'i>(node: Node<'a, 'i>) -> Payload<'a, 'i> {
    let body = match node.attribute("type") {
        Some("text") => Body::Text(text_of(node)),
        Some("xhtml") => Body::Source(serialize(node)),
        _ => Body::Source(text_of(node)),
    };
    Payload { node, body }
}

/// An element whose content is a source string whatever it says: RSS, which has no `type`.
fn source_payload<'a, 'i>(node: Node<'a, 'i>) -> Payload<'a, 'i> {
    Payload {
        node,
        body: Body::Source(text_of(node)),
    }
}

/// A payload as IR.
fn blocks_of(p: &Payload<'_, '_>, base: &Url) -> Vec<Block> {
    match &p.body {
        Body::Source(s) => {
            let frag = Html::parse_fragment(s);
            // Unhinted, and that is the whole reason `blocks_from_public_unhinted` exists: the
            // chrome hints delete a `share`- or `promo`-classed wrapper, which on a whole page
            // is nav and inside a feed summary is the post.
            crate::html::blocks_from_public_unhinted(frag.root_element(), base)
        }
        Body::Text(t) => {
            let t = t.trim();
            if t.is_empty() {
                Vec::new()
            } else {
                vec![Block::Paragraph(vec![Inline::Text(t.to_owned())])]
            }
        }
    }
}

/// A payload's words.
fn plain_of(p: &Payload<'_, '_>) -> String {
    match &p.body {
        Body::Source(s) => Html::parse_fragment(s)
            .root_element()
            .text()
            .collect::<String>(),
        Body::Text(t) => t.clone(),
    }
}

/// The first line of a summary, for an item that carried no headline of its own.
fn lead_in(blocks: &[Block]) -> String {
    let text = blocks_plain(blocks);
    let line = text.lines().map(str::trim).find(|l| !l.is_empty());
    let line = line.unwrap_or_default();
    match line.char_indices().nth(FALLBACK_TITLE) {
        Some((i, _)) => format!("{}…", line[..i].trim_end()),
        None => line.to_owned(),
    }
}

/// A block list's words.
///
/// Not a second text currency: every leaf goes through [`ir::plain_text`], which is the crate's
/// presentation-neutral flatten. This only walks the tree to reach the inlines.
fn blocks_plain(blocks: &[Block]) -> String {
    let mut out = String::new();
    for b in blocks {
        let words = match b {
            Block::Heading { inlines, .. } | Block::Paragraph(inlines) => ir::plain_text(inlines),
            Block::List { items, .. } => items.iter().map(|i| blocks_plain(i)).collect(),
            Block::Quote { blocks, .. } => blocks_plain(blocks),
            Block::Code { text, .. } => text.clone(),
            Block::Figure { caption, .. } => {
                caption.as_deref().map(ir::plain_text).unwrap_or_default()
            }
            Block::Table { headers, rows } => headers
                .iter()
                .chain(rows.iter().flatten())
                .map(|c| ir::plain_text(c))
                .collect::<Vec<_>>()
                .join(" "),
            Block::Rule | Block::Embed { .. } => String::new(),
            Block::Thread(cs) => cs.iter().map(|c| blocks_plain(&c.blocks)).collect(),
            Block::Entries(es) => es.iter().map(|e| ir::plain_text(&e.title)).collect(),
            Block::Tweets(ps) => ps.iter().map(|p| blocks_plain(&p.blocks)).collect(),
            Block::Profile(p) => blocks_plain(&p.bio),
        };
        if !words.trim().is_empty() {
            out.push_str(words.trim());
            out.push('\n');
        }
    }
    out
}

// ---------------------------------------------------------------- XML plumbing

/// Element children of `node` in namespace `ns` with local name `name`.
///
/// The namespace is checked, not just the local name. An RSS `<channel>` routinely carries an
/// `<atom:link rel="self">` **before** its own `<link>`, and matching on the local name alone
/// would hand every entry the feed's own address.
fn children<'a, 'i>(
    node: Node<'a, 'i>,
    ns: Option<&'a str>,
    name: &'a str,
) -> impl Iterator<Item = Node<'a, 'i>> {
    node.children().filter(move |n| {
        n.is_element() && n.tag_name().name() == name && n.tag_name().namespace() == ns
    })
}

fn child<'a, 'i>(node: Node<'a, 'i>, ns: Option<&'a str>, name: &'a str) -> Option<Node<'a, 'i>> {
    children(node, ns, name).next()
}

fn child_text(node: Node<'_, '_>, ns: Option<&str>, name: &str) -> Option<String> {
    let t = text_of(child(node, ns, name)?);
    let t = t.trim();
    (!t.is_empty()).then(|| t.to_owned())
}

/// Everything an element says, with CDATA sections and entity references both already resolved
/// by the parser.
///
/// Walks every descendant text node rather than calling `Node::text`, which returns only the
/// first child: a payload split across a text node and a CDATA section would otherwise lose
/// half of itself.
fn text_of(node: Node<'_, '_>) -> String {
    let mut out = String::new();
    for n in node.descendants() {
        if n.is_text() {
            out.push_str(n.text().unwrap_or_default());
        }
    }
    out
}

/// The `xml:lang` in force at `node`.
fn lang_at(node: Node<'_, '_>) -> Option<String> {
    node.ancestors()
        .find_map(|n| n.attribute((XML, "lang")))
        .map(str::to_owned)
}

/// The base URL in force at `node`: the feed's own address with every `xml:base` from the root
/// down joined onto it in turn, which is what the XML Base spec says the chain means.
fn base_at(node: Node<'_, '_>, base: &Url) -> Url {
    let mut chain: Vec<&str> = node
        .ancestors()
        .filter_map(|n| n.attribute((XML, "base")))
        .collect();
    chain.reverse();
    let mut url = base.clone();
    for step in chain {
        if let Ok(next) = url.join(step) {
            url = next;
        }
    }
    url
}

/// One entry's destination: absolutized against the `xml:base` in force, then put through the
/// same gate a clicked link goes through.
///
/// Unlike `search::destination`, which drops a result it cannot follow, this answers `None` and
/// **the caller keeps the entry**: a search result whose whole purpose is to be clicked is
/// worthless without a destination, whereas a linkless feed item is still readable content, and
/// `entries_ui` already draws an unlinked title.
fn destination(at: Node<'_, '_>, raw: &str, base: &Url) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let url = base_at(at, base).join(raw).ok()?;
    match crate::session::classify_link(url.as_str()) {
        crate::session::Target::Navigate(u) => Some(u.into()),
        _ => None,
    }
}

/// HTML elements that carry no closing tag. An Atom `xhtml` payload is XML, where `<br/>` is
/// ordinary; written back as `<br></br>` an HTML parser reads the close tag as a second break.
const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track",
    "wbr",
];

/// An Atom `xhtml` payload's children, written back as an HTML source string.
///
/// The element itself is not emitted: the payload is the `<div>`'s contents, and wrapping them
/// in the `<content>` tag would put an unknown element at the root of the fragment.
fn serialize(node: Node<'_, '_>) -> String {
    let mut out = String::new();
    write_children(node, &mut out, 0);
    out
}

fn write_children(node: Node<'_, '_>, out: &mut String, depth: usize) {
    if depth >= MAX_DEPTH {
        // Past the budget the subtree is flattened to its words rather than dropped: a
        // pathological document degrades to prose instead of a blank entry. See [`MAX_DEPTH`].
        escape(&text_of(node), out);
        return;
    }
    for child in node.children() {
        if child.is_text() {
            escape(child.text().unwrap_or_default(), out);
        } else if child.is_element() {
            let name = child.tag_name().name();
            out.push('<');
            out.push_str(name);
            for a in child.attributes() {
                // Namespace declarations are XML's plumbing and mean nothing to an HTML parser.
                if a.name() == "xmlns" || a.namespace() == Some(XML) {
                    continue;
                }
                out.push(' ');
                out.push_str(a.name());
                out.push_str("=\"");
                escape(a.value(), out);
                out.push('"');
            }
            out.push('>');
            if !VOID.contains(&name) {
                write_children(child, out, depth + 1);
                out.push_str("</");
                out.push_str(name);
                out.push('>');
            }
        }
    }
}

fn escape(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
}

// ---------------------------------------------------------------- fixtures

/// One synthetic feed, and what reading it must produce.
///
/// A table rather than a test each, in the shape `sites::ENGINES` uses: a new payload shape
/// costs a row. `session::every_feed_fixture_becomes_entries` walks it too, through the call
/// `Session::load` makes, so nothing here can pass the parser and fail the pipeline.
#[cfg(test)]
pub(crate) struct Fixture {
    pub name: &'static str,
    pub kind: Kind,
    pub entries: usize,
    pub source: &'static str,
}

#[cfg(test)]
pub(crate) const FIXTURES: &[Fixture] = &[
    Fixture {
        name: "RSS, plain and entity-escaped payloads",
        kind: Kind::Rss,
        entries: 2,
        source: RSS_PLAIN,
    },
    Fixture {
        name: "RSS, CDATA payload",
        kind: Kind::Rss,
        entries: 1,
        source: RSS_CDATA,
    },
    Fixture {
        name: "RSS, content:encoded",
        kind: Kind::Rss,
        entries: 1,
        source: RSS_CONTENT,
    },
    Fixture {
        name: "Atom, all three content types",
        kind: Kind::Atom,
        entries: 3,
        source: ATOM_THREE_WAYS,
    },
    Fixture {
        name: "Atom, a title and a link and nothing else",
        kind: Kind::Atom,
        entries: 1,
        source: ATOM_BARE,
    },
];

/// A channel-level `<atom:link rel="self">` ahead of the channel's own `<link>`, and an item
/// carrying one too: matching on the local name alone hands every entry the feed's own address.
#[cfg(test)]
const RSS_PLAIN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:atom="http://www.w3.org/2005/Atom">
  <channel>
    <atom:link href="https://example.com/feed.xml" rel="self" type="application/rss+xml"/>
    <title>Lorem Weekly</title>
    <link>https://example.com/</link>
    <language>en-gb</language>
    <lastBuildDate>Thu, 27 Aug 2026 17:17:57 +0000</lastBuildDate>
    <item>
      <atom:link href="https://example.com/feed.xml#1" rel="self"/>
      <title>The first note</title>
      <link>https://example.com/one</link>
      <pubDate>Thu, 27 Aug 2026 09:00:00 +0000</pubDate>
      <description>A plain description with an &amp;amp; in it and no markup at all, which is
      long enough to read as a paragraph of its own.</description>
    </item>
    <item>
      <title>The second note</title>
      <link>/two</link>
      <pubDate>Wed, 26 Aug 2026 09:00:00 +0000</pubDate>
      <description>&lt;p&gt;An entity-escaped paragraph, which the XML parser hands back as a
      source string for the HTML parser to read.&lt;/p&gt;</description>
    </item>
  </channel>
</rss>"#;

/// The measured regression that chose the parser: html5ever reads the CDATA section as a bogus
/// comment running to the first `>` inside it, and returns nothing.
#[cfg(test)]
const RSS_CDATA: &str = r#"<?xml version="1.0"?>
<rss version="2.0">
  <channel>
    <title>Cdata Daily</title>
    <item>
      <title><![CDATA[A <em>marked up</em> headline]]></title>
      <link>https://example.com/cdata</link>
      <description><![CDATA[<p>A paragraph carrying a > greater-than sign, which is where an
      HTML parser reading this as a bogus comment stops swallowing it.</p><p>And a second
      paragraph, which goes with it.</p>]]></description>
    </item>
  </channel>
</rss>"#;

/// `content:encoded` is the post and `<description>` beside it is the teaser. The body arrives
/// inside a `promo`-classed wrapper, which is a class the whole-page chrome hints delete.
#[cfg(test)]
const RSS_CONTENT: &str = r#"<?xml version="1.0"?>
<rss version="2.0" xmlns:content="http://purl.org/rss/1.0/modules/content/">
  <channel>
    <title>Full Text</title>
    <item>
      <title>Everything in the feed</title>
      <link>https://example.com/posts/full</link>
      <description>A teaser standing in for the post.</description>
      <content:encoded><![CDATA[<div class="post-promo"><p>The whole post body, which is what
      content:encoded carries and what the teaser above stands in for.</p>
      <img src="pics/one.png" alt="a picture"/></div>]]></content:encoded>
    </item>
  </channel>
</rss>"#;

/// Atom's `type` has three cases and only one of them is a source string. Entry one also puts
/// two links hww must not follow ahead of the one it must.
#[cfg(test)]
const ATOM_THREE_WAYS: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom" xml:lang="en">
  <title type="text">Atom Three Ways</title>
  <updated>2026-08-27T17:17:57Z</updated>
  <link rel="self" href="https://example.com/atom.xml"/>
  <entry>
    <title type="html">An &lt;em&gt;html&lt;/em&gt; title</title>
    <link rel="self" href="https://example.com/html-entry.atom"/>
    <link rel="replies" href="https://example.com/html-entry/replies"/>
    <link rel="alternate" href="https://example.com/html-entry"/>
    <published>2026-08-27T09:00:00Z</published>
    <content type="html">&lt;p&gt;An escaped HTML paragraph, long enough to read as
    one.&lt;/p&gt;</content>
  </entry>
  <entry xml:base="https://cdn.example.net/posts/">
    <title>A plain title with no type at all</title>
    <link href="xhtml-entry"/>
    <updated>2026-08-26T09:00:00Z</updated>
    <content type="xhtml"><div xmlns="http://www.w3.org/1999/xhtml"><p>A real element subtree,
    which <code>node.text()</code> alone would flatten past every tag in it.</p>
    <img src="two.png" alt="another picture"/><br/></div></content>
  </entry>
  <entry>
    <title type="text">A title where 5 &lt; 6</title>
    <link href="https://example.com/text-entry"/>
    <summary type="text">Plain prose where 1 &lt; 2 and a &lt;bracket&gt; is not a tag, set
    down at enough length to be a paragraph.</summary>
  </entry>
</feed>"#;

#[cfg(test)]
const ATOM_BARE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Bare Atom</title>
  <entry>
    <title>Only a headline</title>
    <link href="https://example.com/bare"/>
  </entry>
</feed>"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn at() -> Url {
        Url::parse("https://example.com/feed.xml").unwrap()
    }

    fn feed(source: &str) -> (Document, Report) {
        match read(source, &at()) {
            Outcome::Feed(d, r) => (d, r),
            other => panic!("expected a feed, got {other:?}"),
        }
    }

    fn rows(doc: &Document) -> &[Entry] {
        match doc.blocks.as_slice() {
            [Block::Entries(es)] => es,
            other => panic!("a feed document is one run of entries, got {other:?}"),
        }
    }

    fn title(e: &Entry) -> String {
        ir::plain_text(&e.title)
    }

    fn summary(e: &Entry) -> String {
        blocks_plain(&e.summary)
    }

    /// Every fixture reads as the format it claims, with the entries it claims, and every row
    /// has something to show for itself.
    #[test]
    fn every_fixture_reads_as_the_feed_it_claims_to_be() {
        for f in FIXTURES {
            let (doc, report) = feed(f.source);
            assert_eq!(report.kind, f.kind, "{}", f.name);
            assert_eq!(report.entries, f.entries, "{}", f.name);
            let es = rows(&doc);
            assert_eq!(es.len(), f.entries, "{}", f.name);
            assert!(doc.title.is_some(), "{}: no masthead", f.name);
            assert!(doc.text_len() > 0, "{}: no text", f.name);
            for e in es {
                assert!(
                    !title(e).is_empty(),
                    "{}: an entry with no headline",
                    f.name
                );
            }
        }
    }

    /// The measurement that chose an XML parser. html5ever returns 0 characters here on a real
    /// feed where roxmltree returns 3,255, and how much is lost depends on where the first `>`
    /// falls, so the failure is silent and data-dependent.
    #[test]
    fn a_cdata_summary_survives() {
        let (doc, _) = feed(RSS_CDATA);
        let e = &rows(&doc)[0];
        let text = summary(e);
        assert!(text.contains("greater-than sign"), "{text:?}");
        // Both paragraphs, which is the half an HTML parser eats.
        assert!(text.contains("And a second"), "{text:?}");
        assert_eq!(e.summary.len(), 2, "{:?}", e.summary);
        // And the markup in a CDATA headline is read, not printed.
        assert_eq!(title(e), "A marked up headline");
    }

    /// An HTML page opens `<!DOCTYPE` or `<html` and never reaches the parser. A page merely
    /// carrying the word `feed` does not either.
    #[test]
    fn an_html_page_is_not_a_feed() {
        for page in [
            "<!DOCTYPE html><html><body><p>a feed of news</p></body></html>",
            "<html><head><title>feed</title></head><body>rss</body></html>",
            "  \n<!doctype html>\n<html><body>x</body></html>",
            "<feedback><item>x</item></feedback>",
        ] {
            assert!(
                matches!(read(page, &at()), Outcome::NotAFeed),
                "{page:?} read as a feed"
            );
        }
    }

    /// RSS 1.0 is `rdf:RDF`, which this does not read. It falls through to the generic
    /// extractor rather than to a blank screen, which is why `is_web_page` passes
    /// `application/rdf+xml`.
    #[test]
    fn an_rdf_feed_falls_through() {
        let rdf = r#"<?xml version="1.0"?>
<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"
         xmlns="http://purl.org/rss/1.0/">
  <channel><title>Old School</title></channel>
  <item><title>One</title><link>https://example.com/one</link></item>
</rdf:RDF>"#;
        assert!(matches!(read(rdf, &at()), Outcome::NotAFeed));
    }

    /// It said it was a feed and would not parse. Falling through silently would hand the
    /// reader a near-blank page under a caution blaming JavaScript, because the whole premise
    /// of choosing an XML parser is that the HTML extractor gets nothing out of these.
    #[test]
    fn a_malformed_feed_is_a_remark_not_a_blank_page() {
        // `&nbsp;` is not an XML entity, and it is the commonest way a feed breaks.
        let broken = r#"<rss version="2.0"><channel><title>Broken&nbsp;Weekly</title>
        <item><title>One</title></item></channel></rss>"#;
        let Outcome::Unreadable(msg) = read(broken, &at()) else {
            panic!("a named feed that will not parse owes a remark");
        };
        assert!(msg.contains("nbsp"), "{msg:?}");
        assert!(msg.chars().count() <= MAX_MESSAGE + 1, "{msg:?}");
    }

    /// The declaration is not the claim: nearly every generator emits one, so reading it as the
    /// whole story made [`Outcome::Unreadable`] unreachable for real feeds and sent the
    /// commonest breakage of all down the silent fallthrough.
    #[test]
    fn a_declared_feed_that_will_not_parse_is_still_a_remark() {
        let broken = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0"><channel><title>Broken&nbsp;Weekly</title>
<item><title>One</title></item></channel></rss>"#;
        let Outcome::Unreadable(msg) = read(broken, &at()) else {
            panic!("a feed that names itself past its declaration owes a remark");
        };
        assert!(msg.contains("nbsp"), "{msg:?}");
        // And the same holds past a stylesheet instruction and a comment, which is what a
        // generator puts between the declaration and the root.
        let decorated = r#"<?xml version="1.0"?>
<?xml-stylesheet type="text/xsl" href="/feed.xsl"?>
<!-- generated -->
<feed xmlns="http://www.w3.org/2005/Atom"><title>Broken&nbsp;Atom</title></feed>"#;
        assert!(matches!(read(decorated, &at()), Outcome::Unreadable(_)));
    }

    /// A document that only ever claimed to be XML could be anything, so a failure there blames
    /// nobody and lets the extractor try.
    #[test]
    fn malformed_xml_that_never_claimed_to_be_a_feed_falls_through() {
        let x = r#"<?xml version="1.0"?><catalogue><item>unclosed</catalogue>"#;
        assert!(matches!(read(x, &at()), Outcome::NotAFeed));
    }

    /// `allow_dtd` is off, which refuses a billion-laughs bomb at the parse rather than
    /// expanding it, and refuses an XHTML page for its `<!DOCTYPE` before any element is read.
    #[test]
    fn a_doctype_is_refused() {
        let bomb = r#"<?xml version="1.0"?>
<!DOCTYPE lolz [ <!ENTITY lol "lol"> <!ENTITY lol2 "&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;">
 <!ENTITY lol3 "&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;"> ]>
<rss version="2.0"><channel><title>&lol3;</title></channel></rss>"#;
        assert!(matches!(read(bomb, &at()), Outcome::NotAFeed));
        let xhtml = r#"<?xml version="1.0"?><!DOCTYPE html><html><body>x</body></html>"#;
        assert!(matches!(read(xhtml, &at()), Outcome::NotAFeed));
    }

    /// `read` admits an Atom feed that dropped its `xmlns`, so everything under the root has to
    /// be looked for in the namespace the root actually has. Asking for Atom's regardless
    /// matched nothing and reported a feed full of posts as one that listed none.
    #[test]
    fn an_atom_feed_without_a_namespace_still_has_its_entries() {
        let bare = r#"<?xml version="1.0"?>
<feed><title>Bare Atom</title>
<entry><title>First</title><link href="https://example.com/1"/></entry>
<entry><title>Second</title><link href="https://example.com/2"/></entry></feed>"#;
        let (doc, report) = feed(bare);
        assert_eq!(report.kind, Kind::Atom);
        assert_eq!(
            report.entries, 2,
            "the entries were looked for in the wrong namespace"
        );
        assert_eq!(doc.title.as_deref(), Some("Bare Atom"));
        let es = rows(&doc);
        assert_eq!(title(&es[0]), "First");
        assert_eq!(es[1].href.as_deref(), Some("https://example.com/2"));
    }

    /// An element that is present and yields nothing is not an answer. The `<guid>` beside it
    /// carries the permalink, and taking the empty `<link>` as the decision threw it away.
    #[test]
    fn an_empty_link_falls_through_to_the_guid() {
        let src = r#"<rss version="2.0"><channel><title>W</title>
<item><title>One</title><link></link>
<guid isPermaLink="true">https://example.com/one</guid></item>
<item><title>Two</title><link>javascript:alert(1)</link>
<guid>https://example.com/two</guid></item></channel></rss>"#;
        let (doc, _) = feed(src);
        let es = rows(&doc);
        assert_eq!(es[0].href.as_deref(), Some("https://example.com/one"));
        // And a link that resolves to nothing hww will navigate to is the same case.
        assert_eq!(es[1].href.as_deref(), Some("https://example.com/two"));
    }

    /// The headline chain has to end somewhere. `entries_ui` wraps whatever is here in a link
    /// unconditionally, so an empty string is a headline with no width and a Tab stop with
    /// nothing at it.
    #[test]
    fn an_entry_with_nothing_to_name_it_still_gets_a_headline() {
        let src = r#"<rss version="2.0"><channel><title>W</title>
<item><link>https://example.com/posts/the-slug</link></item>
<item><pubDate>Mon, 24 Aug 2026 10:00:00 GMT</pubDate></item>
<item><link>https://example.com/</link></item></channel></rss>"#;
        let (doc, _) = feed(src);
        for e in rows(&doc) {
            assert!(!title(e).trim().is_empty(), "an entry with no headline");
        }
        let es = rows(&doc);
        assert_eq!(title(&es[0]), "the-slug", "the address names the post");
        assert!(
            title(&es[1]).contains("2026"),
            "the date is the next resort"
        );
        assert_eq!(title(&es[2]), UNTITLED, "and a word before a blank line");
    }

    /// The empty-feed title is the publisher's string and it reaches a page remark whole, which
    /// is the argument that caps the parser message beside it.
    #[test]
    fn an_empty_feed_s_title_is_capped() {
        let long = "x".repeat(4_000);
        let src = format!(r#"<rss version="2.0"><channel><title>{long}</title></channel></rss>"#);
        let Outcome::Empty { title } = read(&src, &at()) else {
            panic!("an empty feed owes a remark");
        };
        assert!(title.chars().count() <= MAX_TITLE + 1, "{}", title.len());
    }

    /// A feed that parsed and carried nothing. The reading column holds page content only, so
    /// this is a page remark rather than an empty run of entries, and it names the publication.
    #[test]
    fn an_empty_feed_is_a_remark_not_a_blank_page() {
        let empty = r#"<?xml version="1.0"?>
<rss version="2.0"><channel><title>Quiet Weekly</title>
<link>https://example.com/</link></channel></rss>"#;
        let Outcome::Empty { title } = read(empty, &at()) else {
            panic!("an empty feed owes a remark");
        };
        assert_eq!(title, "Quiet Weekly");
        let atom = r#"<feed xmlns="http://www.w3.org/2005/Atom"><title>Also Quiet</title></feed>"#;
        assert!(matches!(read(atom, &at()), Outcome::Empty { .. }));
    }

    /// `rel` defaults to `alternate`, so selecting on the attribute alone drops every
    /// conformant entry that leaves it out.
    #[test]
    fn an_atom_link_without_a_rel_is_the_entry_link() {
        let (doc, _) = feed(ATOM_BARE);
        assert_eq!(
            rows(&doc)[0].href.as_deref(),
            Some("https://example.com/bare")
        );
    }

    /// `self`, `replies`, and the rest name something other than the post, and one of them
    /// coming first must not win.
    #[test]
    fn a_self_link_is_not_the_entry_link() {
        let (doc, _) = feed(ATOM_THREE_WAYS);
        assert_eq!(
            rows(&doc)[0].href.as_deref(),
            Some("https://example.com/html-entry")
        );
        // Same trap in RSS, where the alien link is namespaced rather than `rel`-tagged.
        let (doc, _) = feed(RSS_PLAIN);
        assert_eq!(
            rows(&doc)[0].href.as_deref(),
            Some("https://example.com/one")
        );
        // And a relative RSS link resolves against the feed.
        assert_eq!(
            rows(&doc)[1].href.as_deref(),
            Some("https://example.com/two")
        );
    }

    /// A `guid` is an identifier and only sometimes an address. Generators put counters and
    /// `tag:` URIs in one without setting `isPermaLink`, and joining a counter against the feed
    /// URL turns it into a confident link to a page that does not exist — worse than no link.
    #[test]
    fn only_a_guid_that_is_a_web_address_stands_in_for_a_link() {
        let src = r#"<?xml version="1.0"?>
<rss version="2.0"><channel><title>Guids</title>
<item><title>A permalink guid</title><guid>https://example.com/real</guid></item>
<item><title>A counter</title><guid>12345</guid></item>
<item><title>A tag URI</title><guid isPermaLink="false">tag:example.com,2026:1</guid></item>
<item><title>An opted-out URL</title><guid isPermaLink="false">https://example.com/no</guid></item>
</channel></rss>"#;
        let (doc, _) = feed(src);
        let hrefs: Vec<Option<&str>> = rows(&doc).iter().map(|e| e.href.as_deref()).collect();
        assert_eq!(
            hrefs,
            [Some("https://example.com/real"), None, None, None],
            "{hrefs:?}"
        );
        // And a real `<link>` always wins over the guid beside it.
        let src = r#"<?xml version="1.0"?>
<rss version="2.0"><channel><title>Both</title>
<item><title>One</title><link>https://example.com/link</link>
<guid>https://example.com/guid</guid></item></channel></rss>"#;
        let (doc, _) = feed(src);
        assert_eq!(
            rows(&doc)[0].href.as_deref(),
            Some("https://example.com/link")
        );
    }

    /// The deliberate divergence from `search::destination`, which drops a result it cannot
    /// follow. A row whose only purpose is to be clicked is worthless without a destination; a
    /// feed item is readable content, and `entries_ui` already draws an unlinked title.
    #[test]
    fn an_unusable_link_keeps_the_entry() {
        let src = r#"<?xml version="1.0"?>
<rss version="2.0"><channel><title>Odd Links</title>
<item><title>A javascript link</title><link>javascript:alert(1)</link>
<description>Still worth reading, and still a row on the page.</description></item>
<item><title>No link at all</title>
<description>Also still a row, with a headline nothing can be clicked on.</description></item>
</channel></rss>"#;
        let (doc, report) = feed(src);
        assert_eq!(report.entries, 2);
        for e in rows(&doc) {
            assert_eq!(e.href, None);
            assert!(!title(e).is_empty());
            assert!(!summary(e).is_empty());
        }
    }

    /// Three cases and only one of them is a source string. `xhtml` is the trap: the payload is
    /// a real child subtree, so `node.text()` returns the words with every tag gone.
    #[test]
    fn each_atom_content_type_reads() {
        let (doc, _) = feed(ATOM_THREE_WAYS);
        let es = rows(&doc);
        // `html`: an escaped source string, so the tags are read rather than printed.
        assert_eq!(title(&es[0]), "An html title");
        assert!(matches!(es[0].summary.as_slice(), [Block::Paragraph(_)]));
        assert!(summary(&es[0]).contains("An escaped HTML paragraph"));
        assert!(!summary(&es[0]).contains("<p>"));
        // `xhtml`: a real subtree, serialized back out.
        assert!(summary(&es[1]).contains("A real element subtree"));
        assert!(summary(&es[1]).contains("node.text()"));
        // `text`: prose, and a `<` in it is a less-than sign rather than the start of a tag.
        assert_eq!(title(&es[2]), "A title where 5 < 6");
        assert!(summary(&es[2]).contains("1 < 2"), "{:?}", summary(&es[2]));
        assert!(
            summary(&es[2]).contains("<bracket>"),
            "{:?}",
            summary(&es[2])
        );
    }

    /// A payload is IR, not a wall of text: the same blocks any other document produces.
    #[test]
    fn a_payload_becomes_blocks() {
        let src = r#"<?xml version="1.0"?>
<rss version="2.0"><channel><title>Shapes</title>
<item><title>One of each</title><link>https://example.com/shapes</link>
<description><![CDATA[<p>A lead paragraph with enough words in it to stand as one.</p>
<blockquote><p>Something somebody else said, at similar length.</p></blockquote>
<ul><li>a first item</li><li>a second item</li></ul>]]></description></item>
</channel></rss>"#;
        let (doc, _) = feed(src);
        assert!(
            matches!(
                rows(&doc)[0].summary.as_slice(),
                [Block::Paragraph(_), Block::Quote { .. }, Block::List { .. }]
            ),
            "{:?}",
            rows(&doc)[0].summary
        );
    }

    /// `xml:base` is what the payload says its own links resolve against, and it outranks both
    /// the entry's link and the feed's URL. Two feeds in the measured sample set it.
    #[test]
    fn xml_base_beats_the_feed_url() {
        let (doc, _) = feed(ATOM_THREE_WAYS);
        let es = rows(&doc);
        // The entry's own link, resolved against the `xml:base` on the entry element.
        assert_eq!(
            es[1].href.as_deref(),
            Some("https://cdn.example.net/posts/xhtml-entry")
        );
        // And the picture inside the payload, against the same chain.
        let srcs = image_srcs(&es[1].summary);
        assert_eq!(srcs, ["https://cdn.example.net/posts/two.png"]);
    }

    /// With no `xml:base` in force, a relative link in a payload resolves against the post the
    /// entry points at rather than against the feed's own address.
    #[test]
    fn a_payload_without_a_base_resolves_against_the_post() {
        let (doc, _) = feed(RSS_CONTENT);
        assert_eq!(
            image_srcs(&rows(&doc)[0].summary),
            ["https://example.com/posts/pics/one.png"]
        );
    }

    fn image_srcs(blocks: &[Block]) -> Vec<String> {
        blocks
            .iter()
            .flat_map(|b| match b {
                Block::Figure { image, .. } => vec![image.src.clone()],
                Block::Quote { blocks, .. } => image_srcs(blocks),
                _ => vec![],
            })
            .collect()
    }

    /// The reason `html::blocks_from_public_unhinted` exists. The chrome hints delete a
    /// `share`- or `promo`-classed wrapper, which on a whole page is nav and inside a feed
    /// summary is the post. Deleted, it fails silently: the entry simply comes out short.
    #[test]
    fn a_promo_class_in_a_summary_survives() {
        let (doc, _) = feed(RSS_CONTENT);
        let text = summary(&rows(&doc)[0]);
        assert!(text.contains("The whole post body"), "{text:?}");
        // And `content:encoded` beat the teaser beside it.
        assert!(!text.contains("standing in for the post"), "{text:?}");
    }

    /// RSS permits an item carrying only a description, and link blogs use it. `ir::Entry::title`
    /// is a `Vec<Inline>` rather than an `Option`, and `entries_ui` builds its link from that
    /// vec unconditionally: an empty one is an empty headline with nothing to click.
    #[test]
    fn a_title_less_item_still_has_a_headline() {
        let src = r#"<?xml version="1.0"?>
<rss version="2.0"><channel><title>Link Blog</title>
<item><link>https://example.com/linked</link>
<description><![CDATA[<p>The whole of this item is its description, which is how a link blog
writes one, and there is no title element anywhere in it at all.</p>]]></description></item>
</channel></rss>"#;
        let (doc, _) = feed(src);
        let e = &rows(&doc)[0];
        assert!(
            title(e).starts_with("The whole of this item"),
            "{:?}",
            title(e)
        );
        assert!(
            title(e).chars().count() <= FALLBACK_TITLE + 1,
            "{:?}",
            title(e)
        );
        assert!(title(e).ends_with('…'), "{:?}", title(e));
    }

    /// Document order, always. Feeds are conventionally newest-first and the spec does not
    /// require it, so sorting by date would reorder a publisher who meant something by it.
    #[test]
    fn entries_keep_document_order() {
        let src = r#"<?xml version="1.0"?>
<rss version="2.0"><channel><title>Out Of Order</title>
<item><title>Oldest</title><pubDate>Mon, 03 Aug 2026 09:00:00 +0000</pubDate></item>
<item><title>Newest</title><pubDate>Thu, 27 Aug 2026 09:00:00 +0000</pubDate></item>
<item><title>Middle</title><pubDate>Sun, 16 Aug 2026 09:00:00 +0000</pubDate></item>
<item><title>Newest</title><pubDate>Thu, 27 Aug 2026 09:00:00 +0000</pubDate></item>
</channel></rss>"#;
        let (doc, _) = feed(src);
        let order: Vec<String> = rows(&doc).iter().map(title).collect();
        assert_eq!(order, ["Oldest", "Newest", "Middle", "Newest"]);
    }

    /// Mirrors `search::results_carry_no_images`, pinning the v1 decision. A payload's own
    /// `<img>` already arrives as a `Block::Figure` inside the summary; an entry thumbnail
    /// would be a second, separate third-party offer per row.
    #[test]
    fn no_entry_carries_an_image() {
        for f in FIXTURES {
            let (doc, _) = feed(f.source);
            for e in rows(&doc) {
                assert!(e.image.is_none(), "{}", f.name);
                assert!(e.address.is_none(), "{}", f.name);
            }
        }
    }

    /// The masthead, and the one field it deliberately leaves empty.
    #[test]
    fn the_masthead_names_the_feed_once() {
        let (doc, _) = feed(RSS_PLAIN);
        assert_eq!(doc.title.as_deref(), Some("Lorem Weekly"));
        // `document_header` prints `site_name` as an eyebrow above the `<h1>` it makes from
        // `title`; filling both prints one name twice, and with `None` the eyebrow is the host.
        assert_eq!(doc.site_name, None);
        assert_eq!(
            doc.published.as_deref(),
            Some("Thu, 27 Aug 2026 17:17:57 +0000")
        );
        assert_eq!(doc.lang.as_deref(), Some("en-gb"));
        let (doc, _) = feed(ATOM_THREE_WAYS);
        assert_eq!(doc.title.as_deref(), Some("Atom Three Ways"));
        assert_eq!(doc.lang.as_deref(), Some("en"));
    }

    /// The favicon is the one request hww makes without being asked, so it comes from the
    /// feed's own origin and never from a host named inside the document.
    #[test]
    fn the_favicon_comes_from_the_feed_not_from_the_document() {
        let src = r#"<?xml version="1.0"?>
<rss version="2.0"><channel><title>Pointed Elsewhere</title>
<link>https://elsewhere.example.net/</link>
<item><title>One</title><link>https://elsewhere.example.net/one</link></item>
</channel></rss>"#;
        let (doc, _) = feed(src);
        assert_eq!(
            doc.favicon.as_deref(),
            Some("https://example.com/favicon.ico")
        );
    }

    /// The publisher's raw stamp, as every other producer in this crate leaves it. Shortening
    /// is `reader::title::short_date`'s job and the plain renderer wants it whole.
    #[test]
    fn an_entry_keeps_the_publisher_s_own_date() {
        let (doc, _) = feed(RSS_PLAIN);
        assert_eq!(
            rows(&doc)[0].published.as_deref(),
            Some("Thu, 27 Aug 2026 09:00:00 +0000")
        );
        assert_eq!(
            crate::reader::title::short_date(rows(&doc)[0].published.as_deref().unwrap()),
            "2026-08-27"
        );
    }

    #[test]
    fn a_report_says_which_format_answered() {
        let (_, r) = feed(RSS_PLAIN);
        assert_eq!(r.to_string(), "RSS feed, 2 entries");
        let (_, r) = feed(ATOM_BARE);
        assert_eq!(r.to_string(), "Atom feed, 1 entry");
    }
}
