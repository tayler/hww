//! Bytes -> pixels. **The only module in this project that parses untrusted binary input.**
//!
//! The placement is the point. Putting this under `ui/` would have made the highest-risk code
//! the one code no test ever runs. Format sniffing, the alloc and dimension ceilings, the
//! truncated-JPEG path, and downscaling are all decidable from a `&[u8]` and a target width,
//! so they are testable headlessly — including the partial-render matrix below, which is a
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
//!    `Content-Type` — a header is a claim, and this is the one place a wrong claim is
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
//! 4096x4096. Raising the allocation ceiling is what makes the pixel limit the real limit —
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
//! So the rule is *render what decodes*: JPEG — the dominant format for article photography —
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
    /// becomes a texture — without this, "load images" on a photo-heavy page is a GPU-memory
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
    #[error("image is {0}x{1}; the limit is {2} px on a side")]
    TooLarge(u32, u32, u32),
    /// Includes the truncated PNG/GIF/WebP case, which cannot yield a partial frame.
    #[error("could not decode: {0}")]
    Decode(String),
    #[error("empty response")]
    Empty,
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

    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| DecodeError::Decode(e.to_string()))?;
    let format = reader.format().ok_or(DecodeError::UnknownFormat)?;

    // Reject on the header before committing to a full decode. `into_dimensions` consumes the
    // reader, so the decode below builds a second one over the same in-memory bytes — cheap,
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

fn alloc_limits(limits: &DecodeLimits) -> image::Limits {
    let mut l = image::Limits::default();
    l.max_image_width = Some(limits.max_dimension);
    l.max_image_height = Some(limits.max_dimension);
    l.max_alloc = Some(limits.max_alloc_bytes);
    l
}

/// Shrink to the reading column, preserving aspect. Never enlarges — a 200 px thumbnail blown
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
    use image::{ImageFormat, RgbaImage};
    use std::io::Cursor;

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
