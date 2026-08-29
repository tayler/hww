//! HTTP with privacy enforced as code, not as policy.
//!
//! Ported from the Phase 0 spike (`spike/fetch.py`), which validated these rules against
//! 150 real URLs. Observed there: 56% of responses tried to set a cookie. None were stored,
//! because no cookie jar exists: reqwest is built without its `cookies` feature, so the
//! capability is absent at compile time rather than merely unused.
//!
//! Errors are typed. The charter asks that a read-timeout stay distinct from a transport
//! fault (Phase 0 measured bot detection failing as a silent hang rather than a 403), and
//! [`FetchError`] keeps that distinction *in the type system* rather than distinct-then-erased
//! by an `anyhow::bail!`. That is what lets a reader show the right diagnosis instead of a
//! generic network error.

use std::time::{Duration, Instant};
use url::Url;

pub const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) \
                      Chrome/128.0.0.0 Safari/537.36";

pub const MAX_BYTES: usize = 5_000_000;
pub const MAX_REDIRECTS: usize = 5;
pub const TIMEOUT: Duration = Duration::from_secs(15);

/// An image is allowed to be bigger than a document, and the asymmetry is deliberate rather
/// than sloppy: article photography lands in the 1–10 MB range routinely, and by the time this
/// cap applies the user has already clicked to ask for the picture. The cap is not a privacy
/// control; the click settled disclosure. It bounds resident bytes during transfer and
/// nothing else; it does **not** raise the decoded-memory ceiling, which
/// `reader::image_decode` owns independently.
pub const MAX_IMAGE_BYTES: usize = 10_000_000;
/// 10 MB inside the document timeout would need 5.3 Mbit/s sustained, which tethered and rural
/// links do not clear.
pub const IMAGE_TIMEOUT: Duration = Duration::from_secs(30);

pub const ACCEPT_HTML: &str = "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8";
/// Never advertise a format the decoder declines.
///
/// This listed `image/avif` while `image_decode` refuses AVIF, so a server doing content
/// negotiation was being *asked* for the one format hww cannot show, and the decline became a
/// wasted transfer the reader had already been told about. Removing it is the cheapest way to
/// turn current failures into pictures.
///
/// `*/*;q=0.8` stays: too many servers ignore `Accept` entirely, and refusing them the
/// fallback loses images that decode. It sorts below the explicit entries, so a server that
/// does negotiate now prefers one of them.
pub const ACCEPT_IMAGE: &str = "image/webp,image/png,image/jpeg,image/gif,*/*;q=0.8";

/// What, if anything, a class of request discloses about the page being read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Referer {
    /// The absolute default for documents. No document fetch ever carries a Referer.
    None,
    /// Scheme and host of the page being read, **never** the path.
    ///
    /// `referer(false)` stays in the builder: reqwest still never attaches a Referer on its
    /// own, on any request or redirect hop. What this adds is one header, set deliberately, at
    /// one call site, under manual redirect control.
    ///
    /// The reason it exists is a failure hww cannot otherwise detect. Hotlink protection that
    /// answers 403 degrades safely: no picture, visible failure. Hotlink protection that
    /// answers **200 with a substitute image** degrades silently: a valid JPEG that decodes
    /// clean and renders in the reading column, with no status code, decode check, or
    /// heuristic available to tell it from the real one. Sending the origin removes the
    /// trigger; sending it only after a 403 cannot, because there is no 403 to react to.
    ///
    /// The origin adds nearly nothing to what the request already discloses: a CDN asked for
    /// a host's asset inferred the referring site from the path before we said a word. The
    /// *path* is the real leak, and it is the part never sent. Browsers converged on the same
    /// split (`strict-origin-when-cross-origin`), and hotlink checks compare hosts rather than
    /// paths, so origin-only clears them at full effectiveness.
    PageOrigin,
}

/// Per-request policy. One `Client`, several classes of request, so every privacy rule
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
/// A partial body still renders: the cap and the timeout bound cost, they do not decide the
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
    /// The connection died mid-body, well short of the cap. Carries what did arrive, because
    /// naming a limit that was never approached ("may be truncated at 5 MB" after 3 KB) both
    /// misstates the size and hides the real cause.
    Incomplete(usize),
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
    /// [`FetchError::Timeout`] instead; accusing a CDN of blocking when the link is merely
    /// slow would be the same mistake in the other direction.
    #[error("timed out; treat as bot-block, not a network fault")]
    LikelyBlocked,
    /// Ran out of time on a request where a hang is *not* evidence of a block.
    #[error("timed out after {0:?}")]
    Timeout(Duration),
    #[error("refused https -> http downgrade at {0}")]
    SchemeDowngrade(Url),
    /// Carries the hops, because a redirect loop is usually a consent wall and the chain is
    /// what shows it.
    #[error("exceeded {MAX_REDIRECTS} redirects")]
    TooManyRedirects(Vec<Url>),
    /// A `Location` header that is not a URL. Its own arm because it is a server fault with a
    /// useful message, not a transport failure.
    #[error("unusable redirect target: {0}")]
    BadLocation(String),
    /// A 3xx with no `Location` at all. Its own arm because falling out of the redirect loop
    /// reported it as [`FetchError::TooManyRedirects`] with an *empty* hop list, telling the
    /// reader a single 304 was a consent wall, which is the confidently-wrong diagnosis
    /// `LoadError::NotWebPage` exists to avoid.
    #[error("{0} redirect with no Location header")]
    RedirectWithoutLocation(u16),
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

    /// `referrer` is the *page* the request is made on behalf of, never the target. Required
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

        // One budget for the whole fetch. Applying `limits.timeout` per hop instead let a
        // 5-redirect chain run six times the timeout that is documented, surfaced on the
        // failure screen, and used to decide `LikelyBlocked`: the reader sat on "Loading"
        // for 90s with nothing to distinguish it from a hang.
        let deadline = Instant::now() + limits.timeout;

        for _ in 0..=MAX_REDIRECTS {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(timeout_error(limits));
            }
            let mut req = self
                .client
                .get(current.clone())
                .header("Accept", limits.accept)
                .header("Accept-Language", "en-US,en;q=0.9")
                // Overrides the client default; no second `Client`, so nothing else changes.
                .timeout(remaining);
            // Same value across every hop; CDNs that bounce to a signed URL break otherwise.
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
                    return Err(FetchError::RedirectWithoutLocation(status.as_u16()));
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
        Err(FetchError::TooManyRedirects(hops))
    }
}

/// The Referer header's value, or `None` for no header at all.
///
/// Built from [`Url::origin`] so that no code path can emit a full URL even by mistake: the
/// path is the disclosure that matters, and this function is where "never the path" is a
/// property of the construction rather than a promise in a comment.
fn referer_header(policy: Referer, referrer: Option<&Url>) -> Option<String> {
    match (policy, referrer) {
        (Referer::PageOrigin, Some(page)) => page_origin(page),
        _ => None,
    }
}

/// The `Referer` value a request made on behalf of `page` would carry, or `None`.
///
/// Public because the *count* has to be decided by the same function as the header. The reader's
/// image-referer setting being on is not the same question as a header going out: a page hww
/// built itself has an opaque origin (`hww:bookmarks` has no authority component at all), so its
/// site marks disclose no referrer whatever the setting says. `ImageStore::record_request` asks
/// this rather than the setting, or the panel reports a disclosure that never left the machine —
/// which is the plausible-looking lie `pageinfo` exists to refuse.
pub fn page_origin(page: &Url) -> Option<String> {
    let origin = page.origin();
    // An opaque origin serializes to "null", which discloses nothing and helps nothing.
    origin.is_tuple().then(|| origin.ascii_serialization())
}

fn timeout_error(limits: &Limits) -> FetchError {
    if limits.timeout_is_block {
        FetchError::LikelyBlocked
    } else {
        FetchError::Timeout(limits.timeout)
    }
}

/// Decide what a short body means and, for a read that failed outright with nothing to show,
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
        } else if body.len() >= limits.max_bytes {
            Truncation::MaybeAtCap(limits.max_bytes)
        } else {
            Truncation::Incomplete(body.len())
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
/// memory, and **keep whatever arrived** when the read fails partway.
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

/// Decode bytes to text. Precedence: HTTP `charset` -> `<?xml encoding>` -> `<meta charset>`
/// prescan -> UTF-8.
///
/// Phase 0 measured 100% UTF-8 across the sample and zero legacy encodings, so this is
/// correctness insurance rather than a hot path. It matters for the older web the client
/// is aimed at, which that sample happened not to reach.
///
/// The XML declaration sits ahead of the prescan because it is the narrower claim: it is one
/// element of one document, at byte zero, and it says outright what the bytes are. The prescan
/// hunts a 4 KiB window for the literal `charset=`, which a feed's own payload may well
/// contain.
///
/// It is skipped entirely for `text/html`, and the HTML spec's own sniffing algorithm skips it
/// for the same reason: a prolog left behind by a conversion out of XHTML outlives the encoding
/// it names, and the `<meta charset>` under it is the one the page is actually written in.
/// Honouring the stale declaration there decoded a correct UTF-8 page as windows-1252. The gate
/// is on `text/html` alone rather than on an XML allowlist, so a feed served under an
/// unregistered type, or under none, keeps the declaration it declared.
pub fn decode(
    bytes: &[u8],
    content_type: Option<&str>,
) -> (String, &'static encoding_rs::Encoding) {
    let html = content_type.is_some_and(is_html);
    let enc = content_type
        .and_then(charset_from_content_type)
        .or_else(|| (!html).then(|| xml_declaration_encoding(bytes)).flatten())
        .or_else(|| meta_charset_prescan(bytes))
        .unwrap_or(encoding_rs::UTF_8);
    let (text, actual, _had_errors) = enc.decode(bytes);
    (text.into_owned(), actual)
}

/// Whether a response is HTML proper. `application/xhtml+xml` is not: it is XML, its
/// declaration is current rather than left over, and it keeps the precedence.
fn is_html(ct: &str) -> bool {
    ct.split(';')
        .next()
        .is_some_and(|m| m.trim().eq_ignore_ascii_case("text/html"))
}

fn charset_from_content_type(ct: &str) -> Option<&'static encoding_rs::Encoding> {
    let idx = ct.to_ascii_lowercase().find("charset=")?;
    let raw = ct[idx + 8..].trim().trim_matches(['"', '\'']);
    let label = raw.split(';').next()?.trim();
    encoding_rs::Encoding::for_label(label.as_bytes())
}

/// How far in a leading `<?xml … ?>` declaration is allowed to run before it stops being one.
///
/// Deliberately tiny. The declaration is the first thing in the document and carries at most a
/// version, an encoding, and a standalone flag; anything longer is not a declaration and the
/// scan should not follow it into the feed.
const XML_DECL_WINDOW: usize = 256;

/// The `encoding` named by a leading XML declaration, when the document opens with one.
///
/// [`meta_charset_prescan`] cannot see it: an XML declaration writes `encoding="…"` and never
/// `charset=`, so a legacy-encoded feed with no HTTP charset decoded as UTF-8 and reached the
/// parser as replacement characters. Scoped to the declaration itself rather than to a window,
/// because `encoding=` is an ordinary word that appears in prose and in a feed's own payload,
/// and a false positive here mis-decodes the whole document.
///
/// It has to happen at decode. `roxmltree` takes a `&str`, so by the time the feed parser sees
/// the document this decision is already made, and it ignores the declaration.
fn xml_declaration_encoding(bytes: &[u8]) -> Option<&'static encoding_rs::Encoding> {
    // No BOM handling: `Encoding::decode` sniffs one and overrides whatever is chosen here,
    // which is the right precedence and would make honouring a declaration behind a BOM a
    // no-op with extra steps.
    let b = &bytes[..bytes.len().min(XML_DECL_WINDOW)];
    let start = b.iter().position(|c| !c.is_ascii_whitespace())?;
    let b = &b[start..];
    if !b.starts_with(b"<?xml") {
        return None;
    }
    let end = b.windows(2).position(|w| w == b"?>")?;
    // Lossy, and only over the declaration: every encoding this can usefully name is
    // ASCII-compatible, so the declaration's own bytes are ASCII whatever the body is.
    let decl = String::from_utf8_lossy(&b[..end]).to_ascii_lowercase();
    let rest = decl[decl.find("encoding")? + "encoding".len()..].trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    let quote = rest.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let label = rest[quote.len_utf8()..].split(quote).next()?;
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
    /// threading model in `reader::ui::net` stops compiling, which is the point of asserting
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

    /// The header and the count are one decision, which is why [`page_origin`] is public.
    ///
    /// A page hww built itself is the case that made this matter: the bookmarks' site marks are
    /// requested on behalf of `hww:bookmarks`, which has no authority component at all, so no
    /// `Referer` can leave — and `ImageStore::record_request` asks this rather than the reader's
    /// setting, or page info would report a disclosure that never happened.
    #[test]
    fn a_page_hww_built_has_no_origin_to_disclose() {
        let built = Url::parse(crate::reader::archive::BOOKMARKS).unwrap();
        assert_eq!(page_origin(&built), None);
        assert_eq!(referer_header(Referer::PageOrigin, Some(&built)), None);
        // And the ordinary case still answers, or the two would agree by both being silent.
        let page = Url::parse("https://example.com/a").unwrap();
        assert_eq!(page_origin(&page).as_deref(), Some("https://example.com"));
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

    /// A feed names its encoding in the XML declaration, which carries no `charset=` for the
    /// prescan to find. Without this a legacy-encoded feed decoded as UTF-8 and reached the
    /// parser as replacement characters.
    #[test]
    fn an_xml_declaration_names_the_encoding() {
        let xml = b"<?xml version=\"1.0\" encoding=\"windows-1251\"?><rss><channel/></rss>";
        assert_eq!(decode(xml, None).1.name(), "windows-1251");
        // Single quotes and leading whitespace are both real in the wild.
        let xml = "\n  <?xml version='1.0' encoding='iso-8859-1' ?><feed/>";
        assert_eq!(decode(xml.as_bytes(), None).1.name(), "windows-1252");
        // The HTTP header still wins.
        let xml = b"<?xml version=\"1.0\" encoding=\"windows-1251\"?><rss/>";
        assert_eq!(
            decode(xml, Some("application/rss+xml; charset=utf-8"))
                .1
                .name(),
            "UTF-8"
        );
    }

    /// A prolog outlives the encoding it names. An XHTML page converted to HTML keeps the
    /// declaration and gains a `<meta charset>`, and the HTML spec's own sniffing ignores the
    /// declaration for `text/html` for exactly that reason; honouring it decoded a correct
    /// UTF-8 page as windows-1252.
    #[test]
    fn a_stale_declaration_does_not_outrank_a_meta_charset_in_html() {
        let page = b"<?xml version=\"1.0\" encoding=\"iso-8859-1\"?>\n\
                     <html><head><meta charset=\"utf-8\"></head><body>x</body></html>";
        assert_eq!(decode(page, Some("text/html")).1.name(), "UTF-8");
        assert_eq!(
            decode(page, Some("text/html; charset=")).1.name(),
            "UTF-8",
            "an unparseable charset parameter is still text/html"
        );
        // XML keeps the precedence: `application/xhtml+xml` is XML, and its declaration is
        // current rather than left over. So is a feed, and so is one served under no type at
        // all, which is the case the gate must not catch.
        assert_eq!(
            decode(page, Some("application/xhtml+xml")).1.name(),
            "windows-1252"
        );
        let feed = b"<?xml version=\"1.0\" encoding=\"iso-8859-1\"?><rss><channel/></rss>";
        assert_eq!(
            decode(feed, Some("application/rss+xml")).1.name(),
            "windows-1252"
        );
        assert_eq!(decode(feed, None).1.name(), "windows-1252");
    }

    /// The scan is the declaration and nothing past it. `encoding=` is an ordinary word, and a
    /// document that never opened with a declaration must not have one read out of its prose.
    #[test]
    fn the_word_encoding_in_a_document_is_not_a_declaration() {
        let html = b"<html><body><p>set encoding=\"windows-1251\" in your editor</p></body>";
        assert_eq!(decode(html, None).1.name(), "UTF-8");
        // A declaration with no encoding attribute names nothing.
        let xml = b"<?xml version=\"1.0\"?><rss>encoding=\"windows-1251\"</rss>";
        assert_eq!(decode(xml, None).1.name(), "UTF-8");
    }

    #[test]
    fn prescan_window_is_bounded() {
        // A charset declared past the 4 KiB prescan window must not be honoured.
        let mut html = vec![b' '; 5000];
        html.extend_from_slice(b"<meta charset=\"windows-1251\">");
        let (_, enc) = decode(&html, None);
        assert_eq!(enc.name(), "UTF-8");
    }

    /// A one-shot loopback server, so the redirect and body-truncation paths are exercised
    /// rather than reasoned about. No outbound network: CI stays offline.
    fn serve(replies: impl Fn(usize, &mut std::net::TcpStream) + Send + 'static) -> u16 {
        use std::io::Read;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for (i, stream) in listener.incoming().enumerate().take(MAX_REDIRECTS + 2) {
                let Ok(mut stream) = stream else { continue };
                let _ = stream.read(&mut [0u8; 2048]);
                replies(i, &mut stream);
            }
        });
        port
    }

    fn local(port: u16, path: &str) -> Url {
        Url::parse(&format!("http://127.0.0.1:{port}{path}")).unwrap()
    }

    /// A 3xx with no `Location` used to fall out of the redirect loop into
    /// `TooManyRedirects` (with an *empty* hop list), so the reader told the user a single
    /// 304 was "usually a consent wall".
    #[test]
    fn a_redirect_without_a_location_is_not_a_redirect_loop() {
        use std::io::Write;
        let port = serve(|_, s| {
            let _ = s.write_all(b"HTTP/1.1 302 Found\r\nContent-Length: 0\r\n\r\n");
        });
        let err = Fetcher::new()
            .unwrap()
            .get_with(&local(port, "/noloc"), None, &Limits::HTML)
            .unwrap_err();
        assert!(
            matches!(err, FetchError::RedirectWithoutLocation(302)),
            "got {err:?}"
        );
    }

    /// `MaybeAtCap` names a limit. Naming one that was never approached ("may be truncated at
    /// 5 MB" after 3 KB arrived) both misstates the size and hides the dropped connection.
    #[test]
    fn a_dropped_connection_is_not_reported_as_hitting_the_cap() {
        use std::io::Write;
        let port = serve(|_, s| {
            let _ = s.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 100000\r\n\r\n",
            );
            let _ = s.write_all(&[b'a'; 3000]);
            // ...and hang up, well short of the promised body.
        });
        let resp = Fetcher::new()
            .unwrap()
            .get_with(&local(port, "/short"), None, &Limits::HTML)
            .expect("a partial body still renders");
        assert_eq!(resp.truncation, Truncation::Incomplete(3000));
        assert!(resp.truncated());
    }

    /// `TIMEOUT` is documented as *the* timeout, shown on the failure screen, and used to
    /// decide `LikelyBlocked`. Applied per hop instead, a redirect chain ran to six times it.
    #[test]
    fn the_timeout_bounds_the_whole_fetch_not_each_hop() {
        use std::io::Write;
        let budget = Duration::from_millis(1200);
        let hop = Duration::from_millis(1000);
        let port = serve(move |i, s| {
            // Inside the budget on its own, so a per-hop timeout would never fire; six of
            // them run to five times it. `Connection: close` because the client otherwise
            // pools a socket the server drops in the same breath, and reusing it races into
            // a transport error instead of the timeout under test.
            std::thread::sleep(hop);
            let _ = s.write_all(
                format!(
                    "HTTP/1.1 302 Found\r\nLocation: /hop{}\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
                    i + 1
                )
                .as_bytes(),
            );
        });
        let limits = Limits {
            timeout: budget,
            ..Limits::HTML
        };
        // The clock starts once the client exists. Building one loads the platform root
        // store, tens of milliseconds here and far more on a cold Windows runner, and that
        // cost was being charged to the redirect chain the assertion is about.
        let fetcher = Fetcher::new().unwrap();
        let start = Instant::now();
        let err = fetcher
            .get_with(&local(port, "/hop0"), None, &limits)
            .unwrap_err();
        let elapsed = start.elapsed();
        // Bounded correctly, the second hop runs out of budget at ~1.2s. Bounded per hop,
        // six 1s hops end in `TooManyRedirects` at ~6s. The ceiling sits between the two
        // with 1.8s of slack, because a contended runner oversleeps and a tight multiple of
        // the budget made this the flakiest test in the suite.
        let ceiling = budget + hop * 2;
        assert!(
            elapsed < ceiling,
            "the chain ran {elapsed:?} against a {budget:?} budget"
        );
        // Out of time, not out of redirects.
        assert!(matches!(err, FetchError::LikelyBlocked), "got {err:?}");
    }
}
