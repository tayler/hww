//! Bytes -> pixels. **The only module in this project that parses untrusted binary input.**
//!
//! The placement is the point. Putting this under `ui/` would have made the highest-risk code
//! the one code no test ever runs. Format sniffing, the alloc and dimension ceilings, the
//! truncated-JPEG path, and downscaling are all decidable from a `&[u8]` and a target width,
//! so they are testable headlessly, including the partial-render matrix below, which is a
//! regression test rather than a footnote. `ui/images.rs` keeps only what genuinely needs an
//! `egui::Context`: upload and the texture LRU.
//!
//! It compiles under `gui` only because the `image` crate rides with it, and `image` stays
//! optional so the default build keeps shrinking as the optional one grows. That costs
//! nothing here: `cargo test --features gui` runs every test below without opening a window.
//!
//! Controls, in order:
//!
//! 1. The 10 MB transfer cap, applied upstream in `fetch`.
//! 2. **Decode whatever arrived, partial included.** The caps bound cost; they do not decide
//!    the user has earned nothing. `read_capped` truncates a *document* at 5 MB and
//!    `html::extract` renders what it has rather than refusing, so images behaving the same
//!    way is consistency, not a special case.
//! 3. Sniff the format from the bytes with `with_guessed_format`, **never** from
//!    `Content-Type`: a header is a claim, and this is the one place a wrong claim is
//!    expensive.
//! 4. [`DecodeLimits`], enforced through `image`'s own `Limits`: 8192 px per side and a
//!    256 MiB allocation ceiling.
//! 5. Reject on dimensions *before* a full decode.
//! 6. Downscale to the reading column before the caller ever sees a texture.
//!
//! # Why 256 MiB and not 64 MiB
//!
//! At 64 MiB the pixel limit was dead code: 8192x8192 RGBA is 256 MiB, four times that
//! ceiling, so allocation always bound first and 8192 px never fired. 64 MiB is exactly
//! 4096x4096. Raising the allocation ceiling is what makes the pixel limit the real limit,
//! the one that reads as a deliberate choice.
//!
//! # Partial images
//!
//! Measured against `image` 0.25, because the formats do not agree:
//!
//! | Format | Truncated at 50% | Result |
//! |---|---|---|
//! | JPEG | decodes | rows up to the last complete MCU are real, the rest is neutral gray; dimensions intact |
//! | PNG / GIF / WebP | `unexpected end of file` | chunk CRCs and container framing refuse |
//!
//! So the rule is *render what decodes*: JPEG, the dominant format for article photography,
//! gets the half-loaded picture for free, and the other three fall back to a labelled failure.
//! Recovering rows from a partial PNG means dropping to the `png` crate's streaming reader, a
//! second decode path for the minority case. **Not worth it**, and recorded here so it is not
//! re-proposed as an oversight.
//!
//! # Still images only
//!
//! `ImageReader::decode()` yields the first frame, and nothing here may reach for
//! `into_frames()`. An animated GIF or WebP is bounded by frame count rather than by the pixel
//! ceiling, which is the one amplification path the transfer cap was genuinely holding shut.

use crate::fetch::Truncation;

/// Longest side, in pixels, that will be decoded at all.
pub const MAX_DIMENSION: u32 = 8192;
/// Transient decode buffer ceiling. 8192x8192 RGBA is exactly this.
pub const MAX_ALLOC_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct DecodeLimits {
    /// The reading column, in physical pixels. Anything wider is downscaled *before* it
    /// becomes a texture; without this, "load images" on a photo-heavy page is a GPU-memory
    /// denial of service driven by untrusted input.
    pub max_width: u32,
    pub max_dimension: u32,
    pub max_alloc_bytes: u64,
}

impl Default for DecodeLimits {
    fn default() -> Self {
        Self {
            max_width: 1024,
            max_dimension: MAX_DIMENSION,
            max_alloc_bytes: MAX_ALLOC_BYTES,
        }
    }
}

/// A decoded, already-downscaled image, plus what the reader must say about it.
pub struct Decoded {
    pub width: u32,
    pub height: u32,
    /// Row-major RGBA8.
    pub rgba: Vec<u8>,
    /// The dimensions before downscaling, so the strip can say what was actually served.
    pub source_width: u32,
    pub source_height: u32,
    /// `Some` when the bytes were incomplete. A partial that renders **must** be labelled: a
    /// half-gray photograph the user cannot tell from the real one, from a flaky link, or from
    /// a limit hww imposed is worse than no photograph.
    pub partial: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("not a recognised image format")]
    UnknownFormat,
    /// A format hww deliberately does not decode. Named, so the strip can say which rather
    /// than calling a perfectly valid file unrecognisable.
    #[error("{0} images are not supported")]
    Unsupported(&'static str),
    #[error("image is {0}x{1}; the limit is {2} px on a side")]
    TooLarge(u32, u32, u32),
    /// Includes the truncated PNG/GIF/WebP case, which cannot yield a partial frame.
    #[error("could not decode: {0}")]
    Decode(String),
    #[error("empty response")]
    Empty,
}

impl DecodeError {
    /// Whether asking the same server for the same bytes could plausibly end differently.
    ///
    /// The reader draws a retry control on the strength of this, so a `true` it cannot back up
    /// is a button that re-fetches and re-fails for as long as the page is open. `Unsupported`
    /// and `TooLarge` are properties of the file: a second copy of it is the same file.
    ///
    /// `Decode` stays retryable on purpose. It is what a truncated transfer produces, which is
    /// the one decode failure a second request genuinely can fix.
    pub fn offers_retry(&self) -> bool {
        match self {
            Self::Unsupported(_) | Self::TooLarge(..) => false,
            Self::UnknownFormat | Self::Decode(_) | Self::Empty => true,
        }
    }
}

/// Human-readable label for an incomplete transfer, or `None` if it was complete.
///
/// `AtCap` says "partial" because `Content-Length` proved there was more. `MaybeAtCap` hedges,
/// because a body that ends exactly at the cap with no `Content-Length` is indistinguishable
/// from one that simply ended there. Re-fetching without a cap is deliberately not offered:
/// the label explains the limit, and an unbounded retry would delete the control the cap
/// exists to be.
pub fn partial_label(t: Truncation) -> Option<String> {
    match t {
        Truncation::Complete => None,
        Truncation::AtCap(n) => Some(format!("partial: {} MB limit reached", n / 1_000_000)),
        Truncation::MaybeAtCap(n) => Some(format!(
            "may be partial: {} MB limit reached",
            n / 1_000_000
        )),
        Truncation::Timeout(d) => Some(format!("partial: timed out after {}s", d.as_secs())),
        // Naming the cap here would be a lie: the transfer died nowhere near it.
        Truncation::Incomplete(n) => Some(format!("partial: connection dropped at {n} bytes")),
    }
}

pub fn decode(
    bytes: &[u8],
    transfer: &Truncation,
    limits: &DecodeLimits,
) -> Result<Decoded, DecodeError> {
    use image::ImageReader;
    use std::io::Cursor;

    if bytes.is_empty() {
        return Err(DecodeError::Empty);
    }

    if let Some(name) = deliberately_unsupported(bytes) {
        return Err(DecodeError::Unsupported(name));
    }

    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| DecodeError::Decode(e.to_string()))?;
    let format = reader.format().ok_or(DecodeError::UnknownFormat)?;

    // Reject on the header before committing to a full decode. `into_dimensions` consumes the
    // reader, so the decode below builds a second one over the same in-memory bytes: cheap,
    // and the list above is a sequence of steps rather than of one object.
    let (w, h) = reader
        .into_dimensions()
        .map_err(|e| DecodeError::Decode(e.to_string()))?;
    if w > limits.max_dimension || h > limits.max_dimension {
        return Err(DecodeError::TooLarge(w, h, limits.max_dimension));
    }

    let mut reader = ImageReader::new(Cursor::new(bytes));
    reader.set_format(format);
    reader.limits(alloc_limits(limits));
    // `decode`, never `into_frames`: still images only.
    let img = reader
        .decode()
        .map_err(|e| DecodeError::Decode(e.to_string()))?;

    let (source_width, source_height) = (img.width(), img.height());
    let img = downscale(img, limits.max_width);
    let rgba = img.to_rgba8();
    Ok(Decoded {
        width: rgba.width(),
        height: rgba.height(),
        source_width,
        source_height,
        rgba: rgba.into_raw(),
        partial: partial_label(*transfer),
    })
}

/// Formats this reader declines on purpose, recognised from their bytes so the failure names
/// the format instead of calling a valid file unrecognisable.
///
/// The third of the three declines and the only one that is not a claim, so it is the one that
/// stays here beside the decoder. The table it answers, and the reasons each format is on it,
/// are in [`crate::reader::image_formats`]; `the_declines_all_name_the_same_formats` below is
/// what holds this in step with the other two.
fn deliberately_unsupported(bytes: &[u8]) -> Option<&'static str> {
    // ISO-BMFF: [4-byte size][b"ftyp"][brand]. `avif` and `avis` are the AVIF brands.
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" && matches!(&bytes[8..12], b"avif" | b"avis") {
        return Some("AVIF");
    }
    if xml_root_is_svg(bytes) {
        return Some("SVG");
    }
    None
}

/// How far into a file the root element is still worth looking for. Generous for a prolog and
/// cheap to walk; past it, the file is answering a different question than "is this an SVG".
const PROLOG_SCAN: usize = 64 * 1024;

/// The one widening, and its ceiling.
///
/// The retry below reconverts, and `from_utf8_lossy` allocates up to three bytes per input byte
/// for a buffer that is not valid UTF-8. Over the whole of a [`crate::fetch::MAX_IMAGE_BYTES`]
/// file that is 30M on the decode worker to answer "is this an SVG", reachable from any
/// third-party address that serves 64 KiB of unterminated `<!--`. A prolog past a megabyte is
/// not a document this reader was going to draw.
const PROLOG_SCAN_WIDE: usize = 1024 * 1024;

/// Whether the first element of this XML document is `<svg>`.
///
/// A scan and not a substring test. The old check lowercased the first 1024 bytes and looked
/// for `<svg`, which was wrong in both directions: an SVG whose editor wrote a long DOCTYPE or
/// a metadata header pushed the root past the window and was reported as "not a recognised
/// image format", the one message [`DecodeError::Unsupported`] exists to prevent, and a raster
/// whose leading metadata happened to mention `<svg` was called an SVG. Widening the window
/// makes the second failure *more* likely, not less, so the prolog is parsed instead.
///
/// Skipped: a UTF-8 BOM, whitespace, `<?xml ...?>` and other processing instructions,
/// `<!-- -->` comments, and `<!DOCTYPE ...>` including an internal `[ ... ]` subset. A
/// namespace prefix is accepted, so `<svg:svg>` counts.
///
/// UTF-16 is not handled. It is vanishingly rare for SVG on the web, and the cost of missing
/// it is the old wrong message rather than a crash.
///
/// `.svgz` needs no gzip branch here: `reqwest` is built with the `gzip` feature, so a
/// correctly served one arrives already inflated, and the mis-served raw-gzip case is refused
/// by [`declined_by_url`] before a byte is fetched.
fn xml_root_is_svg(bytes: &[u8]) -> bool {
    // Before the conversion below, because that conversion is not free. Every raster this
    // reader decodes reaches here first, and for one the head is not valid UTF-8, so
    // `from_utf8_lossy` allocates an owned string of up to three bytes per input byte and
    // validates the whole window — per image, on the decode worker — to reach a `<` test that
    // the first byte already answered. `0x89` (PNG), `0xFF` (JPEG), `GIF8`, and `RIFF` all
    // fail here for the cost of a scan over leading whitespace.
    if !bytes
        .iter()
        .find(|b| !b.is_ascii_whitespace())
        .is_some_and(|b| *b == b'<' || *b == 0xef)
    {
        return false;
    }
    // Lossy is safe here: this only ever asks whether ASCII markup is present, and a
    // replacement char matches none of it.
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(PROLOG_SCAN)]);
    match walk_prolog(&head) {
        Some(answer) => answer,
        // The prolog did not finish inside the window: a comment or a DOCTYPE subset straddles
        // the edge, so its terminator is simply not in `head`. Answering `false` here is the
        // old 1024-byte failure moved out to 64 KiB rather than removed — it reports a long
        // SVG as "not a recognised image format", which is the message this function exists to
        // prevent. Widen once, to [`PROLOG_SCAN_WIDE`] and not to the whole file; only a file
        // that is XML-ish *and* has a prolog past the first window pays for the second
        // conversion at all, and every raster is long gone by here.
        None if bytes.len() > PROLOG_SCAN => {
            let wide = String::from_utf8_lossy(&bytes[..bytes.len().min(PROLOG_SCAN_WIDE)]);
            walk_prolog(&wide).unwrap_or(false)
        }
        // Unterminated with nothing left to read. The document has no root element, so it is
        // not an SVG and not anything else either; the raster decoders get their say next.
        None => false,
    }
}

/// The prolog walk itself. `Some(is_svg)` when it reached the first element, `None` when a
/// construct ran off the end of what it was given — which is a question about the window, not
/// about the file, and is why this is an `Option` rather than a `bool`.
fn walk_prolog(head: &str) -> Option<bool> {
    let mut s = head.strip_prefix('\u{feff}').unwrap_or(head);
    loop {
        s = s.trim_start();
        let rest = if let Some(r) = s.strip_prefix("<!--") {
            &r[r.find("-->")? + 3..]
        } else if let Some(r) = s.strip_prefix("<?") {
            &r[r.find("?>")? + 2..]
        } else if let Some(r) = s.strip_prefix("<!") {
            // A DOCTYPE may carry an internal subset, whose own `>`s must not end it.
            skip_declaration(r)?
        } else {
            // The first thing that is not prolog. Either it opens the root element or this is
            // not XML at all.
            return Some(
                s.strip_prefix('<')
                    .map(element_name)
                    .is_some_and(|name| name.eq_ignore_ascii_case("svg")),
            );
        };
        s = rest;
    }
}

/// Past a `<!...>` declaration, honouring an internal `[ ... ]` subset. `s` starts just after
/// the `<!`.
fn skip_declaration(s: &str) -> Option<&str> {
    let mut depth = 0usize;
    for (i, c) in s.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => depth = depth.saturating_sub(1),
            '>' if depth == 0 => return Some(&s[i + 1..]),
            _ => {}
        }
    }
    None
}

/// The local name of an element whose `<` has been consumed, with any namespace prefix
/// dropped: `svg:svg` and `svg` both answer `svg`.
fn element_name(s: &str) -> &str {
    let end = s
        .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
        .unwrap_or(s.len());
    let name = &s[..end];
    name.rsplit_once(':')
        .map(|(_, local)| local)
        .unwrap_or(name)
}

fn alloc_limits(limits: &DecodeLimits) -> image::Limits {
    let mut l = image::Limits::default();
    l.max_image_width = Some(limits.max_dimension);
    l.max_image_height = Some(limits.max_dimension);
    l.max_alloc = Some(limits.max_alloc_bytes);
    l
}

/// Shrink to the reading column, preserving aspect. Never enlarges: a 200 px thumbnail blown
/// up to the column width is worse than a 200 px thumbnail.
fn downscale(img: image::DynamicImage, max_width: u32) -> image::DynamicImage {
    let max_width = max_width.max(1);
    if img.width() <= max_width {
        return img;
    }
    let height =
        ((u64::from(img.height()) * u64::from(max_width)) / u64::from(img.width())).max(1) as u32;
    // Triangle: article photography downscaled with nearest-neighbour looks broken, and
    // Lanczos costs more than the difference is worth at reading sizes.
    img.resize_exact(max_width, height, image::imageops::FilterType::Triangle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::image_formats::{DECLINED, Format, declined_by_mime, declined_by_url};
    use image::{ImageFormat, RgbaImage};
    use std::io::Cursor;
    use url::Url;

    /// A gradient rather than a flat fill: a flat image survives a truncated decode looking
    /// identical to a complete one, which would make the partial tests prove nothing.
    fn sample(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_fn(w, h, |x, y| {
            image::Rgba([(x % 256) as u8, (y % 256) as u8, 128, 255])
        })
    }

    fn encode(w: u32, h: u32, format: ImageFormat) -> Vec<u8> {
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgba8(sample(w, h))
            .to_rgb8()
            .write_to(&mut Cursor::new(&mut buf), format)
            .unwrap();
        buf
    }

    fn limits(max_width: u32) -> DecodeLimits {
        DecodeLimits {
            max_width,
            ..DecodeLimits::default()
        }
    }

    #[test]
    fn a_complete_jpeg_round_trips_and_is_not_labelled_partial() {
        let bytes = encode(300, 200, ImageFormat::Jpeg);
        let d = decode(&bytes, &Truncation::Complete, &limits(4096)).unwrap();
        assert_eq!((d.width, d.height), (300, 200));
        assert_eq!((d.source_width, d.source_height), (300, 200));
        assert_eq!(d.rgba.len(), 300 * 200 * 4);
        assert!(d.partial.is_none());
    }

    #[test]
    fn format_comes_from_the_bytes_not_from_a_claim() {
        // PNG bytes decode as PNG whatever anybody says they are.
        let bytes = encode(32, 32, ImageFormat::Png);
        assert!(decode(&bytes, &Truncation::Complete, &limits(4096)).is_ok());
        assert!(matches!(
            decode(b"not an image at all", &Truncation::Complete, &limits(4096)),
            Err(DecodeError::UnknownFormat | DecodeError::Decode(_))
        ));
        assert!(matches!(
            decode(b"", &Truncation::Complete, &limits(4096)),
            Err(DecodeError::Empty)
        ));
    }

    /// The dominant format for article photography gets the half-loaded picture for free.
    #[test]
    fn a_jpeg_truncated_at_half_still_renders_and_says_so() {
        let bytes = encode(320, 240, ImageFormat::Jpeg);
        let half = &bytes[..bytes.len() / 2];
        let d = decode(
            half,
            &Truncation::Timeout(std::time::Duration::from_secs(30)),
            &limits(4096),
        )
        .expect("a truncated jpeg decodes to the rows that arrived");
        assert_eq!((d.width, d.height), (320, 240), "dimensions survive");
        assert_eq!(d.partial.as_deref(), Some("partial: timed out after 30s"));
    }

    /// Chunk CRCs and container framing refuse; the reader falls back to a labelled failure.
    #[test]
    fn truncated_png_gif_and_webp_refuse_rather_than_half_decode() {
        for format in [ImageFormat::Png, ImageFormat::Gif] {
            let bytes = encode(320, 240, format);
            let half = &bytes[..bytes.len() / 2];
            assert!(
                decode(half, &Truncation::AtCap(10_000_000), &limits(4096)).is_err(),
                "{format:?} must not produce a silent half-image"
            );
        }
    }

    /// Declined on purpose, and named: "SVG images are not supported" is a different
    /// statement from "not a recognised image format", and only one of them is true.
    #[test]
    fn svg_and_avif_are_declined_by_name() {
        let svg = br#"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg"/>"#;
        assert!(matches!(
            decode(svg, &Truncation::Complete, &limits(4096)),
            Err(DecodeError::Unsupported("SVG"))
        ));
        let mut avif = vec![0u8, 0, 0, 0x20];
        avif.extend_from_slice(b"ftypavif");
        avif.extend_from_slice(&[0u8; 16]);
        assert!(matches!(
            decode(&avif, &Truncation::Complete, &limits(4096)),
            Err(DecodeError::Unsupported("AVIF"))
        ));
        // A format that *is* supported must not be caught by the sniffs above.
        let png = encode(16, 16, ImageFormat::Png);
        assert!(decode(&png, &Truncation::Complete, &limits(4096)).is_ok());
    }

    /// One valid byte sequence per name in [`DECLINED`], so the structural test below can ask
    /// the byte sniff about every format the URL sniff knows.
    fn declined_sample(name: &str) -> Vec<u8> {
        match name {
            "SVG" => br#"<svg xmlns="http://www.w3.org/2000/svg"/>"#.to_vec(),
            "AVIF" => {
                let mut v = vec![0u8, 0, 0, 0x20];
                v.extend_from_slice(b"ftypavif");
                v.extend_from_slice(&[0u8; 16]);
                v
            }
            other => panic!("{other} is in DECLINED with no sample; add one"),
        }
    }

    /// The three declines must name the same formats.
    ///
    /// Each disagreement is silent and each is a different bug. A format the address refuses but
    /// the bytes accept loads from any URL with no extension. A format the bytes refuse but the
    /// address does not costs a request every time. A format both refuse but no `type` names is
    /// one `html::image_from` still picks out of a `<picture>` while a decodable sibling sits
    /// beside it. So the agreement is asserted rather than remembered.
    ///
    /// Here rather than in `image_formats` because this is the one test that needs both halves:
    /// the table is ungated and the byte sniff rides with `image`.
    #[test]
    fn the_declines_all_name_the_same_formats() {
        for &Format {
            name,
            suffixes,
            mimes,
        } in DECLINED
        {
            assert_eq!(
                deliberately_unsupported(&declined_sample(name)),
                Some(name),
                "{name} is refused by address but not by bytes"
            );
            for suffix in suffixes {
                let url = Url::parse(&format!("https://example.org/a.{suffix}")).unwrap();
                assert_eq!(
                    declined_by_url(&url),
                    Some(name),
                    "{name} is refused by bytes but not by a .{suffix} address"
                );
            }
            assert!(
                !mimes.is_empty(),
                "{name} is refused by bytes and by an address, so a <source type> claiming it \
                 must be refused too; add its MIME type"
            );
            for mime in mimes {
                assert_eq!(
                    declined_by_mime(mime),
                    Some(name),
                    "{name} is refused by bytes but not by a `{mime}` type attribute"
                );
            }
        }
    }

    /// The failure the 1024-byte window produced: a real SVG reported as unrecognisable.
    #[test]
    fn an_svg_behind_a_long_prolog_is_still_named_svg() {
        let mut doc = br#"<?xml version="1.0" encoding="UTF-8"?>"#.to_vec();
        doc.extend_from_slice(
            br#"<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd">"#,
        );
        doc.extend_from_slice(b"<!-- ");
        doc.extend_from_slice(&b"editor metadata ".repeat(200));
        doc.extend_from_slice(b" -->\n");
        assert!(doc.len() > 1024, "the point of the fixture is the length");
        doc.extend_from_slice(br#"<svg xmlns="http://www.w3.org/2000/svg"/>"#);
        assert!(matches!(
            decode(&doc, &Truncation::Complete, &limits(4096)),
            Err(DecodeError::Unsupported("SVG"))
        ));
    }

    /// The same failure the window itself produces one size up: a construct whose terminator
    /// sits past `PROLOG_SCAN` is unterminated only from where the walk was looking.
    #[test]
    fn a_prolog_longer_than_the_scan_window_is_still_read_whole() {
        let mut doc = b"<!-- ".to_vec();
        doc.extend_from_slice(&b"editor metadata ".repeat(6000));
        doc.extend_from_slice(b" -->\n");
        assert!(
            doc.len() > PROLOG_SCAN,
            "the fixture must straddle the window"
        );
        doc.extend_from_slice(br#"<svg xmlns="http://www.w3.org/2000/svg"/>"#);
        assert!(xml_root_is_svg(&doc));
        assert!(matches!(
            decode(&doc, &Truncation::Complete, &limits(4096)),
            Err(DecodeError::Unsupported("SVG"))
        ));
    }

    /// Widening must not turn every long text file into an SVG. A prolog that never ends has
    /// no root element, and the raster decoders get their say.
    #[test]
    fn a_prolog_that_never_ends_is_not_an_svg() {
        let mut doc = b"<!-- ".to_vec();
        doc.extend_from_slice(&b"unterminated ".repeat(6000));
        assert!(doc.len() > PROLOG_SCAN);
        assert!(!xml_root_is_svg(&doc));
    }

    /// And the widening has an end. A prolog past `PROLOG_SCAN_WIDE` is not read further,
    /// because the alternative is converting the whole `MAX_IMAGE_BYTES` ceiling to answer one
    /// question about the first element.
    #[test]
    fn the_widened_window_has_a_ceiling_of_its_own() {
        let mut doc = b"<!-- ".to_vec();
        doc.extend_from_slice(&b"editor metadata ".repeat(PROLOG_SCAN_WIDE / 16 + 16));
        doc.extend_from_slice(b" -->\n");
        assert!(doc.len() > PROLOG_SCAN_WIDE);
        doc.extend_from_slice(br#"<svg xmlns="http://www.w3.org/2000/svg"/>"#);
        assert!(!xml_root_is_svg(&doc));
    }

    /// A DOCTYPE's internal subset carries `>` of its own, which must not end it.
    #[test]
    fn a_doctype_with_an_internal_subset_does_not_end_early() {
        let doc = br#"<!DOCTYPE svg [<!ENTITY nb "&#160;">]><svg:svg xmlns:svg="http://www.w3.org/2000/svg"/>"#;
        assert_eq!(deliberately_unsupported(doc), Some("SVG"));
    }

    /// The other half of the old substring test's failure: a supported file whose leading
    /// metadata mentions `<svg` is not an SVG and must still decode.
    #[test]
    fn a_raster_whose_metadata_mentions_svg_still_decodes() {
        let mut png = encode(16, 16, ImageFormat::Png);
        // Ahead of the pixels but behind the signature, the way a real metadata chunk sits.
        let at = 8;
        png.splice(
            at..at,
            br#"<svg xmlns="http://www.w3.org/2000/svg">"#.iter().copied(),
        );
        assert!(
            !xml_root_is_svg(&png),
            "a PNG signature is not an XML prolog, whatever follows it"
        );
    }

    /// The retry control is drawn from this, so a wrong answer is a button that cannot work.
    #[test]
    fn only_a_property_of_the_file_refuses_a_retry() {
        assert!(!DecodeError::Unsupported("SVG").offers_retry());
        assert!(!DecodeError::TooLarge(9000, 9000, 8192).offers_retry());
        // A truncated transfer lands here, and a second request is exactly what fixes it.
        assert!(DecodeError::Decode("unexpected end of file".to_owned()).offers_retry());
        assert!(DecodeError::Empty.offers_retry());
        assert!(DecodeError::UnknownFormat.offers_retry());
    }

    #[test]
    fn an_oversized_image_is_rejected_from_its_header() {
        let bytes = encode(600, 40, ImageFormat::Png);
        let tight = DecodeLimits {
            max_dimension: 256,
            ..limits(4096)
        };
        assert!(matches!(
            decode(&bytes, &Truncation::Complete, &tight),
            Err(DecodeError::TooLarge(600, 40, 256))
        ));
    }

    #[test]
    fn wide_images_are_downscaled_to_the_column_and_narrow_ones_are_left_alone() {
        let bytes = encode(800, 400, ImageFormat::Png);
        let d = decode(&bytes, &Truncation::Complete, &limits(300)).unwrap();
        assert_eq!((d.width, d.height), (300, 150), "aspect ratio preserved");
        assert_eq!((d.source_width, d.source_height), (800, 400));
        assert_eq!(d.rgba.len(), 300 * 150 * 4);

        // A 200 px thumbnail blown up to the column width is worse than a 200 px thumbnail.
        let small = encode(120, 60, ImageFormat::Png);
        let d = decode(&small, &Truncation::Complete, &limits(900)).unwrap();
        assert_eq!((d.width, d.height), (120, 60));
    }

    #[test]
    fn partial_labels_say_which_limit_and_hedge_when_they_must() {
        assert_eq!(partial_label(Truncation::Complete), None);
        assert_eq!(
            partial_label(Truncation::AtCap(10_000_000)).unwrap(),
            "partial: 10 MB limit reached"
        );
        assert_eq!(
            partial_label(Truncation::MaybeAtCap(10_000_000)).unwrap(),
            "may be partial: 10 MB limit reached"
        );
        assert_eq!(
            partial_label(Truncation::Timeout(std::time::Duration::from_secs(30))).unwrap(),
            "partial: timed out after 30s"
        );
    }

    /// The pixel ceiling must be the limit that fires, not dead code behind the alloc ceiling.
    #[test]
    fn the_allocation_ceiling_is_large_enough_for_the_pixel_ceiling_to_matter() {
        let square = u64::from(MAX_DIMENSION) * u64::from(MAX_DIMENSION) * 4;
        assert_eq!(square, MAX_ALLOC_BYTES);
    }
}
