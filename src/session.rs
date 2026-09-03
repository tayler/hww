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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadOptions {
    /// Apply the builtin rewrite table. `--no-rewrite` and the reader's bare reload clear it.
    pub rewrite: bool,
    /// Apply the builtin profile table to the page that arrives. `--no-profile` and the
    /// reader's bare reload clear it.
    pub profile: bool,
    /// Read a known engine's result page as results rather than as prose. `--no-search` and
    /// the reader's bare reload clears it, which is how you see the SERP the engine actually
    /// sent when a shape looks wrong.
    pub search: bool,
    /// Read an RSS or Atom response as a feed rather than as prose. `--no-feed` and the
    /// reader's bare reload clear it, which is how you see the raw XML the generic extractor
    /// makes of a feed — the only way to tell a stale reading from a stale publisher.
    ///
    /// Not keyed on a host, unlike the three tables beside it: a feed is recognised from its
    /// own first bytes. It is a switch here because everything `BARE` turns off is something
    /// that reads a response as other than prose, and a feed reader is the fourth of those.
    pub feed: bool,
}

impl Default for LoadOptions {
    fn default() -> Self {
        Self {
            rewrite: true,
            profile: true,
            search: true,
            feed: true,
        }
    }
}

impl LoadOptions {
    /// None of them: the page as the host sent it, read by the extractor unaided.
    pub const BARE: LoadOptions = LoadOptions {
        rewrite: false,
        profile: false,
        search: false,
        feed: false,
    };
}

/// Emitted *before* the request is dispatched. Charter point 4 is unconditional: a fetch that
/// fails or hangs must not be able to swallow the fact that hww went somewhere the user did
/// not type.
#[derive(Debug, Clone)]
pub enum Rewrite {
    Applied {
        from: Url,
        to: Url,
    },
    /// A search. The user named terms, not a host, so "same operator" cannot apply and the
    /// disclosure does the whole job: the host and the words both appear before the request
    /// goes out, and a hang cannot swallow them.
    Searched {
        host: &'static str,
        terms: String,
    },
}

impl std::fmt::Display for Rewrite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Rewrite::Applied { from, to } => write!(
                f,
                "[rewrote {} -> {}]",
                from.host_str().unwrap_or("?"),
                to.host_str().unwrap_or("?")
            ),
            Rewrite::Searched { host, terms } => {
                write!(f, "[searching {host} for {terms:?}]")
            }
        }
    }
}

/// Everything the fetch disclosed, in one value. The reader shows it in the status strip and
/// headless `--why` prints it on stderr; both read the same fields, so neither can drift into a
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
    /// What the site profile did to the page, if one applied. `None` under `--no-profile` or
    /// for a host with no profile. Not in `Display`: headless `--why` prints it on its own line,
    /// so the provenance line stays what it was.
    pub profile: Option<crate::profile::ProfileReport>,
    /// The response was read as a feed, and which one. `None` under `--no-feed` and for every
    /// page that is not a feed.
    ///
    /// One field, three consumers, which is the whole reason it lives here rather than as an
    /// `Explanation::feed` beside `Explanation::search`: `notice::about_page` gates the
    /// thin-text caution on it, `pageinfo::rows` draws it, and `bin/hww.rs` prints it. Two
    /// representations of one fact is the drift this struct's doc comment forbids.
    ///
    /// It matters most to the first of those. `ir::block_text_len` counts a `Figure` as its
    /// caption alone, so a picture-only feed scores under `ir::THIN_TEXT` and draws the
    /// JavaScript caution over a feed that parsed perfectly. The IR must not change to fix it —
    /// text length has one currency — so the pipeline says the page was a feed instead.
    pub feed: Option<crate::feed::Report>,
    /// The document is a run of tweets: `Block::Tweets`, the shape X serves and the one social
    /// host measured.
    ///
    /// Here for exactly the reason `feed` above is, and against the same failure. A tweet is a
    /// few dozen words by design, so a page that parsed perfectly still measures under
    /// `ir::THIN_TEXT` and would draw the JavaScript caution over content hww has in hand — the
    /// confidently-wrong diagnosis that `tweet.rs` was written to end. The IR does not change to
    /// fix it, because text length has one currency; the pipeline says the page was tweets
    /// instead, and `notice::about_page` and `pageinfo::rows` both read this one field.
    pub tweets: bool,
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

impl Provenance {
    /// The provenance of a page hww built itself: an address, and zeros.
    ///
    /// Every other field describes a request, and this page made none, so there is nothing
    /// honest to put in them. **The zeros are only safe because `pageinfo::Arrival::Built` stops
    /// them being drawn**; a `status: 200` here instead would be the plausible-looking failure
    /// `docs/findings.md` names three times, with the panel reporting that a server answered a
    /// request that was never sent.
    ///
    /// It lives here, beside the fields, rather than in the reader: a `Provenance` is thirteen
    /// fields and two `Url`s, and the copy made at a call site is the copy that drifts.
    pub fn built(url: Url) -> Self {
        Self {
            requested: url.clone(),
            rewritten_to: None,
            rule_appears_dead: false,
            status: 0,
            encoding: "UTF-8",
            final_url: url,
            bytes: 0,
            chars: 0,
            hops: vec![],
            cookie_attempts: 0,
            truncation: Truncation::Complete,
            profile: None,
            feed: None,
            tweets: false,
        }
    }
}

/// A plain 200 `Provenance`, for tests in this crate that need one to vary.
///
/// It lives here rather than in each test module because a `Provenance` is eleven fields and
/// two `Url`s: copied into every module that reports on one, the copies drift, and a test then
/// asserts about a page shape the fetcher can no longer produce.
#[cfg(test)]
impl Provenance {
    pub(crate) fn fixture(url: &str) -> Self {
        Self {
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
            profile: None,
            feed: None,
            tweets: false,
        }
    }
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
    /// The engine answered with a challenge instead of results.
    ///
    /// Deliberately not [`FetchError::LikelyBlocked`], which means a document read-timeout and
    /// nothing else. The measured shape here is the opposite: a prompt 2xx carrying a CAPTCHA.
    /// Folding them together would cost the distinction Phase 0 paid for and would report a
    /// hung host and a challenging one with the same wording and the same useless remedy.
    #[error("{engine} asked for a human check instead of answering")]
    SearchBlocked { engine: &'static str },
    /// The engine answered and found nothing. Not a failure of the fetch, but the reading
    /// column carries page content only, so "nothing" has to be said as a page remark.
    #[error("{engine} found nothing for {terms:?}")]
    SearchEmpty { engine: &'static str, terms: String },
    /// The feed parsed and carried no items. Not a failure of the fetch, and the same argument
    /// as [`Self::SearchEmpty`]: the reading column carries page content only, so "nothing" has
    /// to be said as a page remark rather than drawn as an empty run of entries.
    ///
    /// A `String` rather than an `Option<String>`, because `thiserror`'s `#[error(...)]`
    /// interpolates unconditionally. `session::load` fills a feed that named no title with the
    /// host it came from, which is a name and not a guess.
    #[error("{title} carried no entries")]
    FeedEmpty { title: String },
    /// It said it was a feed and would not parse.
    ///
    /// `why` is the parser's message and position, and it is **publisher-controlled**: it
    /// embeds the offending entity name or namespace prefix. `feed::read` caps its length and
    /// `bin/hww.rs` sanitizes everything it prints, because a terminal reads an escape sequence
    /// in one of those as a command.
    ///
    /// `truncated` is whether hww is the one that broke it, and the remark turns on it. The
    /// transfer cap and the read clock both cut a document mid-element, and XML that stops
    /// partway is unclosed through no fault of the publisher's — `feed.rs` measured one feed in
    /// 28 over the 5 MB cap. Without this the failure page told a reader to wait for a
    /// generator fix that was never coming, for a document hww had truncated itself.
    #[error("this feed would not parse: {why}")]
    FeedUnreadable { why: String, truncated: bool },
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
                | "application/x-rss+xml"
                | "application/atom+xml"
                | "application/rdf+xml"
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
    /// `notify` is called before the request is dispatched, never after. Headless `--why`
    /// passes an `eprintln!`; the reader's sends the notice to the UI thread and repaints, so the
    /// line is on screen for the whole of a 15 s hang instead of arriving with the corpse.
    pub fn load(
        &self,
        requested: &Url,
        opts: &LoadOptions,
        notify: &mut dyn FnMut(&Rewrite),
    ) -> Result<Loaded, LoadError> {
        let f = self.fetch_page(requested, opts, notify)?;
        if let Some((engine, terms)) = f.search(opts) {
            match crate::search::read(&f.source, &f.resp.final_url, engine, f.resp.status) {
                crate::search::Outcome::Results(entries) => {
                    let doc = crate::search::document(&f.resp.final_url, &terms, entries);
                    let chars = doc.text_len();
                    return Ok(Loaded {
                        doc,
                        prov: f.provenance(requested, chars, None),
                    });
                }
                crate::search::Outcome::Blocked => {
                    return Err(LoadError::SearchBlocked {
                        engine: engine.label,
                    });
                }
                crate::search::Outcome::Empty => {
                    return Err(LoadError::SearchEmpty {
                        engine: engine.label,
                        terms,
                    });
                }
                // The shape has gone stale, or this is a page it was not written for. The
                // generic extractor reads every one of these engines acceptably, so falling
                // through costs a rougher page rather than a blank one.
                crate::search::Outcome::Unrecognized => {}
            }
        }
        // After search, because a result page is never a feed, and before the profile lookup,
        // because a profile keys on a host and describes an HTML tree. No method on `Fetched`:
        // `search` and `profile` exist to consult a `sites` table keyed on the final URL, and a
        // feed is recognised from its own bytes.
        if opts.feed {
            match crate::feed::read(&f.source, &f.resp.final_url) {
                crate::feed::Outcome::Feed(doc, report) => {
                    let chars = doc.text_len();
                    let mut prov = f.provenance(requested, chars, None);
                    prov.feed = Some(report);
                    return Ok(Loaded { doc, prov });
                }
                crate::feed::Outcome::Empty { title } => {
                    return Err(LoadError::FeedEmpty {
                        title: named(&title, &f.resp.final_url),
                    });
                }
                crate::feed::Outcome::Unreadable(why) => {
                    return Err(LoadError::FeedUnreadable {
                        why,
                        truncated: f.resp.truncation != Truncation::Complete,
                    });
                }
                // Not a feed at all, including an RSS 1.0 / RDF document. The generic extractor
                // has it, which is a rougher page rather than a blank one.
                crate::feed::Outcome::NotAFeed => {}
            }
        }
        let (host, profile) = f.profile(opts);
        let x = html::extract_with_profile(&f.source, &f.resp.final_url, profile);
        let report = x.profile.map(|mut r| {
            r.host = host.unwrap_or_default().to_owned();
            r
        });
        let doc = x.doc;
        let mut prov = f.provenance(requested, doc.text_len(), report);
        // Asked of the document rather than tracked through the extractor: one question, one
        // answer, and `explain` below reaches the same fact by the same route.
        prov.tweets = doc.blocks.iter().any(|b| matches!(b, ir::Block::Tweets(_)));
        Ok(Loaded { doc, prov })
    }

    /// [`Self::load`], keeping the extractor's account of itself instead of the document.
    /// Same fetch, same notice, same provenance: `--why` is the reader's own pipeline
    /// explaining itself, not a second one.
    pub fn explain(
        &self,
        requested: &Url,
        opts: &LoadOptions,
        notify: &mut dyn FnMut(&Rewrite),
    ) -> Result<(html::Explanation, Provenance), LoadError> {
        let f = self.fetch_page(requested, opts, notify)?;
        let (host, profile) = f.profile(opts);
        let mut why = html::explain_with_profile(&f.source, &f.resp.final_url, profile);
        if let Some(r) = &mut why.profile {
            r.host = host.unwrap_or_default().to_owned();
        }
        // `load` may never reach the extractor at all on these pages. Saying which engine
        // matched, and what reading its shape produced, is the difference between `--why`
        // explaining the pipeline and `--why` explaining a path that did not run.
        if let Some((engine, terms)) = f.search(opts) {
            why.search = Some(
                match crate::search::read(&f.source, &f.resp.final_url, engine, f.resp.status) {
                    crate::search::Outcome::Results(entries) => format!(
                        "{} shape matched {} result(s) for {terms:?}; the candidates below did not run",
                        engine.label,
                        entries.len()
                    ),
                    crate::search::Outcome::Blocked => {
                        format!("{} answered a challenge, not results", engine.label)
                    }
                    crate::search::Outcome::Empty => {
                        format!("{} answered, and found nothing for {terms:?}", engine.label)
                    }
                    crate::search::Outcome::Unrecognized => format!(
                        "{} shape matched nothing; fell back to the extractor below",
                        engine.label
                    ),
                },
            );
        }
        // `explain` does not go through `load`, so without this call `prov.feed` is `None` on
        // precisely the path built for diagnosis, and `--why` would rank content-root candidates
        // for a feed while saying nothing about the feed. The duplicate parse is the price, on
        // a path that renders no page and opens no window.
        let feed = opts
            .feed
            .then(|| crate::feed::read(&f.source, &f.resp.final_url))
            .and_then(|o| match o {
                crate::feed::Outcome::Feed(_, r) => Some(r),
                _ => None,
            });
        let mut prov = f.provenance(requested, why.text_len, why.profile.clone());
        prov.feed = feed;
        prov.tweets = why.tweets;
        Ok((why, prov))
    }

    /// Rewrite, notify, fetch, gate on content type, decode. Everything `load` and `explain`
    /// share, which is everything but the last step.
    fn fetch_page(
        &self,
        requested: &Url,
        opts: &LoadOptions,
        notify: &mut dyn FnMut(&Rewrite),
    ) -> Result<Fetched, LoadError> {
        let url = if opts.rewrite {
            crate::sites::apply(requested)
        } else {
            requested.clone()
        };
        let rewritten = url != *requested;
        if rewritten {
            notify(&Rewrite::Applied {
                from: requested.clone(),
                to: url.clone(),
            });
        }
        if let Some(n) = search_notice(&url) {
            notify(&n);
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
        Ok(Fetched {
            url,
            rewritten,
            resp,
            source,
            encoding: enc.name(),
        })
    }
}

/// A feed's own title, or the host that served it when the feed named none.
///
/// A remark that opens with a blank is a remark that reads as broken. The host is a name the
/// reader can act on and not a guess about the publication.
fn named(title: &str, url: &Url) -> String {
    match title.trim() {
        "" => url.host_str().unwrap_or("This feed").to_owned(),
        t => t.to_owned(),
    }
}

/// The disclosure a request to a search engine owes, before it is dispatched.
///
/// Keyed on the URL about to be requested rather than the one that arrives: the charter's
/// promise is that nothing leaves silently, and a notice that waited for the response could not
/// keep it.
///
/// It takes no [`LoadOptions`], and that is the rule rather than an omission.
/// `LoadOptions::search` decides how the *response* is read, while `search::resolve_input` has
/// already turned the reader's words into this URL whatever the flag says. Gating the
/// disclosure on it would make `--no-search` and a bare reload the two quietest ways to search,
/// which is the opposite of what both are for.
fn search_notice(url: &Url) -> Option<Rewrite> {
    let engine = crate::sites::engine_for(url)?;
    let terms = crate::search::terms_of(engine, url)?;
    Some(Rewrite::Searched {
        host: engine.host,
        terms,
    })
}

/// A page after decode and before extraction.
struct Fetched {
    /// What was requested, after the rewrite table had its say.
    url: Url,
    rewritten: bool,
    resp: fetch::Response,
    source: String,
    encoding: &'static str,
}

impl Fetched {
    /// The engine whose result page arrived, and the terms it was asked for.
    ///
    /// Keyed on the final URL, as `profile` is, so a redirect that lands somewhere else is not
    /// read as results. The pre-dispatch notice is keyed on the *requested* URL instead; they
    /// differ only when an engine bounces, which is itself worth seeing rather than hiding.
    fn search(&self, opts: &LoadOptions) -> Option<(&'static crate::sites::SearchEngine, String)> {
        if !opts.search {
            return None;
        }
        let engine = crate::sites::engine_for(&self.resp.final_url)?;
        let terms = crate::search::terms_of(engine, &self.resp.final_url)?;
        Some((engine, terms))
    }

    /// The profile for the page that arrived, keyed on the final URL: a profile describes
    /// bytes in hand, and the rewrite table has already had its say on the entry URL.
    fn profile(
        &self,
        opts: &LoadOptions,
    ) -> (Option<&'static str>, &'static crate::profile::Profile) {
        if !opts.profile {
            return (None, &crate::profile::Profile::NONE);
        }
        match crate::sites::profile_for(&self.resp.final_url) {
            Some((host, p)) => (Some(host), p),
            None => (None, &crate::profile::Profile::NONE),
        }
    }

    fn provenance(
        self,
        requested: &Url,
        chars: usize,
        profile: Option<crate::profile::ProfileReport>,
    ) -> Provenance {
        // The bounce is usually to a sibling of the requested host rather than to that host
        // itself, so comparing against the *request* would miss the common case.
        let rule_appears_dead =
            self.rewritten && self.resp.final_url.host_str() != self.url.host_str();
        Provenance {
            requested: requested.clone(),
            rewritten_to: self.rewritten.then_some(self.url),
            rule_appears_dead,
            status: self.resp.status,
            encoding: self.encoding,
            final_url: self.resp.final_url,
            bytes: self.resp.body.len(),
            chars,
            hops: self.resp.hops,
            cookie_attempts: self.resp.cookie_attempts,
            truncation: self.resp.truncation,
            profile,
            // Filled by the one arm that read a feed, on the value it gets back. This method
            // already takes three arguments, two of them `Option`s, and a fourth would be one
            // more thing to pass `None` to from every caller that never saw a feed.
            feed: None,
            tweets: false,
        }
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
    /// Carried rather than dropped, because a document and an image disagree about what a
    /// non-2xx means. A 404 body is still a page and the reader shows it; a 404 body is never
    /// an image, and handing it to the decoder turns "this address has no picture" into
    /// "unrecognised format", which is a retryable answer to a permanent question.
    pub status: u16,
}

impl Session {
    /// Fetch an image on behalf of `page`. Reached from an explicit click on an article
    /// image, and from the masthead favicon auto-load.
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
            status: resp.status,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prov(url: &str) -> Provenance {
        Provenance::fixture(url)
    }

    #[test]
    fn provenance_line_matches_the_cli_format() {
        assert_eq!(
            prov("https://example.com/a").to_string(),
            "[200 UTF-8 | https://example.com/a | 1234 bytes -> 567 chars | \
             0 redirect(s) | 2 cookie attempt(s) discarded]"
        );
    }

    /// The branch headless `--why` had and no test ever reached.
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
            // Every type a feed is served as. RDF is here because `feed::read` answers
            // `NotAFeed` for an RSS 1.0 root and lets the generic extractor have it: refusing
            // the content type would turn that fallback into a failure screen. `x-rss+xml` is
            // what a number of older publishers still send.
            Some("application/rss+xml"),
            Some("application/x-rss+xml"),
            Some("application/atom+xml"),
            Some("application/rdf+xml"),
            Some("application/xml; charset=utf-8"),
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
        let n = Rewrite::Applied {
            from: Url::parse("https://reddit.com/r/rust/x").unwrap(),
            to: Url::parse("https://old.reddit.com/r/rust/x").unwrap(),
        };
        assert_eq!(n.to_string(), "[rewrote reddit.com -> old.reddit.com]");
    }

    /// Every shipped profile, run over its own fixture with and without the profile: every
    /// field matched, and the document changed. A profile that no longer does anything on the
    /// markup it was written for fails here before it ships stale.
    /// A search is disclosed before the request goes out, naming both the host and the words.
    /// The words matter: "hww went to a host you did not type" is only half the disclosure when
    /// what it carried was your query.
    #[test]
    fn a_search_notice_names_the_host_and_the_terms() {
        // The host comes from the table, not from a literal here: `sites.rs` is the only file
        // in the crate that spells one, tests included.
        let engine = crate::sites::engine_by_id(crate::sites::EngineId::default());
        let n = Rewrite::Searched {
            host: engine.host,
            terms: "rust ownership".to_owned(),
        };
        let line = n.to_string();
        assert!(line.contains(engine.host), "{line}");
        assert!(line.contains("rust ownership"), "{line}");
    }

    /// The search disclosure is owed by the request, not by how the answer will be read.
    ///
    /// `--no-search` and a bare reload turn off *reading* a result page as results. They used to
    /// turn off saying that a search was leaving, which made them the two quietest ways to
    /// send a query to a third party. The signature is the guarantee: no `LoadOptions` reaches
    /// this decision.
    #[test]
    fn the_search_disclosure_does_not_depend_on_the_options() {
        let engine = crate::sites::engine_by_id(crate::sites::EngineId::default());
        let searched = crate::search::query_url(engine, "rust ownership");
        let line = search_notice(&searched)
            .expect("a query URL owes a notice")
            .to_string();
        assert!(line.contains(engine.host), "{line}");
        assert!(line.contains("rust ownership"), "{line}");
        // An engine's other pages, and a query with no terms in it, are not searches.
        let front = Url::parse(&format!("https://{}/", engine.host)).unwrap();
        assert!(search_notice(&front).is_none());
        assert!(search_notice(&crate::search::query_url(engine, "")).is_none());
        assert!(search_notice(&Url::parse("https://lorem.test/one").unwrap()).is_none());
    }

    /// A bare reload and `--no-search` are how you see the result page the engine actually sent.
    /// If `BARE` left search on, a stale shape would be unfalsifiable from inside the reader.
    #[test]
    fn bare_reads_a_result_page_as_a_page() {
        const { assert!(!LoadOptions::BARE.search) };
        assert!(LoadOptions::default().search);
    }

    /// The same argument for feeds, and it is the sharper one: the generic extractor reads a
    /// result page acceptably, and reads an XML document as nonsense. A bare reload on a feed is
    /// how you see what `feed::read` was handed, which is the only way to tell a stale reading
    /// from a publisher who changed their generator.
    ///
    /// Asserted by hand because neither this test nor
    /// `bare_options_apply_neither_table_and_the_default_applies_both` is exhaustive over the
    /// struct: a fourth field could be added to `BARE` as `true` and nothing would say so.
    #[test]
    fn bare_reads_a_feed_as_a_page() {
        const { assert!(!LoadOptions::BARE.feed) };
        assert!(LoadOptions::default().feed);
    }

    /// Every engine's own fixture reads as results through the same call `load` makes, and
    /// produces a document made of entries rather than prose.
    #[test]
    fn every_engine_fixture_becomes_entries() {
        for e in crate::sites::ENGINES {
            let url = crate::search::query_url(e, "lorem ipsum");
            let crate::search::Outcome::Results(entries) =
                crate::search::read(e.fixture, &url, e, 200)
            else {
                panic!("{:?}: its own fixture did not read as results", e.id);
            };
            let doc = crate::search::document(&url, "lorem ipsum", entries);
            assert!(
                matches!(doc.blocks.as_slice(), [ir::Block::Entries(es)] if es.len() >= 3),
                "{:?}: document is not a run of entries",
                e.id
            );
            assert!(doc.text_len() > 0);
        }
    }

    /// Every feed fixture reads through the same call `load` makes, and produces a document of
    /// entries rather than prose. Mirrors `every_engine_fixture_becomes_entries`: a shape that
    /// passes its own parser and falls over in the pipeline fails here instead of on a screen.
    #[test]
    fn every_feed_fixture_becomes_entries() {
        let url = Url::parse("https://example.com/feed.xml").unwrap();
        for f in crate::feed::FIXTURES {
            let crate::feed::Outcome::Feed(doc, report) = crate::feed::read(f.source, &url) else {
                panic!("{}: its own fixture did not read as a feed", f.name);
            };
            assert_eq!(report.entries, f.entries, "{}", f.name);
            assert!(
                matches!(doc.blocks.as_slice(), [ir::Block::Entries(es)] if es.len() == f.entries),
                "{}: document is not a run of entries",
                f.name
            );
            assert!(doc.text_len() > 0, "{}", f.name);
        }
    }

    /// The feed branch sits after search and before the profile lookup, and `BARE` turns it
    /// off, so the four ways a response can be read as other than prose are one list.
    #[test]
    fn a_feed_is_read_as_a_feed_only_when_the_option_says_so() {
        let url = Url::parse("https://example.com/feed.xml").unwrap();
        let src = crate::feed::FIXTURES[0].source;
        assert!(matches!(
            crate::feed::read(src, &url),
            crate::feed::Outcome::Feed(..)
        ));
        // And an ordinary page never diverts into it, whatever the option says.
        let page = "<!DOCTYPE html><html><body><p>a feed of news</p></body></html>";
        assert!(matches!(
            crate::feed::read(page, &url),
            crate::feed::Outcome::NotAFeed
        ));
    }

    #[test]
    fn every_profile_earns_its_keep() {
        for p in crate::sites::PROFILES {
            let url = Url::parse(&format!(
                "https://{}{}",
                p.host,
                p.path_contains.unwrap_or("/")
            ))
            .unwrap();
            let plain = html::extract(p.fixture, &url);
            let x = html::extract_with_profile(p.fixture, &url, &p.profile);
            let report = x.profile.unwrap_or_else(|| panic!("{}: no report", p.host));
            for f in &report.fields {
                assert!(
                    matches!(f.outcome, crate::profile::Outcome::Matched(_)),
                    "{}: {} {}",
                    p.host,
                    f.field,
                    f.outcome
                );
            }
            assert_ne!(
                serde_json::to_string(&plain).unwrap(),
                serde_json::to_string(&x.doc).unwrap(),
                "{}: the profile changed nothing on its fixture",
                p.host
            );
            // And the lookup reaches it.
            assert!(
                crate::sites::profile_for(&url).is_some_and(|(h, _)| h == p.host),
                "{}: profile_for does not find it at {url}",
                p.host
            );
        }
    }

    #[test]
    fn bare_options_apply_neither_table_and_the_default_applies_both() {
        let bare = LoadOptions::BARE;
        let dflt = LoadOptions::default();
        assert_eq!(
            (bare.rewrite, bare.profile, bare.feed),
            (false, false, false)
        );
        assert_eq!((dflt.rewrite, dflt.profile, dflt.feed), (true, true, true));
    }
}
