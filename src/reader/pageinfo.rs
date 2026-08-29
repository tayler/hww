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
//! Truncation, a dead rewrite rule, a thin extraction, and RTL text are bars, because none of
//! them is accounting. The bar is the alarm; this module is the record, including after a bar
//! is closed. See [`notice::about_page`](crate::reader::notice::about_page).

use crate::fetch::Truncation;
use crate::session::Provenance;
use std::time::Duration;
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
    /// Which heading the row sits under. Rows are emitted grouped, so the panel draws a
    /// heading when the group changes and never sorts.
    pub group: Group,
    pub label: &'static str,
    pub value: String,
    pub note: Option<String>,
}

/// The four questions the panel answers, in the order a reader asks them: where am I, how did
/// the request get here, what came back, and who else was contacted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Address,
    Route,
    Response,
    ThirdParties,
}

impl Group {
    /// The heading, as drawn: a letter-spaced chrome label.
    pub fn title(self) -> &'static str {
        match self {
            Group::Address => "Address",
            Group::Route => "Route",
            Group::Response => "Response",
            Group::ThirdParties => "Third parties",
        }
    }
}

impl Row {
    fn new(group: Group, label: &'static str, value: impl Into<String>) -> Self {
        Self {
            group,
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
/// are **per page**: `ImageStore::clear_page` drops them when a page commits, so every row here
/// answers the same question as the rest of the panel, how the page on screen arrived. They
/// were once session totals and had to carry a "(this session)" qualifier to say so, which is
/// a different quantity sitting under the same heading as the rest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Counts {
    pub images: usize,
    /// Distinct hosts contacted for images. Sorted by the caller.
    pub hosts: Vec<String>,
    pub cookie_attempts: usize,
    pub referer_requests: usize,
}

/// How the page in the column got there.
///
/// Every other row in this module describes a request, and one of these two answers did not
/// make one. Named rather than a bare `bool` or `Option<Duration>` so the call site reads as a
/// statement about the page rather than as a flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrival {
    Fetched,
    /// Back or forward re-opened a document the reader still had. `age` is how long ago the
    /// fetch the rest of these rows describe actually happened.
    FromMemory {
        age: Duration,
    },
    /// hww assembled this page itself: the bookmarks, and whatever joins it. **No document was
    /// requested.**
    ///
    /// The reason this variant exists rather than a fabricated `status: 200`. A built page's
    /// `Provenance` carries its address and zeros, and a panel that printed `Server answered 0`
    /// and `Downloaded 0 bytes` would be reporting a response nobody received; one that printed
    /// `200` would be worse, because it would be the plausible-looking lie `docs/findings.md`
    /// names three times. This is what guarantees those zeros are never drawn: [`rows`] emits the
    /// address, one Route row, and no Response row at all.
    ///
    /// It does emit Third parties. "No request was made" was true of this variant until the
    /// bookmarks grew its column of site marks, and the difference is the one this doc has to
    /// keep straight: no *document* was fetched to draw the page, and the third-party requests
    /// the page then made on its own are reported exactly as they are on a page that was.
    Built,
}

/// Whether the page on screen is in the reader's bookmarks.
///
/// Named rather than a bare `bool` because [`rows`] already takes one, and two adjacent bools at
/// a call site is a swap that compiles. See [`crate::reader::archive`] for the doctrine this row
/// discloses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bookmarked {
    Yes,
    No,
    /// Keeping is switched off in the settings, so "no" would be true and unhelpful: it would
    /// read as something the reader could change by pressing a key, and they cannot until the
    /// switch goes back on.
    Off,
}

/// Everything worth saying about how this page arrived, in reading order.
///
/// `rtl` is the document's, not the provenance's: epaint has no bidi, so a Hebrew or Arabic
/// page is a fact about what was extracted, and it has to survive the caution being closed.
pub fn rows(
    prov: &Provenance,
    counts: &Counts,
    rtl: bool,
    arrival: Arrival,
    bookmarked: Bookmarked,
) -> Vec<Row> {
    use Group::*;
    let mut out = vec![Row::new(Address, "Address", prov.final_url.as_str())];

    // A page hww built has no response and no third parties, so those headings are absent
    // rather than filled with zeros. Everything below this point describes a request, and this
    // page is the one kind that never made one.
    if arrival == Arrival::Built {
        out.push(
            Row::new(Route, "Built by hww", "no request was made").with_note(
                "This page is hww's own, assembled on this machine from what you have saved. \
                 Nothing was fetched to draw it, so there is no response to report.",
            ),
        );
        // The one thing a built page does contact the network for: the site icons in the
        // bookmarks's left column. `Arrival::Built` used to end this function here, and the note
        // above used to finish "and no third party to report" — which stopped being true the
        // day the bookmarks grew a favicon column, and a panel that says a page contacted nobody
        // while it is contacting twenty hosts is the plausible-looking lie this module exists
        // to refuse. Every row below is the same row a fetched page gets, from the same
        // counters.
        out.extend(third_parties(counts, 0));
        return out;
    }

    // Only when they differ, which is a rewrite or a redirect. On the great majority of pages
    // this row would just repeat the one above it.
    if prov.requested != prov.final_url {
        out.push(Row::new(Address, "Requested", prov.requested.as_str()));
    }
    // The archive doctrine's disclosure, and the last Address row: the reader asking how this
    // page arrived is the reader asking what hww has of it. It sits under Address rather than
    // under a heading of its own because it is a fact about this address and not about the
    // request, and a group of one row would draw a heading for a single word.
    out.push(match bookmarked {
        Bookmarked::Yes => Row::new(Address, "Bookmarked", "yes").with_note(
            "hww keeps the address, the title, and the address of the site's icon, so the \
                 bookmarks can draw it. Nothing else about this page.",
        ),
        Bookmarked::No => Row::new(Address, "Bookmarked", "no"),
        Bookmarked::Off => Row::new(Address, "Bookmarked", "no")
            .with_note("hww is set not to keep pages; change Keep pages in Settings."),
    });
    // The first Route row, and ahead of every Response row, which is the whole disclosure
    // argument: Route answers how the request got here, and this is the row that corrects it.
    // A reader who is about to read `Downloaded 82 kB` has to have been told first that those
    // bytes did not move for this visit.
    //
    // "the document", precisely: `ensure_favicon` does re-request the site icon on a restored
    // page, because `clear_page` dropped it. "Nothing was requested" would be a lie of exactly
    // the kind this panel exists not to tell.
    if let Arrival::FromMemory { age } = arrival {
        out.push(
            Row::new(
                Route,
                "Restored",
                format!("from memory, fetched {}", ago(age)),
            )
            .with_note(
                "Back and forward re-open the document hww still had, so it was not requested \
                 again and every row below describes that original fetch. Pictures are not kept \
                 and are offered again, as on any page.",
            ),
        );
    }
    if let Some(to) = &prov.rewritten_to {
        let host = to.host_str().unwrap_or(to.as_str());
        out.push(
            Row::new(Route, "Rewrite rule", format!("sent to {host}"))
                .with_note("Compiled in, never fetched. hww ships no updatable rule list."),
        );
    }
    if let Some(r) = &prov.profile {
        let what: Vec<String> = r
            .fields
            .iter()
            .map(|f| format!("{} {}", f.field, f.outcome))
            .collect();
        out.push(
            Row::new(
                Route,
                "Site profile",
                format!("{}: {}", r.host, what.join(", ")),
            )
            .with_note(
                "Compiled in, never fetched. A field that matched nothing is listed, so a \
                 stale profile is visible here rather than silently doing nothing.",
            ),
        );
    }
    if !prov.hops.is_empty() {
        out.push(
            Row::new(Route, "Redirects", plural(prov.hops.len(), "hop")).with_note(
                prov.hops
                    .iter()
                    .map(Url::as_str)
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
        );
    }

    out.push(Row::new(
        Response,
        "Server answered",
        prov.status.to_string(),
    ));
    out.push(Row::new(Response, "Text encoding", prov.encoding));
    out.push(Row::new(Response, "Downloaded", human_bytes(prov.bytes)));
    // Under Response, because what came back is a fact about the response and not about how the
    // request travelled, and *before* `Article text`, because it is the context that count has
    // to be read in. `Article text` keeps its label: `a_clean_page_still_reports_how_it_arrived`
    // asserts it is always present, and renaming it for one kind of page would put a different
    // question under the same heading.
    if let Some(f) = &prov.feed {
        out.push(Row::new(Response, "Format", f.to_string()).with_note(
            "Read with an XML parser rather than the HTML one. Entry summaries are the \
             publisher's own, and hww requested none of the posts behind them.",
        ));
    }
    let mut article = Row::new(
        Response,
        "Article text",
        format!("{} characters", group_digits(prov.chars)),
    );
    // Same event as the thin-extraction bar. The count alone is easy to miss; the note is
    // the record after the bar is closed.
    //
    // Not on a feed, for the same reason `about_page` does not draw the bar on one:
    // `ir::block_text_len` counts a `Figure` as its caption alone, so a picture-only feed scores
    // under the floor while having parsed perfectly, and the note would be false about it.
    if prov.chars < crate::ir::THIN_TEXT && prov.feed.is_none() {
        article = article.with_note(
            "Below the extraction floor. This page may need JavaScript to show its content.",
        );
    }
    out.push(article);

    // Deliberately the same event as the truncation bar. The bar is the alarm and is seen
    // without asking; this is the record, and a record with a hole where the interesting case
    // goes is worse than a line that appears twice.
    if let Some(v) = truncation_value(prov.truncation) {
        out.push(Row::new(Response, "Transfer", v));
    }
    if rtl {
        out.push(Row::new(Response, "Layout", "right-to-left").with_note(
            "epaint has no bidi, so a Hebrew or Arabic run is drawn in the wrong order, \
                 or as missing glyphs.",
        ));
    }

    out.extend(third_parties(counts, prov.cookie_attempts));
    out
}

/// The third-party rows, shared by a fetched page and a built one.
///
/// `document_cookies` is what the page's own response tried to set, which a built page has none
/// of; everything else is counted the same way whether the request went out for an article
/// picture or for a bookmarks list row's icon, because it is the same request through the same door.
fn third_parties(counts: &Counts, document_cookies: usize) -> Vec<Row> {
    use Group::*;
    let mut out = Vec::new();
    let cookies = document_cookies + counts.cookie_attempts;
    if cookies > 0 {
        out.push(
            Row::new(ThirdParties, "Cookies discarded", group_digits(cookies)).with_note(
                "hww is built with no cookie store at all, so these were counted and dropped. \
                 The capability is absent, not merely unused.",
            ),
        );
    }
    if counts.images > 0 {
        out.push(
            Row::new(
                ThirdParties,
                "Images loaded",
                format!(
                    "{}, from {}",
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
                ThirdParties,
                "Referer sent",
                plural(counts.referer_requests, "request"),
            )
            .with_note(
                "Scheme and host only, never the path, and only on image requests. Document \
                 fetches carry no Referer, and neither do requests made on behalf of a page \
                 hww built itself, which has no origin to send.",
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

/// How long ago, at the resolution a sentence deserves.
///
/// Floored at a minute rather than counting seconds, for two reasons. The common case is
/// twenty seconds — follow a link, come back — and "fetched 23 seconds ago" invites the reader
/// to care about a number that means nothing. And a row whose text changes every second is a
/// row `hww-shot` cannot baseline, so the scene that photographs it would have to be `volatile`
/// and would then never be compared again.
fn ago(d: Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        "less than a minute ago".to_owned()
    } else if secs < 3600 {
        format!("{} ago", plural((secs / 60) as usize, "minute"))
    } else {
        format!("{} ago", plural((secs / 3600) as usize, "hour"))
    }
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
        let rows = rows(
            &prov(),
            &Counts::default(),
            false,
            Arrival::Fetched,
            Bookmarked::No,
        );
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
            "Site profile",
            "Redirects",
            "Format",
            "Transfer",
            "Layout",
            "Images loaded",
            "Referer sent",
        ] {
            assert!(find(&rows, label).is_none(), "unexpected {label}");
        }
    }

    /// The whole reason [`Arrival::Built`] exists rather than a fabricated `status: 200`.
    ///
    /// A built page's provenance is its address and zeros. Every Response row would be a claim
    /// about a response nobody received. The address, one Route row, and — once the bookmarks grew
    /// a column of site marks — whatever those marks contacted, which the next test pins.
    #[test]
    fn a_built_page_reports_no_response() {
        let p = Provenance::built(Url::parse(crate::reader::archive::BOOKMARKS).unwrap());
        let rows = rows(
            &p,
            &Counts::default(),
            false,
            Arrival::Built,
            Bookmarked::No,
        );
        assert_eq!(
            find(&rows, "Address").map(|r| r.value.as_str()),
            Some("hww:bookmarks")
        );
        let built = find(&rows, "Built by hww").expect("the one Route row");
        assert_eq!(built.group, Group::Route);
        assert!(
            built
                .note
                .as_deref()
                .unwrap()
                .contains("Nothing was fetched")
        );
        for label in [
            "Server answered",
            "Text encoding",
            "Downloaded",
            "Format",
            "Article text",
            "Cookies discarded",
            "Bookmarked",
            "Restored",
        ] {
            assert!(
                find(&rows, label).is_none(),
                "a built page reported {label}"
            );
        }
        assert!(
            rows.iter()
                .all(|r| matches!(r.group, Group::Address | Group::Route))
        );
        // And the zeros really are in the provenance, so nothing but this variant is stopping
        // them from being drawn.
        assert_eq!(p.status, 0);
        assert_eq!(p.bytes, 0);
    }

    /// The bookmarks' icon column is the one thing a page hww built asks the network for, and a
    /// panel that answered "no third party to report" while twenty hosts were being contacted
    /// would be the plausible-looking lie this module exists to refuse.
    #[test]
    fn a_built_page_reports_the_hosts_its_site_marks_contacted() {
        let p = Provenance::built(Url::parse(crate::reader::archive::BOOKMARKS).unwrap());
        let counts = Counts {
            images: 2,
            hosts: vec!["a.example".to_owned(), "b.example".to_owned()],
            cookie_attempts: 1,
            // Zero, and not because the reader switched the setting off: `hww:bookmarks` has an
            // opaque origin, so `fetch::page_origin` yields nothing and `record_request` counts
            // no disclosure that never left. A built page reporting `Referer sent` would be
            // this module's own failure mode.
            referer_requests: 0,
        };
        let rows = rows(&p, &counts, false, Arrival::Built, Bookmarked::No);
        let loaded = find(&rows, "Images loaded").expect("the marks are reported");
        assert_eq!(loaded.group, Group::ThirdParties);
        assert_eq!(loaded.value, "2, from 2 hosts");
        assert_eq!(loaded.note.as_deref(), Some("a.example\nb.example"));
        assert!(find(&rows, "Cookies discarded").is_some());
        assert!(find(&rows, "Referer sent").is_none());
        // Still no response: the marks are requests this page made, not a response it got.
        assert!(find(&rows, "Server answered").is_none());
        // And the note above them no longer says nothing was contacted.
        let built = find(&rows, "Built by hww").expect("the one Route row");
        assert!(!built.note.as_deref().unwrap().contains("third party"));
    }

    /// The archive doctrine's disclosure: whether the page on screen is bookmarked, said where the
    /// reader asks how it arrived.
    #[test]
    fn a_fetched_page_says_whether_it_is_kept() {
        for (kept, value, has_note) in [
            (Bookmarked::Yes, "yes", true),
            (Bookmarked::No, "no", false),
            (Bookmarked::Off, "no", true),
        ] {
            let rows = rows(&prov(), &Counts::default(), false, Arrival::Fetched, kept);
            let row = find(&rows, "Bookmarked").expect("the disclosure row");
            assert_eq!(row.group, Group::Address);
            assert_eq!(row.value, value);
            assert_eq!(row.note.is_some(), has_note, "{kept:?}");
        }
        // The switch being off is a different sentence from merely not having kept this one.
        let off = rows(
            &prov(),
            &Counts::default(),
            false,
            Arrival::Fetched,
            Bookmarked::Off,
        );
        assert!(
            find(&off, "Bookmarked")
                .unwrap()
                .note
                .as_deref()
                .unwrap()
                .contains("Settings")
        );
    }

    /// `Requested` repeats `Address` on nearly every page, so it earns its row only when the
    /// request did not end where it started.
    #[test]
    fn requested_appears_only_when_it_differs_from_the_address() {
        assert!(
            find(
                &rows(
                    &prov(),
                    &Counts::default(),
                    false,
                    Arrival::Fetched,
                    Bookmarked::No
                ),
                "Requested"
            )
            .is_none()
        );

        let mut p = prov();
        p.final_url = Url::parse("https://example.com/b").unwrap();
        let rows = rows(
            &p,
            &Counts::default(),
            false,
            Arrival::Fetched,
            Bookmarked::No,
        );
        assert_eq!(
            find(&rows, "Requested").unwrap().value,
            "https://example.com/a"
        );
    }

    /// The image counters are the page's own (`ImageStore::clear_page` drops them at commit),
    /// so the rows state them plainly instead of qualifying them out of the panel's subject.
    #[test]
    fn the_image_counters_are_stated_as_facts_about_this_page() {
        let counts = Counts {
            images: 3,
            hosts: vec!["cdn.example.net".to_owned(), "img.example.org".to_owned()],
            cookie_attempts: 0,
            referer_requests: 2,
        };
        let rows = rows(&prov(), &counts, false, Arrival::Fetched, Bookmarked::No);
        let images = find(&rows, "Images loaded").unwrap();
        assert_eq!(images.value, "3, from 2 hosts");
        assert!(images.note.as_ref().unwrap().contains("cdn.example.net"));
        assert_eq!(find(&rows, "Referer sent").unwrap().value, "2 requests");
        for r in &rows {
            assert!(
                !r.value.contains("session"),
                "session total in {:?}",
                r.value
            );
        }
    }

    /// The strip summed page and image cookie attempts into one number; the row keeps that,
    /// because the reader's question is "how many did this page throw away", not "which fetch
    /// saw them".
    #[test]
    fn cookie_attempts_sum_the_page_and_its_images() {
        let mut p = prov();
        p.cookie_attempts = 2;
        let counts = Counts {
            cookie_attempts: 3,
            ..Counts::default()
        };
        assert_eq!(
            find(
                &rows(&p, &counts, false, Arrival::Fetched, Bookmarked::No),
                "Cookies discarded"
            )
            .unwrap()
            .value,
            "5"
        );

        p.cookie_attempts = 0;
        assert!(
            find(
                &rows(
                    &p,
                    &Counts::default(),
                    false,
                    Arrival::Fetched,
                    Bookmarked::No
                ),
                "Cookies discarded"
            )
            .is_none()
        );
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
            let rows = rows(
                &p,
                &Counts::default(),
                false,
                Arrival::Fetched,
                Bookmarked::No,
            );
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
        for row in rows(&p, &counts, false, memory(), Bookmarked::No) {
            assert!(!row.value.contains('→'), "arrow in {:?}", row.value);
            assert!(!row.label.contains('→'));
            if let Some(n) = &row.note {
                assert!(!n.contains('→'), "arrow in {n:?}");
            }
        }
    }

    /// Rows come out grouped and the groups come out in reading order, so the panel can draw a
    /// heading on every change of group without sorting: a sort would reorder the rows inside
    /// a group, and "Address" before "Requested" is an order that means something.
    #[test]
    fn rows_are_grouped_in_reading_order() {
        let mut p = prov();
        p.rewritten_to = Some(Url::parse("https://alt.example.net/a").unwrap());
        p.final_url = Url::parse("https://www.example.org/a").unwrap();
        p.hops = vec![Url::parse("https://hop.example/1").unwrap()];
        p.cookie_attempts = 1;
        let counts = Counts {
            images: 1,
            hosts: vec!["cdn.example.net".to_owned()],
            cookie_attempts: 0,
            referer_requests: 1,
        };
        let order = [
            Group::Address,
            Group::Route,
            Group::Response,
            Group::ThirdParties,
        ];
        // Both arrivals: the restored row is emitted mid-Route and must not break the run.
        for arrival in [Arrival::Fetched, memory()] {
            let rows = rows(&p, &counts, false, arrival, Bookmarked::No);
            let mut last = 0usize;
            for r in &rows {
                let at = order.iter().position(|g| *g == r.group).unwrap();
                assert!(at >= last, "{} is out of group order", r.label);
                last = at;
            }
            for g in order {
                assert!(rows.iter().any(|r| r.group == g), "{g:?} is empty here");
                assert!(!g.title().is_empty());
            }
        }
    }

    #[test]
    fn plural_never_reads_one_hops() {
        assert_eq!(plural(1, "hop"), "1 hop");
        assert_eq!(plural(2, "hop"), "2 hops");
        assert_eq!(plural(0, "hop"), "0 hops");
    }

    // ------------------------------------------------- a page that was not fetched again

    fn memory() -> Arrival {
        Arrival::FromMemory {
            age: Duration::from_secs(240),
        }
    }

    /// The disclosure argument in one assertion. The panel draws rows in the order this
    /// function emits them, so a "Restored" row after `Downloaded` would let the reader take
    /// eleven rows of fetch accounting at face value before being told no fetch happened.
    #[test]
    fn a_restored_page_says_so_before_it_reports_the_fetch() {
        let rows = rows(&prov(), &Counts::default(), false, memory(), Bookmarked::No);
        let at = rows
            .iter()
            .position(|r| r.label == "Restored")
            .expect("a restored page has the row");
        assert_eq!(rows[at].group, Group::Route);
        assert!(rows[at].value.contains("from memory"));
        let first_response = rows
            .iter()
            .position(|r| r.group == Group::Response)
            .expect("every page reports a response");
        assert!(at < first_response, "the qualifier comes after the numbers");
    }

    /// The note must not claim more than it can. `ensure_favicon` re-requests the site icon on
    /// a restored page, so "nothing was requested" would be false.
    #[test]
    fn the_restored_note_is_about_the_document_and_not_the_page() {
        let rows = rows(&prov(), &Counts::default(), false, memory(), Bookmarked::No);
        let note = find(&rows, "Restored")
            .and_then(|r| r.note.as_deref())
            .expect("the row carries a note");
        assert!(note.contains("document"));
        assert!(!note.to_lowercase().contains("nothing was requested"));
    }

    #[test]
    fn a_fetched_page_has_no_restored_row() {
        let rows = rows(
            &prov(),
            &Counts::default(),
            false,
            Arrival::Fetched,
            Bookmarked::No,
        );
        assert!(find(&rows, "Restored").is_none());
    }

    /// Floored at a minute, which is what lets a scene photograph this row and be compared.
    #[test]
    fn the_age_of_a_restored_page_reads_as_a_sentence() {
        for (secs, want) in [
            (0, "less than a minute ago"),
            (59, "less than a minute ago"),
            (60, "1 minute ago"),
            (120, "2 minutes ago"),
            (3599, "59 minutes ago"),
            (3600, "1 hour ago"),
            (7200, "2 hours ago"),
        ] {
            assert_eq!(ago(Duration::from_secs(secs)), want, "at {secs}s");
        }
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

    /// Charter point 5 for profiles: what the profile did is a row, dead fields included, and
    /// the note says where the table comes from.
    #[test]
    fn a_profiled_page_reports_its_profile_as_a_row() {
        let mut p = prov();
        p.profile = Some(crate::profile::ProfileReport {
            host: "example.com".into(),
            fields: vec![
                crate::profile::FieldReport {
                    field: "strip",
                    outcome: crate::profile::Outcome::Matched(2),
                },
                crate::profile::FieldReport {
                    field: "strip",
                    outcome: crate::profile::Outcome::Dead,
                },
            ],
        });
        let rows = rows(
            &p,
            &Counts::default(),
            false,
            Arrival::Fetched,
            Bookmarked::No,
        );
        let row = find(&rows, "Site profile").expect("a row");
        assert_eq!(
            row.value,
            "example.com: strip matched 2, strip matched nothing"
        );
        assert!(
            row.note
                .as_deref()
                .is_some_and(|n| n.contains("Compiled in"))
        );
    }

    /// The thin-extraction bar's quiet half: the character count is always there, and below
    /// the floor it carries the same diagnosis the bar does, so closing the bar loses nothing.
    #[test]
    fn a_thin_page_notes_the_extraction_floor_on_article_text() {
        let mut p = prov();
        p.chars = crate::ir::THIN_TEXT - 1;
        let rows_thin = rows(
            &p,
            &Counts::default(),
            false,
            Arrival::Fetched,
            Bookmarked::No,
        );
        let thin = find(&rows_thin, "Article text").unwrap();
        assert!(
            thin.note
                .as_deref()
                .is_some_and(|n| n.contains("JavaScript")),
            "{thin:?}"
        );
        p.chars = crate::ir::THIN_TEXT;
        assert!(
            find(
                &rows(
                    &p,
                    &Counts::default(),
                    false,
                    Arrival::Fetched,
                    Bookmarked::No
                ),
                "Article text"
            )
            .unwrap()
            .note
            .is_none()
        );
    }

    /// A feed says which format answered and how many entries came with it, above the character
    /// count it is the context for. The below-the-floor note goes away: `ir::block_text_len`
    /// counts a `Figure` as its caption alone, so a picture-only feed scores under the floor
    /// having parsed perfectly, and the note would be false about it.
    #[test]
    fn a_feed_reports_its_format_and_drops_the_extraction_floor_note() {
        let mut p = prov();
        p.chars = crate::ir::THIN_TEXT - 1;
        p.feed = Some(crate::feed::Report {
            kind: crate::feed::Kind::Atom,
            entries: 5,
        });
        let rows = rows(
            &p,
            &Counts::default(),
            false,
            Arrival::Fetched,
            Bookmarked::No,
        );
        let row = find(&rows, "Format").expect("a feed says which format answered");
        assert_eq!(row.group, Group::Response);
        assert_eq!(row.value, "Atom feed, 5 entries");
        assert!(find(&rows, "Article text").unwrap().note.is_none());
        // And it is context for the count, so it comes first.
        let at = |label| rows.iter().position(|r| r.label == label).unwrap();
        assert!(at("Format") < at("Article text"));
    }

    /// The RTL caution's quiet half: a row, so dismissing the bar is not how the fact dies.
    #[test]
    fn an_rtl_page_reports_its_layout() {
        assert!(
            find(
                &rows(
                    &prov(),
                    &Counts::default(),
                    false,
                    Arrival::Fetched,
                    Bookmarked::No
                ),
                "Layout"
            )
            .is_none()
        );
        let rows_rtl = rows(
            &prov(),
            &Counts::default(),
            true,
            Arrival::Fetched,
            Bookmarked::No,
        );
        let row = find(&rows_rtl, "Layout").expect("a row");
        assert_eq!(row.value, "right-to-left");
        assert!(row.note.as_deref().is_some_and(|n| n.contains("bidi")));
    }
}
