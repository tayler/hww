//! Which picture formats this reader declines, and the three claims that can name one.
//!
//! One table, three lookups, and a test that walks it. A format named by one lookup and not
//! the others is a format that half-works: refused by the address but accepted by the bytes and
//! it loads from any URL with no extension; refused by the bytes but not the address and every
//! one costs a request; refused by both but not by a `<picture>`'s `type` and extraction hands
//! the renderer the one candidate it cannot draw while a decodable sibling sits beside it.
//! `image_decode::the_declines_all_name_the_same_formats` asserts the agreement rather than
//! trusting anyone to remember it.
//!
//! Outside the `gui` gate, unlike [`crate::reader::image_decode`] next door. Nothing here needs
//! the `image` crate — it is string work over a table — and two of the three callers are not
//! GUI code: `html::image_from` chooses between a `<picture>`'s candidates while building the
//! IR, in the default build the fast CI job tests. The byte sniff stays with the decoder,
//! because that one is about bytes.
//!
//! # A claim is not bytes, and the asymmetry is the point
//!
//! `image_decode`'s standing rule is to sniff bytes rather than trust claims, and every lookup
//! here trusts one. What differs is the consequence. Trusting a claim about *how to decode*
//! means decoding mislabelled bytes; trusting one about *what not to fetch*, or about which of
//! several encodings of one picture to ask for, costs at worst a picture that would have
//! loaded. The byte sniff stays the authority for anything that does get fetched.

use url::Url;

/// The formats this reader declines on purpose: the name each is reported under, the path
/// suffixes that claim it, and the MIME types that do.
///
/// **SVG** would need a second layout engine. `resvg` is exactly that, and a client whose whole
/// thesis is not running the page's layout should not acquire one to draw a logo. **AVIF** would
/// need `dav1d`, a C decoder surface, on untrusted input; the only module in this crate that
/// parses untrusted binary input is pure Rust and stays that way.
///
/// `image/avif-sequence` is here for the same reason the `avis` brand is in the byte sniff: it
/// is the animated profile of the same codec, declined for the same missing decoder, and a
/// `<source>` wearing it is no more usable than one wearing `image/avif`.
pub const DECLINED: &[Format] = &[
    Format {
        name: "SVG",
        suffixes: &["svg", "svgz"],
        mimes: &["image/svg+xml"],
    },
    Format {
        name: "AVIF",
        suffixes: &["avif"],
        mimes: &["image/avif", "image/avif-sequence"],
    },
];

/// One declined format and the claims that name it.
///
/// Named fields rather than a tuple because the three lookups below and the agreement test next
/// door each want a different pair of them, and `.find(|(_, s, _)| ...).map(|(n, _, _)| n)` says
/// nothing about which is which.
pub struct Format {
    /// What the reader is told, through `image_decode::DecodeError::Unsupported`.
    pub name: &'static str,
    /// Path suffixes that claim this format, lowercase and without the dot.
    pub suffixes: &'static [&'static str],
    /// MIME types that claim it, lowercase and without parameters.
    pub mimes: &'static [&'static str],
}

/// The format an address claims by its extension, decidable before a byte is fetched.
///
/// The exchange is worth it because the alternative is not just a wasted transfer. Every caller
/// that fetches on this names the host to the reader before requesting, so without it the
/// reader discloses a contact it never makes.
///
/// Two knock-ons, both accepted: a `.svg` address that redirects to a raster is now refused
/// unseen, and a declined format behind an extensionless address still costs one request before
/// the bytes are sniffed.
pub fn declined_by_url(url: &Url) -> Option<&'static str> {
    // Decoded first. `path_segments` yields the segment as it appears in the URL, so
    // `logo%2Esvg` and `logo%2Egz` carry no `.` at all and would walk straight past this
    // filter — fetched in full, and their host named to the reader — to be refused by the byte
    // sniff afterwards, which is the whole cost this function exists to avoid.
    let last = percent_decoded(url.path_segments()?.next_back()?);
    // Compared case-insensitively rather than lowercased into a `String`: three drawing
    // surfaces ask this per picture per frame, and the table is two entries long.
    let ext = last.rsplit_once('.')?.1;
    DECLINED
        .iter()
        .find(|f| f.suffixes.iter().any(|s| s.eq_ignore_ascii_case(ext)))
        .map(|f| f.name)
}

/// The same question for a `src` a page wrote, resolved against the page it was written on.
///
/// The one shape every caller wants, and the one they each used to write out: a relative `src`
/// carries the extension and no host, so the join is not optional, and a join that fails is
/// **not** a decline — an address hww cannot resolve is a different complaint, made elsewhere,
/// and answering "declined" here would put a format's name on a page that never had one.
///
/// Here rather than beside its callers because there is no one place to put it beside: the
/// extractor asks it while choosing between a `<picture>`'s candidates, `autoload::plan` and
/// `ReaderApp::load_all_images` ask it before requesting, and two drawing surfaces ask it
/// before offering a control. Ungated and tested by the fast job, like the table itself.
pub fn declined_by_href(src: &str, base: &Url) -> Option<&'static str> {
    declined_by_url(&base.join(src).ok()?)
}

/// The format a `type` attribute claims.
///
/// The one claim of the three that the page makes deliberately and about a specific file:
/// a `<source type>` exists to be read by a client deciding whether it can show that candidate,
/// which is the question being asked. Parameters after a `;` are dropped — `image/svg+xml;
/// charset=utf-8` is well-formed and names the same format — and the type is matched
/// case-insensitively, because MIME types are.
///
/// An unrecognised or absent type is `None` and means "try it": this list is what hww *cannot*
/// draw, never a list of what it can, so a format nobody here has heard of goes to the decoder
/// rather than being dropped on suspicion.
pub fn declined_by_mime(mime: &str) -> Option<&'static str> {
    let t = mime.split(';').next().unwrap_or_default().trim();
    DECLINED
        .iter()
        .find(|f| f.mimes.iter().any(|m| m.eq_ignore_ascii_case(t)))
        .map(|f| f.name)
}

/// `%XX` back to bytes, then lossily to text.
///
/// Enough for the one question asked of it — where the last `.` is and what follows it — and
/// small enough not to be worth a dependency. A stray `%` or a bad pair is left as written,
/// which is what a browser does with one and keeps a malformed address from losing its
/// extension.
fn percent_decoded(segment: &str) -> String {
    if !segment.contains('%') {
        return segment.to_owned();
    }
    let b = segment.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = (i + 2 < b.len())
            .then(|| std::str::from_utf8(&b[i + 1..i + 3]).ok())
            .flatten()
            .filter(|_| b[i] == b'%')
            .and_then(|s| u8::from_str_radix(s, 16).ok());
        match hex {
            Some(byte) => {
                out.push(byte);
                i += 3;
            }
            None => {
                out.push(b[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_is_declined_on_its_extension_and_nothing_else() {
        let declined = |u: &str| declined_by_url(&Url::parse(u).unwrap());
        assert_eq!(declined("https://e.org/a.svg"), Some("SVG"));
        assert_eq!(declined("https://e.org/a.SVG"), Some("SVG"));
        assert_eq!(declined("https://e.org/a.svgz?v=2#f"), Some("SVG"));
        assert_eq!(declined("https://e.org/deep/path/a.avif"), Some("AVIF"));
        // The extension is the claim. A directory named for a format is not one, and neither
        // is a query parameter: refusing those would drop pictures that decode.
        assert_eq!(declined("https://e.org/svg/photo.png"), None);
        assert_eq!(declined("https://e.org/photo.png?fmt=svg"), None);
        assert_eq!(declined("https://e.org/notsvg"), None);
        assert_eq!(declined("https://e.org/"), None);
        // The measured case that made the query rule worth spelling out: an image CDN takes
        // its source in a query and answers the same address with WebP for a client that asks
        // for one, so the `.avif` in there is not a claim about what comes back.
        assert_eq!(
            declined("https://e.org/_next/image?url=%2Fa.avif&w=1920"),
            None
        );
    }

    /// A segment is served percent-encoded, so the `.` an extension hangs off need not be a
    /// literal one. Missed, the address is fetched in full and its host named to the reader
    /// before the byte sniff refuses it.
    #[test]
    fn an_encoded_extension_is_still_an_extension() {
        let declined = |u: &str| declined_by_url(&Url::parse(u).unwrap());
        assert_eq!(declined("https://e.org/logo%2Esvg"), Some("SVG"));
        assert_eq!(declined("https://e.org/deep/a%2Eavif?v=2"), Some("AVIF"));
        assert_eq!(declined("https://e.org/logo.sv%67"), Some("SVG"));
        // And the decoding must not invent an extension where the address has none.
        assert_eq!(declined("https://e.org/100%25"), None);
        assert_eq!(declined("https://e.org/photo%2Epng"), None);
    }

    /// Every caller holds the `src` a page wrote rather than a resolved URL, so resolving
    /// against the wrong base — or not resolving at all — puts a control back on every relative
    /// address, which is most of them. The only symptom is a press that contacts nobody and
    /// prints what it knew before it was pressed.
    #[test]
    fn a_declined_address_is_recognised_relative_to_the_page_it_came_from() {
        let base = Url::parse("https://example.org/post/one").expect("a literal URL parses");
        assert_eq!(declined_by_href("../img/logo.svg", &base), Some("SVG"));
        assert_eq!(declined_by_href("/hero.avif", &base), Some("AVIF"));
        assert_eq!(declined_by_href("photo.jpg", &base), None);
        // A join that fails is not a decline: `request_image` has the honest message for an
        // address hww cannot resolve, and it is not a format's name.
        assert_eq!(declined_by_href("http://[bad", &base), None);
        // The case the bytes still have to answer, and the reason a control cannot be dropped
        // on suspicion: an image CDN taking its source in a query says nothing about what it
        // will serve, since the same address answers a browser with WebP.
        assert_eq!(declined_by_href("/_i?url=%2Fa.avif&w=800", &base), None);
    }

    #[test]
    fn a_malformed_escape_keeps_the_rest_of_the_segment() {
        assert_eq!(percent_decoded("a%2Esvg"), "a.svg");
        assert_eq!(percent_decoded("plain.svg"), "plain.svg");
        assert_eq!(percent_decoded("trailing%"), "trailing%");
        assert_eq!(percent_decoded("bad%zzpair.svg"), "bad%zzpair.svg");
        assert_eq!(percent_decoded("cut%2"), "cut%2");
    }

    #[test]
    fn a_type_attribute_is_declined_by_its_type_and_not_its_parameters() {
        assert_eq!(declined_by_mime("image/avif"), Some("AVIF"));
        assert_eq!(declined_by_mime("IMAGE/AVIF"), Some("AVIF"));
        assert_eq!(
            declined_by_mime("image/svg+xml; charset=utf-8"),
            Some("SVG")
        );
        assert_eq!(declined_by_mime(" image/svg+xml "), Some("SVG"));
        // Everything hww might draw, and everything it has never heard of, goes to the
        // decoder: this table is what cannot be drawn, never a list of what can.
        assert_eq!(declined_by_mime("image/webp"), None);
        assert_eq!(declined_by_mime("image/jxl"), None);
        assert_eq!(declined_by_mime(""), None);
        // A near miss is not a match. `image/svg+xml` is the type; the word alone is not.
        assert_eq!(declined_by_mime("image/svg"), None);
        assert_eq!(declined_by_mime("text/avif"), None);
    }
}
