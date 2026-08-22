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

use crate::session::LoadError;
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
pub fn about_page(status: u16, text_len: usize) -> Vec<Notice> {
    let mut out = Vec::new();
    if status >= 400 {
        // Many 404s are readable and many paywalls answer 403 with the article still in the
        // markup, so this is a remark *over* the content, never instead of it.
        out.push(Notice::new(
            Severity::Caution,
            format!("The server answered {status}. Showing what it sent anyway."),
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
        all.extend(about_page(404, 10));
        all.extend(about_page(503, 9_000));
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
            for t in text {
                assert!(!t.contains('→'), "unrenderable arrow in {t:?}");
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
        assert!(about_page(200, 5_000).is_empty());
    }

    /// A readable 404 is not a failure. Many paywalls answer 403 with the article intact, and
    /// replacing that with an error screen would throw away a page the reader can read.
    #[test]
    fn a_readable_404_is_a_caution_not_a_failure() {
        let notices = about_page(404, 5_000);
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
        assert_eq!(about_page(200, THIN_TEXT).len(), 0);
        assert_eq!(about_page(200, THIN_TEXT - 1).len(), 1);
    }

    /// Loudest first: the remark most likely to change how the page is read sits nearest the
    /// top, where it is read before the prose it is about.
    #[test]
    fn a_page_that_is_both_broken_and_empty_leads_with_the_status() {
        let notices = about_page(500, 10);
        assert_eq!(notices.len(), 2);
        assert!(notices[0].body.contains("500"));
        assert!(notices[1].body.contains("JavaScript"));
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
