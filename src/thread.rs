//! Thread extraction: the second content algorithm.
//!
//! Article extraction picks the single best subtree. A discussion has no single best
//! subtree: the content is N sibling subtrees, and scoring will happily return one comment
//! out of eighty-eight. Phase 0 measured 8/8 forum pages broken and 5.6% of content
//! recovered under article extraction alone. See docs/phase0-findings.md.
//!
//! The approach here is structural rather than heuristic: find the largest set of sibling
//! elements sharing a class signature, and treat each as a post.

use crate::ir::{self, Block, Comment};
use ego_tree::NodeRef;
use scraper::{ElementRef, Html, Node};
use std::collections::HashMap;
use url::Url;

/// Minimum sibling repetitions before a group is considered a comment list.
const MIN_SIBLINGS: usize = 3;
/// Minimum median text per post, to reject navigation and tag-cloud repetition.
const MIN_MEDIAN_TEXT: usize = 40;
/// A post whose text is mostly inside anchors is a story card, not a comment. Measured: a
/// comment's links are a username and a "reply"; a card's headline and dek are both links.
const MAX_LINK_DENSITY: f64 = 0.5;

const AUTHOR_HINT: &[&str] = &[
    "hnuser",
    "author",
    "username",
    "user-name",
    "byline",
    "poster",
];
pub(crate) const TIME_HINT: &[&str] = &["age", "date", "time", "timestamp", "created"];
const BODY_HINT: &[&str] = &[
    "commtext",
    "comment-body",
    "usertext-body",
    "post-body",
    "md",
    "entry-content",
    "message",
];

pub fn extract_thread(html: &Html, base: &Url) -> Option<Vec<Comment>> {
    extract_thread_traced(html, base, &crate::cards::CardMap::new()).0
}

/// [`extract_thread`], plus the account of every group it weighed. One function under both
/// entry points, so `--why` reports the decision that was made rather than a re-enactment.
///
/// `cards` is what the story-card detector accepted; a group it claimed is not a thread,
/// whatever its bylines say. A news-feed card carries an author and a date exactly as a post
/// does, and its dek can hold its link density under the floor.
pub fn extract_thread_traced(
    html: &Html,
    base: &Url,
    cards: &crate::cards::CardMap,
) -> (Option<Vec<Comment>>, Verdict) {
    let (group, mut verdict) = best_group(html, cards);
    let Some(group) = group else {
        return (None, verdict);
    };
    let mut comments: Vec<Comment> = Vec::new();
    for el in &group {
        let body = body_of(el, base);
        if body.is_empty() {
            continue;
        }
        comments.push(Comment {
            author: find_hint(el, AUTHOR_HINT),
            timestamp: find_hint(el, TIME_HINT),
            depth: depth_of(el, &group),
            id: el.value().id().map(str::to_owned),
            blocks: body,
        });
    }
    if comments.len() < MIN_SIBLINGS {
        if let Some(i) = verdict.chosen.take()
            && let Some(g) = verdict.groups.get_mut(i)
        {
            g.rejected = Some("fewer than 3 posts had a body");
        }
        return (None, verdict);
    }
    (Some(comments), verdict)
}

/// One sibling group the detector weighed, as `--why` reports it. No element references:
/// this outlives the parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupReport {
    /// `tag.class.class`, as [`signature`] builds it.
    pub signature: String,
    pub count: usize,
    /// How many members carried an author or a timestamp hint.
    pub attributed: usize,
    pub median_text: usize,
    pub total_text: usize,
    /// Why the group was not a thread, or `None` for one that passed every test.
    pub rejected: Option<&'static str>,
}

/// What the thread detector saw: the largest groups by text, and which one it chose.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Verdict {
    /// Largest first. Capped at [`Verdict::KEEP`], so a long page does not report hundreds.
    pub groups: Vec<GroupReport>,
    /// Index into `groups` of the one that became the thread, if any.
    pub chosen: Option<usize>,
}

impl Verdict {
    pub const KEEP: usize = 6;
}

impl std::fmt::Display for GroupReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} x{} attributed={} median={} total={}",
            self.signature, self.count, self.attributed, self.median_text, self.total_text
        )?;
        if let Some(why) = self.rejected {
            write!(f, " ({why})")?;
        }
        Ok(())
    }
}

/// Normalise a class list into a stable signature, dropping per-item noise
/// (`c00`, `depth-3`, hashed module names) that would otherwise split a group.
pub(crate) fn signature(el: &ElementRef) -> String {
    let mut classes: Vec<&str> = el
        .value()
        .attr("class")
        .unwrap_or("")
        .split_whitespace()
        .filter(|c| c.len() > 1 && !c.chars().any(|ch| ch.is_ascii_digit()) && c.len() < 30)
        .collect();
    classes.sort_unstable();
    classes.dedup();
    format!("{}.{}", el.value().name(), classes.join("."))
}

/// The largest set of same-signature siblings that looks like posts rather than chrome, and
/// the account of every group that was weighed.
/// Every set of three or more same-signature siblings on the page, in document order of
/// their first member. The raw material of both the thread detector and `cards`: a
/// discussion is N sibling posts, a front page is N sibling story cards, and the two are
/// told apart by what the members contain, not by how they were found.
pub(crate) fn sibling_groups(html: &Html) -> Vec<Vec<ElementRef<'_>>> {
    let mut groups: HashMap<(ego_tree::NodeId, String), Vec<ElementRef<'_>>> = HashMap::new();
    let mut order: Vec<(ego_tree::NodeId, String)> = Vec::new();
    for node in html.tree.root().descendants() {
        let Some(el) = ElementRef::wrap(node) else {
            continue;
        };
        let Some(parent) = node.parent() else {
            continue;
        };
        let sig = signature(&el);
        // A bare tag with no classes is too weak a signal; every <li> on the page would match.
        if sig.ends_with('.') {
            continue;
        }
        let key = (parent.id(), sig);
        let entry = groups.entry(key.clone()).or_default();
        if entry.is_empty() {
            order.push(key);
        }
        entry.push(el);
    }
    order
        .into_iter()
        .filter_map(|k| groups.remove(&k))
        .filter(|g| g.len() >= MIN_SIBLINGS)
        .collect()
}

fn best_group<'a>(
    html: &'a Html,
    cards: &crate::cards::CardMap,
) -> (Option<Vec<ElementRef<'a>>>, Verdict) {
    use crate::cards::CardMapExt;
    let mut weighed: Vec<(Vec<ElementRef<'a>>, GroupReport)> = sibling_groups(html)
        .into_iter()
        .map(|g| {
            let mut lens: Vec<usize> = g.iter().map(|e| text_len(**e)).collect();
            lens.sort_unstable();
            let median = lens[lens.len() / 2];
            // Decisive test: posts carry an author or a timestamp; article paragraphs do not.
            // Without this, a run of sibling <p> tags in a blog post is mistaken for a thread
            // and the article is rendered as a list of comments by "anon".
            let attributed = g
                .iter()
                .filter(|e| {
                    find_hint(e, AUTHOR_HINT).is_some() || find_hint(e, TIME_HINT).is_some()
                })
                .count();
            // Comments are prose and cards are links. A front page's story cards carry a
            // byline and a date exactly as a post does, and the one thing that tells the
            // two apart is that a card's text is mostly inside its anchors.
            let mut densities: Vec<f64> = g.iter().map(link_density).collect();
            densities.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let median_links = densities[densities.len() / 2];
            let rejected = if cards.holds(&g[0]) {
                Some("a list of story cards, not posts")
            } else if crate::html::in_chrome(&g[0]) {
                Some("inside chrome")
            } else if median < MIN_MEDIAN_TEXT {
                Some("median text too short")
            } else if attributed * 2 < g.len() {
                Some("attribution under a majority")
            } else if median_links > MAX_LINK_DENSITY {
                Some("mostly links: a card list, not a thread")
            } else {
                None
            };
            let report = GroupReport {
                signature: signature(&g[0]),
                count: g.len(),
                attributed,
                median_text: median,
                total_text: lens.iter().sum(),
                rejected,
            };
            (g, report)
        })
        .collect();
    // Largest first, so the winner is the first unrejected entry and the report reads in the
    // order the detector ranked it. Ties keep HashMap order, which is why the tests never
    // build two groups of the same size.
    weighed.sort_by_key(|w| std::cmp::Reverse(w.1.total_text));
    let chosen = weighed.iter().position(|(_, r)| r.rejected.is_none());
    let group = chosen.map(|i| std::mem::take(&mut weighed[i].0));
    let mut reports: Vec<GroupReport> = weighed.into_iter().map(|(_, r)| r).collect();
    // The chosen group stays in view even when it is not among the largest.
    let chosen = chosen.map(|i| {
        if i >= Verdict::KEEP {
            let r = reports.remove(i);
            reports.truncate(Verdict::KEEP - 1);
            reports.push(r);
            Verdict::KEEP - 1
        } else {
            i
        }
    });
    reports.truncate(Verdict::KEEP);
    (
        group,
        Verdict {
            groups: reports,
            chosen,
        },
    )
}

/// Reply nesting. HN encodes depth in an `indent` attribute on a spacer cell; nested markup
/// (Discourse, Reddit) encodes it by containment. Support both.
fn depth_of(el: &ElementRef, group: &[ElementRef]) -> u16 {
    for node in el.descendants() {
        if let Some(e) = ElementRef::wrap(node)
            && let Some(v) = e.value().attr("indent").and_then(|v| v.parse::<u16>().ok())
        {
            return v;
        }
    }
    let ids: Vec<_> = group.iter().map(|g| g.id()).collect();
    let mut depth = 0u16;
    let mut cur = el.parent();
    while let Some(n) = cur {
        if ids.contains(&n.id()) {
            depth += 1;
        }
        cur = n.parent();
    }
    depth
}

pub(crate) fn find_hint(el: &ElementRef, hints: &[&str]) -> Option<String> {
    for node in el.descendants() {
        let Some(e) = ElementRef::wrap(node) else {
            continue;
        };
        let attrs = format!(
            "{} {} {}",
            e.value().attr("class").unwrap_or(""),
            e.value().id().unwrap_or(""),
            e.value().name()
        );
        // Token match, not substring: `age` is not in `image`. See `hint`.
        if crate::hint::matches(&attrs, hints) {
            let t = text_of(node);
            if !t.is_empty() && t.len() < 120 {
                return Some(t);
            }
        }
    }
    None
}

/// The post body: a hinted container if present, else the largest text-bearing descendant.
fn body_of(el: &ElementRef, base: &Url) -> Vec<Block> {
    for node in el.descendants() {
        let Some(e) = ElementRef::wrap(node) else {
            continue;
        };
        if crate::hint::matches(e.value().attr("class").unwrap_or(""), BODY_HINT) {
            return crate::html::blocks_from_public(e, base);
        }
    }
    // No hint: take the element itself, minus the metadata line.
    crate::html::blocks_from_public(*el, base)
}

pub(crate) fn text_of(node: NodeRef<'_, Node>) -> String {
    let mut s = String::new();
    collect(node, &mut s);
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn text_len(node: NodeRef<'_, Node>) -> usize {
    text_of(node).len()
}

/// Share of an element's text that sits inside its anchors.
fn link_density(el: &ElementRef) -> f64 {
    let total = text_len(**el);
    if total == 0 {
        return 0.0;
    }
    // A card that *is* an anchor (`<a class="card">` around a headline and a dek) has no
    // anchor descendants and is all link.
    if el.value().name() == "a" {
        return 1.0;
    }
    let a = scraper::Selector::parse("a").unwrap();
    let linked: usize = el.select(&a).map(|l| text_len(*l)).sum();
    (linked as f64 / total as f64).min(1.0)
}

/// Iterative: a comment page is untrusted input, and one stack frame per DOM level is an
/// abort waiting for a deep enough page. Same reasoning as `html::collect_text`.
fn collect(node: NodeRef<'_, Node>, out: &mut String) {
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        match n.value() {
            Node::Text(t) => out.push_str(t),
            Node::Element(e) if matches!(e.name.local.as_ref(), "script" | "style" | "svg") => {}
            _ => stack.extend(n.children().rev()),
        }
    }
}

/// Sum of text in a block list, for deciding thread-vs-article.
///
/// One of the three shared text-length counters (see `AGENTS.md`). It used to build a
/// throwaway `Document` per comment just to reach `text_len`, which cloned every block tree
/// it measured.
pub fn comments_text_len(cs: &[Comment]) -> usize {
    cs.iter().map(|c| ir::blocks_text_len(&c.blocks)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(body: &str) -> Html {
        Html::parse_document(&format!("<html><body>{body}</body></html>"))
    }
    fn base() -> Url {
        Url::parse("https://example.test/a").unwrap()
    }

    /// Regression: every article on the top-ten US news sites grew a "discussion" of three
    /// to nine `anon` posts at its tail, because the caption containers were classed
    /// `image_large`, `image-ct`, `wp-block-image`, and `age` is a substring of `image`.
    #[test]
    fn image_captions_are_not_attributed_posts() {
        let figs = (0..5)
            .map(|i| {
                format!(
                    "<div class=\"image_large image_large__hide-placeholder\"><p>Caption {i}: \
                     a person stands at a lectern in a courtroom on a weekday afternoon.</p></div>"
                )
            })
            .collect::<String>();
        let html = parse(&format!("<article>{figs}</article>"));
        let (thread, verdict) =
            extract_thread_traced(&html, &base(), &crate::cards::CardMap::new());
        assert!(thread.is_none(), "captions became a thread: {verdict:?}");
        let g = verdict
            .groups
            .iter()
            .find(|g| g.signature.starts_with("div.image"))
            .expect("the caption group was weighed");
        assert_eq!(g.attributed, 0, "{g:?}");
        assert_eq!(g.rejected, Some("attribution under a majority"));
    }

    /// Regression: a front page's story cards carry a byline and a date like any post, and
    /// two front pages rendered as a four-post thread. A card is mostly links; a comment is not.
    #[test]
    fn a_list_of_bylined_cards_is_not_a_thread() {
        let cards = (0..6)
            .map(|i| {
                format!(
                    "<a class=\"card\" href=\"/story-{i}\"><h3>Headline number {i} about a thing \
                     that happened somewhere today</h3><span class=\"byline\">By Reporter {i}</span> \
                     <span class=\"date\">2 hours ago</span><p>A dek long enough to clear the \
                     median floor with a sentence of summary.</p></a>"
                )
            })
            .collect::<String>();
        let html = parse(&format!("<main>{cards}</main>"));
        let (thread, verdict) =
            extract_thread_traced(&html, &base(), &crate::cards::CardMap::new());
        assert!(thread.is_none(), "cards became a thread: {verdict:?}");
        let g = &verdict.groups[0];
        assert_eq!(g.attributed, 6, "{g:?}");
        assert_eq!(g.rejected, Some("mostly links: a card list, not a thread"));
    }

    /// The control: a prose thread with a username and a reply link per post still passes
    /// both guards.
    #[test]
    fn a_prose_thread_with_reply_links_still_passes() {
        let posts = (0..4)
            .map(|i| {
                format!(
                    "<div class=\"comment\"><span class=\"user-name\">user{i}</span> \
                     <span class=\"age\">3 hours ago</span><div class=\"comment-body\">A \
                     comment with enough words in it to clear the median text floor and be \
                     counted as a post in its own right, twice over.</div> \
                     <a href=\"/reply/{i}\">reply</a></div>"
                )
            })
            .collect::<String>();
        let html = parse(&format!("<div class=\"comments\">{posts}</div>"));
        let (thread, verdict) =
            extract_thread_traced(&html, &base(), &crate::cards::CardMap::new());
        let thread = thread.unwrap_or_else(|| panic!("no thread: {verdict:?}"));
        assert_eq!(thread.len(), 4);
        assert_eq!(thread[0].author.as_deref(), Some("user0"));
    }
}
