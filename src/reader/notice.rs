//! What the reader has to say about a page, and how loud. **No egui types, no colours.**
//!
//! `src/ir.rs` keeps the page's *styling* out of the document. This module is the same fence
//! one layer out: it keeps the reader's own *wording* out of `ui/`, so the text hww puts on
//! screen is decided where `cargo test` can read it. That matters more than it looks. Two strings
//! in the old error screen contained `→`, which the faces egui embedded by default did not carry,
//! and prose review did not catch that both rendered as tofu. The former
//! `no_notice_contains_an_arrow` test is gone because `reader::ui::fonts` now embeds faces that
//! carry arrows in every role. What the episode leaves behind is the reason this module sits
//! outside `ui/` at all.
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
//! # Who did what
//!
//! Every sentence here names its actor as the grammatical subject: *hww* for what the reader
//! did (a built-in rewrite rule, the 5 MB cap), the *host* for what the server did (a redirect,
//! a 404). "Opened text.example.com instead of example.com" reads as a choice the browser made,
//! and it was; "text.example.com sent the request on to www.example.com" reads as the server's,
//! and it was. Neither hides behind a passive, and a reader deciding whether to trust the page
//! can tell which side of the wire each fact came from.
//!
//! # The shape the words are drawn in
//!
//! The reader no longer prints a `[hww]` mark or the severity word: a notice about a loaded
//! page is an **infobar** under the top chrome, in the form every browser shares (an icon, a
//! sentence, the actions as buttons, a close), and a failure is an **error page**. The severity
//! survives as the icon's shape and as [`Severity::word`], which is the icon's accessible name
//! and hover text, so a screen reader and a monochrome eye both still get it. `[image]` and
//! `[loading]` stay inline on purpose: they mark one thing at one place in the document.
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

/// The extracted-text length below which a page is treated as having failed to extract.
///
/// Re-exported rather than restated: this *is* `html::extract`'s `<body>` fallback threshold,
/// and a page that took the fallback is exactly the page worth warning about. A copy of the
/// number here would let the two drift silently, with the reader quietly ceasing to warn.
pub use crate::ir::THIN_TEXT;

/// The deadline a document fetch is held to.
///
/// Re-exported for the same reason as [`THIN_TEXT`] just above: it is the denominator of
/// [`elapsed_fraction`], and a copy of the number here would let the progress rule go on
/// measuring against 15 seconds after `fetch` had moved to something else.
pub use crate::fetch::TIMEOUT;

/// The mark on a link whose page is being fetched.
///
/// A bracket, not a colour or a second underline. Everything the reader says *inside* the reading
/// column carries this convention (`[image]` is the other) because a bracket survives a
/// screenshot, a monochrome eye, and a theme nobody has designed yet, and because a coloured
/// underline on a link would be the reader borrowing the page's own idiom to speak in, which is
/// the one thing the column rule forbids. It is positional, which is what admits it inline at
/// all: it marks one link at one place, and a bar docked above the page cannot point.
pub const PENDING: &str = "[loading]";

/// How long a transient status stays up: "Copied URL", "loading 2 images from …", "no link
/// focused". The Material range is four to ten seconds; six, because the one message in the set
/// that matters is the disclosure of which hosts `I` is about to contact, and a reader looking
/// at the page rather than the pill needs a moment to notice it. It used to be twelve in the
/// status strip, where a line of dim text needs longer to be found than a pill does.
pub const TOAST_FOR: Duration = Duration::from_secs(6);

/// How loud a notice is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Information: something the reader did on the page's behalf and should own up to. An
    /// information bar; nothing is wrong.
    Quiet,
    /// Something about the page the reader would otherwise mis-read. A caution bar over a
    /// readable page.
    Caution,
    /// There is no page. An error page, which owns the viewport.
    Failure,
}

impl Severity {
    /// The severity, as a word: the accessible name and hover text of the painted icon.
    ///
    /// Colour is the obvious way to say how loud a notice is and the one that fails quietest: a
    /// hue can be missed, screenshotted into greyscale, or read by an eye that does not separate
    /// rust from brown. The icon's *shape* carries it for a sighted reader (a triangle, a circled
    /// `i`, a crossed circle); this word carries it for a screen reader, which is told nothing
    /// by a shape, and for the pointer that rests on it.
    ///
    /// Wording, so it lives here rather than in `ui/` for the same reason every other notice
    /// string does: the fast CI job never compiles egui, and this is where a test can read it.
    pub fn word(self) -> &'static str {
        match self {
            Severity::Quiet => "note",
            Severity::Caution => "caution",
            Severity::Failure => "failure",
        }
    }
}

/// What a notice offers to do about itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Retry,
    RetryWithoutRewrite,
    CopyUrl,
}

impl Button {
    /// "Reload", which is the word on the button every browser puts on this page, rather than
    /// "Retry", which is the word in the code.
    pub fn label(self) -> &'static str {
        match self {
            Button::Retry => "Reload",
            Button::RetryWithoutRewrite => "Reload without rewrite",
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
    /// Set dimmer under the body: the URL, the redirect chain.
    pub detail: Vec<String>,
    pub actions: &'static [Button],
    /// Failures only: the short name of what went wrong, the way an error page prints an
    /// `ERR_` code under the prose. For the reader who files a bug, and for nobody else.
    pub code: Option<&'static str>,
}

impl Notice {
    fn new(severity: Severity, body: impl Into<String>) -> Self {
        Self {
            severity,
            heading: None,
            body: body.into(),
            detail: Vec::new(),
            actions: &[],
            code: None,
        }
    }

    /// What a bar is dismissed by: its own words. A notice is rebuilt from the provenance every
    /// frame, so the identity has to be something the words themselves carry, and two notices
    /// with the same words about the same page are the same remark.
    pub fn key(&self) -> &str {
        &self.body
    }
}

/// A built-in rewrite rule was applied: the reader asked a different host than the one named.
///
/// An information bar and not a caution, because nothing is wrong: the rule is compiled in,
/// cited, and reported. But it is hww's doing and not the site's, and the bar says so in those
/// words, with the one action a reader might want beside it. The status strip still prints the
/// terse version before the request goes out (charter point 4); this is the readable one once
/// the page is up, and it lasts as long as the page does.
pub fn rewrite(prov: &Provenance) -> Option<Notice> {
    let to = prov.rewritten_to.as_ref()?;
    let aimed = to.host_str().unwrap_or("another host");
    let asked = prov.requested.host_str().unwrap_or("the host you named");
    let mut n = Notice::new(
        Severity::Quiet,
        format!(
            "hww asked {aimed} for this page instead of {asked}, by a built-in rule for this site."
        ),
    );
    n.actions = &[Button::RetryWithoutRewrite];
    Some(n)
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
                "{aimed} sent the request on to {landed}. The rule that chose {aimed} appears \
                 dead, so this may not be the page you asked for."
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
        let host = prov.final_url.host_str().unwrap_or("The server");
        out.push(Notice::new(
            Severity::Caution,
            format!(
                "{host} answered {}. Showing what it sent anyway.",
                prov.status
            ),
        ));
    }
    if text_len < THIN_TEXT {
        out.push(Notice::new(
            Severity::Caution,
            "hww found little text on this page, which may need JavaScript to show its content.",
        ));
    }
    out
}

/// The bar prose for a body that did not arrive whole.
///
/// One string per variant, hedging exactly where the evidence hedges: `len >= cap` alone
/// cannot tell a body that was cut from one that happened to end on the boundary, so
/// `MaybeAtCap` says "may" and `AtCap` does not. The cap and the clock are hww's, and the
/// sentences say so. The terser phrasing for the page-info panel is
/// [`crate::reader::pageinfo`]'s; this one is the alarm, that one is the record.
fn truncation_body(t: Truncation) -> Option<String> {
    Some(match t {
        Truncation::Complete => return None,
        Truncation::AtCap(n) => format!(
            "hww stopped reading at its {} MB cap. The rest of this page was not fetched.",
            n / 1_000_000
        ),
        Truncation::MaybeAtCap(n) => format!(
            "hww may have cut this page short at its {} MB cap.",
            n / 1_000_000
        ),
        Truncation::Timeout(d) => format!(
            "hww stopped waiting after {}s with the body still arriving. This article is \
             incomplete.",
            d.as_secs()
        ),
        Truncation::Incomplete(_) => {
            "hww lost the connection partway. This article is incomplete.".to_owned()
        }
    })
}

/// Whether the document carries a run in the RTL blocks. Public so the page-info panel can
/// keep the record after the caution is closed: dismissing the bar must not be the way the
/// fact disappears.
pub fn carries_rtl(doc: &crate::ir::Document) -> bool {
    fn has_rtl(s: &str) -> bool {
        s.chars().any(|c| ('\u{0590}'..='\u{08FF}').contains(&c))
    }
    fn blocks_have_rtl(blocks: &[crate::ir::Block]) -> bool {
        use crate::ir::Block;
        blocks.iter().any(|b| match b {
            Block::Heading { inlines, .. } | Block::Paragraph(inlines) => {
                has_rtl(&crate::ir::plain_text(inlines))
            }
            Block::List { items, .. } => items.iter().any(|i| blocks_have_rtl(i)),
            Block::Quote { blocks, .. } => blocks_have_rtl(blocks),
            Block::Code { text, .. } => has_rtl(text),
            Block::Figure { caption, .. } => caption
                .as_deref()
                .is_some_and(|c| has_rtl(&crate::ir::plain_text(c))),
            Block::Table { headers, rows } => {
                headers.iter().any(|c| has_rtl(&crate::ir::plain_text(c)))
                    || rows
                        .iter()
                        .flatten()
                        .any(|c| has_rtl(&crate::ir::plain_text(c)))
            }
            Block::Thread(cs) => cs.iter().any(|c| blocks_have_rtl(&c.blocks)),
            Block::Entries(es) => es
                .iter()
                .any(|e| has_rtl(&crate::ir::plain_text(&e.title)) || blocks_have_rtl(&e.summary)),
            Block::Rule | Block::Embed { .. } => false,
        })
    }
    doc.title.as_deref().is_some_and(has_rtl) || blocks_have_rtl(&doc.blocks)
}

/// A page carrying right-to-left text. epaint shapes but does not reorder, so Hebrew and
/// Arabic lay out in logical order, backwards, or as tofu (the tail face is chosen to carry no
/// RTL script so the failure is visible rather than plausible). The honest completion of that
/// choice is to say so. `U+0590..=U+08FF` is Hebrew, Arabic, Syriac, Thaana, NKo, Samaritan,
/// Mandaic, and Arabic Supplement/Extended-A: the RTL blocks in one span.
pub fn rtl(doc: &crate::ir::Document) -> Option<Notice> {
    carries_rtl(doc).then(|| {
        Notice::new(
            Severity::Caution,
            "hww cannot lay out the right-to-left text on this page: a Hebrew or Arabic run \
             is drawn in the wrong order, or as missing glyphs."
                .to_owned(),
        )
    })
}

/// The screen a failed navigation gets.
///
/// Each error gets its own words and its own actions: a wrong-but-confident diagnosis is the
/// worst kind, because it blames the site.
pub fn failure(error: &LoadError, url: &Url) -> Notice {
    use crate::fetch::FetchError;

    let (heading, body, actions, code): (&'static str, String, &'static [Button], &'static str) =
        match error {
            LoadError::Fetch(FetchError::LikelyBlocked) => (
                "This site did not answer.",
                // Phase 0 measured bot detection failing as a silent hang rather than a 403.
                "The request timed out with no reply. On a document fetch that is usually bot \
             detection failing silently rather than a network fault."
                    .to_owned(),
                &[Button::Retry, Button::RetryWithoutRewrite, Button::CopyUrl],
                "LikelyBlocked",
            ),
            LoadError::Fetch(FetchError::Timeout(d)) => (
                "Timed out.",
                format!(
                    "No reply within {}s. That is bandwidth rather than a block.",
                    d.as_secs()
                ),
                // No retry-without-rewrite: bandwidth is not a rewrite rule's fault.
                &[Button::Retry, Button::CopyUrl],
                "Timeout",
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
                "SchemeDowngrade",
            ),
            LoadError::Fetch(FetchError::TooManyRedirects(_)) => (
                "Too many redirects.",
                "The request bounced past the redirect limit. A loop like this is usually a \
             consent wall."
                    .to_owned(),
                &[Button::Retry, Button::RetryWithoutRewrite, Button::CopyUrl],
                "TooManyRedirects",
            ),
            LoadError::Fetch(FetchError::RedirectWithoutLocation(status)) => (
                "The server sent a broken redirect.",
                format!(
                    "It answered {status}, which means \"go somewhere else\", but did not say \
                 where. There is nothing to follow."
                ),
                &[Button::Retry, Button::CopyUrl],
                "RedirectWithoutLocation",
            ),
            LoadError::Fetch(FetchError::BadLocation(to)) => (
                "The server redirected somewhere unusable.",
                format!("It pointed at {to}, which is not an address hww can fetch."),
                &[Button::Retry, Button::CopyUrl],
                "BadLocation",
            ),
            LoadError::NotWebPage { content_type } => (
                "Not a web page.",
                format!("The server sent {content_type}, which hww has no reader for."),
                &[Button::CopyUrl],
                "NotWebPage",
            ),
            other => (
                "The request failed.",
                other.to_string(),
                &[Button::Retry, Button::CopyUrl],
                "Transport",
            ),
        };

    let mut detail = vec![url.to_string()];
    if let LoadError::Fetch(FetchError::TooManyRedirects(hops)) = error {
        // A redirect loop is usually a consent wall, and the chain is what shows it.
        detail.extend(hops.iter().map(|h| format!("  -> {h}")));
    }

    Notice {
        severity: Severity::Failure,
        heading: Some(heading),
        body,
        detail,
        actions,
        code: Some(code),
    }
}

/// The idle screen: the one screen where the reader is the page.
///
/// A wordmark, one line of what this is, and the two keys that matter, each with its meaning.
/// The waveform that sits over the wordmark is paint, not wording, so it lives in
/// `ui::notice_ui` with the rest of how this screen is drawn. Not a notice: nothing has
/// happened yet, so there is nothing to report, and a Quiet band that said "press Ctrl+L" was
/// the reader describing its own chrome in the column. The URL bar is open and focused over
/// it (`ReaderApp::new`), which is where the typing goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Splash {
    pub wordmark: &'static str,
    pub tagline: &'static str,
    /// `(keys, what they do)`, set as keycaps by the reader.
    pub keys: &'static [(&'static str, &'static str)],
}

pub fn idle() -> Splash {
    Splash {
        wordmark: "hww",
        tagline: "A new browser for the human-wide web.",
        keys: &[("Ctrl+L", "Open a URL"), ("?", "Keyboard shortcuts")],
    }
}

/// A navigation in flight, when there is nothing on screen to read while it runs: one dim line
/// in the middle of an empty page, naming what is being fetched.
///
/// **`None` when a page is still up.** A fetch used to replace the article the moment the link
/// was clicked, which on a fast connection is a line that flashes for two frames and cannot be
/// read, and on a slow one is a blank screen that reads as having arrived somewhere empty rather
/// than as waiting. The outgoing page stays now, and while it does the reader reports the errand
/// from the chrome instead: the strip counts the seconds and `ui::notice_ui::progress_rule` draws
/// what is left of the budget. A browser paints nothing on a page that has not arrived and moves
/// an indicator in the chrome; this is that, plus the one line, because fifteen seconds of blank
/// with a hairline moving at the bottom reads as a hang to anyone not watching the hairline.
///
/// This returns the `Option` rather than letting the caller decide, because the caller is
/// `app.rs`: the fast CI job never compiles egui, so a decision made there is a decision no test
/// reads.
///
/// `elapsed` is shown because a 15 s bot-block hang and a slow CDN look identical for the
/// first ten seconds, and a number that keeps moving is the difference between waiting and
/// wondering whether it is stuck.
pub fn loading(url: &Url, elapsed: Duration, over_a_page: bool) -> Option<String> {
    if over_a_page {
        return None;
    }
    Some(format!(
        "loading {} … {:.1}s",
        host_and_path(url),
        elapsed.as_secs_f32()
    ))
}

/// How much of the fetch deadline a navigation has spent, as `0.0..=1.0`.
///
/// The number behind the progress rule on the status strip. Determinate rather than an
/// indeterminate sweep, and the denominator is honest: [`TIMEOUT`] bounds the *whole* fetch
/// including every redirect hop, which `fetch::the_timeout_bounds_the_whole_fetch_not_each_hop`
/// is what pins. So the far edge means the request is at its deadline, rather than meaning
/// nothing at all.
///
/// Approximate at both ends, and clamped for it. `Page::Loading::started` is stamped when the job
/// is dispatched, while the fetcher's own deadline starts when a worker picks it up, and
/// `session::load` rewrites and extracts *after* the body is in. Neither gap is worth a second
/// clock; a bar that saturates a little early on a page that was always going to time out costs
/// nothing.
pub fn elapsed_fraction(elapsed: Duration) -> f32 {
    (elapsed.as_secs_f32() / TIMEOUT.as_secs_f32()).clamp(0.0, 1.0)
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

    /// Three severities, three words, and none of them a prefix of another.
    ///
    /// The word is the icon's accessible name, so loudness reaches a screen reader; two
    /// severities sharing a word, or one reading as an abbreviation of the next, would put that
    /// back where it was.
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
            assert!(
                n.code.is_some(),
                "a failure with no code: {}",
                n.heading.unwrap()
            );
        }
        // And the codes are distinct too: the code is what a bug report quotes.
        let mut codes: Vec<&str> = notices.iter().filter_map(|n| n.code).collect();
        codes.sort_unstable();
        let before = codes.len();
        codes.dedup();
        assert_eq!(before, codes.len(), "two failures share a code");
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

    /// The whole point of the third argument: a page still on screen gets no line, because the
    /// strip and the progress rule are reporting instead.
    #[test]
    fn a_load_over_a_readable_page_draws_no_line() {
        let u = url("https://example.com/a");
        assert!(loading(&u, Duration::from_secs(3), true).is_none());
        let line = loading(&u, Duration::from_secs(3), false).expect("no page under it");
        assert!(line.contains("example.com/a"));
        assert!(line.contains("3.0s"), "the clock is the point: {line}");
    }

    /// Inside Material's range for a snackbar, and long enough to read the hosts `I` names.
    #[test]
    fn a_toast_lives_between_four_and_ten_seconds() {
        assert!(TOAST_FOR >= Duration::from_secs(4));
        assert!(TOAST_FOR <= Duration::from_secs(10));
    }

    /// The idle screen names the two keys that matter and nothing the chrome already shows.
    #[test]
    fn the_splash_names_its_keys() {
        let s = idle();
        assert!(!s.wordmark.is_empty());
        assert!(s.keys.iter().any(|(k, _)| *k == "Ctrl+L"));
        assert!(s.keys.iter().any(|(k, _)| *k == "?"));
        assert!(s.keys.iter().all(|(_, what)| !what.is_empty()));
    }

    /// The rewrite bar is hww owning up, so `hww` is its subject and the one action it offers
    /// is the way out of it. It is not a caution: nothing is wrong.
    #[test]
    fn a_rewrite_is_an_information_bar_naming_hww_as_the_actor() {
        assert!(rewrite(&page(200)).is_none(), "no rule, no bar");
        let mut p = page(200);
        p.rewritten_to = Some(url("https://text.example.com/a"));
        let n = rewrite(&p).expect("a bar");
        assert_eq!(n.severity, Severity::Quiet);
        assert!(n.body.starts_with("hww "), "{}", n.body);
        assert!(n.body.contains("text.example.com"));
        assert!(n.body.contains("example.com"));
        assert_eq!(n.actions, &[Button::RetryWithoutRewrite]);
    }

    /// Every sentence about a loaded page names who did the thing: the host for what the server
    /// did, `hww` for what the reader did. A passive ("the body was cut") says neither.
    fn names_hww_or_a_host(body: &str) {
        let first = body.split_whitespace().next().unwrap_or("");
        assert!(
            first == "hww" || first.contains('.'),
            "no actor as subject: {body}"
        );
    }

    #[test]
    fn every_remark_names_its_actor() {
        let mut p = dead_rule();
        p.status = 404;
        p.truncation = Truncation::AtCap(5_000_000);
        let notices = about_page(&p, 10);
        assert!(
            notices[0].body.starts_with("alt.example.net "),
            "{}",
            notices[0].body
        );
        assert!(notices[1].body.starts_with("hww "), "{}", notices[1].body);
        assert!(
            notices[2].body.starts_with("www.example.org "),
            "{}",
            notices[2].body
        );
        assert!(notices[3].body.starts_with("hww "), "{}", notices[3].body);
        for n in &notices {
            names_hww_or_a_host(&n.body);
            assert!(!n.body.contains("The server"), "{}", n.body);
        }
        for t in every_truncation() {
            let mut p = page(200);
            p.truncation = t;
            for n in about_page(&p, 9_000) {
                names_hww_or_a_host(&n.body);
            }
        }
        let mut rewritten = page(200);
        rewritten.rewritten_to = Some(url("https://text.example.com/a"));
        names_hww_or_a_host(&rewrite(&rewritten).expect("a bar").body);

        let mut d = crate::html::extract(
            "<html><body><article><p>A page in Latin script, long enough to be a paragraph on \
             its own merits and then some.</p></article></body></html>",
            &url::Url::parse("https://example.test/").unwrap(),
        );
        d.blocks
            .push(crate::ir::Block::Paragraph(vec![crate::ir::Inline::Text(
                "שלום עולם".to_owned(),
            )]));
        names_hww_or_a_host(&rtl(&d).expect("a caution").body);
    }

    /// Pinned against `TIMEOUT`, never the literal 15: the whole reason it is re-exported is so
    /// that moving the deadline moves the bar with it.
    #[test]
    fn the_progress_fraction_spans_the_fetch_deadline() {
        assert_eq!(elapsed_fraction(Duration::ZERO), 0.0);
        assert_eq!(elapsed_fraction(TIMEOUT), 1.0);
        let half = elapsed_fraction(TIMEOUT / 2);
        assert!((half - 0.5).abs() < 1e-6, "{half}");
    }

    /// `started` covers rewrite and extraction as well as the fetch, so it outruns the deadline
    /// on a slow page. A bar wider than its track is a panic in egui, not a cosmetic problem.
    #[test]
    fn the_progress_fraction_never_exceeds_its_track() {
        assert_eq!(elapsed_fraction(TIMEOUT * 4), 1.0);
    }

    /// The inline marks all carry it, and this is the newest of them.
    #[test]
    fn the_pending_mark_carries_the_bracket_convention() {
        assert!(PENDING.starts_with('['));
        assert!(PENDING.ends_with(']'));
        assert!(!PENDING.contains(char::is_whitespace));
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
    fn a_dead_rule_is_a_bar_naming_both_hosts() {
        let notices = about_page(&dead_rule(), 9_000);
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].severity, Severity::Caution);
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
    fn every_truncation_is_a_bar_and_a_whole_body_is_not() {
        let mut clean = page(200);
        clean.truncation = Truncation::Complete;
        assert!(about_page(&clean, 9_000).is_empty());

        for t in every_truncation() {
            let mut p = page(200);
            p.truncation = t;
            let notices = about_page(&p, 9_000);
            assert_eq!(notices.len(), 1, "{t:?} produced {} bars", notices.len());
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
    fn the_bars_are_ordered_loudest_first() {
        let mut p = dead_rule();
        p.status = 500;
        p.truncation = Truncation::Incomplete(812_345);
        let notices = about_page(&p, 10);
        assert_eq!(notices.len(), 4);
        assert!(notices[0].body.contains("appears dead"));
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
    fn a_page_with_hebrew_or_arabic_gets_the_rtl_caution_and_a_latin_page_does_not() {
        let mut d = crate::html::extract(
            "<html><body><article><p>A page in Latin script, long enough to be a paragraph on \
             its own merits and then some.</p></article></body></html>",
            &url::Url::parse("https://example.test/").unwrap(),
        );
        assert!(rtl(&d).is_none());
        d.blocks
            .push(crate::ir::Block::Paragraph(vec![crate::ir::Inline::Text(
                "שלום עולם".to_owned(),
            )]));
        let n = rtl(&d).expect("a caution");
        assert_eq!(n.severity, Severity::Caution);
    }
}
