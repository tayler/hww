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

const AUTHOR_HINT: &[&str] = &[
    "hnuser",
    "author",
    "username",
    "user-name",
    "byline",
    "poster",
];
const TIME_HINT: &[&str] = &["age", "date", "time", "timestamp", "created"];
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
    extract_thread_traced(html, base).0
}

/// [`extract_thread`], plus the account of every group it weighed. One function under both
/// entry points, so `--why` reports the decision that was made rather than a re-enactment.
pub fn extract_thread_traced(html: &Html, base: &Url) -> (Option<Vec<Comment>>, Verdict) {
    let (group, mut verdict) = best_group(html);
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
fn signature(el: &ElementRef) -> String {
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
fn best_group<'a>(html: &'a Html) -> (Option<Vec<ElementRef<'a>>>, Verdict) {
    let mut groups: HashMap<(ego_tree::NodeId, String), Vec<ElementRef<'a>>> = HashMap::new();
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
        groups.entry((parent.id(), sig)).or_default().push(el);
    }

    let mut weighed: Vec<(Vec<ElementRef<'a>>, GroupReport)> = groups
        .into_values()
        .filter(|g| g.len() >= MIN_SIBLINGS)
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
            let rejected = if median < MIN_MEDIAN_TEXT {
                Some("median text too short")
            } else if attributed * 2 < g.len() {
                Some("attribution under a majority")
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

fn find_hint(el: &ElementRef, hints: &[&str]) -> Option<String> {
    for node in el.descendants() {
        let Some(e) = ElementRef::wrap(node) else {
            continue;
        };
        let attrs = format!(
            "{} {} {}",
            e.value().attr("class").unwrap_or(""),
            e.value().id().unwrap_or(""),
            e.value().name()
        )
        .to_lowercase();
        if hints.iter().any(|h| attrs.contains(h)) {
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
        let class = e.value().attr("class").unwrap_or("").to_lowercase();
        if BODY_HINT
            .iter()
            .any(|h| class.split_whitespace().any(|c| c == *h))
        {
            return crate::html::blocks_from_public(e, base);
        }
    }
    // No hint: take the element itself, minus the metadata line.
    crate::html::blocks_from_public(*el, base)
}

fn text_of(node: NodeRef<'_, Node>) -> String {
    let mut s = String::new();
    collect(node, &mut s);
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn text_len(node: NodeRef<'_, Node>) -> usize {
    text_of(node).len()
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
