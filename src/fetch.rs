//! HTTP with privacy enforced as code, not as policy.
//!
//! Ported from the Phase 0 spike (`spike/fetch.py`), which validated these rules against
//! 150 real URLs. Observed there: 56% of responses tried to set a cookie. None were stored,
//! because no cookie jar exists — reqwest is built without its `cookies` feature, so the
//! capability is absent at compile time rather than merely unused.
//!
//! Errors are typed. The charter asks that a read-timeout stay distinct from a transport
//! fault — Phase 0 measured bot detection failing as a silent hang rather than a 403 — and
//! [`FetchError`] keeps that distinction *in the type system* rather than distinct-then-erased
//! by an `anyhow::bail!`. That is what lets a reader show the right diagnosis instead of a
//! generic network error.

use std::time::Duration;
use url::Url;

pub const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) \
                      Chrome/128.0.0.0 Safari/537.36";

pub const MAX_BYTES: usize = 5_000_000;
pub const MAX_REDIRECTS: usize = 5;
pub const TIMEOUT: Duration = Duration::from_secs(15);

/// An image is allowed to be bigger than a document, and the asymmetry is deliberate rather
/// than sloppy: article photography lands in the 1–10 MB range routinely, and by the time this
/// cap applies the user has already clicked to ask for the picture. The cap is not a privacy
/// control — the click settled disclosure. It bounds resident bytes during transfer and
/// nothing else; it does **not** raise the decoded-memory ceiling, which
/// `reader::image_decode` owns independently.
pub const MAX_IMAGE_BYTES: usize = 10_000_000;
/// 10 MB inside the document timeout would need 5.3 Mbit/s sustained, which tethered and rural
/// links do not clear.
pub const IMAGE_TIMEOUT: Duration = Duration::from_secs(30);

pub const ACCEPT_HTML: &str = "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8";
pub const ACCEPT_IMAGE: &str = "image/webp,image/avif,image/png,image/jpeg,image/gif,*/*;q=0.8";

/// What, if anything, a class of request discloses about the page being read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Referer {
    /// The absolute default for documents. No document fetch ever carries a Referer.
    None,
    /// Scheme and host of the page being read — **never** the path.
    ///
    /// `referer(false)` stays in the builder: reqwest still never attaches a Referer on its
    /// own, on any request or redirect hop. What this adds is one header, set deliberately, at
    /// one call site, under manual redirect control.
    ///
    /// The reason it exists is a failure hww cannot otherwise detect. Hotlink protection that
    /// answers 403 degrades safely — no picture, visible failure. Hotlink protection that
    /// answers **200 with a substitute image** degrades silently: a valid JPEG that decodes
    /// clean and renders in the reading column, with no status code, decode check, or
    /// heuristic available to tell it from the real one. Sending the origin removes the
    /// trigger; sending it only after a 403 cannot, because there is no 403 to react to.
    ///
    /// The origin adds nearly nothing to what the request already discloses — a CDN asked for
    /// a host's asset inferred the referring site from the path before we said a word. The
    /// *path* is the real leak, and it is the part never sent. Browsers converged on the same
    /// split (`strict-origin-when-cross-origin`), and hotlink checks compare hosts rather than
    /// paths, so origin-only clears them at full effectiveness.
    PageOrigin,
}

/// Per-request policy. One `Client`, several classes of request — so every privacy rule
/// (manual redirect inspection, downgrade refusal, `referer(false)`, cookie counting, the byte
/// cap) is inherited unchanged rather than re-implemented on a second path.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub accept: &'static str,
    pub max_bytes: usize,
    pub timeout: Duration,
    /// Phase 0's "a hang is bot detection" finding was measured on *document* fetches.
    /// Applying it to a subresource turns slow bandwidth into an accusation.
    pub timeout_is_block: bool,
    pub referer: Referer,
}

impl Limits {
    pub const HTML: Self = Self {
        accept: ACCEPT_HTML,
        max_bytes: MAX_BYTES,
        timeout: TIMEOUT,
        timeout_is_block: true,
        referer: Referer::None,
    };
    pub const IMAGE: Self = Self {
        accept: ACCEPT_IMAGE,
        max_bytes: MAX_IMAGE_BYTES,
        timeout: IMAGE_TIMEOUT,
        timeout_is_block: false,
        referer: Referer::PageOrigin,
    };
}

/// Why a body might be short, and how sure we are.
///
/// A partial body still renders — the cap and the timeout bound cost, they do not decide the
/// user has earned nothing, and the HTML path has always behaved this way. What must not
/// happen is an *unlabelled* partial, which the reader cannot tell from a bad image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Truncation {
    Complete,
    /// Hit the transfer cap, and `Content-Length` confirms there was more.
    AtCap(usize),
    /// Ended exactly at the cap with no `Content-Length` to check against. `len >= max_bytes`
    /// alone mislabels a body that is exactly the cap, so this hedges rather than asserts.
    MaybeAtCap(usize),
    /// Ran out of time mid-body. Whatever arrived is kept.
    Timeout(Duration),
}

impl Truncation {
    pub fn is_truncated(self) -> bool {
        !matches!(self, Truncation::Complete)
    }
}

#[derive(Debug)]
pub struct Response {
    pub final_url: Url,
    pub status: u16,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
    pub hops: Vec<Url>,
    /// Responses that *attempted* to set a cookie. Recorded for reporting only; nothing is stored.
    pub cookie_attempts: usize,
    pub truncation: Truncation,
}

impl Response {
    pub fn truncated(&self) -> bool {
        self.truncation.is_truncated()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    /// Phase 0 finding: bot detection fails as a silent read-timeout, not a 403.
    /// A hang is a block signal, not a network error, and must be surfaced as such.
    ///
    /// Measured on *document* fetches only. A subresource that runs out of time is
    /// [`FetchError::Timeout`] instead — accusing a CDN of blocking when the link is merely
    /// slow would be the same mistake in the other direction.
    #[error("timed out — treat as bot-block, not a network fault")]
    LikelyBlocked,
    /// Ran out of time on a request where a hang is *not* evidence of a block.
    #[error("timed out after {0:?}")]
    Timeout(Duration),
    #[error("refused https -> http downgrade at {0}")]
    SchemeDowngrade(Url),
    #[error("exceeded {MAX_REDIRECTS} redirects")]
    TooManyRedirects,
    /// A `Location` header that is not a URL. Its own arm because it is a server fault with a
    /// useful message, not a transport failure.
    #[error("unusable redirect target: {0}")]
    BadLocation(String),
    #[error(transparent)]
    Transport(#[from] reqwest::Error),
    /// The connection died partway through the body. Distinct from `Transport` because it is
    /// the arm that carries partial bytes in the subresource case.
    #[error("read failed: {0}")]
    Io(#[from] std::io::Error),
}

pub struct Fetcher {
    client: reqwest::blocking::Client,
}

impl Fetcher {
    pub fn new() -> Result<Self, FetchError> {
        let client = reqwest::blocking::Client::builder()
            .user_agent(UA)
            // Manual redirect handling: the policy below must inspect every hop.
            .redirect(reqwest::redirect::Policy::none())
            .timeout(TIMEOUT)
            .referer(false) // never send Referer
            .build()?;
        Ok(Self { client })
    }

    /// A document fetch: no Referer, HTML `Accept`, and a hang read as a bot-block.
    pub fn get(&self, url: &Url) -> Result<Response, FetchError> {
        self.get_with(url, None, &Limits::HTML)
    }

    /// `referrer` is the *page* the request is made on behalf of — never the target. Required
    /// when `limits.referer` is [`Referer::PageOrigin`]; a debug assertion catches the
    /// mismatch, and `None` degrades to sending no header rather than to a panic in a release
    /// build.
    pub fn get_with(
        &self,
        url: &Url,
        referrer: Option<&Url>,
        limits: &Limits,
    ) -> Result<Response, FetchError> {
        debug_assert!(
            limits.referer == Referer::None || referrer.is_some(),
            "a PageOrigin request needs the page it is made on behalf of"
        );
        let referer_value = referer_header(limits.referer, referrer);

        let mut current = url.clone();
        let mut hops = Vec::new();
        let mut cookie_attempts = 0usize;

        for _ in 0..=MAX_REDIRECTS {
            let mut req = self
                .client
                .get(current.clone())
                .header("Accept", limits.accept)
                .header("Accept-Language", "en-US,en;q=0.9")
                // Overrides the client default; no second `Client`, so nothing else changes.
                .timeout(limits.timeout);
            // Same value across every hop — CDNs that bounce to a signed URL break otherwise.
            if let Some(v) = &referer_value {
                req = req.header(reqwest::header::REFERER, v);
            }

            let resp = match req.send() {
                Ok(r) => r,
                Err(e) if e.is_timeout() => return Err(timeout_error(limits)),
                Err(e) => return Err(e.into()),
            };

            let status = resp.status();
            if resp.headers().contains_key(reqwest::header::SET_COOKIE) {
                cookie_attempts += 1; // counted, never stored
            }

            if status.is_redirection() {
                let Some(loc) = resp.headers().get(reqwest::header::LOCATION) else {
                    break;
                };
                let loc = loc.to_str().map_err(|_| {
                    FetchError::BadLocation(String::from_utf8_lossy(loc.as_bytes()).into_owned())
                })?;
                let next = current
                    .join(loc)
                    .map_err(|_| FetchError::BadLocation(loc.to_owned()))?;
                if current.scheme() == "https" && next.scheme() == "http" {
                    return Err(FetchError::SchemeDowngrade(next));
                }
                hops.push(next.clone());
                current = next;
                continue;
            }

            let content_type = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            // Present only when reqwest did not transparently decompress, so when it is here
            // it describes the same bytes `read_capped` counted.
            let content_length = resp
                .headers()
                .get(reqwest::header::CONTENT_LENGTH)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<usize>().ok());

            let (body, read_err) = read_capped(resp, limits.max_bytes);
            let truncation = classify(&body, content_length, read_err, limits)?;
            return Ok(Response {
                final_url: current,
                status: status.as_u16(),
                content_type,
                body,
                hops,
                cookie_attempts,
                truncation,
            });
        }
        Err(FetchError::TooManyRedirects)
    }
}

/// The Referer header's value, or `None` for no header at all.
///
/// Built from [`Url::origin`] so that no code path can emit a full URL even by mistake — the
/// path is the disclosure that matters, and this function is where "never the path" is a
/// property of the construction rather than a promise in a comment.
fn referer_header(policy: Referer, referrer: Option<&Url>) -> Option<String> {
    match (policy, referrer) {
        (Referer::PageOrigin, Some(page)) => {
            let origin = page.origin();
            // An opaque origin serializes to "null", which discloses nothing and helps nothing.
            origin.is_tuple().then(|| origin.ascii_serialization())
        }
        _ => None,
    }
}

fn timeout_error(limits: &Limits) -> FetchError {
    if limits.timeout_is_block {
        FetchError::LikelyBlocked
    } else {
        FetchError::Timeout(limits.timeout)
    }
}

/// Decide what a short body means — and, for a read that failed outright with nothing to show,
/// hand back the error instead.
fn classify(
    body: &[u8],
    content_length: Option<usize>,
    read_err: Option<std::io::Error>,
    limits: &Limits,
) -> Result<Truncation, FetchError> {
    if let Some(e) = read_err {
        let timed_out = io_is_timeout(&e);
        // Nothing arrived: there is no partial worth labelling, so this is simply a failure.
        if body.is_empty() {
            return Err(if timed_out {
                timeout_error(limits)
            } else {
                FetchError::Io(e)
            });
        }
        return Ok(if timed_out {
            Truncation::Timeout(limits.timeout)
        } else {
            Truncation::MaybeAtCap(limits.max_bytes)
        });
    }
    if body.len() < limits.max_bytes {
        return Ok(Truncation::Complete);
    }
    Ok(match content_length {
        Some(n) if n > body.len() => Truncation::AtCap(limits.max_bytes),
        Some(_) => Truncation::Complete,
        None => Truncation::MaybeAtCap(limits.max_bytes),
    })
}

fn io_is_timeout(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    ) || e.to_string().contains("timed out")
}

/// Read at most `max_bytes`, so a hostile or accidental multi-GB response cannot exhaust
/// memory — and **keep whatever arrived** when the read fails partway.
///
/// Discarding the buffer through `?` is what the old version did, and it is what makes a
/// half-loaded photograph impossible to show. Whether those bytes are worth rendering is the
/// caller's decision; throwing them away here removes the choice.
fn read_capped(
    resp: reqwest::blocking::Response,
    max_bytes: usize,
) -> (Vec<u8>, Option<std::io::Error>) {
    use std::io::Read;
    let mut buf = Vec::new();
    let err = resp.take(max_bytes as u64).read_to_end(&mut buf).err();
    (buf, err)
}

/// Decode bytes to text. Precedence: HTTP `charset` -> `<meta charset>` prescan -> UTF-8.
///
/// Phase 0 measured 100% UTF-8 across the sample and zero legacy encodings, so this is
/// correctness insurance rather than a hot path. It matters for the older web the client
/// is aimed at, which that sample happened not to reach.
pub fn decode(
    bytes: &[u8],
    content_type: Option<&str>,
) -> (String, &'static encoding_rs::Encoding) {
    let enc = content_type
        .and_then(charset_from_content_type)
        .or_else(|| meta_charset_prescan(bytes))
        .unwrap_or(encoding_rs::UTF_8);
    let (text, actual, _had_errors) = enc.decode(bytes);
    (text.into_owned(), actual)
}

fn charset_from_content_type(ct: &str) -> Option<&'static encoding_rs::Encoding> {
    let idx = ct.to_ascii_lowercase().find("charset=")?;
    let raw = ct[idx + 8..].trim().trim_matches(['"', '\'']);
    let label = raw.split(';').next()?.trim();
    encoding_rs::Encoding::for_label(label.as_bytes())
}

/// Scan the first 4 KiB for `charset=...`, mirroring the HTML spec's prescan window.
fn meta_charset_prescan(bytes: &[u8]) -> Option<&'static encoding_rs::Encoding> {
    let window = &bytes[..bytes.len().min(4096)];
    let hay = String::from_utf8_lossy(window).to_ascii_lowercase();
    let idx = hay.find("charset=")?;
    let rest = &hay[idx + 8..];
    let label: String = rest
        .trim_start_matches(['"', '\''])
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    encoding_rs::Encoding::for_label(label.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reader shares one `Fetcher` across worker threads. If this stops holding, the
    /// threading model in `reader::ui::net` stops compiling — which is the point of asserting
    /// it here, next to the builder that could quietly break it.
    #[test]
    fn fetcher_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Fetcher>();
    }

    /// The whole point of the amendment: origin, never path. If this ever fails, the client is
    /// telling a CDN which article is being read.
    #[test]
    fn referer_is_origin_only_and_never_carries_a_path() {
        let page = Url::parse("https://example.com/2026/the-article?utm=x#section").unwrap();
        let v = referer_header(Referer::PageOrigin, Some(&page)).unwrap();
        assert_eq!(v, "https://example.com");
        assert!(!v.contains("the-article"));
        assert!(!v.contains('?') && !v.contains('#'));
        assert_eq!(v.matches('/').count(), 2); // exactly the "https://" pair
    }

    #[test]
    fn documents_never_carry_a_referer() {
        let page = Url::parse("https://example.com/a").unwrap();
        assert_eq!(referer_header(Referer::None, Some(&page)), None);
        assert_eq!(Limits::HTML.referer, Referer::None);
    }

    /// A non-web page has an opaque origin; "null" would disclose nothing and help nothing.
    #[test]
    fn opaque_origin_sends_no_referer() {
        let page = Url::parse("file:///home/reader/notes.html").unwrap();
        assert_eq!(referer_header(Referer::PageOrigin, Some(&page)), None);
    }

    /// A hang on a document is a bot-block; a hang on a subresource is bandwidth. Keeping the
    /// two arms distinct is the same reasoning that made `LikelyBlocked` distinct at all.
    #[test]
    fn a_slow_image_is_not_accused_of_blocking() {
        assert!(matches!(
            timeout_error(&Limits::HTML),
            FetchError::LikelyBlocked
        ));
        assert!(matches!(
            timeout_error(&Limits::IMAGE),
            FetchError::Timeout(d) if d == IMAGE_TIMEOUT
        ));
    }

    #[test]
    fn a_body_exactly_at_the_cap_is_hedged_not_asserted() {
        let body = vec![0u8; MAX_BYTES];
        // Content-Length agrees the body ended: not truncated at all.
        assert_eq!(
            classify(&body, Some(MAX_BYTES), None, &Limits::HTML).unwrap(),
            Truncation::Complete
        );
        // Content-Length says there was more: certainly cut.
        assert_eq!(
            classify(&body, Some(MAX_BYTES + 1), None, &Limits::HTML).unwrap(),
            Truncation::AtCap(MAX_BYTES)
        );
        // Nothing to check against: say so rather than guess.
        assert_eq!(
            classify(&body, None, None, &Limits::HTML).unwrap(),
            Truncation::MaybeAtCap(MAX_BYTES)
        );
        assert_eq!(
            classify(&body[..10], None, None, &Limits::HTML).unwrap(),
            Truncation::Complete
        );
    }

    /// A timeout partway through keeps what arrived; a timeout before anything arrived is
    /// simply a failure, because there is no partial worth labelling.
    #[test]
    fn a_timed_out_read_keeps_its_bytes() {
        let timed_out = || std::io::Error::new(std::io::ErrorKind::TimedOut, "timed out");
        assert_eq!(
            classify(b"half a jpeg", None, Some(timed_out()), &Limits::IMAGE).unwrap(),
            Truncation::Timeout(IMAGE_TIMEOUT)
        );
        assert!(matches!(
            classify(b"", None, Some(timed_out()), &Limits::IMAGE),
            Err(FetchError::Timeout(_))
        ));
    }

    #[test]
    fn charset_from_http_header_wins() {
        let (_, enc) = decode("abc".as_bytes(), Some("text/html; charset=iso-8859-1"));
        assert_eq!(enc.name(), "windows-1252"); // encoding_rs maps latin1 to its superset
    }

    #[test]
    fn meta_prescan_finds_legacy_encoding() {
        let html = b"<html><head><meta charset=\"windows-1251\"><title>x</title></head>";
        let (_, enc) = decode(html, None);
        assert_eq!(enc.name(), "windows-1251");
    }

    #[test]
    fn defaults_to_utf8_when_unlabelled() {
        let (text, enc) = decode("plain ascii".as_bytes(), None);
        assert_eq!(enc.name(), "UTF-8");
        assert_eq!(text, "plain ascii");
    }

    #[test]
    fn prescan_window_is_bounded() {
        // A charset declared past the 4 KiB prescan window must not be honoured.
        let mut html = vec![b' '; 5000];
        html.extend_from_slice(b"<meta charset=\"windows-1251\">");
        let (_, enc) = decode(&html, None);
        assert_eq!(enc.name(), "UTF-8");
    }
}
