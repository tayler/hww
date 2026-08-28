//! Web search: a query becomes a URL, a result page becomes [`Block::Entries`].
//!
//! Phase 0 measured search as 42.6% of document-shaped browsing, the largest single class and
//! larger than the articles hww was built for. Until now none of it was reachable: the URL bar
//! rejected anything with a space, so a session could only start with a URL found in another
//! browser.
//!
//! Two things are separated here, on the same line `profile.rs` and `sites.rs` draw. A
//! [`SearchShape`] is **data**: CSS selectors and a query-parameter name, no hostnames, no code.
//! The hosts those shapes belong to live in `sites.rs`, which is still the only file in the
//! crate that names one. One walker reads any shape, so a fourth engine costs a table row and a
//! fixture rather than a fourth parser.
//!
//! # What was measured (2026-08-24)
//!
//! `duckduckgo.com/?q=` is a JavaScript shell: zero result links in the HTML. The address a
//! user would type cannot be read at all, which is why the table names no-JS endpoints instead
//! and why every entry is reported before the request goes out.
//!
//! Google, Startpage, and public SearxNG instances answer a plain client with an interstitial,
//! a proof-of-work challenge, and an antibot page respectively. None is expressible as a shape
//! and none is in the table.
//!
//! DuckDuckGo's no-JS endpoint answers four rapid queries and then returns **HTTP 202 with a
//! 123-byte CAPTCHA**. That is a 2xx, so [`crate::fetch::FetchError::LikelyBlocked`] — which is
//! a document read-timeout and nothing else — cannot see it. [`Outcome::Blocked`] is the
//! separate signal, and the two must stay separate: widening `LikelyBlocked` would destroy the
//! distinction Phase 0 paid for.
//!
//! Brave's markup carries a build hash (`svelte-1qq4ge7`) that rotates on every deploy, so no
//! selector may name one. Its results do carry stable `data-pos` and `data-type` attributes,
//! and `no_shape_depends_on_a_build_hash` holds the line against a future edit.
//!
//! # Why a parser rather than the generic extractor
//!
//! `html::extract` already finds the result container on all four engines. What it cannot do is
//! say which text is a title, which is a snippet, and which is chrome, so results arrive as
//! linked paragraphs interleaved with shopping carousels and related-search rubble. A shape
//! knows, and `Block::Entries` is the IR that already carries the answer.
//!
//! The parser is a candidate with a floor, never an override: zero entries on an otherwise
//! healthy page falls through to the generic extractor, so an engine that redesigns degrades to
//! a rougher page instead of a blank one.

use crate::ir::{Block, Document, Entry, Inline};
use crate::profile::Sel;
use scraper::{ElementRef, Html, Selector};
use url::Url;

/// How one engine's result page is laid out. Selector data only: no hostname, no code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchShape {
    /// The engine's own result frame: the element it renders whether or not it found anything.
    ///
    /// This is what separates "answered, found nothing" from "did not answer". Without it, a
    /// search for nonsense and a rate-limit interstitial are both "nothing matched", and hww
    /// has to guess which on a page where guessing wrong blames the wrong party.
    pub frame: Sel,
    /// One match per result. Everything else is scoped inside a match.
    pub result: Sel,
    /// The result's title, within a result.
    pub title: Sel,
    /// The element carrying the destination `href`, within a result. Often the same element as
    /// `title`; named separately because Brave puts the link on a wrapper.
    pub href: Sel,
    /// The result's snippet, within a result. `None` where the only candidate element is
    /// identified by a build hash: a title and a URL are still a usable row.
    pub snippet: Option<Sel>,
    /// Query parameter holding the real destination when the engine wraps result links in its
    /// own redirect. Unwrapping is a privacy improvement as much as a parsing one: the click
    /// goes to the site rather than through the engine's click tracker.
    pub unwrap_param: Option<&'static str>,
}

/// What reading a result page produced.
///
/// Deliberately not `PartialEq`: `ir::Entry` is not, and deriving it there to serve a test
/// here would put an ordering promise on the IR that nothing in the renderers wants.
#[derive(Debug, Clone)]
pub enum Outcome {
    /// Results, in page order.
    Results(Vec<Entry>),
    /// The engine asked for a human check. A 2xx that is not 200, carrying almost no text:
    /// the measured shape of DuckDuckGo's CAPTCHA. Distinct from a transport failure and from
    /// an honest empty result, because the remedy differs for each.
    Blocked,
    /// The engine answered normally and found nothing.
    Empty,
    /// The shape matched nothing but the page has content. The selectors have gone stale, or
    /// this is not the page the shape was written for. The caller falls back to the generic
    /// extractor rather than showing a blank page.
    Unrecognized,
}

/// Below this, a 2xx-but-not-200 body is a challenge page rather than a thin result page. The
/// measured CAPTCHA carries 123 characters; a real result page carries thousands.
const CHALLENGE_TEXT: usize = crate::ir::THIN_TEXT;

/// Above this, a result frame the shape found no rows in has been redesigned rather than
/// answered empty. The same shared threshold, asked about one element instead of a document:
/// "no results for …" is a sentence, and a screen of results the shape can no longer name is a
/// page. Neither number is tuned to an engine; both are [`crate::ir::THIN_TEXT`].
const FRAME_TEXT: usize = crate::ir::THIN_TEXT;

/// Build the query URL for `terms`.
///
/// `query_pairs_mut` percent-encodes once and correctly. Formatting the query into a string
/// would double-encode a term that already contains a `%`, and would let a `&` in the terms
/// forge a second parameter.
pub fn query_url(engine: &crate::sites::SearchEngine, terms: &str) -> Url {
    let mut url = Url::parse(&format!("https://{}", engine.host))
        .expect("engine host is a literal, checked by builtin_table_is_sound");
    url.set_path(engine.path);
    url.query_pairs_mut()
        .append_pair(engine.query_key, terms.trim());
    url
}

/// The terms an engine's result URL was built from, for the disclosure notice.
pub fn terms_of(engine: &crate::sites::SearchEngine, url: &Url) -> Option<String> {
    url.query_pairs()
        .find(|(k, _)| k == engine.query_key)
        .map(|(_, v)| v.into_owned())
        .filter(|t| !t.is_empty())
}

/// Read a result page.
///
/// Four outcomes, and the frame is what keeps them apart: whether it is there, and how much
/// text is inside it. An engine that found nothing still renders its own frame, with a sentence
/// in it; an engine that declined renders something else entirely; an engine that renamed its
/// result rows renders the frame full. Collapsing the first two would tell a reader searching
/// for a rare phrase that they had been rate-limited, or tell a rate-limited reader that their
/// words were no good, and collapsing the first and third would blame those words for a deploy.
pub fn read(source: &str, base: &Url, engine: &crate::sites::SearchEngine, status: u16) -> Outcome {
    let html = Html::parse_document(source);
    let shape = &engine.shape;
    let entries = parse(&html, base, shape);
    if !entries.is_empty() {
        return Outcome::Results(entries);
    }
    if let Some(frame) = first(&html, &shape.frame) {
        // The engine's own frame, with nothing in it: the one honest "no results". A frame
        // holding a page's worth of text that the shape found no rows in is the other stale
        // case — the engine kept its container and renamed the rows inside it — and telling
        // that reader "found nothing" over a screen of results is the one lie the frame rule
        // exists to prevent. The floor in the module doc applies to both.
        //
        // The cost is paid by a broad frame: `#search-page` wraps a whole page, so a genuine
        // empty answer on that engine may carry more than `THIN_TEXT` of chrome and fall
        // through. That reader sees the engine's own "no results" page, which says the same
        // thing in the engine's words. The error runs the safe way round.
        return if text_len(frame) < FRAME_TEXT {
            Outcome::Empty
        } else {
            Outcome::Unrecognized
        };
    }
    // No frame, so this is not a result page. Either the engine declined to serve one or it has
    // redesigned. A 2xx it chose instead of 200 is the measured shape of the first — DuckDuckGo
    // answers its CAPTCHA with 202 — and so is a body too thin to be any readable page.
    if (200..300).contains(&status) && status != 200 {
        return Outcome::Blocked;
    }
    if crate::html::extract(source, base).text_len() < CHALLENGE_TEXT {
        return Outcome::Blocked;
    }
    // Enough text to be a page, just not one this shape describes. The generic extractor reads
    // it, which is right for a redesign and right for an interstitial that explains itself: the
    // reader gets the engine's own words instead of hww's guess about them.
    Outcome::Unrecognized
}

/// The first element matching `sel`. An unparseable selector is a table bug, caught by
/// `every_engine_parses_its_fixture`; at runtime it answers "nothing matched" rather than
/// panicking.
fn first<'a>(html: &'a Html, sel: &str) -> Option<ElementRef<'a>> {
    let sel = Selector::parse(sel).ok()?;
    html.select(&sel).next()
}

/// Length of an element's text with whitespace collapsed, in the same currency as
/// [`crate::ir::Document::text_len`]: one space between words, none at either end.
fn text_len(el: ElementRef<'_>) -> usize {
    let mut n = 0;
    for chunk in el.text() {
        for word in chunk.split_whitespace() {
            if n > 0 {
                n += 1;
            }
            n += word.chars().count();
        }
    }
    n
}

/// Walk one shape over a parsed page.
///
/// A result with no usable destination is dropped rather than rendered as dead text: the whole
/// point of the row is that it can be followed.
pub fn parse(html: &Html, base: &Url, shape: &SearchShape) -> Vec<Entry> {
    let (Ok(result_sel), Ok(title_sel), Ok(href_sel)) = (
        Selector::parse(&shape.result),
        Selector::parse(&shape.title),
        Selector::parse(&shape.href),
    ) else {
        // A shape that does not parse is a table bug, caught by `every_engine_parses_its_fixture`.
        // At runtime the honest answer is "nothing matched", which routes to the generic
        // extractor rather than to a blank page.
        return Vec::new();
    };
    let snippet_sel = shape
        .snippet
        .as_deref()
        .and_then(|s| Selector::parse(s).ok());

    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for result in html.select(&result_sel) {
        let Some(href) = destination(result, &href_sel, base, shape.unwrap_param) else {
            continue;
        };
        let title = text_of(result, &title_sel);
        if title.is_empty() {
            continue;
        }
        // Every engine measured repeats a destination between its main column and its
        // "discussions" or "cluster" strip. One row per destination, first position wins.
        if !seen.insert(href.clone()) {
            continue;
        }
        let summary = snippet_sel
            .as_ref()
            .map(|s| text_of(result, s))
            .filter(|t| !t.is_empty())
            .map(|t| vec![Block::Paragraph(vec![Inline::Text(t)])])
            .unwrap_or_default();
        out.push(Entry {
            title: vec![Inline::Text(title)],
            href: Some(href),
            summary,
            published: None,
            // The engines print a breadcrumb of their own inside the result, which arrives as
            // part of `summary`. See `ir::Entry::address`.
            address: None,
            // Result favicons come from the engine's own image host
            // (`imgs.search.brave.com`, `external-content.duckduckgo.com`). Carrying one would
            // put a third-party image offer on every row of the page.
            image: None,
        });
    }
    out
}

/// The destination of one result: first descendant carrying an `href`, absolutized, unwrapped
/// if the engine wraps it, and accepted only if it is a web URL.
fn destination(
    result: ElementRef<'_>,
    href_sel: &Selector,
    base: &Url,
    unwrap_param: Option<&str>,
) -> Option<String> {
    let raw = result
        .select(href_sel)
        .find_map(|e| e.value().attr("href"))?;
    // Engines serve protocol-relative and root-relative hrefs; `join` settles both.
    let url = base.join(raw).ok()?;
    let url = unwrap_param
        .and_then(|p| unwrap_redirect(&url, p))
        .unwrap_or(url);
    // The same gate a clicked link goes through, rather than a second opinion about schemes.
    match crate::session::classify_link(url.as_str()) {
        crate::session::Target::Navigate(u) => Some(u.into()),
        _ => None,
    }
}

/// Pull the real destination out of an engine's click-tracking redirect.
///
/// Returns `None` when the parameter is absent, unparseable, or names a scheme that is not the
/// web. Falling back to the wrapped URL in those cases is correct: it is still the engine's own
/// host, and still followable.
fn unwrap_redirect(url: &Url, param: &str) -> Option<Url> {
    let inner = url.query_pairs().find(|(k, _)| k == param)?.1;
    let inner = Url::parse(&inner).ok()?;
    matches!(inner.scheme(), "http" | "https").then_some(inner)
}

/// Concatenated text of the first match, whitespace collapsed.
fn text_of(scope: ElementRef<'_>, sel: &Selector) -> String {
    let Some(el) = scope.select(sel).next() else {
        return String::new();
    };
    let mut out = String::new();
    for chunk in el.text() {
        for word in chunk.split_whitespace() {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(word);
        }
    }
    out
}

/// A result page as a document.
///
/// The title is the query, so history, the window title, and the URL bar all name what was
/// asked rather than the engine's own boilerplate.
pub fn document(url: &Url, terms: &str, entries: Vec<Entry>) -> Document {
    Document {
        url: url.to_string(),
        title: Some(terms.to_owned()),
        byline: None,
        published: None,
        site_name: None,
        favicon: None,
        lang: None,
        blocks: vec![Block::Entries(entries)],
    }
}

/// Whether typed input is an address or something to search for.
///
/// One place, so the reader's URL bar and `bin/hww`'s argument cannot drift. The old rule lived
/// in both and was `contains("://")` alone, which sent `rust` to `https://rust/` and refused
/// `rust ownership` outright.
pub fn is_address(typed: &str) -> bool {
    let typed = typed.trim();
    if typed.is_empty() {
        return false;
    }
    // No address contains a space, and almost every query does. This is the cheap half of the
    // decision and it carries most of the traffic, and it comes first: `://` inside a sentence
    // is a sentence ("what does :// mean", a pasted log line), not intent to navigate.
    if typed.split_whitespace().count() > 1 {
        return false;
    }
    // An explicit scheme is explicit intent, whatever follows it.
    if typed.contains("://") {
        return true;
    }
    let Ok(url) = Url::parse(&format!("https://{typed}")) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    // A literal address is unambiguous and has neither a dot nor a path: `[::1]` is not a word
    // anyone searches for, and sending it to an engine would leak an intranet address.
    if matches!(url.host(), Some(url::Host::Ipv6(_))) {
        return true;
    }
    // A single label is a search term far more often than it is a host. `localhost` is the
    // exception people actually type; a bare intranet name is not, and asking for it with a
    // trailing slash still works because that reaches the `://`-free parse above with a path.
    host.contains('.') || host == "localhost" || url.path() != "/"
}

/// Typed input as a URL: itself if it is an address, a search if it is not.
///
/// Empty input is neither, and is refused here rather than in each caller. A blank bar is what
/// `Ctrl+L` opens with when there is no page, so Enter on it is a slip; searching for nothing
/// would send the engine a request that the disclosure cannot even name, because there are no
/// terms in it to report.
pub fn resolve_input(
    typed: &str,
    engine: &crate::sites::SearchEngine,
) -> Result<Url, url::ParseError> {
    let typed = typed.trim();
    if typed.is_empty() {
        return Err(url::ParseError::EmptyHost);
    }
    if !is_address(typed) {
        return Ok(query_url(engine, typed));
    }
    if typed.contains("://") {
        Url::parse(typed)
    } else {
        // A bare host is what people type. Assuming https is the only assumption that does not
        // silently downgrade.
        Url::parse(&format!("https://{typed}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sites::{EngineId, SearchEngine};

    fn engine(id: EngineId) -> &'static SearchEngine {
        crate::sites::engine_by_id(id)
    }

    #[test]
    fn every_engine_parses_its_fixture() {
        for e in crate::sites::ENGINES {
            let base = query_url(e, "lorem ipsum");
            let html = Html::parse_document(e.fixture);
            let entries = parse(&html, &base, &e.shape);
            assert!(
                entries.len() >= 3,
                "{:?}: shape matched {} results in its own fixture",
                e.id,
                entries.len()
            );
            for entry in &entries {
                let href = entry.href.as_deref().unwrap_or_default();
                assert!(
                    href.starts_with("http://") || href.starts_with("https://"),
                    "{:?}: entry href is not a web URL: {href:?}",
                    e.id
                );
                assert!(
                    !crate::ir::plain_text(&entry.title).is_empty(),
                    "{:?}: entry has no title",
                    e.id
                );
                // A destination still on the engine's own site is a wrapper this shape did
                // not unwrap, and the common case is a sponsored row sitting in the result
                // list with its anchor pointing at the ad tracker. Compared at a label
                // boundary in both directions: the endpoint that serves the results and the
                // host that collects the click are siblings, not the same name.
                let host = Url::parse(href)
                    .ok()
                    .and_then(|u| u.host_str().map(str::to_owned))
                    .unwrap_or_default();
                assert!(
                    !same_site(&host, e.host),
                    "{:?}: entry still points at the engine: {href:?}",
                    e.id
                );
            }
        }
    }

    #[test]
    fn a_fixture_with_a_snippet_produces_one() {
        for e in crate::sites::ENGINES {
            if e.shape.snippet.is_none() {
                continue;
            }
            let base = query_url(e, "lorem");
            let entries = parse(&Html::parse_document(e.fixture), &base, &e.shape);
            assert!(
                entries.iter().any(|x| !x.summary.is_empty()),
                "{:?}: declares a snippet selector, matched none in its fixture",
                e.id
            );
        }
    }

    /// Brave's class names carry a build hash that changes on every deploy. A shape that names
    /// one works until their next release and then silently stops.
    #[test]
    fn no_shape_depends_on_a_build_hash() {
        for e in crate::sites::ENGINES {
            let shape = &e.shape;
            let all = [
                Some(&shape.frame),
                Some(&shape.result),
                Some(&shape.title),
                Some(&shape.href),
                shape.snippet.as_ref(),
            ];
            for sel in all.into_iter().flatten() {
                assert!(
                    !sel.contains("svelte-"),
                    "{:?}: selector names a build hash: {sel}",
                    e.id
                );
            }
        }
    }

    /// Every fixture must carry the engine's frame, or the "answered and found nothing" path
    /// is never exercised against real markup and the selector could be wrong for years.
    #[test]
    fn every_fixture_carries_its_frame() {
        for e in crate::sites::ENGINES {
            let html = Html::parse_document(e.fixture);
            assert!(
                first(&html, &e.shape.frame).is_some(),
                "{:?}: fixture is missing the frame {}",
                e.id,
                e.shape.frame
            );
        }
    }

    /// A frame the shape found no rows in, holding a page's worth of text, is a redesign.
    ///
    /// The frames are deliberately broad and outlive the markup inside them, so the likely
    /// staleness mode is exactly this: the engine keeps its container and renames the result
    /// rows. Reporting that as "found nothing" would blame the reader's words for the engine's
    /// deploy, and there is no Retry that would ever clear it.
    #[test]
    fn a_full_frame_with_no_rows_is_a_redesign_not_an_empty_answer() {
        for e in crate::sites::ENGINES {
            let base = query_url(e, "lorem");
            let prose = "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod \
                 tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, \
                 quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo.";
            let renamed = format!(
                "<html><body>{}</body></html>",
                frame_only_markup(&e.shape.frame).replace(
                    "></div>",
                    &format!("><div class=\"renamed-in-a-later-deploy\">{prose}</div></div>")
                )
            );
            assert!(
                matches!(read(&renamed, &base, e, 200), Outcome::Unrecognized),
                "{:?}: a full frame with no parsable rows did not fall through",
                e.id
            );
        }
    }

    /// The frame must be the page's, not the results': an engine that found nothing still    /// The frame must be the page's, not the results': an engine that found nothing still
    /// renders it. A frame that only exists when results do would report every empty search as
    /// a block.
    #[test]
    fn an_empty_frame_reads_as_empty_not_blocked() {
        for e in crate::sites::ENGINES {
            let base = query_url(e, "zxqwvbnmlkjhgfdsa");
            // The fixture's frame element alone, with none of the results inside it.
            let bare = format!(
                "<html><body>{}</body></html>",
                frame_only_markup(&e.shape.frame)
            );
            assert!(
                matches!(read(&bare, &base, e, 200), Outcome::Empty),
                "{:?}: its own frame with no results did not read as empty",
                e.id
            );
        }
    }

    /// Whether two hosts belong to the same site, at a label boundary.
    fn same_site(a: &str, b: &str) -> bool {
        a == b || a.ends_with(&format!(".{b}")) || b.ends_with(&format!(".{a}"))
    }

    /// The smallest element matching a `#id` or `.class` frame selector.
    fn frame_only_markup(sel: &str) -> String {
        match sel.strip_prefix('#') {
            Some(id) => format!("<div id=\"{id}\"></div>"),
            None => {
                let class = sel.trim_start_matches('.');
                format!("<div class=\"{class}\"></div>")
            }
        }
    }

    /// An interstitial that carries no frame and almost no text is the engine declining, not a
    /// search that found nothing. Measured on DuckDuckGo's 202 CAPTCHA, which carries 123
    /// characters. Marginalia's countdown is *not* an instance: it extracts to 214 against a
    /// `THIN_TEXT` of 200, so it falls through to the extractor and the reader sees the
    /// engine's own page. The body below is thin on purpose, and stands for the first only.
    #[test]
    fn a_frameless_thin_page_is_a_block() {
        for e in crate::sites::ENGINES {
            let base = query_url(e, "lorem");
            let thin = "<html><body><p>Please wait a moment.</p></body></html>";
            assert!(
                matches!(read(thin, &base, e, 200), Outcome::Blocked),
                "{:?}: a thin frameless page did not read as a block",
                e.id
            );
        }
    }

    #[test]
    fn query_url_encodes_once() {
        let e = engine(EngineId::Mojeek);
        let u = query_url(e, "rust & c++ 100%");
        assert_eq!(u.query_pairs().count(), 1);
        assert_eq!(
            u.query_pairs().next().unwrap().1.into_owned(),
            "rust & c++ 100%"
        );
    }

    /// A `&` in the terms must not forge a second parameter.
    #[test]
    fn terms_cannot_forge_a_parameter() {
        let e = engine(EngineId::Mojeek);
        let u = query_url(e, "cats&safe=off");
        assert_eq!(u.query_pairs().count(), 1);
        assert_eq!(terms_of(e, &u).as_deref(), Some("cats&safe=off"));
    }

    #[test]
    fn address_or_query() {
        let cases = [
            ("rust ownership", false),
            ("what is the borrow checker", false),
            ("example.com", true),
            ("a.example.com/newest", true),
            ("https://example.com", true),
            ("http://x", true),
            ("localhost", true),
            ("localhost:8080", true),
            ("192.168.1.1", true),
            // A literal address has neither a dot nor a path, and must not be sent to an engine.
            ("[::1]", true),
            ("[::1]:8080", true),
            // A scheme inside a sentence is a sentence.
            ("what does :// mean", false),
            // The wart this replaces: a single label used to become https://rust/.
            ("rust", false),
            ("C++", false),
            ("", false),
        ];
        for (input, want) in cases {
            assert_eq!(is_address(input), want, "is_address({input:?})");
        }
    }

    /// Enter on an empty URL bar must not become a request.
    ///
    /// `Ctrl+L` opens the bar empty when there is no page, so this is one keystroke away at
    /// launch. It used to reach `Url::parse("https://")`; a search for nothing would reach the
    /// engine with a query the disclosure cannot name, which is a request that leaves silently.
    #[test]
    fn empty_input_is_not_a_request() {
        let e = engine(EngineId::default());
        for typed in ["", "   ", "\t\n"] {
            assert!(
                resolve_input(typed, e).is_err(),
                "resolve_input({typed:?}) built a URL"
            );
        }
    }

    #[test]
    fn resolve_input_searches_or_navigates() {
        let e = engine(EngineId::Mojeek);
        let searched = resolve_input("rust ownership", e).unwrap();
        assert_eq!(searched.host_str(), Some(e.host));
        assert_eq!(terms_of(e, &searched).as_deref(), Some("rust ownership"));

        let navigated = resolve_input("example.com", e).unwrap();
        assert_eq!(navigated.as_str(), "https://example.com/");
    }

    #[test]
    fn unwrap_takes_only_web_urls() {
        let wrapped = |v: &str| {
            let mut u = Url::parse("https://engine.example/l/").unwrap();
            u.query_pairs_mut().append_pair("uddg", v);
            u
        };
        assert_eq!(
            unwrap_redirect(&wrapped("https://example.com/a"), "uddg")
                .map(String::from)
                .as_deref(),
            Some("https://example.com/a")
        );
        assert_eq!(
            unwrap_redirect(&wrapped("javascript:alert(1)"), "uddg"),
            None
        );
        assert_eq!(unwrap_redirect(&wrapped("data:text/html,x"), "uddg"), None);
        assert_eq!(unwrap_redirect(&wrapped("not a url"), "uddg"), None);
        assert_eq!(
            unwrap_redirect(&Url::parse("https://x.test/l/").unwrap(), "uddg"),
            None
        );
    }

    /// The engine that wraps its links must not leave one wrapped.
    #[test]
    fn duckduckgo_results_point_at_the_site() {
        let e = engine(EngineId::DuckDuckGo);
        let base = query_url(e, "lorem");
        let entries = parse(&Html::parse_document(e.fixture), &base, &e.shape);
        assert!(!entries.is_empty());
        for entry in &entries {
            let href = entry.href.as_deref().unwrap();
            assert!(!href.contains("uddg="), "left wrapped: {href}");
        }
    }

    #[test]
    fn a_repeated_destination_appears_once() {
        let shape = SearchShape {
            frame: "ul.r".into(),
            result: "ul.r > li".into(),
            title: "a".into(),
            href: "a".into(),
            snippet: None,
            unwrap_param: None,
        };
        let doc = "<html><body><ul class=\"r\">\
            <li><a href=\"https://a.test/1\">One</a></li>\
            <li><a href=\"https://a.test/1\">One again</a></li>\
            <li><a href=\"https://a.test/2\">Two</a></li>\
            </ul></body></html>";
        let base = Url::parse("https://e.test/search?q=x").unwrap();
        let entries = parse(&Html::parse_document(doc), &base, &shape);
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn a_result_without_a_usable_link_is_dropped() {
        let shape = SearchShape {
            frame: "ul.r".into(),
            result: "ul.r > li".into(),
            title: "a".into(),
            href: "a".into(),
            snippet: None,
            unwrap_param: None,
        };
        let doc = "<html><body><ul class=\"r\">\
            <li><a href=\"javascript:void(0)\">Script</a></li>\
            <li><a>No href at all</a></li>\
            <li><a href=\"https://a.test/ok\">Fine</a></li>\
            </ul></body></html>";
        let base = Url::parse("https://e.test/search?q=x").unwrap();
        let entries = parse(&Html::parse_document(doc), &base, &shape);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].href.as_deref(), Some("https://a.test/ok"));
    }

    /// The measured DuckDuckGo block: HTTP 202, a CAPTCHA, almost no text.
    #[test]
    fn a_thin_two_oh_two_is_a_block() {
        let e = engine(EngineId::DuckDuckGo);
        let base = query_url(e, "lorem");
        let challenge = "<html><body><h1>DuckDuckGo</h1>\
            <p>Unfortunately, bots use DuckDuckGo too.</p>\
            <p>Please complete the following challenge.</p></body></html>";
        assert!(matches!(read(challenge, &base, e, 202), Outcome::Blocked));
    }

    /// An engine that answers normally and finds nothing must not report as blocked: the
    /// remedy for "try another engine" is not the remedy for "try other words".
    #[test]
    fn an_empty_two_hundred_is_not_a_block() {
        let e = engine(EngineId::Mojeek);
        let base = query_url(e, "asdfghjkl");
        let empty = "<html><body><div class=\"container serp-results\">\
            <ul class=\"results-standard\"></ul>\
            <p>No results found.</p></div></body></html>";
        assert!(matches!(read(empty, &base, e, 200), Outcome::Empty));
    }

    /// A page the shape does not describe falls back rather than reporting a block or an
    /// empty result.
    #[test]
    fn an_unrecognized_page_falls_back() {
        let e = engine(EngineId::Mojeek);
        let base = query_url(e, "lorem");
        let redesigned = &format!(
            "<html><body><main><h1>Results</h1><div class=\"brand-new-markup\">{}</div>\
             </main></body></html>",
            // Enough prose to be a readable page: a redesign is not an interstitial.
            "<p>A paragraph of result-like prose, long enough to clear the thin-document \
             floor, repeated so the page reads as content rather than as a challenge.</p>"
                .repeat(4)
        );
        assert!(matches!(
            read(redesigned, &base, e, 200),
            Outcome::Unrecognized
        ));
    }

    /// A challenge served as a plain 200 is still a challenge. Marginalia's rate limiter
    /// answers 200 with a one-second wait page, so status alone cannot carry this; the frame
    /// is what says the engine never ran the search.
    #[test]
    fn a_two_hundred_challenge_is_still_a_block() {
        let e = engine(EngineId::Mojeek);
        let base = query_url(e, "lorem");
        let challenge = "<html><body><p>Verifying your browser…</p></body></html>";
        assert!(matches!(read(challenge, &base, e, 200), Outcome::Blocked));
    }

    #[test]
    fn document_is_titled_by_the_query() {
        let e = engine(EngineId::Mojeek);
        let url = query_url(e, "rust ownership");
        let entries = parse(&Html::parse_document(e.fixture), &url, &e.shape);
        let doc = document(&url, "rust ownership", entries);
        assert_eq!(doc.title.as_deref(), Some("rust ownership"));
        assert!(matches!(doc.blocks.as_slice(), [Block::Entries(_)]));
    }

    /// A result row must never carry an image: the engines' favicons come from their own image
    /// hosts, and an `Entry::image` is an offer to fetch one per row.
    #[test]
    fn results_carry_no_images() {
        for e in crate::sites::ENGINES {
            let base = query_url(e, "lorem");
            for entry in parse(&Html::parse_document(e.fixture), &base, &e.shape) {
                assert!(entry.image.is_none(), "{:?}: result carries an image", e.id);
            }
        }
    }
}
