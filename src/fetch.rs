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

#[derive(Debug)]
pub struct Response {
    pub final_url: Url,
    pub status: u16,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
    pub hops: Vec<Url>,
    /// Responses that *attempted* to set a cookie. Recorded for reporting only; nothing is stored.
    pub cookie_attempts: usize,
    pub truncated: bool,
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

    pub fn get(&self, url: &Url) -> Result<Response, FetchError> {
        let mut current = url.clone();
        let mut hops = Vec::new();
        let mut cookie_attempts = 0usize;

        for _ in 0..=MAX_REDIRECTS {
            let resp = match self
                .client
                .get(current.clone())
                .header(
                    "Accept",
                    "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
                )
                .header("Accept-Language", "en-US,en;q=0.9")
                .send()
            {
                Ok(r) => r,
                Err(e) if e.is_timeout() => return Err(FetchError::LikelyBlocked),
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

            let body = read_capped(resp)?;
            let truncated = body.len() >= MAX_BYTES;
            return Ok(Response {
                final_url: current,
                status: status.as_u16(),
                content_type,
                body,
                hops,
                cookie_attempts,
                truncated,
            });
        }
        Err(FetchError::TooManyRedirects)
    }
}

/// Read at most `MAX_BYTES`, so a hostile or accidental multi-GB response cannot exhaust memory.
fn read_capped(resp: reqwest::blocking::Response) -> Result<Vec<u8>, FetchError> {
    use std::io::Read;
    let mut buf = Vec::new();
    resp.take(MAX_BYTES as u64).read_to_end(&mut buf)?;
    Ok(buf)
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
