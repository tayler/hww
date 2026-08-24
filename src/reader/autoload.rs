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
//! computes, and three sets the reader keeps. Putting the decision here is what lets the tests
//! run in the fast CI job: the mistakes worth catching — re-requesting an evicted picture
//! forever, naming a host twice, letting the queue grow without bound — are all arithmetic on
//! those sets, and none of them are visible in a screenshot.

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

/// What to request, and what to say about it.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// In document order, deduplicated. The column loads top to bottom.
    pub srcs: Vec<String>,
    /// The hosts among `srcs` this page has not named yet, sorted. Empty is the ordinary case
    /// after the first screenful, and it means the remark is skipped rather than repeated.
    pub hosts: Vec<String>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.srcs.is_empty()
    }
}

/// Choose the pictures to request this frame.
///
/// `in_band` is every `src` the column drew inside the band, in document order and possibly
/// with repeats. `attempts` is how many times this page has already asked for each, `announced`
/// the hosts it has already named, and `pending` how many requests are outstanding. `known`
/// answers whether the image store has any state for a `src` at all.
///
/// `attempts` and `known` are both consulted, and neither one covers the other. `known` is
/// false for a picture the LRU evicted while it was still on the page, and without a count of
/// attempts that picture would be requested again the moment it was drawn, evicted again a
/// frame later, and so on for as long as the reader sat still. `attempts` is zero for a picture
/// the reader loaded by hand before switching the policy on, and without `known` that one would
/// be fetched a second time and counted twice in the disclosures.
pub fn plan(
    base: &Url,
    in_band: &[String],
    attempts: &HashMap<String, u32>,
    announced: &HashSet<String>,
    pending: usize,
    known: &dyn Fn(&str) -> bool,
) -> Plan {
    let room = MAX_OUTSTANDING.saturating_sub(pending);
    if room == 0 {
        return Plan::default();
    }
    let mut plan = Plan::default();
    let mut taken: HashSet<&str> = HashSet::new();
    let mut hosts: HashSet<String> = HashSet::new();
    for src in in_band {
        if plan.srcs.len() == room {
            break;
        }
        if attempts.get(src).copied().unwrap_or(0) >= MAX_ATTEMPTS
            || known(src)
            || !taken.insert(src.as_str())
        {
            continue;
        }
        // Resolved here rather than at the request, because an address that will not resolve
        // has no host to name and no fetch to make. Requesting it would mark the placeholder
        // failed without anything having been contacted, which is a claim about the network
        // that nothing did.
        let Some(host) = base
            .join(src)
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned))
        else {
            continue;
        };
        if !announced.contains(&host) {
            hosts.insert(host);
        }
        plan.srcs.push(src.clone());
    }
    plan.hosts = hosts.into_iter().collect();
    plan.hosts.sort_unstable();
    plan
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

    fn set(items: &[&str]) -> HashSet<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    /// Attempt counts, spelled `("src", n)`.
    fn tries(items: &[(&str, u32)]) -> HashMap<String, u32> {
        items.iter().map(|(s, n)| ((*s).to_owned(), *n)).collect()
    }

    #[test]
    fn a_first_screenful_is_requested_in_document_order() {
        let band = ["/a.png".to_owned(), "/b.png".to_owned()];
        let p = plan(&base(), &band, &tries(&[]), &set(&[]), 0, &nothing_known);
        assert_eq!(p.srcs, vec!["/a.png".to_owned(), "/b.png".to_owned()]);
        assert_eq!(p.hosts, vec!["example.com".to_owned()]);
    }

    /// The queue bound. Nothing is planned while the pool is already carrying its share, or the
    /// reader's next click queues behind a page of pictures.
    #[test]
    fn the_outstanding_ceiling_is_a_ceiling() {
        let band: Vec<String> = (0..40).map(|i| format!("/{i}.png")).collect();
        let p = plan(&base(), &band, &tries(&[]), &set(&[]), 0, &nothing_known);
        assert_eq!(p.srcs.len(), MAX_OUTSTANDING);

        let p = plan(&base(), &band, &tries(&[]), &set(&[]), 6, &nothing_known);
        assert_eq!(p.srcs.len(), MAX_OUTSTANDING - 6);

        let p = plan(
            &base(),
            &band,
            &tries(&[]),
            &set(&[]),
            MAX_OUTSTANDING,
            &nothing_known,
        );
        assert!(p.is_empty());
        // And a count above the ceiling does not wrap into a very large allowance.
        let p = plan(&base(), &band, &tries(&[]), &set(&[]), 99, &nothing_known);
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
            &set(&["example.com"]),
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
        let p = plan(
            &base(),
            &band,
            &tries(&[("/a.png", 1)]),
            &set(&["example.com"]),
            0,
            &nothing_known,
        );
        assert_eq!(p.srcs, vec!["/a.png".to_owned()]);
        // And the host is not named again, having been named the first time.
        assert!(p.hosts.is_empty());
    }

    /// And the other half: a picture the reader loaded by hand is in the store but was never
    /// auto-attempted, and fetching it again would count one picture as two.
    #[test]
    fn a_picture_already_in_the_store_is_left_alone() {
        let band = ["/a.png".to_owned(), "/b.png".to_owned()];
        let p = plan(&base(), &band, &tries(&[]), &set(&[]), 0, &|s| {
            s == "/a.png"
        });
        assert_eq!(p.srcs, vec!["/b.png".to_owned()]);
    }

    /// A repeated `src` — a logo in the header and again in the footer, a sprite — is one
    /// request, and the disclosure has to say one.
    #[test]
    fn a_src_drawn_twice_in_the_band_is_planned_once() {
        let band = ["/logo.png".to_owned(), "/logo.png".to_owned()];
        let p = plan(&base(), &band, &tries(&[]), &set(&[]), 0, &nothing_known);
        assert_eq!(p.srcs, vec!["/logo.png".to_owned()]);
    }

    /// Each host is named once per page. The remark is for "who is being contacted", and a
    /// page of forty pictures from one CDN is one answer to that, repeated forty times only if
    /// this is wrong.
    #[test]
    fn a_host_is_named_once_and_new_hosts_are_still_named() {
        let band = [
            "https://cdn.example.net/a.png".to_owned(),
            "https://img.example.org/b.png".to_owned(),
            "https://cdn.example.net/c.png".to_owned(),
        ];
        let p = plan(
            &base(),
            &band,
            &tries(&[]),
            &set(&["cdn.example.net"]),
            0,
            &nothing_known,
        );
        assert_eq!(p.srcs.len(), 3);
        assert_eq!(p.hosts, vec!["img.example.org".to_owned()]);
    }

    /// An address that will not resolve has no host to name and no request to make, so it is
    /// not planned at all rather than planned and failed.
    #[test]
    fn an_unusable_address_is_not_planned() {
        let band = [
            "http://[".to_owned(),
            "mailto:someone@example.com".to_owned(),
            "/good.png".to_owned(),
        ];
        let p = plan(&base(), &band, &tries(&[]), &set(&[]), 0, &nothing_known);
        assert_eq!(p.srcs, vec!["/good.png".to_owned()]);
        assert_eq!(p.hosts, vec!["example.com".to_owned()]);
    }

    /// Nothing in the band is nothing to do, and no remark either.
    #[test]
    fn an_empty_band_plans_nothing() {
        let p = plan(&base(), &[], &tries(&[]), &set(&[]), 0, &nothing_known);
        assert!(p.is_empty());
        assert!(p.hosts.is_empty());
    }
}
