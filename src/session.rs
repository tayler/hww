//! The pipeline, owned in one place: rewrite -> fetch -> decode -> extract.
//!
//! # Why this module exists
//!
//! `rewrite` may only be reached from here, and nothing here reaches back into an extractor.
//! That is the same rule the charter used to state as "nothing imports `rewrite` except
//! `main.rs`", and it protects the same three properties: hostnames live in one file, no
//! extractor keys on a host, and a rewrite applies to the entry URL only, once, reported
//! before dispatch.
//!
//! Naming *this* module rather than `main.rs` strengthens the third. As an `eprintln!` in a
//! driver, the notice was something a second driver could simply forget; as a value the one
//! function owning the pipeline hands back before it dispatches, skipping it is structurally
//! impossible. The alternative ("entry-point drivers only") permits N drivers each
//! re-implementing the notice, and every new one is a chance to drop it.

use crate::fetch::{self, FetchError, Fetcher, Limits, Referer, Truncation};
use crate::{html, ir};
use url::Url;

pub struct LoadOptions {
    /// Apply the builtin rewrite table. `--no-rewrite` and the reader's `R` clear it.
    pub rewrite: bool,
}

impl Default for LoadOptions {
    fn default() -> Self {
        Self { rewrite: true }
    }
}

/// Emitted *before* the request is dispatched. Charter point 4 is unconditional: a fetch that
/// fails or hangs must not be able to swallow the fact that hww went somewhere the user did
/// not type.
#[derive(Debug, Clone)]
pub enum Notice {
    Rewrote { from: Url, to: Url },
}

impl std::fmt::Display for Notice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Notice::Rewrote { from, to } => write!(
                f,
                "[rewrote {} -> {}]",
                from.host_str().unwrap_or("?"),
                to.host_str().unwrap_or("?")
            ),
        }
    }
}

/// Everything the fetch disclosed, in one value. The reader shows it in the status strip and
/// the CLI prints it on stderr; both read the same fields, so neither can drift into a
/// prettier version of the truth than the other.
#[derive(Debug, Clone)]
pub struct Provenance {
    pub requested: Url,
    pub rewritten_to: Option<Url>,
    /// The request did not end on the host the rule aimed at. Not an error, but the rule is
    /// dead, and saying so is the whole early-warning system, because a failed rewrite is
    /// never retried.
    pub rule_appears_dead: bool,
    pub status: u16,
    pub encoding: &'static str,
    pub final_url: Url,
    pub bytes: usize,
    pub chars: usize,
    pub hops: Vec<Url>,
    pub cookie_attempts: usize,
    pub truncation: Truncation,
}

impl std::fmt::Display for Provenance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let dead = if self.rule_appears_dead {
            let to = self.rewritten_to.as_ref().and_then(Url::host_str);
            format!(
                " | {} redirected to {}, rule appears dead",
                to.unwrap_or("?"),
                self.final_url.host_str().unwrap_or("?")
            )
        } else {
            String::new()
        };
        // Truncation is fetched today and was never reported. Appending it here is a
        // deliberate output change on exactly the pages whose output was a silent lie, and it
        // hedges where the evidence hedges, because `len >= cap` alone cannot tell a body that
        // was cut from one that happened to end there.
        let truncated = match self.truncation {
            Truncation::Complete => String::new(),
            Truncation::AtCap(n) => format!(" | truncated at {} MB", n / 1_000_000),
            Truncation::MaybeAtCap(n) => format!(" | may be truncated at {} MB", n / 1_000_000),
            Truncation::Timeout(d) => format!(" | truncated: timed out after {}s", d.as_secs()),
            Truncation::Incomplete(n) => format!(" | truncated: connection dropped at {n} bytes"),
        };
        write!(
            f,
            "[{} {} | {}{} | {} bytes -> {} chars | {} redirect(s) | {} cookie attempt(s) discarded{}]",
            self.status,
            self.encoding,
            self.final_url,
            dead,
            self.bytes,
            self.chars,
            self.hops.len(),
            self.cookie_attempts,
            truncated
        )
    }
}

pub struct Loaded {
    pub doc: ir::Document,
    pub prov: Provenance,
}

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error(transparent)]
    Fetch(#[from] FetchError),
    /// The response was not a web page.
    ///
    /// Without this, a PDF, a zip, or a bare image reaches `html::extract`, yields nothing,
    /// and lands on "this page may require JavaScript": a confident, wrong diagnosis, and the
    /// worst kind, because it blames the site.
    #[error("{content_type} is not a web page")]
    NotWebPage { content_type: String },
}

/// What following a link should do, decided **before** anything is dispatched.
///
/// `html::extract` absolutizes every href through `base.join`, which passes non-HTTP schemes
/// through untouched, and nothing looked at what came back, because typing a URL made the
/// question rare. Clicking one makes it routine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Navigate(Url),
    /// hww does not hand a URL to a system handler, the same reason eframe's `links` feature
    /// is off. Offer the text instead and let the user decide.
    OfferCopy {
        url: String,
        note: &'static str,
    },
    /// Refused with a reason, never opaquely.
    Refuse {
        url: String,
        reason: String,
    },
}

pub fn classify_link(href: &str) -> Target {
    let Ok(url) = Url::parse(href) else {
        return Target::Refuse {
            url: href.to_owned(),
            reason: "not a usable address".to_owned(),
        };
    };
    match url.scheme() {
        "http" | "https" => Target::Navigate(url),
        "mailto" => Target::OfferCopy {
            url: href.to_owned(),
            note: "an email address (hww does not open a mail client)",
        },
        "tel" => Target::OfferCopy {
            url: href.to_owned(),
            note: "a phone number (hww does not open a dialler)",
        },
        "javascript" => Target::Refuse {
            url: href.to_owned(),
            reason: "a javascript: link (there is no JavaScript here to run it)".to_owned(),
        },
        "data" => Target::Refuse {
            url: href.to_owned(),
            reason: "a data: link (hww will not render bytes a page inlines into a link)"
                .to_owned(),
        },
        other => Target::Refuse {
            url: href.to_owned(),
            reason: format!("no handler for {other}: links yet"),
        },
    }
}

/// Whether a `Content-Type` describes something `html::extract` can read.
///
/// Deliberately generous: an absent header, and anything `text/*`, passes through, because
/// refusing on absence would break a great many pages and a `.txt` file is at least words.
/// What it catches is the confident wrong diagnosis: a PDF, an archive, or a bare image.
fn is_web_page(content_type: Option<&str>) -> bool {
    let Some(ct) = content_type else {
        return true;
    };
    let essence = ct
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    essence.is_empty()
        || essence.starts_with("text/")
        || matches!(
            essence.as_str(),
            "application/xhtml+xml"
                | "application/xml"
                | "application/rss+xml"
                | "application/atom+xml"
                | "application/json"
        )
}

pub struct Session {
    fetcher: Fetcher,
}

impl Session {
    pub fn new() -> Result<Self, LoadError> {
        Ok(Self {
            fetcher: Fetcher::new()?,
        })
    }

    /// Fetch and extract one page.
    ///
    /// `notify` is called before the request is dispatched, never after. The CLI's closure is
    /// an `eprintln!`; the reader's sends the notice to the UI thread and repaints, so the
    /// line is on screen for the whole of a 15 s hang instead of arriving with the corpse.
    pub fn load(
        &self,
        requested: &Url,
        opts: &LoadOptions,
        notify: &mut dyn FnMut(&Notice),
    ) -> Result<Loaded, LoadError> {
        let url = if opts.rewrite {
            crate::rewrite::apply(requested)
        } else {
            requested.clone()
        };
        let rewritten = url != *requested;
        if rewritten {
            notify(&Notice::Rewrote {
                from: requested.clone(),
                to: url.clone(),
            });
        }

        let resp = self.fetcher.get(&url)?;
        if !is_web_page(resp.content_type.as_deref()) {
            return Err(LoadError::NotWebPage {
                content_type: resp
                    .content_type
                    .unwrap_or_default()
                    .split(';')
                    .next()
                    .unwrap_or("unknown")
                    .trim()
                    .to_owned(),
            });
        }
        let (source, enc) = fetch::decode(&resp.body, resp.content_type.as_deref());
        let doc = html::extract(&source, &resp.final_url);

        // The bounce is usually to a sibling of the requested host rather than to that host
        // itself, so comparing against the *request* would miss the common case.
        let rule_appears_dead = rewritten && resp.final_url.host_str() != url.host_str();

        let prov = Provenance {
            requested: requested.clone(),
            rewritten_to: rewritten.then_some(url),
            rule_appears_dead,
            status: resp.status,
            encoding: enc.name(),
            final_url: resp.final_url,
            bytes: resp.body.len(),
            chars: doc.text_len(),
            hops: resp.hops,
            cookie_attempts: resp.cookie_attempts,
            truncation: resp.truncation,
        };
        Ok(Loaded { doc, prov })
    }
}

/// One image subresource. Bytes and their partial-ness; decoding is the reader's job, and
/// deliberately not this module's; see `reader::image_decode` for why the only parser of
/// untrusted binary input in this crate lives outside the feature-gated half.
pub struct FetchedImage {
    pub bytes: Vec<u8>,
    pub partial: Truncation,
    pub final_url: Url,
    pub cookie_attempts: usize,
}

impl Session {
    /// Fetch an image on behalf of `page`. Reached only from an explicit click.
    ///
    /// `page` is the document being read, and only its **origin** is disclosed; see
    /// [`fetch::Referer::PageOrigin`] for the whole argument.
    ///
    /// `send_referer` is the settings toggle, and it is applied *here*, by choosing the
    /// `Limits`, not by the caller counting differently. A setting that changes a report
    /// rather than the request is the kind of no-op this project has already deleted once.
    pub fn fetch_image(
        &self,
        url: &Url,
        page: &Url,
        send_referer: bool,
    ) -> Result<FetchedImage, FetchError> {
        let limits = if send_referer {
            Limits::IMAGE
        } else {
            Limits {
                referer: Referer::None,
                ..Limits::IMAGE
            }
        };
        // `get_with` debug-asserts that a PageOrigin request names its page; with the policy
        // off there is nothing to name, so pass nothing rather than something ignored.
        let referrer = send_referer.then_some(page);
        let resp = self.fetcher.get_with(url, referrer, &limits)?;
        Ok(FetchedImage {
            bytes: resp.body,
            partial: resp.truncation,
            final_url: resp.final_url,
            cookie_attempts: resp.cookie_attempts,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prov(url: &str) -> Provenance {
        Provenance {
            requested: Url::parse(url).unwrap(),
            rewritten_to: None,
            rule_appears_dead: false,
            status: 200,
            encoding: "UTF-8",
            final_url: Url::parse(url).unwrap(),
            bytes: 1234,
            chars: 567,
            hops: vec![],
            cookie_attempts: 2,
            truncation: Truncation::Complete,
        }
    }

    #[test]
    fn provenance_line_matches_the_cli_format() {
        assert_eq!(
            prov("https://example.com/a").to_string(),
            "[200 UTF-8 | https://example.com/a | 1234 bytes -> 567 chars | \
             0 redirect(s) | 2 cookie attempt(s) discarded]"
        );
    }

    /// The branch the CLI had and no test ever reached.
    #[test]
    fn dead_rule_names_both_hosts() {
        let mut p = prov("https://example.com/a");
        p.rewritten_to = Some(Url::parse("https://alt.example.com/a").unwrap());
        p.rule_appears_dead = true;
        p.final_url = Url::parse("https://www.example.com/a").unwrap();
        assert!(
            p.to_string()
                .contains("| alt.example.com redirected to www.example.com, rule appears dead |")
        );
    }

    #[test]
    fn truncation_is_reported_and_only_when_it_happened() {
        let mut p = prov("https://example.com/a");
        assert!(!p.to_string().contains("truncated"));
        p.truncation = Truncation::AtCap(crate::fetch::MAX_BYTES);
        assert!(
            p.to_string()
                .ends_with("cookie attempt(s) discarded | truncated at 5 MB]")
        );
        // No Content-Length to check against: hedge rather than assert.
        p.truncation = Truncation::MaybeAtCap(crate::fetch::MAX_BYTES);
        assert!(p.to_string().contains("may be truncated at 5 MB"));
    }

    #[test]
    fn web_schemes_navigate_and_everything_else_says_what_it_is() {
        assert!(matches!(
            classify_link("https://example.com/a"),
            Target::Navigate(_)
        ));
        assert!(matches!(
            classify_link("mailto:someone@example.com"),
            Target::OfferCopy { .. }
        ));
        assert!(matches!(
            classify_link("tel:+15550100"),
            Target::OfferCopy { .. }
        ));
        for href in [
            "javascript:alert(1)",
            "data:text/html,<b>x",
            "ftp://example.com/f",
        ] {
            match classify_link(href) {
                Target::Refuse { reason, .. } => assert!(!reason.is_empty()),
                other => panic!("{href} should be refused, got {other:?}"),
            }
        }
        // gemini and gopher are on the roadmap; until then, say which scheme it was.
        match classify_link("gemini://example.com/") {
            Target::Refuse { reason, .. } => assert!(reason.contains("gemini")),
            other => panic!("expected a named refusal, got {other:?}"),
        }
    }

    /// The point is not to be strict, it is to stop blaming the site for a PDF.
    #[test]
    fn only_clearly_non_page_content_types_are_refused() {
        for ct in [
            None,
            Some("text/html; charset=utf-8"),
            Some("TEXT/HTML"),
            Some("application/xhtml+xml"),
            Some("text/plain"),
            Some(""),
        ] {
            assert!(is_web_page(ct), "{ct:?} should be readable");
        }
        for ct in [
            "application/pdf",
            "image/jpeg",
            "application/zip",
            "video/mp4",
            "application/octet-stream",
        ] {
            assert!(!is_web_page(Some(ct)), "{ct} is not a web page");
        }
    }

    #[test]
    fn rewrite_notice_names_hosts_not_urls() {
        let n = Notice::Rewrote {
            from: Url::parse("https://reddit.com/r/rust/x").unwrap(),
            to: Url::parse("https://old.reddit.com/r/rust/x").unwrap(),
        };
        assert_eq!(n.to_string(), "[rewrote reddit.com -> old.reddit.com]");
    }
}
