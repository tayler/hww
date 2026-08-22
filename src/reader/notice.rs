//! What the reader has to say about a page, and how loud. **No egui types, no colours.**
//!
//! `src/ir.rs` keeps the page's *styling* out of the document. This module is the same fence
//! one layer out: it keeps the reader's own *wording* out of `ui/`, so the text hww puts on
//! screen is decided where `cargo test` can read it. That matters more than it looks. Two strings in the old
//! error screen contained `→`, which egui's embedded fonts do not carry, and both had been
//! rendering as tofu since the day they were written; AGENTS.md stated the rule in prose the
//! whole time. [`tests::no_notice_contains_an_arrow`] is that rule in a form that fails a
//! build.
//!
//! # Severity, not colour
//!
//! [`Severity`] is how loud a notice is. `theme::notice_ink` turns a severity into paint, and
//! it lives under `ui/` because that is the only file in the crate where a `Color32` appears.
//! Splitting them is what lets the words be tested without a window and the palette be retuned
//! without touching a call site.
//!
//! # Two kinds, because there are two situations
//!
//! Notices divide on whether there is a page to read. A `LoadError` means there is not, so the reader
//! owns the viewport and gets a heading, prose, and actions. A 4xx that still carried an
//! article, or an extraction that came back nearly empty, means the page *is* there and the notice
//! is only annotating it; dressing that as an error screen would be a lie, because the prose is
//! readable immediately below. [`about_page`] returns the second kind and is empty for a page
//! that arrived clean, which is most of them.
//!
//! # The mark
//!
//! [`MARK`] is the bracket convention `session::Notice` (`[rewrote a -> b]`) and
//! `images::placeholder` (`[image] …`) already used, made explicit and given an owner. A
//! bracket survives a screenshot, a colour-blind reader, and a theme nobody has designed yet.
//! Its inline cousins stay inline on purpose: `[image]` and `(N replies hidden)` mark one thing
//! at one place in the document, and a full-bleed band cannot point at a place.
//!
//! # Why this is not under `ui/`
//!
//! Two reasons. It is decidable without a `Context`, so the fast CI job that never compiles
//! egui tests it. And `main.rs` prints a bare `LoadError` today while the GUI has considered
//! wording for the same eight cases; [`failure`] is compiled for the CLI too, whenever that is
//! wired up.

use crate::fetch::Truncation;
use crate::session::{LoadError, Provenance};
use std::time::Duration;
use url::Url;

/// The prefix every notice carries.
pub const MARK: &str = "[hww]";

/// The extracted-text length below which a page is treated as having failed to extract.
///
/// Re-exported rather than restated: this *is* `html::extract`'s `<body>` fallback threshold,
/// and a page that took the fallback is exactly the page worth warning about. A copy of the
/// number here would let the two drift silently, with the reader quietly ceasing to warn.
pub use crate::ir::THIN_TEXT;

/// How loud a notice is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// An aside, or a page that has not arrived yet. Nothing is wrong.
    Quiet,
    /// Something about the page the reader would otherwise mis-read.
    Caution,
    /// There is no page. The notice owns the whole viewport.
    Failure,
}

impl Severity {
    /// Whether this severity claims the viewport rather than sitting above a readable page.
    ///
    /// A `Caution` must not: there is real prose under it, and covering that would make the
    /// remark worse than the thing it is remarking on.
    pub fn fills_the_viewport(self) -> bool {
        matches!(self, Severity::Quiet | Severity::Failure)
    }

    /// The severity, as a word, drawn beside [`MARK`].
    ///
    /// Colour is the obvious way to say how loud a notice is and the one that fails quietest: a
    /// hue can be missed, screenshotted into greyscale, or read by an eye that does not separate
    /// rust from brown. The variants are already named, so the cheapest fix is to print the
    /// name. It costs one token on every band, including the ones about pages that are basically
    /// fine, and that is the trade; a page that is actually fine emits no band at all.
    ///
    /// Wording, so it lives here rather than in `ui/` for the same reason every other notice
    /// string does: the fast CI job never compiles egui, and this is where a test can read it.
    pub fn word(self) -> &'static str {
        match self {
            Severity::Quiet => "quiet",
            Severity::Caution => "caution",
            Severity::Failure => "failure",
        }
    }
}

/// What a failure offers to do about itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Retry,
    RetryWithoutRewrite,
    CopyUrl,
}

impl Button {
    pub fn label(self) -> &'static str {
        match self {
            Button::Retry => "Retry",
            Button::RetryWithoutRewrite => "Retry without rewrite",
            Button::CopyUrl => "Copy URL",
        }
    }
}

/// One thing the reader has to say, with no idea how it will be drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub severity: Severity,
    /// Failures only. A caution is one line and a heading would inflate it into a screen.
    pub heading: Option<&'static str>,
    pub body: String,
    /// Set smaller and dimmer under the body: the URL, the redirect chain.
    pub detail: Vec<String>,
    pub actions: &'static [Button],
}

impl Notice {
    fn new(severity: Severity, body: impl Into<String>) -> Self {
        Self {
            severity,
            heading: None,
            body: body.into(),
            detail: Vec::new(),
            actions: &[],
        }
    }
}

/// Everything worth saying *about* a page that loaded, loudest first.
///
/// Empty for a page that arrived clean, so the column costs nothing on the pages that are fine.
/// Takes no fragment argument: an unresolved `#fragment` is a navigation event, not a fact
/// about the page's content, and it is reported once in the status strip.
///
/// The first two are here rather than in the status strip because neither is accounting. A
/// dead rule means the article on screen may not be the article asked for, and a truncated
/// body means it stops early with nothing to say so: both are things the reader would
/// otherwise mis-read, which is what [`Severity::Caution`] is for. They were a dim fragment at
/// the right-hand end of a truncated strip row and a twelve-second flash respectively, so the
/// first was often not drawn at all and the second could not be recalled once it faded.
pub fn about_page(prov: &Provenance, text_len: usize) -> Vec<Notice> {
    let mut out = Vec::new();
    if prov.rule_appears_dead {
        // Not an error: the fetch succeeded. But it succeeded somewhere else, and no rewrite
        // is ever retried, so this is the whole early-warning system.
        let aimed = prov
            .rewritten_to
            .as_ref()
            .and_then(Url::host_str)
            .unwrap_or("the rewritten host");
        let landed = prov.final_url.host_str().unwrap_or("somewhere else");
        let mut n = Notice::new(
            Severity::Caution,
            format!(
                "A rewrite rule sent this to {aimed}, which redirected to {landed}. \
                 The rule appears dead, and this may not be the page you asked for."
            ),
        );
        n.detail = vec![format!("asked for {}", prov.requested)];
        out.push(n);
    }
    if let Some(body) = truncation_body(prov.truncation) {
        out.push(Notice::new(Severity::Caution, body));
    }
    if prov.status >= 400 {
        // Many 404s are readable and many paywalls answer 403 with the article still in the
        // markup, so this is a remark *over* the content, never instead of it.
        out.push(Notice::new(
            Severity::Caution,
            format!(
                "The server answered {}. Showing what it sent anyway.",
                prov.status
            ),
        ));
    }
    if text_len < THIN_TEXT {
        out.push(Notice::new(
            Severity::Caution,
            "Little content extracted. This page may require JavaScript.",
        ));
    }
    out
}

/// The band prose for a body that did not arrive whole.
///
/// One string per variant, hedging exactly where the evidence hedges: `len >= cap` alone
/// cannot tell a body that was cut from one that happened to end on the boundary, so
/// `MaybeAtCap` says "may" and `AtCap` does not. The terser phrasing for the page-info panel
/// is [`crate::reader::pageinfo`]'s; this one is the alarm, that one is the record.
fn truncation_body(t: Truncation) -> Option<String> {
    Some(match t {
        Truncation::Complete => return None,
        Truncation::AtCap(n) => format!(
            "Stopped at the {} MB body cap. The rest of this page was not fetched.",
            n / 1_000_000
        ),
        Truncation::MaybeAtCap(n) => format!(
            "The body reached the {} MB cap and may be cut short.",
            n / 1_000_000
        ),
        Truncation::Timeout(d) => format!(
            "The body was still arriving after {}s. This article is incomplete.",
            d.as_secs()
        ),
        Truncation::Incomplete(_) => {
            "The connection dropped partway. This article is incomplete.".to_owned()
        }
    })
}

/// The screen a failed navigation gets.
///
/// Each error gets its own words and its own actions: a wrong-but-confident diagnosis is the
/// worst kind, because it blames the site.
pub fn failure(error: &LoadError, url: &Url) -> Notice {
    use crate::fetch::FetchError;

    let (heading, body, actions): (&'static str, String, &'static [Button]) = match error {
        LoadError::Fetch(FetchError::LikelyBlocked) => (
            "This site did not answer.",
            // Phase 0 measured bot detection failing as a silent hang rather than a 403.
            "The request timed out with no reply. On a document fetch that is usually bot \
             detection failing silently rather than a network fault."
                .to_owned(),
            &[Button::Retry, Button::RetryWithoutRewrite, Button::CopyUrl],
        ),
        LoadError::Fetch(FetchError::Timeout(d)) => (
            "Timed out.",
            format!(
                "No reply within {}s. That is bandwidth rather than a block.",
                d.as_secs()
            ),
            // No retry-without-rewrite: bandwidth is not a rewrite rule's fault.
            &[Button::Retry, Button::CopyUrl],
        ),
        LoadError::Fetch(FetchError::SchemeDowngrade(to)) => (
            "Refused an https -> http downgrade.",
            format!(
                "A redirect tried to send this request to {} without encryption. \
                 hww does not follow that.",
                to.host_str().unwrap_or("an unnamed host")
            ),
            // No retry: refusing is correct, and a bypass would be a policy hole.
            &[Button::CopyUrl],
        ),
        LoadError::Fetch(FetchError::TooManyRedirects(_)) => (
            "Too many redirects.",
            "The request bounced past the redirect limit. A loop like this is usually a \
             consent wall."
                .to_owned(),
            &[Button::Retry, Button::RetryWithoutRewrite, Button::CopyUrl],
        ),
        LoadError::Fetch(FetchError::RedirectWithoutLocation(status)) => (
            "The server sent a broken redirect.",
            format!(
                "It answered {status}, which means \"go somewhere else\", but did not say \
                 where. There is nothing to follow."
            ),
            &[Button::Retry, Button::CopyUrl],
        ),
        LoadError::Fetch(FetchError::BadLocation(to)) => (
            "The server redirected somewhere unusable.",
            format!("It pointed at {to}, which is not an address hww can fetch."),
            &[Button::Retry, Button::CopyUrl],
        ),
        LoadError::NotWebPage { content_type } => (
            "Not a web page.",
            format!("The server sent {content_type}, which hww has no reader for."),
            &[Button::CopyUrl],
        ),
        other => (
            "The request failed.",
            other.to_string(),
            &[Button::Retry, Button::CopyUrl],
        ),
    };

    let mut detail = vec![url.to_string()];
    if let LoadError::Fetch(FetchError::TooManyRedirects(hops)) = error {
        // A redirect loop is usually a consent wall, and the chain is what shows it.
        // `->`, never `→`: see the module doc.
        detail.extend(hops.iter().map(|h| format!("  -> {h}")));
    }

    Notice {
        severity: Severity::Failure,
        heading: Some(heading),
        body,
        detail,
        actions,
    }
}

/// The splash, when nothing has been asked for.
pub fn idle() -> Notice {
    Notice::new(
        Severity::Quiet,
        "A reading client for the quiet web. Ctrl+L for a URL, ? for keys.",
    )
}

/// A navigation in flight.
///
/// `elapsed` is shown because a 15 s bot-block hang and a slow CDN look identical for the
/// first ten seconds, and a number that keeps moving is the difference between waiting and
/// wondering whether it is stuck.
pub fn loading(url: &Url, elapsed: Duration) -> Notice {
    let mut notice = Notice::new(Severity::Quiet, format!("Loading {}…", host_and_path(url)));
    notice.detail = vec![format!("{:.1}s", elapsed.as_secs_f32())];
    notice
}

/// `example.com/a/b`, or bare `example.com` for a root path.
///
/// A wording decision, so it lives with the other wording decisions rather than in `ui/`.
pub fn host_and_path(url: &Url) -> String {
    let host = url.host_str().unwrap_or(url.scheme());
    let path = url.path();
    if path == "/" {
        host.to_owned()
    } else {
        format!("{host}{path}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch::FetchError;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    /// The shape `session` builds when a rewrite lands somewhere the rule did not aim at.
    fn dead_rule() -> Provenance {
        let mut p = Provenance::fixture("https://example.com/a");
        p.rewritten_to = Some(url("https://alt.example.net/a"));
        p.rule_appears_dead = true;
        p.final_url = url("https://www.example.org/a");
        p
    }

    /// Every way a body can fail to arrive whole. `Complete` is deliberately absent: it is the
    /// one variant that must produce nothing, and it is asserted separately.
    fn every_truncation() -> Vec<Truncation> {
        vec![
            Truncation::AtCap(5_000_000),
            Truncation::MaybeAtCap(5_000_000),
            Truncation::Timeout(Duration::from_secs(15)),
            Truncation::Incomplete(812_345),
        ]
    }

    /// A clean 200 for `about_page` to vary one field of at a time.
    ///
    /// Shared with `session`'s own tests via `Provenance::fixture` rather than restated: an
    /// eleven-field literal copied per module is one the fetcher can outgrow silently.
    fn page(status: u16) -> Provenance {
        let mut p = Provenance::fixture("https://example.com/a");
        p.status = status;
        p
    }

    /// Every failure that can be built without a live `reqwest::Error`.
    ///
    /// `FetchError::Transport` wraps one and cannot be constructed here, so the fallback arm
    /// is reached through `Io` instead.
    fn every_failure() -> Vec<Notice> {
        let u = url("https://example.com/a");
        [
            LoadError::Fetch(FetchError::LikelyBlocked),
            LoadError::Fetch(FetchError::Timeout(Duration::from_secs(15))),
            LoadError::Fetch(FetchError::SchemeDowngrade(url("http://example.net/x"))),
            LoadError::Fetch(FetchError::TooManyRedirects(vec![
                url("https://a.example/1"),
                url("https://b.example/2"),
            ])),
            LoadError::Fetch(FetchError::RedirectWithoutLocation(302)),
            LoadError::Fetch(FetchError::BadLocation("::not a url::".to_owned())),
            LoadError::Fetch(FetchError::Io(std::io::Error::other("boom"))),
            LoadError::NotWebPage {
                content_type: "application/pdf".to_owned(),
            },
        ]
        .iter()
        .map(|e| failure(e, &u))
        .collect()
    }

    fn every_notice() -> Vec<Notice> {
        let mut all = every_failure();
        all.push(idle());
        all.push(loading(
            &url("https://example.com/"),
            Duration::from_secs(3),
        ));
        all.extend(about_page(&page(404), 10));
        all.extend(about_page(&page(503), 9_000));
        all.extend(about_page(&dead_rule(), 9_000));
        for t in every_truncation() {
            let mut p = page(200);
            p.truncation = t;
            all.extend(about_page(&p, 9_000));
        }
        all
    }

    /// The executable form of the embedded-font gap.
    ///
    /// egui's `default_fonts` carry no `→`, so one renders as tofu. AGENTS.md said so in prose
    /// and two strings shipped with it anyway, including a heading nobody could read. Prose
    /// does not fail a build.
    #[test]
    fn no_notice_contains_an_arrow() {
        for notice in every_notice() {
            let mut text = vec![notice.body.clone()];
            text.extend(notice.heading.map(str::to_owned));
            text.extend(notice.detail.clone());
            text.extend(notice.actions.iter().map(|b| b.label().to_owned()));
            // The severity word is drawn on every band, so it is swept with the rest.
            text.push(notice.severity.word().to_owned());
            for t in text {
                assert!(!t.contains('→'), "unrenderable arrow in {t:?}");
            }
        }
    }

    /// Three severities, three words, and none of them a prefix of another.
    ///
    /// The word is there so loudness survives greyscale and a screenshot; two severities sharing
    /// a word, or one reading as an abbreviation of the next, would put that back where it was.
    #[test]
    fn each_severity_says_something_different() {
        let words = [Severity::Quiet, Severity::Caution, Severity::Failure].map(Severity::word);
        for (i, a) in words.iter().enumerate() {
            assert!(!a.is_empty());
            assert_eq!(*a, a.to_lowercase(), "{a:?} is not set like the rest");
            for b in &words[i + 1..] {
                assert_ne!(a, b);
                assert!(!a.starts_with(b) && !b.starts_with(a), "{a:?} and {b:?}");
            }
        }
    }

    /// Refusing the downgrade is correct, and a bypass would be a policy hole. That was a
    /// comment inside a `Ui` closure, which is to say it was not checked by anything.
    #[test]
    fn a_scheme_downgrade_is_never_retryable() {
        let notice = failure(
            &LoadError::Fetch(FetchError::SchemeDowngrade(url("http://example.net/"))),
            &url("https://example.net/"),
        );
        assert_eq!(notice.actions, &[Button::CopyUrl]);
    }

    /// A rewrite rule can be the reason a request is blocked or looping, so those two offer a
    /// way round it. A timeout cannot be, so it does not.
    #[test]
    fn only_failures_a_rewrite_could_cause_offer_retry_without_rewrite() {
        let u = url("https://example.com/");
        let offers = |e: LoadError| {
            failure(&e, &u)
                .actions
                .contains(&Button::RetryWithoutRewrite)
        };
        assert!(offers(LoadError::Fetch(FetchError::LikelyBlocked)));
        assert!(offers(LoadError::Fetch(FetchError::TooManyRedirects(
            vec![]
        ))));
        assert!(!offers(LoadError::Fetch(FetchError::Timeout(
            Duration::from_secs(15)
        ))));
    }

    #[test]
    fn every_failure_gets_its_own_words() {
        let notices = every_failure();
        let mut headings: Vec<&str> = notices.iter().filter_map(|n| n.heading).collect();
        assert_eq!(headings.len(), notices.len(), "a failure with no heading");
        headings.sort_unstable();
        let before = headings.len();
        headings.dedup();
        assert_eq!(before, headings.len(), "two failures share a heading");
        for n in &notices {
            assert!(!n.body.trim().is_empty());
            assert_eq!(n.severity, Severity::Failure);
            // The URL is always the first detail line, so "which page was that?" is answered
            // on every failure screen without going back to the strip.
            assert_eq!(
                n.detail.first().map(String::as_str),
                Some("https://example.com/a")
            );
        }
    }

    #[test]
    fn a_redirect_loop_lists_its_hops_in_order() {
        let notice = failure(
            &LoadError::Fetch(FetchError::TooManyRedirects(vec![
                url("https://a.example/1"),
                url("https://b.example/2"),
            ])),
            &url("https://start.example/"),
        );
        assert!(notice.detail[1].contains("a.example/1"));
        assert!(notice.detail[2].contains("b.example/2"));
    }

    /// The column is free on the pages that are fine, which is nearly all of them.
    #[test]
    fn a_healthy_page_says_nothing() {
        assert!(about_page(&page(200), 5_000).is_empty());
    }

    /// A readable 404 is not a failure. Many paywalls answer 403 with the article intact, and
    /// replacing that with an error screen would throw away a page the reader can read.
    #[test]
    fn a_readable_404_is_a_caution_not_a_failure() {
        let notices = about_page(&page(404), 5_000);
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].severity, Severity::Caution);
        assert!(
            notices[0].heading.is_none(),
            "a caution is one line, not a screen"
        );
        assert!(notices[0].actions.is_empty());
        assert!(
            !notices[0].severity.fills_the_viewport(),
            "it would cover the article"
        );
    }

    /// The threshold is `ir::THIN_TEXT`, which `html::extract` reads too, so this pins the
    /// boundary behaviour rather than the number: whatever the extractor falls back at, that is
    /// where the warning starts.
    #[test]
    fn the_thin_extraction_threshold_matches_the_extractor() {
        assert_eq!(about_page(&page(200), THIN_TEXT).len(), 0);
        assert_eq!(about_page(&page(200), THIN_TEXT - 1).len(), 1);
    }

    /// Loudest first: the remark most likely to change how the page is read sits nearest the
    /// top, where it is read before the prose it is about.
    #[test]
    fn a_page_that_is_both_broken_and_empty_leads_with_the_status() {
        let notices = about_page(&page(500), 10);
        assert_eq!(notices.len(), 2);
        assert!(notices[0].body.contains("500"));
        assert!(notices[1].body.contains("JavaScript"));
    }

    /// The early warning that stands in for a retry, in the one place it cannot expire.
    ///
    /// It used to be a twelve-second `flash` in the status strip. A reader who looked away had
    /// no way to learn that the page in front of them came from a host the rule did not aim
    /// at, and no way to ask.
    #[test]
    fn a_dead_rule_is_a_band_naming_both_hosts() {
        let notices = about_page(&dead_rule(), 9_000);
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].severity, Severity::Caution);
        assert!(
            !notices[0].severity.fills_the_viewport(),
            "the page under it is readable"
        );
        assert!(
            notices[0].body.contains("alt.example.net"),
            "the host aimed at"
        );
        assert!(
            notices[0].body.contains("www.example.org"),
            "the host that answered"
        );
        assert!(
            notices[0].detail[0].contains("example.com/a"),
            "what was asked for"
        );
    }

    /// A truncated body ends mid-article with nothing on screen to say so, which is the exact
    /// definition of something the reader would otherwise mis-read.
    #[test]
    fn every_truncation_is_a_band_and_a_whole_body_is_not() {
        let mut clean = page(200);
        clean.truncation = Truncation::Complete;
        assert!(about_page(&clean, 9_000).is_empty());

        for t in every_truncation() {
            let mut p = page(200);
            p.truncation = t;
            let notices = about_page(&p, 9_000);
            assert_eq!(notices.len(), 1, "{t:?} produced {} bands", notices.len());
            assert_eq!(notices[0].severity, Severity::Caution);
            assert!(!notices[0].body.is_empty());
        }
    }

    /// `len >= cap` cannot tell a body that was cut from one that ended on the boundary, so the
    /// two caps must not claim the same certainty. Losing this distinction is how a reader comes
    /// to distrust the warning on the pages where it is right.
    #[test]
    fn the_two_caps_hedge_differently() {
        let mut certain = page(200);
        certain.truncation = Truncation::AtCap(5_000_000);
        let mut hedged = page(200);
        hedged.truncation = Truncation::MaybeAtCap(5_000_000);

        assert!(!about_page(&certain, 9_000)[0].body.contains("may"));
        assert!(about_page(&hedged, 9_000)[0].body.contains("may"));
    }

    /// Loudest first, across all four. A dead rule outranks everything because it is the only
    /// one that says the article may be the wrong article.
    #[test]
    fn the_bands_are_ordered_loudest_first() {
        let mut p = dead_rule();
        p.status = 500;
        p.truncation = Truncation::Incomplete(812_345);
        let notices = about_page(&p, 10);
        assert_eq!(notices.len(), 4);
        assert!(notices[0].body.contains("rule appears dead"));
        assert!(notices[1].body.contains("incomplete"));
        assert!(notices[2].body.contains("500"));
        assert!(notices[3].body.contains("JavaScript"));
    }

    /// Moved out of `ui/app.rs`, where it had no test.
    #[test]
    fn host_and_path_drops_a_bare_slash() {
        assert_eq!(host_and_path(&url("https://example.com/")), "example.com");
        assert_eq!(
            host_and_path(&url("https://example.com/a/b?q=1")),
            "example.com/a/b"
        );
    }

    #[test]
    fn the_mark_matches_the_conventions_it_generalises() {
        assert!(MARK.starts_with('['));
        assert!(MARK.ends_with(']'));
        assert!(!MARK.contains(char::is_whitespace));
    }
}
