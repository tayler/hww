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
//! impossible. The alternative — "entry-point drivers only" — permits N drivers each
//! re-implementing the notice, and every new one is a chance to drop it.

use crate::fetch::{self, FetchError, Fetcher, MAX_BYTES};
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
    /// The request did not end on the host the rule aimed at. Not an error — but the rule is
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
    pub truncated: bool,
}

impl std::fmt::Display for Provenance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let dead = if self.rule_appears_dead {
            let to = self.rewritten_to.as_ref().and_then(Url::host_str);
            format!(
                " | {} redirected to {} — rule appears dead",
                to.unwrap_or("?"),
                self.final_url.host_str().unwrap_or("?")
            )
        } else {
            String::new()
        };
        // `truncated` is fetched today and was never reported. Appending it here is a
        // deliberate output change on exactly the pages whose output was a silent lie.
        let truncated = if self.truncated {
            format!(" | truncated at {} MB", MAX_BYTES / 1_000_000)
        } else {
            String::new()
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
            truncated: resp.truncated,
        };
        Ok(Loaded { doc, prov })
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
            truncated: false,
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
                .contains("| alt.example.com redirected to www.example.com — rule appears dead |")
        );
    }

    #[test]
    fn truncation_is_reported_and_only_when_it_happened() {
        let mut p = prov("https://example.com/a");
        assert!(!p.to_string().contains("truncated"));
        p.truncated = true;
        assert!(
            p.to_string()
                .ends_with("cookie attempt(s) discarded | truncated at 5 MB]")
        );
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
