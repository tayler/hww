//! Which pictures `ImagePolicy::Auto` asks for, and which it leaves alone.
//!
//! The policy is "a screenful ahead", not "the page". A gallery is a document with two hundred
//! images in it, and a reader that requested all of them on arrival would contact two hundred
//! hosts for a page someone might close after the first screen, spend the whole worker pool
//! (`ui::net::WORKERS` is four, shared with navigation) doing it, and stall the next click
//! behind a queue of pictures nobody will see. Bounding the request set by the layout band
//! answers all three at once: what is fetched is what is about to be looked at.
//!
//! The band is `measure::Band`, one window either side of the visible one, and it is already
//! the boundary the reading column lays out for real. Reusing it rather than inventing a
//! second distance means the pictures fetched are exactly the ones a scroll can reach without
//! a frame of layout, and there is one number to be wrong about instead of two.
//!
//! # Why the planning is here and not in `ui/`
//!
//! Nothing below needs an `egui::Context`. What it needs is the band's answer, which the column
//! computes, and two sets the reader keeps. Putting the decision here is what lets the tests
//! run in the fast CI job: the mistakes worth catching — re-requesting an evicted picture
//! forever, requesting an address that cannot resolve, letting the queue grow without bound —
//! are all arithmetic on those sets, and none of them are visible in a screenshot.

use std::collections::{HashMap, HashSet};
use url::Url;

/// Auto-loads allowed to be outstanding at once.
///
/// The number that keeps a click responsive. The pool has four workers and one FIFO with no
/// priority lane, so every queued image is a job the reader's next navigation waits behind;
/// eight bounds that wait at roughly one image's worth rather than a page's. It is not a cap on
/// how many pictures a page may load — the rest are planned on later frames as these resolve —
/// so raising it buys a fuller screen slightly sooner and costs exactly the responsiveness this
/// whole module exists to keep.
pub const MAX_OUTSTANDING: usize = 8;

/// Times one picture may be auto-requested on one page.
///
/// Two, and not one, because the LRU is smaller than some pages: `ImageStore` holds 48
/// textures, and a reader who scrolls a long gallery to the bottom and back finds the top of it
/// evicted. One attempt would leave those placeholders permanently empty under the policy whose
/// whole promise is that they fill; the second refills them when they come back into the band.
///
/// And not unbounded, because eviction is what makes the picture requestable again. On a page
/// dense enough to hold more than 48 pictures inside the band at once, every request evicts
/// something else that is also on screen, and an uncapped policy would fetch the same pictures
/// from the same hosts for as long as the reader sat still. A third attempt would not fix that
/// page — the cache is smaller than the screenful — so the cap says so instead of paying for
/// it. The click path is unaffected and has no cap.
pub const MAX_ATTEMPTS: u32 = 2;

/// Choose the pictures to request this frame, in document order and deduplicated.
///
/// `in_band` is every `src` the column drew inside the band, in document order and possibly
/// with repeats. `attempts` is how many times this page has already asked for each, and
/// `pending` how many requests are outstanding. `known` answers whether this `src` is already
/// settled, one way or another — the store holds a state for it, or it is an address the reader
/// will never request, such as a format `image_decode` declines. The caller answers that second
/// half, because the format tables ride with the `gui` feature and this module does not.
///
/// `attempts` and `known` are both consulted, and neither one covers the other. `known` is
/// false for a picture the LRU evicted while it was still on the page, and without a count of
/// attempts that picture would be requested again the moment it was drawn, evicted again a
/// frame later, and so on for as long as the reader sat still. `attempts` is zero for a picture
/// the reader loaded by hand before switching the policy on, and without `known` that one would
/// be fetched a second time.
pub fn plan(
    base: &Url,
    in_band: &[String],
    attempts: &HashMap<String, u32>,
    pending: usize,
    known: &dyn Fn(&str) -> bool,
) -> Vec<String> {
    let room = MAX_OUTSTANDING.saturating_sub(pending);
    if room == 0 {
        return Vec::new();
    }
    let mut srcs: Vec<String> = Vec::new();
    let mut taken: HashSet<&str> = HashSet::new();
    for src in in_band {
        if srcs.len() == room {
            break;
        }
        if attempts.get(src).copied().unwrap_or(0) >= MAX_ATTEMPTS
            || known(src)
            || !taken.insert(src.as_str())
        {
            continue;
        }
        // Resolved here rather than at the request, because an address that will not resolve
        // has no fetch to make. Requesting it would mark the placeholder failed without
        // anything having been contacted, which is a claim about the network that nothing did.
        if base
            .join(src)
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned))
            .is_none()
        {
            continue;
        }
        srcs.push(src.clone());
    }
    srcs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Url {
        Url::parse("https://example.com/article").expect("a literal URL parses")
    }

    fn nothing_known(_: &str) -> bool {
        false
    }

    /// A `src` a declined format claims, which is how `ReaderApp::autoload` answers `known`
    /// for one. Spelled here rather than reached for, because `image_decode` rides with the
    /// `gui` feature and this module is tested without it.
    fn declined(src: &str) -> bool {
        src.ends_with(".svg")
    }

    /// Attempt counts, spelled `("src", n)`.
    fn tries(items: &[(&str, u32)]) -> HashMap<String, u32> {
        items.iter().map(|(s, n)| ((*s).to_owned(), *n)).collect()
    }

    #[test]
    fn a_first_screenful_is_requested_in_document_order() {
        let band = ["/a.png".to_owned(), "/b.png".to_owned()];
        let p = plan(&base(), &band, &tries(&[]), 0, &nothing_known);
        assert_eq!(p, vec!["/a.png".to_owned(), "/b.png".to_owned()]);
    }

    /// The queue bound. Nothing is planned while the pool is already carrying its share, or the
    /// reader's next click queues behind a page of pictures.
    #[test]
    fn the_outstanding_ceiling_is_a_ceiling() {
        let band: Vec<String> = (0..40).map(|i| format!("/{i}.png")).collect();
        let p = plan(&base(), &band, &tries(&[]), 0, &nothing_known);
        assert_eq!(p.len(), MAX_OUTSTANDING);

        let p = plan(&base(), &band, &tries(&[]), 6, &nothing_known);
        assert_eq!(p.len(), MAX_OUTSTANDING - 6);

        let p = plan(&base(), &band, &tries(&[]), MAX_OUTSTANDING, &nothing_known);
        assert!(p.is_empty());
        // And a count above the ceiling does not wrap into a very large allowance.
        let p = plan(&base(), &band, &tries(&[]), 99, &nothing_known);
        assert!(p.is_empty());
    }

    /// The eviction loop this module exists to not have. A picture the LRU dropped is still on
    /// the page and still in the band, so nothing but `attempted` stops it being requested
    /// again, dropped again, and requested again for as long as the reader sits still.
    #[test]
    fn an_evicted_picture_is_not_requested_a_second_time() {
        let band = ["/a.png".to_owned()];
        // `known` is false: the store has forgotten it entirely.
        let p = plan(
            &base(),
            &band,
            &tries(&[("/a.png", MAX_ATTEMPTS)]),
            0,
            &nothing_known,
        );
        assert!(p.is_empty());
    }

    /// The other side of the same cap: one eviction is refilled, because the LRU is smaller
    /// than a long gallery and a reader who scrolls back must not find the top of it
    /// permanently blank under the policy that promises to fill it.
    #[test]
    fn an_evicted_picture_is_refilled_once() {
        let band = ["/a.png".to_owned()];
        let p = plan(&base(), &band, &tries(&[("/a.png", 1)]), 0, &nothing_known);
        assert_eq!(p, vec!["/a.png".to_owned()]);
    }

    /// And the other half: a picture the reader loaded by hand is in the store but was never
    /// auto-attempted, and fetching it again would count one picture as two.
    #[test]
    fn a_picture_already_in_the_store_is_left_alone() {
        let band = ["/a.png".to_owned(), "/b.png".to_owned()];
        let p = plan(&base(), &band, &tries(&[]), 0, &|s| s == "/a.png");
        assert_eq!(p, vec!["/b.png".to_owned()]);
    }

    /// A repeated `src` — a logo in the header and again in the footer, a sprite — is one
    /// request.
    #[test]
    fn a_src_drawn_twice_in_the_band_is_planned_once() {
        let band = ["/logo.png".to_owned(), "/logo.png".to_owned()];
        let p = plan(&base(), &band, &tries(&[]), 0, &nothing_known);
        assert_eq!(p, vec!["/logo.png".to_owned()]);
    }

    /// An address that will not resolve has no request to make, so it is not planned at all
    /// rather than planned and failed.
    #[test]
    fn an_unusable_address_is_not_planned() {
        let band = [
            "http://[".to_owned(),
            "mailto:someone@example.com".to_owned(),
            "/good.png".to_owned(),
        ];
        let p = plan(&base(), &band, &tries(&[]), 0, &nothing_known);
        assert_eq!(p, vec!["/good.png".to_owned()]);
    }

    /// Nothing in the band is nothing to do.
    #[test]
    fn an_empty_band_plans_nothing() {
        let p = plan(&base(), &[], &tries(&[]), 0, &nothing_known);
        assert!(p.is_empty());
    }

    /// A picture this reader will never request is not planned, so a page of declined formats
    /// contacts nobody rather than failing a placeholder per picture.
    #[test]
    fn a_declined_src_is_not_planned() {
        let band = [
            "https://cdn.example.net/diagram.svg".to_owned(),
            "https://img.example.org/photo.jpg".to_owned(),
        ];
        let p = plan(&base(), &band, &tries(&[]), 0, &declined);
        assert_eq!(
            p,
            vec!["https://img.example.org/photo.jpg"],
            "the declined picture is not requested"
        );
    }

    /// The whole band declined is an empty plan.
    #[test]
    fn a_band_of_only_declined_srcs_plans_nothing() {
        let band = [
            "https://cdn.example.net/a.svg".to_owned(),
            "https://cdn.example.net/b.svg".to_owned(),
        ];
        let p = plan(&base(), &band, &tries(&[]), 0, &declined);
        assert!(p.is_empty());
    }
}
