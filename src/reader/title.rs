//! Title de-duplication.
//!
//! An article's `<h1>` usually appears **both** in `doc.title` (from `<title>` or `og:title`)
//! and as `blocks[0]`, so rendering both shows the headline twice. The text renderer got away
//! with it because a `#` heading under an underlined title reads as structure; a GUI that sets
//! them in the same face does not.
//!
//! Handled here and never in the extractor. `html::extract` reports what the page contains,
//! and "these two strings are the same headline" is a reading decision; a different renderer
//! is entitled to a different answer, and a lossy extractor is not recoverable.
//!
//! The same argument covers the rest of the masthead, which is why [`display`], [`masthead`],
//! and [`short_date`] are here too: the ` | Site` tail on a `<title>`, the "By Name" paragraph
//! the page prints under a headline the reader has already set in the strip, and the
//! `2026-08-22T11:30:29.264Z` a publisher puts in `article:published_time`, are all things the
//! page contains and the reader chooses not to repeat. Measured on the top-ten US news sites,
//! 2026-08-22: four of eight titles carried the site, three of eight repeated the byline.

use crate::ir;

/// The title as the reader shows it: whitespace collapsed and a trailing ` | Site`, ` - Site`,
/// ` — Site` dropped when it names the site. `None` when the document has no title.
pub fn display(doc: &ir::Document) -> Option<String> {
    let t = doc.title.as_deref()?;
    let t = strip_site(t, doc.site_name.as_deref());
    (!t.is_empty()).then_some(t)
}

/// A date as the strip shows it: the calendar date of a machine timestamp as `YYYY-MM-DD`,
/// and anything else verbatim. `2026-08-22T11:30:29.264Z` is a machine's idea of a byline.
///
/// Two shapes, because two producers write dates here. HTML pages put ISO 8601 in
/// `article:published_time`, and this truncates it. Feeds put **RFC 822** in `<pubDate>`
/// (`Thu, 27 Aug 2026 17:17:57 +0000`), which is 31 characters of mostly offset, and there is
/// nothing to truncate: it has to be reformatted. `blocks::entries_ui` measures this stamp and
/// subtracts its width from the headline's, floored at half the row, so an un-shortened RFC 822
/// date clamps every headline in a feed to that floor. Layout damage, not cosmetics.
///
/// Both come out ISO, so a date has one shape everywhere in the reader whatever the source
/// wrote. `render.rs` is deliberately left alone: the text renderer has no width pressure and
/// no reason to restate what the page said.
pub fn short_date(s: &str) -> String {
    let t = s.trim();
    iso_calendar_date(t)
        .or_else(|| rfc822_calendar_date(t))
        .unwrap_or_else(|| t.to_owned())
}

/// `2026-08-22T11:30:29.264Z` -> `2026-08-22`. The date part only, and only when what follows
/// it is a timestamp's separator rather than more of a word.
fn iso_calendar_date(t: &str) -> Option<String> {
    let b = t.as_bytes();
    let ok = b.len() >= 10
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[7] == b'-'
        && b[8..10].iter().all(u8::is_ascii_digit)
        && (b.len() == 10 || b[10] == b'T' || b[10] == b' ');
    ok.then(|| t[..10].to_owned())
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// `Thu, 27 Aug 2026 17:17:57 +0000` -> `2026-08-27`.
///
/// The day name is optional, as RFC 822 has it, and everything after the year is dropped
/// unread: hww shows no clock times, so a zone offset is 6 characters of nothing.
///
/// Four-digit years only. RFC 822's two-digit year would need a century guessed for it, and
/// guessing is what makes a wrong date look like a right one. Such a feed keeps its stamp
/// verbatim, which is honest and merely long.
fn rfc822_calendar_date(t: &str) -> Option<String> {
    // "Thu, 27 Aug …". A comma anywhere else ("August 27, 2026") leaves a tail that fails the
    // day parse below, which is the wanted answer.
    let rest = t.split_once(',').map_or(t, |(_, r)| r);
    let mut parts = rest.split_whitespace();
    let day: u8 = parts.next()?.parse().ok()?;
    if !(1..=31).contains(&day) {
        return None;
    }
    let name = parts.next()?;
    let month = MONTHS.iter().position(|m| m.eq_ignore_ascii_case(name))? + 1;
    let year = parts.next()?;
    if year.len() != 4 || !year.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(format!("{year}-{month:02}-{day:02}"))
}

/// Which leading blocks the masthead already says.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Masthead {
    /// `blocks[i]` is the headline `doc.title` already shows. See [`dedupe`].
    pub title_block: Option<usize>,
    /// `blocks[i]` is "By Name" for the `doc.byline` already in the strip.
    pub byline_block: Option<usize>,
}

impl Masthead {
    pub fn skips(&self, i: usize) -> bool {
        self.title_block == Some(i) || self.byline_block == Some(i)
    }
}

/// How far into the document a masthead can reach: a kicker, the headline, the byline, and
/// a dateline, before the first paragraph of the article. A matching block beyond this is
/// the article quoting its own headline, which is the article's business.
const MASTHEAD_REACH: usize = 4;

pub fn masthead(doc: &ir::Document) -> Masthead {
    Masthead {
        title_block: dedupe(doc),
        byline_block: byline_block(doc),
    }
}

fn byline_block(doc: &ir::Document) -> Option<usize> {
    let byline = doc.byline.as_deref()?;
    let want = normalize(byline, None);
    if want.is_empty() {
        return None;
    }
    doc.blocks
        .iter()
        .take(MASTHEAD_REACH)
        .position(|b| match b {
            ir::Block::Paragraph(inlines) => {
                let t = ir::plain_text(inlines);
                let t = t.trim();
                let t = t
                    .strip_prefix("By ")
                    .or_else(|| t.strip_prefix("by "))
                    .or_else(|| t.strip_prefix("BY "))
                    .unwrap_or(t);
                normalize(t, None) == want
            }
            _ => false,
        })
}

/// `Some(i)` => skip `blocks[i]` when rendering, because `doc.title` already says it.
///
/// The headline is the first heading within [`MASTHEAD_REACH`] blocks, and only before any
/// paragraph of body length: a kicker ("HEALTH AND WELLNESS") or a dateline may precede it,
/// which is why this is not "block 0", and a heading further down that happens to match the
/// title is a section of the article, not its masthead, which is why the reach is short.
pub fn dedupe(doc: &ir::Document) -> Option<usize> {
    let title = doc.title.as_deref()?;
    let a = normalize(title, doc.site_name.as_deref());
    if a.is_empty() {
        return None;
    }
    for (i, b) in doc.blocks.iter().take(MASTHEAD_REACH).enumerate() {
        match b {
            ir::Block::Heading { level, inlines } if *level <= 2 => {
                let b = normalize(&ir::plain_text(inlines), doc.site_name.as_deref());
                return matches(&a, &b).then_some(i);
            }
            ir::Block::Paragraph(inlines) if ir::plain_text(inlines).len() >= 200 => {
                return None;
            }
            _ => {}
        }
    }
    None
}

/// Equal after normalization, or one contains the other with ≥90% length overlap.
///
/// The overlap rule is what catches `<title>` carrying a subtitle the `<h1>` omits, without
/// letting "Contact" swallow "Contact us about anything at all".
fn matches(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    long.starts_with(short) && short.len() * 10 >= long.len() * 9
}

/// Collapse whitespace and drop a trailing ` — Site` / ` | Site` suffix when it names the site.
fn strip_site(s: &str, site_name: Option<&str>) -> String {
    let mut t = crate::ir::normalize_ws(s);
    if let Some(site) = site_name {
        let site = site.trim().to_lowercase();
        if !site.is_empty() {
            for sep in [" — ", " – ", " - ", " | ", " · ", " :: "] {
                if let Some(head) = t.rsplit_once(sep)
                    && head.1.trim().to_lowercase() == site
                {
                    t = head.0.to_owned();
                    break;
                }
            }
        }
    }
    t
}

/// [`strip_site`], casefolded, trailing punctuation dropped: the comparison form.
fn normalize(s: &str, site_name: Option<&str>) -> String {
    strip_site(s, site_name)
        .to_lowercase()
        .trim_end_matches(['.', ',', ':', ';', '!', '?', '…', '—', '–', '-', '|'])
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(title: Option<&str>, site: Option<&str>, head: Option<(u8, &str)>) -> ir::Document {
        ir::Document {
            url: "https://example.com/a".into(),
            title: title.map(str::to_owned),
            byline: None,
            published: None,
            site_name: site.map(str::to_owned),
            favicon: None,
            lang: None,
            blocks: head
                .map(|(level, text)| ir::Block::Heading {
                    level,
                    inlines: vec![ir::Inline::Text(text.to_owned())],
                })
                .into_iter()
                .collect(),
        }
    }

    #[test]
    fn identical_headline_is_skipped() {
        assert_eq!(
            dedupe(&doc(Some("The Headline"), None, Some((1, "The Headline")))),
            Some(0)
        );
    }

    #[test]
    fn whitespace_case_and_trailing_punctuation_do_not_defeat_it() {
        assert_eq!(
            dedupe(&doc(
                Some("  the   HEADLINE.  "),
                None,
                Some((1, "The Headline"))
            )),
            Some(0)
        );
    }

    #[test]
    fn a_site_suffix_on_the_title_is_stripped() {
        assert_eq!(
            dedupe(&doc(
                Some("The Headline | Example News"),
                Some("Example News"),
                Some((1, "The Headline"))
            )),
            Some(0)
        );
    }

    #[test]
    fn a_suffix_that_is_not_the_site_name_stays() {
        assert_eq!(
            dedupe(&doc(
                Some("The Headline | Something Else Entirely"),
                Some("Example News"),
                Some((1, "The Headline"))
            )),
            None
        );
    }

    #[test]
    fn a_near_prefix_matches_but_a_short_one_does_not() {
        // Title carries a subtitle the h1 omits: still the same headline.
        assert_eq!(
            dedupe(&doc(
                Some("A Long Considered Headline About Things"),
                None,
                Some((1, "A Long Considered Headline About Thing"))
            )),
            Some(0)
        );
        // "Contact" must not swallow the article below it.
        assert_eq!(
            dedupe(&doc(
                Some("Contact"),
                None,
                Some((1, "Contact us about anything at all"))
            )),
            None
        );
    }

    #[test]
    fn a_deep_heading_is_a_section_not_a_masthead() {
        assert_eq!(
            dedupe(&doc(Some("The Headline"), None, Some((3, "The Headline")))),
            None
        );
    }

    #[test]
    fn an_unrelated_first_heading_survives() {
        assert_eq!(
            dedupe(&doc(Some("The Headline"), None, Some((1, "Introduction")))),
            None
        );
        assert_eq!(dedupe(&doc(Some("The Headline"), None, None)), None);
        assert_eq!(dedupe(&doc(None, None, Some((1, "Introduction")))), None);
    }

    #[test]
    fn a_kicker_before_the_headline_does_not_hide_it() {
        let mut d = doc(Some("The Headline"), None, Some((1, "The Headline")));
        d.blocks.insert(
            0,
            ir::Block::Paragraph(vec![ir::Inline::Text("HEALTH AND WELLNESS".into())]),
        );
        assert_eq!(dedupe(&d), Some(1));
    }

    #[test]
    fn a_matching_heading_after_body_text_is_a_section() {
        let mut d = doc(Some("The Headline"), None, Some((1, "The Headline")));
        d.blocks.insert(
            0,
            ir::Block::Paragraph(vec![ir::Inline::Text("x".repeat(300))]),
        );
        assert_eq!(dedupe(&d), None);
    }

    #[test]
    fn display_drops_the_site_and_keeps_the_case() {
        let d = doc(
            Some("  The Headline  | Example News"),
            Some("Example News"),
            None,
        );
        assert_eq!(display(&d).as_deref(), Some("The Headline"));
        let d = doc(Some("The Headline | Other"), Some("Example News"), None);
        assert_eq!(display(&d).as_deref(), Some("The Headline | Other"));
    }

    /// Three of eight news articles printed "By Name" under a headline whose byline the
    /// strip already shows.
    #[test]
    fn a_by_line_paragraph_repeating_the_byline_is_masthead() {
        let mut d = doc(Some("The Headline"), None, Some((1, "The Headline")));
        d.byline = Some("Jane Writer".into());
        d.blocks.push(ir::Block::Paragraph(vec![
            ir::Inline::Text("By ".into()),
            ir::Inline::Link {
                href: "https://example.com/jane".into(),
                inlines: vec![ir::Inline::Text("Jane Writer".into())],
            },
        ]));
        d.blocks.push(ir::Block::Paragraph(vec![ir::Inline::Text(
            "By Jane Writer, the article opens, and goes on at length.".into(),
        )]));
        let m = masthead(&d);
        assert_eq!(
            m,
            Masthead {
                title_block: Some(0),
                byline_block: Some(1)
            }
        );
        assert!(m.skips(0) && m.skips(1) && !m.skips(2));
    }

    #[test]
    fn short_date_keeps_the_calendar_date_of_a_timestamp_and_nothing_else_changes() {
        assert_eq!(short_date("2026-08-22T11:30:29.264Z"), "2026-08-22");
        assert_eq!(short_date("2026-08-21 04:01:35"), "2026-08-21");
        assert_eq!(short_date("2026-08-22"), "2026-08-22");
        assert_eq!(short_date("21 hours ago"), "21 hours ago");
        assert_eq!(short_date("20260822"), "20260822");
        // RFC 822, which is what every feed writes. Reformatted rather than truncated: there
        // is no prefix of it that is the calendar date.
        assert_eq!(short_date("Thu, 27 Aug 2026 17:17:57 +0000"), "2026-08-27");
        assert_eq!(short_date("27 Aug 2026 17:17:57 GMT"), "2026-08-27");
        assert_eq!(short_date("Sun, 02 Feb 2025 00:00:00 -0500"), "2025-02-02");
        // Anything that is not one of the two shapes is still the publisher's own words.
        assert_eq!(short_date("August 27, 2026"), "August 27, 2026");
        assert_eq!(
            short_date("Thu, 27 Aug 26 17:17:57 +0000"),
            "Thu, 27 Aug 26 17:17:57 +0000"
        );
        assert_eq!(short_date("27 Fructidor 2026"), "27 Fructidor 2026");
        assert_eq!(short_date("32 Aug 2026"), "32 Aug 2026");
    }
}
