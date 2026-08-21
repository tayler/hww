//! Title de-duplication.
//!
//! An article's `<h1>` usually appears **both** in `doc.title` (from `<title>` or `og:title`)
//! and as `blocks[0]`, so rendering both shows the headline twice. The text renderer got away
//! with it because a `#` heading under an underlined title reads as structure; a GUI that sets
//! them in the same face does not.
//!
//! Handled here and never in the extractor. `html::extract` reports what the page contains,
//! and "these two strings are the same headline" is a reading decision — a different renderer
//! is entitled to a different answer, and a lossy extractor is not recoverable.

use crate::ir;

/// `Some(i)` => skip `blocks[i]` when rendering, because `doc.title` already says it.
///
/// Only ever block 0: a heading further down that happens to match the title is a section of
/// the article, not its masthead.
pub fn dedupe(doc: &ir::Document) -> Option<usize> {
    let title = doc.title.as_deref()?;
    let ir::Block::Heading { level, inlines } = doc.blocks.first()? else {
        return None;
    };
    if *level > 2 {
        return None;
    }
    let a = normalize(title, doc.site_name.as_deref());
    let b = normalize(&ir::plain_text(inlines), doc.site_name.as_deref());
    (!a.is_empty() && matches(&a, &b)).then_some(0)
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

/// Trim, collapse whitespace, casefold, drop a trailing ` — Site` / ` | Site` suffix when it
/// names the site, then drop trailing punctuation.
fn normalize(s: &str, site_name: Option<&str>) -> String {
    let mut t: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
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
    t.to_lowercase()
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
}
