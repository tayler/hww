//! Story-card detection: the shape of a front page.
//!
//! A news front page or section page is not an article and not a discussion. It is N repeated
//! cards, each a linked headline with an optional dek, kicker, time, and thumbnail, under
//! section headings. Scored as an article it yields a legal notice (the one block of prose on
//! the page); scored as a thread it yields a four-post discussion "by anon". Measured on the
//! top-ten US news sites, 2026-08-22: every front page in the set was wrong in one of those two
//! ways, and the thread detector's verdict already listed the card groups, rejected for
//! attribution.
//!
//! Same raw material as `thread` (`thread::sibling_groups`), a different acceptance test: a
//! card is a member whose longest link reads as a headline and which links to few places. A
//! nav item links to many; a comment's longest link is a username.
//!
//! This module only *finds* cards. What the walker does with a found group is `html.rs`'s
//! decision, and today it is reported through `--why` and nothing else.

use ego_tree::NodeId;
use scraper::{ElementRef, Html, Selector};
use std::collections::HashMap;

/// Parent node -> the member nodes under it that read as cards. What the walker consults.
pub type CardMap = HashMap<NodeId, Vec<NodeId>>;

/// Fewest members for a group to be a list of cards.
const MIN_CARDS: usize = 3;
/// A headline is a link at least this long. Shorter links are nav, tags, and "More".
pub(crate) const MIN_HEADLINE: usize = 15;
/// A card links to its story, perhaps twice (thumbnail and headline), perhaps to a section
/// and an author. A nav item links to a dozen places.
const MAX_HREFS: usize = 4;
/// Median member text under this is a list of section links, not of stories. Measured: the
/// sub-navigation lists that passed the headline test sat at 15 to 33; the thinnest real
/// card list at 43.
const MIN_MEDIAN_TEXT: usize = 40;

/// One sibling group as the detector saw it. Plain data; outlives the parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupReport {
    pub signature: String,
    pub count: usize,
    /// Members that read as a card: a headline-length link, few distinct hrefs.
    pub cards: usize,
    pub with_image: usize,
    pub with_time: usize,
    pub median_text: usize,
    pub total_text: usize,
    pub rejected: Option<&'static str>,
}

impl std::fmt::Display for GroupReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} x{} cards={} images={} times={} median={} total={}",
            self.signature,
            self.count,
            self.cards,
            self.with_image,
            self.with_time,
            self.median_text,
            self.total_text
        )?;
        if let Some(why) = self.rejected {
            write!(f, " ({why})")?;
        }
        Ok(())
    }
}

/// What the detector saw: the largest groups by text, accepted and rejected alike.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Verdict {
    pub groups: Vec<GroupReport>,
}

/// Is this element one of the cards the detector accepted?
pub trait CardMapExt {
    fn holds(&self, el: &ElementRef) -> bool;
}

impl CardMapExt for CardMap {
    fn holds(&self, el: &ElementRef) -> bool {
        el.parent()
            .and_then(|p| self.get(&p.id()))
            .is_some_and(|m| m.contains(&el.id()))
    }
}

impl Verdict {
    pub const KEEP: usize = 8;
    pub fn accepted(&self) -> impl Iterator<Item = &GroupReport> {
        self.groups.iter().filter(|g| g.rejected.is_none())
    }
}

/// Weigh every sibling group on the page as a possible list of cards.
pub fn explain(html: &Html) -> Verdict {
    detect(html).1
}

/// Find the card lists on a page: which children of which containers are cards, and the
/// account of every group weighed.
pub fn detect(html: &Html) -> (CardMap, Verdict) {
    let a = Selector::parse("a[href]").unwrap();
    let img = Selector::parse("img, picture").unwrap();
    let mut map = CardMap::new();
    let mut reports: Vec<GroupReport> = crate::thread::sibling_groups(html)
        .into_iter()
        .map(|g| {
            let mut lens: Vec<usize> = g.iter().map(|e| crate::thread::text_len(**e)).collect();
            lens.sort_unstable();
            let median_text = lens[lens.len() / 2];
            let total_text = lens.iter().sum();
            let cards = g.iter().filter(|e| is_card(e, &a)).count();
            let with_image = g.iter().filter(|e| e.select(&img).next().is_some()).count();
            let with_time = g
                .iter()
                .filter(|e| crate::thread::find_hint(e, crate::thread::TIME_HINT).is_some())
                .count();
            let rejected = if g.len() < MIN_CARDS {
                Some("fewer than 3 members")
            } else if cards * 2 < g.len() {
                Some("most members are not cards")
            } else if median_text < MIN_MEDIAN_TEXT {
                Some("median text too short")
            } else {
                None
            };
            if rejected.is_none()
                && let Some(parent) = g[0].parent()
            {
                // Extend, not insert: a section holds several card lists under one parent.
                map.entry(parent.id())
                    .or_default()
                    .extend(g.iter().filter(|e| is_card(e, &a)).map(|e| e.id()));
            }
            GroupReport {
                signature: crate::thread::signature(&g[0]),
                count: g.len(),
                cards,
                with_image,
                with_time,
                median_text,
                total_text,
                rejected,
            }
        })
        .collect();
    // Accepted first, then largest first: the ones that matter read at the top.
    reports.sort_by_key(|r| (r.rejected.is_some(), std::cmp::Reverse(r.total_text)));
    reports.truncate(Verdict::KEEP);
    (map, Verdict { groups: reports })
}

/// A card: its longest link is headline-length, and it links to few distinct places.
fn is_card(el: &ElementRef, a: &Selector) -> bool {
    let mut longest = 0usize;
    let mut hrefs: Vec<&str> = Vec::new();
    // The card itself may be the anchor.
    let own = (el.value().name() == "a")
        .then(|| el.value().attr("href"))
        .flatten();
    if let Some(h) = own {
        longest = crate::thread::text_len(**el);
        hrefs.push(h);
    }
    for link in el.select(a) {
        longest = longest.max(crate::thread::text_len(*link));
        if let Some(h) = link.value().attr("href")
            && !hrefs.contains(&h)
        {
            hrefs.push(h);
        }
    }
    longest >= MIN_HEADLINE && !hrefs.is_empty() && hrefs.len() <= MAX_HREFS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(body: &str) -> Html {
        Html::parse_document(&format!("<html><body>{body}</body></html>"))
    }

    #[test]
    fn a_row_of_headline_cards_is_accepted_and_a_nav_list_is_not() {
        let cards = (0..6)
            .map(|i| {
                format!(
                    "<div class=\"card\"><a href=\"/s{i}\"><img src=\"/t{i}.jpg\"></a>\
                     <h3><a href=\"/s{i}\">Headline {i} about a thing that happened</a></h3>\
                     <p>A dek sentence for story {i}.</p><span class=\"time\">2h</span></div>"
                )
            })
            .collect::<String>();
        let nav = (0..6)
            .map(|i| {
                format!(
                    "<li class=\"nav-item\"><a href=\"/a{i}\">One</a> <a href=\"/b{i}\">Two</a> \
                     <a href=\"/c{i}\">Three</a> <a href=\"/d{i}\">Four</a> <a href=\"/e{i}\">Five</a> \
                     <a href=\"/f{i}\">A long enough link to be a headline</a></li>"
                )
            })
            .collect::<String>();
        let v = explain(&parse(&format!("<ul>{nav}</ul><main>{cards}</main>")));
        let card = v
            .groups
            .iter()
            .find(|g| g.signature == "div.card")
            .expect("card group weighed");
        assert_eq!(card.rejected, None, "{card:?}");
        assert_eq!((card.cards, card.with_image, card.with_time), (6, 6, 6));
        let nav = v
            .groups
            .iter()
            .find(|g| g.signature == "li.nav-item")
            .expect("nav group weighed");
        assert_eq!(nav.rejected, Some("most members are not cards"));
    }

    /// Sub-navigation lists passed the headline test on three sites; their members are
    /// one short link each.
    #[test]
    fn a_list_of_section_links_is_too_thin_to_be_cards() {
        let nav = (0..6)
            .map(|i| format!("<li class=\"sub\"><a href=\"/sec{i}\">Section number {i}</a></li>"))
            .collect::<String>();
        let v = explain(&parse(&format!("<ul>{nav}</ul>")));
        assert_eq!(v.groups[0].rejected, Some("median text too short"), "{v:?}");
    }

    #[test]
    fn a_card_that_is_itself_the_anchor_counts() {
        let cards = (0..4)
            .map(|i| {
                format!(
                    "<a class=\"card\" href=\"/s{i}\"><h3>Headline {i} about a thing that happened</h3>\
                     <p>A dek sentence for story {i}.</p></a>"
                )
            })
            .collect::<String>();
        let v = explain(&parse(&format!("<main>{cards}</main>")));
        assert_eq!(v.accepted().count(), 1, "{v:?}");
        assert_eq!(v.groups[0].cards, 4);
    }

    #[test]
    fn article_paragraphs_are_not_cards() {
        let ps = (0..6)
            .map(|i| {
                format!(
                    "<p class=\"body-text\">Paragraph {i} of prose with <a href=\"/ref\">a link</a> \
                     and plenty of words around it to clear every floor.</p>"
                )
            })
            .collect::<String>();
        let v = explain(&parse(&format!("<article>{ps}</article>")));
        assert_eq!(v.accepted().count(), 0, "{v:?}");
    }
}
