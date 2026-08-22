//! What the reader knows about how a page arrived, as labelled rows. **No egui types.**
//!
//! This is [`notice`](crate::reader::notice)'s quiet half, and it exists for the same reason:
//! wording decided where `cargo test` can read it rather than inside `ui/`. A notice
//! interrupts the reading column because the reader would otherwise mis-read the page. Nothing
//! here rises to that. Encoding, byte counts, redirect chains, and the tally of cookies thrown
//! away are all retrospective accounting of a request that already went the way it was meant
//! to, and they were previously a single dim eight-point line, `·`-joined, right-aligned, and
//! truncated, competing with the URL for one row of the status strip. On a narrow window its
//! tail was not drawn at all.
//!
//! Moving it behind a click is what buys the labels. `82 kB -> 12431 chars` had to be that
//! terse to fit; `Downloaded` / `Article text` does not, and a reader who has deliberately
//! asked what a page did is owed a sentence rather than a glyph.
//!
//! # What is not here
//!
//! Truncation and a dead rewrite rule were in the same dim line and are now bands, because
//! neither is accounting: one says the article stops early with nothing on screen to say so,
//! the other says the article may not be the one that was asked for. See
//! [`notice::about_page`](crate::reader::notice::about_page).

use crate::fetch::Truncation;
use crate::session::Provenance;
use url::Url;

/// One labelled fact about the page.
///
/// `note` is the second line under the value, set dimmer: the room the panel exists to provide.
/// It carries what a label cannot, such as the list of hosts behind an image count, or the
/// reason a cookie tally is always a count of things discarded rather than things kept.
///
/// Dimmer and *not* smaller, deliberately. `theme` maps `TextStyle::Small` to `chrome_font`,
/// the same size the value already uses, so there is no smaller step to take without inventing
/// a font size outside `theme.rs`. Colour and position carry the distinction instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub label: &'static str,
    pub value: String,
    pub note: Option<String>,
}

impl Row {
    fn new(label: &'static str, value: impl Into<String>) -> Self {
        Self {
            label,
            value: value.into(),
            note: None,
        }
    }

    fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }
}

/// The reader's own counters, which live under `ui/` and cannot be reached from here.
///
/// Built by `ImageStore::counts`, so the field list sits next to the fields it reads. These
/// are **cumulative for the session**, not per page: the store keeps decoded images and the
/// hosts they came from across navigations, so a per-page number would be a different quantity
/// than the one being counted. The rows say so rather than letting the panel imply otherwise.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Counts {
    pub images: usize,
    /// Distinct hosts contacted for images. Sorted by the caller.
    pub hosts: Vec<String>,
    pub cookie_attempts: usize,
    pub referer_requests: usize,
}

/// Everything worth saying about how this page arrived, in reading order.
pub fn rows(prov: &Provenance, counts: &Counts) -> Vec<Row> {
    let mut out = vec![Row::new("Address", prov.final_url.as_str())];

    // Only when they differ, which is a rewrite or a redirect. On the great majority of pages
    // this row would just repeat the one above it.
    if prov.requested != prov.final_url {
        out.push(Row::new("Requested", prov.requested.as_str()));
    }
    if let Some(to) = &prov.rewritten_to {
        let host = to.host_str().unwrap_or(to.as_str());
        out.push(
            Row::new("Rewrite rule", format!("sent to {host}"))
                .with_note("Compiled in, never fetched. hww ships no updatable rule list."),
        );
    }
    if !prov.hops.is_empty() {
        out.push(
            Row::new("Redirects", plural(prov.hops.len(), "hop")).with_note(
                prov.hops
                    .iter()
                    .map(Url::as_str)
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
        );
    }

    out.push(Row::new("Server answered", prov.status.to_string()));
    out.push(Row::new("Text encoding", prov.encoding));
    out.push(Row::new("Downloaded", human_bytes(prov.bytes)));
    out.push(Row::new(
        "Article text",
        format!("{} characters", group_digits(prov.chars)),
    ));

    // Deliberately the same event as the truncation band. The band is the alarm and is seen
    // without asking; this is the record, and a record with a hole where the interesting case
    // goes is worse than a line that appears twice.
    if let Some(v) = truncation_value(prov.truncation) {
        out.push(Row::new("Transfer", v));
    }

    let cookies = prov.cookie_attempts + counts.cookie_attempts;
    if cookies > 0 {
        out.push(
            Row::new("Cookies discarded", group_digits(cookies)).with_note(
                "hww is built with no cookie store at all, so these were counted and dropped. \
                 The capability is absent, not merely unused.",
            ),
        );
    }
    if counts.images > 0 {
        out.push(
            Row::new(
                "Images loaded",
                format!(
                    "{}, from {} (this session)",
                    counts.images,
                    plural(counts.hosts.len(), "host")
                ),
            )
            .with_note(counts.hosts.join("\n")),
        );
    }
    if counts.referer_requests > 0 {
        out.push(
            Row::new(
                "Referer sent",
                format!(
                    "{} (this session)",
                    plural(counts.referer_requests, "request")
                ),
            )
            .with_note(
                "Scheme and host only, never the path, and only on images you asked for. \
                 Document fetches carry no Referer.",
            ),
        );
    }
    out
}

/// The terse counterpart to `notice`'s truncation prose: a value for a labelled row, not a
/// sentence for a band.
fn truncation_value(t: Truncation) -> Option<String> {
    Some(match t {
        Truncation::Complete => return None,
        Truncation::AtCap(n) => format!("cut at the {} MB cap", n / 1_000_000),
        Truncation::MaybeAtCap(n) => format!("reached the {} MB cap; may be cut", n / 1_000_000),
        Truncation::Timeout(d) => format!("timed out after {}s", d.as_secs()),
        Truncation::Incomplete(n) => {
            format!("connection dropped at {} bytes", group_digits(n))
        }
    })
}

/// `1 hop` / `2 hops`, so no row reads `1 hop(s)`.
///
/// The parenthesised plural is what fits when a string is fighting for space on a status
/// strip. Nothing here is.
fn plural(n: usize, unit: &str) -> String {
    if n == 1 {
        format!("1 {unit}")
    } else {
        format!("{n} {unit}s")
    }
}

/// `12,431`. Rust's `format!` has no grouping and this crate takes no dependency for one.
///
/// Not decoration: a bare `12431` next to `82 kB` is exactly the unreadability the panel was
/// built to fix, and a character count is the number most likely to run to five digits.
pub(crate) fn group_digits(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Wire bytes, in the units a reader thinks in. Powers of ten, because that is what a transfer
/// is quoted in.
pub(crate) fn human_bytes(n: usize) -> String {
    if n >= 1_000_000 {
        format!("{:.1} MB", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{} kB", n / 1_000)
    } else {
        format!("{n} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn find<'a>(rows: &'a [Row], label: &str) -> Option<&'a Row> {
        rows.iter().find(|r| r.label == label)
    }

    fn prov() -> Provenance {
        Provenance::fixture("https://example.com/a")
    }

    /// The rows a page always has, so the panel is never empty on a page that loaded.
    #[test]
    fn a_clean_page_still_reports_how_it_arrived() {
        let rows = rows(&prov(), &Counts::default());
        for label in [
            "Address",
            "Server answered",
            "Text encoding",
            "Downloaded",
            "Article text",
        ] {
            assert!(find(&rows, label).is_some(), "missing {label}");
        }
        // Every conditional row stays away on a page that did none of it.
        for label in [
            "Requested",
            "Rewrite rule",
            "Redirects",
            "Transfer",
            "Images loaded",
            "Referer sent",
        ] {
            assert!(find(&rows, label).is_none(), "unexpected {label}");
        }
    }

    /// `Requested` repeats `Address` on nearly every page, so it earns its row only when the
    /// request did not end where it started.
    #[test]
    fn requested_appears_only_when_it_differs_from_the_address() {
        assert!(find(&rows(&prov(), &Counts::default()), "Requested").is_none());

        let mut p = prov();
        p.final_url = Url::parse("https://example.com/b").unwrap();
        let rows = rows(&p, &Counts::default());
        assert_eq!(
            find(&rows, "Requested").unwrap().value,
            "https://example.com/a"
        );
    }

    /// The counters are session-cumulative (`ImageStore` keeps them across navigations), so a
    /// panel with room for real labels must not let them read as facts about this page.
    #[test]
    fn the_session_counters_say_they_are_session_counters() {
        let counts = Counts {
            images: 3,
            hosts: vec!["cdn.example.net".to_owned(), "img.example.org".to_owned()],
            cookie_attempts: 0,
            referer_requests: 2,
        };
        let rows = rows(&prov(), &counts);
        let images = find(&rows, "Images loaded").unwrap();
        assert!(images.value.contains("this session"));
        assert!(images.value.contains("2 hosts"));
        assert!(images.note.as_ref().unwrap().contains("cdn.example.net"));
        assert!(
            find(&rows, "Referer sent")
                .unwrap()
                .value
                .contains("this session")
        );
    }

    /// The strip summed page and image cookie attempts into one number; the row keeps that,
    /// because the reader's question is "how many did this session throw away", not "which
    /// fetch saw them".
    #[test]
    fn cookie_attempts_sum_the_page_and_its_images() {
        let mut p = prov();
        p.cookie_attempts = 2;
        let counts = Counts {
            cookie_attempts: 3,
            ..Counts::default()
        };
        assert_eq!(
            find(&rows(&p, &counts), "Cookies discarded").unwrap().value,
            "5"
        );

        p.cookie_attempts = 0;
        assert!(find(&rows(&p, &Counts::default()), "Cookies discarded").is_none());
    }

    /// Every way a body can arrive short gets a value, and a whole one gets no row.
    #[test]
    fn every_truncation_has_a_value_and_complete_has_none() {
        assert_eq!(truncation_value(Truncation::Complete), None);
        for t in [
            Truncation::AtCap(5_000_000),
            Truncation::MaybeAtCap(5_000_000),
            Truncation::Timeout(Duration::from_secs(15)),
            Truncation::Incomplete(812_345),
        ] {
            let mut p = prov();
            p.truncation = t;
            let rows = rows(&p, &Counts::default());
            assert!(find(&rows, "Transfer").is_some(), "{t:?} produced no row");
        }
    }

    /// The one place a `→` could reach the screen through this module. egui's embedded
    /// proportional faces carry none, so it would render as tofu; see `notice.rs`.
    #[test]
    fn no_row_contains_an_arrow() {
        let mut p = prov();
        p.rewritten_to = Some(Url::parse("https://alt.example.net/a").unwrap());
        p.hops = vec![Url::parse("https://hop.example/1").unwrap()];
        p.truncation = Truncation::Incomplete(812_345);
        p.cookie_attempts = 1;
        let counts = Counts {
            images: 1,
            hosts: vec!["cdn.example.net".to_owned()],
            cookie_attempts: 1,
            referer_requests: 1,
        };
        for row in rows(&p, &counts) {
            assert!(!row.value.contains('→'), "arrow in {:?}", row.value);
            assert!(!row.label.contains('→'));
            if let Some(n) = &row.note {
                assert!(!n.contains('→'), "arrow in {n:?}");
            }
        }
    }

    #[test]
    fn plural_never_reads_one_hops() {
        assert_eq!(plural(1, "hop"), "1 hop");
        assert_eq!(plural(2, "hop"), "2 hops");
        assert_eq!(plural(0, "hop"), "0 hops");
    }

    #[test]
    fn group_digits_breaks_on_thousands() {
        assert_eq!(group_digits(0), "0");
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(1_000), "1,000");
        assert_eq!(group_digits(12_431), "12,431");
        assert_eq!(group_digits(1_000_000), "1,000,000");
    }

    /// Moved out of `ui/app.rs`, where it had no test. The boundaries are where a unit
    /// changes, which is the only place the formatting can be wrong.
    #[test]
    fn human_bytes_switches_units_on_powers_of_ten() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(999), "999 B");
        assert_eq!(human_bytes(1_000), "1 kB");
        assert_eq!(human_bytes(999_999), "999 kB");
        assert_eq!(human_bytes(1_000_000), "1.0 MB");
    }
}
