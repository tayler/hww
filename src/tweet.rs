//! Tweet detection: the shape X serves for one post and the replies under it.
//!
//! Named for the one site it was built from and measured against, and not for social posts in
//! general. Nothing here matches a hostname — the tests are asked of shape and ARIA, like every
//! other extractor's — but the shape *is* X's: the fixtures reproduce X's server-rendered
//! markup, the six [`ir::StatKind`]s are X's engagement row and no other network's, and no
//! second social host has been measured, because the three tried beside it on 2026-09-03
//! (`docs/sites-checked.md`) serve a JavaScript shell with no body at all. The synonyms in
//! [`kind_of_label`] for other networks' words are on paper only. Calling this a social-post
//! detector would claim a generality nothing has tested: a Mastodon at-name (`@user@host`)
//! fails [`is_handle`], a Reddit post carries no at-name, and a Facebook post counts reactions
//! and shares, which no `StatKind` names. A second social host that earns a row in the ledger
//! is how this module widens, and its name should move with what it fits.
//!
//! A tweet is not an article, not a discussion, and not a front page. It is a few dozen words
//! under a name, with a date, usually a picture, and a row of counts saying how it was received.
//! Scored as an article it does not exist: `score = raw_text * (1 - link_density)` and a tweet is
//! nine words wrapped in links and buttons, so it sits under the floor and the `<body>` fallback
//! hands back navigation. Measured on a status permalink, 2026-09-02: 192,483 bytes of
//! server-rendered markup carrying the whole tweet came out as 139 characters, under
//! `ir::THIN_TEXT`, and the reader told the reader the page needed JavaScript. It did not. See
//! docs/findings.md.
//!
//! Same raw material as [`crate::thread`] and [`crate::cards`] — the one sibling-group census
//! `html::extract_traced` takes and lends out — plus every `<article>` on the page, because a
//! permalink shows one tweet and one tweet is not a sibling group.
//!
//! **A different acceptance test.** A card is a headline that links away; a comment is prose
//! under a name; a tweet is prose under a name *with counted actions beside it*. The counts are
//! what separate a tweet from a comment, and the author and date are what separate it from a
//! card. All three questions are asked of shape and ARIA, never of a hostname: this module,
//! like every other extractor, does not know what site it is reading, even though its name
//! says which site's shape it knows.
//!
//! **Counts stay strings.** The markup carries `2.6M`, not `2616339`. The exact figures are on
//! the page, inside a `<script>` as serialized JavaScript, and `html.rs` drops `script` as noise
//! for good reasons that a tweet does not override. Rounding `2.6M` back into an integer would
//! publish a number nobody wrote, so [`ir::Stat::count`] is the page's own string.
//!
//! This module only *finds* tweets. What the pipeline does with a find is `html.rs`'s decision,
//! and it is a candidate with a floor: a tweet run replaces the document only where article and
//! thread extraction came out under `ir::THIN_TEXT`. That is what keeps a news page with share
//! counts on it from being redrawn as somebody's status.

use crate::ir::{self, Image, Stat, StatKind, Tweet};
use ego_tree::NodeId;
use scraper::{ElementRef, Html, Selector};
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;
use url::Url;

/// Fewest counted actions for a run of controls to read as a tweet's engagement row.
///
/// Two rather than one: a lone "N comments" link hangs under half the articles on the web, and
/// one number beside a heading is a card. Two distinct kinds of count in one row is the shape
/// that only a tweet has.
const MIN_STATS: usize = 2;

/// Longest a string may be and still read as a timestamp. "10:47 PM · Jul 5, 2026" is 22.
const MAX_TIME_TEXT: usize = 40;

/// Longest a string may be and still read as a count. "108.2M" is 6.
const MAX_COUNT_TEXT: usize = 12;

static SEL_CANDIDATE: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("article, [role=article]").expect("a literal selector"));
static SEL_A_HREF: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("a[href]").expect("a literal selector"));
static SEL_IMG: LazyLock<Selector> = LazyLock::new(|| Selector::parse("img").expect("a literal"));
static SEL_TIME: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("time").expect("a literal selector"));

/// One candidate as the detector saw it. Plain data; outlives the parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TweetReport {
    pub signature: String,
    pub author: bool,
    pub timestamp: bool,
    pub stats: usize,
    pub text: usize,
    pub rejected: Option<&'static str>,
}

impl std::fmt::Display for TweetReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} author={} time={} stats={} text={}",
            self.signature, self.author, self.timestamp, self.stats, self.text
        )?;
        if let Some(why) = self.rejected {
            write!(f, " ({why})")?;
        }
        Ok(())
    }
}

/// What the detector saw: every candidate weighed, accepted and rejected alike.
///
/// `candidates` is a *display* list and not the census: it holds at most [`Verdict::KEEP`]
/// reports, accepted ones first and the longest first within each half, so a `--why` trace
/// stays readable on a timeline of two hundred entries. `weighed` is how many there really
/// were and `kept` is how many made the run, so the two totals are honest even when the list
/// under them is clipped: on a page with more than `KEEP` tweets, `kept` exceeds
/// `accepted().count()` and nothing is wrong. Do not treat `accepted` as exhaustive.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Verdict {
    pub candidates: Vec<TweetReport>,
    /// Every container the detector looked at, listed or not.
    pub weighed: usize,
    /// How many survived de-duplication and became the run.
    pub kept: usize,
}

impl Verdict {
    pub const KEEP: usize = 6;
    /// The accepted entries *among those listed*; see the struct comment for why this can
    /// come out under `kept`.
    pub fn accepted(&self) -> impl Iterator<Item = &TweetReport> {
        self.candidates.iter().filter(|c| c.rejected.is_none())
    }
}

/// Find the tweets on a page, and the account of every candidate weighed.
///
/// Reached from the tests rather than from the pipeline, which goes through [`detect_in`] with
/// the census and card map it already holds. It stays for the reason `cards::explain` does: it
/// is the entry a caller that has only an `Html` should use.
pub fn detect(html: &Html, base: &Url) -> (Option<Vec<Tweet>>, Verdict) {
    detect_in(html, base, &crate::thread::sibling_groups(html))
}

/// [`detect`] over a census the caller already took. See `thread::extract_thread_in` for why the
/// sibling-group walk is taken once and lent to every detector that wants it.
pub(crate) fn detect_in(
    html: &Html,
    base: &Url,
    groups: &[crate::thread::Group<'_>],
) -> (Option<Vec<Tweet>>, Verdict) {
    // Every `<article>`, plus every member of every sibling group. A permalink shows one tweet
    // and has no group; a timeline is a group and may use no `<article>` at all. Taking both and
    // de-duplicating by node is cheaper than deciding which page this is up front.
    let mut seen: HashSet<NodeId> = HashSet::new();
    let mut candidates: Vec<ElementRef<'_>> = Vec::new();
    for el in html
        .select(&SEL_CANDIDATE)
        .chain(groups.iter().flat_map(|g| g.members.iter().copied()))
    {
        if seen.insert(el.id()) {
            candidates.push(el);
        }
    }
    // Document order, so "the first tweet" means what it looks like it means. One walk building
    // one map, rather than a `position` scan per candidate: the two sources above are a
    // selector and a census and neither arrives ordered against the other, and a page with a
    // hundred group members would otherwise walk the tree a hundred times.
    let mut order: HashMap<NodeId, usize> = HashMap::new();
    for (i, n) in html.tree.root().descendants().enumerate() {
        order.insert(n.id(), i);
    }
    candidates.sort_by_key(|el| order.get(&el.id()).copied().unwrap_or(usize::MAX));

    let nested: HashSet<NodeId> = candidates.iter().map(|el| el.id()).collect();
    let mut reports = Vec::new();
    let mut found: Vec<Tweet> = Vec::new();
    for el in &candidates {
        let (tweet, report) = weigh(el, base, &nested);
        reports.push(report);
        if let Some(p) = tweet {
            found.push(p);
        }
    }
    let weighed = reports.len();
    reports.sort_by_key(|r| (r.rejected.is_some(), usize::MAX - r.text));
    reports.truncate(Verdict::KEEP);

    let tweets = assemble(found, base);
    let kept = tweets.as_ref().map_or(0, Vec::len);
    (
        tweets,
        Verdict {
            candidates: reports,
            weighed,
            kept,
        },
    )
}

/// Weigh one candidate: is this a tweet, and if not, why not?
fn weigh(
    el: &ElementRef<'_>,
    base: &Url,
    candidates: &HashSet<NodeId>,
) -> (Option<Tweet>, TweetReport) {
    let signature = crate::thread::signature(el);
    let mut report = TweetReport {
        signature,
        author: false,
        timestamp: false,
        stats: 0,
        text: 0,
        rejected: None,
    };
    macro_rules! refuse {
        ($why:literal) => {{
            report.rejected = Some($why);
            return (None, report);
        }};
    }

    if crate::html::in_chrome(el) {
        refuse!("inside chrome");
    }
    // A wrapper around other candidates is the page's layout, not a tweet. The tweet itself is
    // the innermost thing that passes, so an outer container carrying a login prompt beside the
    // tweet does not become a second copy of it.
    if el
        .descendants()
        .filter_map(ElementRef::wrap)
        .any(|d| d.id() != el.id() && candidates.contains(&d.id()))
    {
        refuse!("wraps another candidate");
    }

    let stats = stats_of(el);
    report.stats = stats.len();

    let (author, handle, author_href, skip_names) = author_of(el);
    report.author = author.is_some() || handle.is_some();

    let (timestamp, permalink, time_id) = time_of(el, base);
    report.timestamp = timestamp.is_some();

    // Every cheap refusal before the body walk. On a discussion page every comment is a
    // candidate, and walking each one's subtree only to refuse it for want of a count would
    // make this detector cost what the thread detector costs, twice.
    if !report.author {
        refuse!("no name over it");
    }
    // Fewer than two counts is a comment, and `thread` reads those better than this would.
    if stats.len() < MIN_STATS {
        refuse!("fewer than two counted actions");
    }
    if timestamp.is_none() {
        refuse!("no date under it");
    }

    let avatar = avatar_of(el, base, author_href.as_deref());

    // Every part that is attribution rather than what was said, so the body walk can step over
    // it: the name, the at-name, the date, the counts, the face, and any nested candidate.
    let mut skip: HashSet<NodeId> = skip_names;
    skip.extend(time_id);
    skip.extend(stats.iter().map(|(id, _)| *id));
    if let Some((id, _)) = &avatar {
        skip.insert(*id);
    }
    for d in el.descendants().filter_map(ElementRef::wrap) {
        if d.id() != el.id() && candidates.contains(&d.id()) {
            skip.insert(d.id());
        }
    }
    let blocks = crate::html::blocks_from_tweet(*el, base, &skip);
    report.text = ir::blocks_text_len(&blocks);

    if blocks.is_empty() {
        refuse!("nothing was said");
    }

    let tweet = Tweet {
        author,
        handle,
        permalink,
        timestamp,
        avatar: avatar.map(|(_, img)| img),
        depth: 0,
        blocks,
        stats: stats.into_iter().map(|(_, s)| s).collect(),
    };
    (Some(tweet), report)
}

/// De-duplicate, choose the focal tweet, and put the rest under it at depth 1.
///
/// A permalink renders its own tweet more than once — once in the column and once in the
/// conversation under it — and the two copies are the same tweet, not a tweet and a reply. The
/// permalink is what says so, and where a copy has none, [`same_tweet`] says so from the parts
/// the page did print.
///
/// Depth here is one bit. The detector reads no reply chain — nothing on the page says which
/// tweet answers which — so the focal tweet is 0 and everything else is 1. `Tweet::depth` is a
/// `u16` because `Block::Thread` indents by the same field and the renderers share the arithmetic;
/// no tweet from this module reaches 2.
fn assemble(found: Vec<Tweet>, base: &Url) -> Option<Vec<Tweet>> {
    if found.is_empty() {
        return None;
    }
    let mut unique: Vec<Tweet> = Vec::new();
    for p in found {
        match unique.iter().position(|u| same_tweet(u, &p)) {
            // Keep the fuller copy: one of the two is usually clipped.
            Some(i) => {
                let better = richness(&p) > richness(&unique[i]);
                if better {
                    unique[i] = p;
                }
            }
            None => unique.push(p),
        }
    }
    // The focal tweet is the one the address names. Failing that, the first: on a timeline
    // nothing is focal and the page reads top to bottom anyway.
    //
    // Path only, deliberately. The permalink is the address the page wrote for itself; `base`
    // is the address the reader arrived by, which is the same path wearing whatever query a
    // share button appended (`?s=20&t=…`). Comparing queries would make the focal tweet depend
    // on how the link was copied. A site whose tweet identity lives in the query alone gets the
    // first-tweet fallback, which on a permalink is the tweet the page drew first.
    let focal = unique
        .iter()
        .position(|p| {
            p.permalink
                .as_deref()
                .and_then(|l| Url::parse(l).ok())
                .is_some_and(|u| u.path() == base.path())
        })
        .unwrap_or(0);
    let mut out = vec![unique.remove(focal)];
    out[0].depth = 0;
    for mut p in unique {
        p.depth = 1;
        out.push(p);
    }
    Some(out)
}

/// Are these two copies of one tweet?
///
/// Two permalinks answer it outright. Where either copy has none — the date was printed and
/// not linked — the answer comes from what the page did print: the same name, the same date
/// string, and the same length of words, in [`ir::blocks_text_len`]'s currency. A pinned tweet
/// drawn once as the sticky and once in the list is the case; two different tweets by one
/// author at one minute with the same word count is the collision, and it is accepted because
/// the alternative was two copies of every permalinkless tweet.
fn same_tweet(a: &Tweet, b: &Tweet) -> bool {
    if let (Some(x), Some(y)) = (&a.permalink, &b.permalink) {
        return x == y;
    }
    let who = |p: &Tweet| p.handle.clone().or_else(|| p.author.clone());
    who(a).is_some()
        && who(a) == who(b)
        && a.timestamp.is_some()
        && a.timestamp == b.timestamp
        && ir::blocks_text_len(&a.blocks) == ir::blocks_text_len(&b.blocks)
}

fn richness(p: &Tweet) -> usize {
    ir::blocks_text_len(&p.blocks) + p.stats.len() + usize::from(p.avatar.is_some())
}

/// The counted actions in a candidate: what the page said about how the tweet was received.
///
/// Read from `aria-label`, `data-icon` and the visible words, never from a class name, because
/// those are what such a control means outside the site that wrote it — and X's are the ones
/// measured, `aria-label="Reply"` beside `data-icon="icon-reply-stroke"`. All three are consulted:
/// a label is words and can be localised away, an icon name is not, and a count the page simply
/// prints beside its noun ("108.2M Views") wears neither.
///
/// One pass in document order, so the row comes out in the order the page drew it, and the
/// first control to claim a kind keeps it.
fn stats_of(el: &ElementRef<'_>) -> Vec<(NodeId, Stat)> {
    let mut out: Vec<(NodeId, Stat)> = Vec::new();
    for control in el.descendants().filter_map(ElementRef::wrap) {
        if control.id() == el.id() {
            continue;
        }
        let v = control.value();
        let text = ir::normalize_ws(&crate::thread::text_of(*control));
        let labelled = v
            .attr("aria-label")
            .and_then(kind_of_label)
            .or_else(|| v.attr("data-icon").and_then(kind_of_icon))
            .or_else(|| v.attr("data-testid").and_then(kind_of_label));
        // A control the page labelled, or a count printed beside its own noun.
        let Some((kind, count)) = labelled
            .and_then(|k| count_in(&text).map(|c| (k, c)))
            .or_else(|| pair_stat(&text))
            .or_else(|| pair_stat_of_parts(&control))
        else {
            continue;
        };
        if out.iter().any(|(_, s)| s.kind == kind) {
            continue;
        }
        // A control nested inside one already counted is the same action read twice.
        if out
            .iter()
            .any(|(id, _)| control.ancestors().any(|a| a.id() == *id))
        {
            continue;
        }
        out.push((control.id(), Stat { kind, count }));
    }
    out
}

/// A count printed beside the word for what it counts: "108.2M Views", "42 replies". Exactly
/// two tokens, so a sentence that happens to open with a number is not a statistic.
fn pair_stat(text: &str) -> Option<(StatKind, String)> {
    let mut tokens = text.split_whitespace();
    let (a, b) = (tokens.next()?, tokens.next()?);
    if tokens.next().is_some() {
        return None;
    }
    if is_count(a) {
        kind_of_label(b).map(|k| (k, a.to_owned()))
    } else if is_count(b) {
        kind_of_label(a).map(|k| (k, b.to_owned()))
    } else {
        None
    }
}

/// [`pair_stat`] over a control's parts rather than its flattened text.
///
/// `<span>108.2M</span><span>Views</span>` has no whitespace between the two spans, so the words
/// run together into one token and the flat test cannot see them. The page draws them as two
/// boxes; reading them as two parts is reading what it drew.
fn pair_stat_of_parts(control: &ElementRef<'_>) -> Option<(StatKind, String)> {
    let parts: Vec<String> = control
        .children()
        .filter_map(|c| {
            let t = match c.value() {
                scraper::Node::Text(t) => ir::normalize_ws(t),
                scraper::Node::Element(_) => ElementRef::wrap(c)
                    .map(|e| ir::normalize_ws(&crate::thread::text_of(*e)))
                    .unwrap_or_default(),
                _ => String::new(),
            };
            (!t.is_empty()).then_some(t)
        })
        .collect();
    let [a, b] = parts.as_slice() else {
        return None;
    };
    if is_count(a) {
        kind_of_label(b).map(|k| (k, a.clone()))
    } else if is_count(b) {
        kind_of_label(a).map(|k| (k, b.clone()))
    } else {
        None
    }
}

/// The first thing inside a control that reads as a count.
fn count_in(text: &str) -> Option<String> {
    if is_count(text) {
        return Some(text.to_owned());
    }
    text.split_whitespace()
        .find(|t| is_count(t))
        .map(str::to_owned)
}

/// Does this read as a count the page printed? Digits, with the usual thousands punctuation and
/// an optional magnitude suffix. Never parsed into a number: see this module's header.
fn is_count(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() || s.len() > MAX_COUNT_TEXT {
        return false;
    }
    if !s.starts_with(|c: char| c.is_ascii_digit()) {
        return false;
    }
    let mut chars = s.chars().peekable();
    let mut tail = None;
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() || c == '.' || c == ',' || c == '\u{202f}' || c == ' ' {
            continue;
        }
        if chars.peek().is_none() {
            tail = Some(c);
        } else {
            return false;
        }
    }
    tail.is_none_or(|c| matches!(c.to_ascii_uppercase(), 'K' | 'M' | 'B' | 'G' | 'T'))
}

/// The word a count wears, read as a whole-word match so `replies` and `reply` meet.
///
/// `reply`, `repost`, `retweet`, `like`, `quote`, `bookmark`, and `view` are X's own row and
/// the words measured. `comment`, `answer`, `boost`, `renote`, `favorite`, `favourite`,
/// `fav`, and `impression` are what other networks call the same actions and are here on paper
/// only: none of those hosts serves a body to a client without JavaScript, so none of these
/// words has been seen by this code on a live page. They claim nothing; see the module header.
fn kind_of_label(label: &str) -> Option<StatKind> {
    let l = label.to_ascii_lowercase();
    let word = |w: &str| {
        l.split(|c: char| !c.is_ascii_alphabetic())
            .any(|t| t == w || t.trim_end_matches('s') == w)
    };
    if word("reply") || word("replie") || word("comment") || word("answer") {
        Some(StatKind::Replies)
    } else if word("repost") || word("retweet") || word("boost") || word("renote") {
        Some(StatKind::Reposts)
    } else if word("like") || word("favorite") || word("favourite") || word("fav") {
        Some(StatKind::Likes)
    } else if word("quote") {
        Some(StatKind::Quotes)
    } else if word("bookmark") {
        Some(StatKind::Bookmarks)
    } else if word("view") || word("impression") {
        Some(StatKind::Views)
    } else {
        None
    }
}

fn kind_of_icon(icon: &str) -> Option<StatKind> {
    let i = icon.to_ascii_lowercase();
    let has = |n: &str| i.contains(n);
    if has("reply") || has("comment") {
        Some(StatKind::Replies)
    } else if has("retweet") || has("repost") || has("boost") {
        Some(StatKind::Reposts)
    } else if has("heart") || has("like") || has("favorite") || has("favourite") {
        Some(StatKind::Likes)
    } else if has("quote") {
        Some(StatKind::Quotes)
    } else if has("bookmark") {
        Some(StatKind::Bookmarks)
    } else if has("view") || has("analytic") {
        Some(StatKind::Views)
    } else {
        None
    }
}

/// The name over a tweet: the display name, the at-name, the profile href, and the nodes that
/// carried them so the body walk can skip them.
fn author_of(
    el: &ElementRef<'_>,
) -> (
    Option<String>,
    Option<String>,
    Option<String>,
    HashSet<NodeId>,
) {
    let mut handle = None;
    let mut handle_href = None;
    let mut ids = HashSet::new();
    for a in el.select(&SEL_A_HREF) {
        let text = ir::normalize_ws(&crate::thread::text_of(*a));
        if is_handle(&text) {
            handle = Some(text);
            handle_href = a.value().attr("href").map(str::to_owned);
            ids.insert(a.id());
            break;
        }
    }
    // The display name is the other anchor pointing where the at-name points. Asked that way
    // round because a name is any words at all and an at-name is a shape.
    let mut author = None;
    if let Some(href) = &handle_href {
        for a in el.select(&SEL_A_HREF) {
            if a.value().attr("href") != Some(href.as_str()) || ids.contains(&a.id()) {
                continue;
            }
            let text = ir::normalize_ws(&crate::thread::text_of(*a));
            if !text.is_empty() && !is_handle(&text) && text.len() <= 120 {
                author = Some(text);
                ids.insert(a.id());
                break;
            }
        }
    }
    // No at-name anywhere means no name: there is no fallback to the class-name hints the
    // thread detector keeps, because the name decides whether this is a tweet at all and this
    // module's promise is that nothing in that decision is a class name. A forum entry set in
    // `.author` with a like count under it is the thread detector's to read.
    (author, handle, handle_href, ids)
}

/// An at-name: `@` and then a plausible account, nothing else.
fn is_handle(s: &str) -> bool {
    let Some(rest) = s.strip_prefix('@') else {
        return false;
    };
    !rest.is_empty()
        && rest.len() <= 40
        && rest
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == '-')
}

/// The date under a tweet, the address it linked to, and the node that carried it.
fn time_of(el: &ElementRef<'_>, base: &Url) -> (Option<String>, Option<String>, Option<NodeId>) {
    for a in el.select(&SEL_A_HREF) {
        let text = ir::normalize_ws(&crate::thread::text_of(*a));
        if looks_like_time(&text) {
            let href = a
                .value()
                .attr("href")
                .and_then(|h| base.join(h).ok())
                .map(String::from);
            return (Some(text), href, Some(a.id()));
        }
    }
    // A date that is not a link still dates the tweet; it just does not point anywhere. Only a
    // `<time>` element is read for it — a tag, which is the page saying what the thing is —
    // and never the class-name hints `thread` keeps, for the reason `author_of` gives.
    for t in el.select(&SEL_TIME) {
        let text = ir::normalize_ws(&crate::thread::text_of(*t));
        if looks_like_time(&text) {
            return (Some(text), None, Some(t.id()));
        }
    }
    (None, None, None)
}

const MONTHS: &[&str] = &[
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

/// Does this read as a date or a time? Deliberately loose about *format* and strict about
/// *length*: a permalink writes `10:47 PM · Jul 5, 2026`, a timeline abbreviates the same
/// date to `Jul 5` or `5h`, and none of them is a paragraph.
fn looks_like_time(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() || s.len() > MAX_TIME_TEXT || !s.chars().any(|c| c.is_ascii_digit()) {
        return false;
    }
    let lower = s.to_ascii_lowercase();
    // A clock, a month, a four-digit year, or a relative age ("5h", "2 days ago").
    lower.contains(':')
        || MONTHS.iter().any(|m| lower.contains(m))
        || lower.contains("ago")
        || lower
            .split(|c: char| !c.is_ascii_digit())
            .any(|t| t.len() == 4 && t.starts_with(|c: char| ('1'..='2').contains(&c)))
        || is_relative_age(&lower)
}

/// "5h", "2d", "3w" — the shorthand a timeline uses when the tweet is recent.
fn is_relative_age(s: &str) -> bool {
    let s = s.trim();
    let digits = s.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    let unit = &s[digits.len()..];
    !digits.is_empty()
        && digits.chars().all(|c| c.is_ascii_digit())
        && matches!(unit, "s" | "m" | "h" | "d" | "w" | "y" | "mo")
}

/// The face over a tweet, and the node that carried it.
///
/// Found by where it points rather than by how it looks: the avatar is the picture inside the
/// link to the author's own page. That is how X draws one, and it keeps the
/// tweet's own media — which links to the tweet, not to the author — out of the slot.
fn avatar_of(
    el: &ElementRef<'_>,
    base: &Url,
    author_href: Option<&str>,
) -> Option<(NodeId, Image)> {
    let href = author_href?;
    let target = base.join(href).ok()?;
    for a in el.select(&SEL_A_HREF) {
        let Some(this) = a.value().attr("href").and_then(|h| base.join(h).ok()) else {
            continue;
        };
        if this != target {
            continue;
        }
        if let Some(img) = a.select(&SEL_IMG).next()
            && let Some(image) = crate::html::image_from(&img, base)
        {
            return Some((img.id(), image));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Url {
        Url::parse("https://example.invalid/erling/status/9001").expect("a literal URL")
    }

    /// The shape a status permalink serves, in X's own attribute vocabulary: a name, an
    /// at-name, the words, a picture, a date, and a row of counted actions. Class names are
    /// deliberately meaningless here, because the detector is not allowed to read them.
    fn tweet_markup(handle: &str, id: u32, words: &str, extra: &str) -> String {
        format!(
            r#"<article class="flex flex-col">
                 <div class="flex gap-2">
                   <a href="/{handle}"><img alt="@{handle}" src="/pfp/{handle}.jpg"></a>
                   <a href="https://example.invalid/{handle}">Real Name</a>
                   <a href="https://example.invalid/{handle}">@{handle}</a>
                 </div>
                 <div dir="auto"><span>{words}</span></div>
                 {extra}
                 <div class="stamp">
                   <span><a href="/{handle}/status/{id}">10:47 PM · Jul 5, 2026</a></span>
                   <span><a href="/{handle}/status/{id}"><span>108.2M</span><span>Views</span></a></span>
                 </div>
                 <div class="row">
                   <a aria-label="Reply" href="/i/status/{id}">
                     <svg data-icon="icon-reply-stroke"></svg><span dir="ltr">56K</span></a>
                   <button aria-label="Repost">
                     <svg data-icon="icon-retweet-stroke"></svg><span dir="ltr">166K</span></button>
                   <button aria-label="Like">
                     <svg data-icon="icon-heart-stroke"></svg><span dir="ltr">2.6M</span></button>
                   <button aria-label="Bookmark">
                     <svg data-icon="icon-bookmark-stroke"></svg><span dir="ltr">44K</span></button>
                 </div>
               </article>"#
        )
    }

    fn page(body: &str) -> Html {
        Html::parse_document(&format!("<html><body><main>{body}</main></body></html>"))
    }

    #[test]
    fn a_tweet_with_a_name_a_date_and_counts_is_found() {
        let html = page(&tweet_markup("erling", 9001, "Well well well", ""));
        let (tweets, why) = detect(&html, &base());
        let tweets = tweets.expect("the tweet is found");
        assert_eq!(tweets.len(), 1, "{why:?}");
        let p = &tweets[0];
        assert_eq!(p.author.as_deref(), Some("Real Name"));
        assert_eq!(p.handle.as_deref(), Some("@erling"));
        assert_eq!(p.timestamp.as_deref(), Some("10:47 PM · Jul 5, 2026"));
        assert_eq!(p.depth, 0);
        assert!(
            p.avatar
                .as_ref()
                .is_some_and(|a| a.src.ends_with("erling.jpg")),
            "the face is the picture inside the link to the author: {:?}",
            p.avatar
        );
        assert!(
            ir::plain_text(&match &p.blocks[0] {
                ir::Block::Paragraph(v) => v.clone(),
                other => panic!("expected the words, got {other:?}"),
            })
            .contains("Well well well"),
            "{:?}",
            p.blocks
        );
    }

    /// Every count the page printed, labelled, and none of the attribution counted twice.
    #[test]
    fn the_counts_are_kept_as_the_page_printed_them() {
        let html = page(&tweet_markup("erling", 9001, "Well well well", ""));
        let (tweets, _) = detect(&html, &base());
        let stats = &tweets.expect("found").swap_remove(0).stats;
        let got: Vec<(StatKind, &str)> = stats.iter().map(|s| (s.kind, s.count.as_str())).collect();
        assert_eq!(
            got,
            vec![
                (StatKind::Views, "108.2M"),
                (StatKind::Replies, "56K"),
                (StatKind::Reposts, "166K"),
                (StatKind::Likes, "2.6M"),
                (StatKind::Bookmarks, "44K"),
            ]
        );
    }

    /// The whole argument of the module header: `2.6M` is what was published, so `2.6M` is what
    /// is carried. Nothing here may start parsing magnitudes back into integers.
    #[test]
    fn a_rounded_count_is_never_expanded() {
        let html = page(&tweet_markup("erling", 9001, "Well well well", ""));
        let (tweets, _) = detect(&html, &base());
        for s in &tweets.expect("found")[0].stats {
            assert!(
                s.count.chars().any(|c| !c.is_ascii_digit()) || s.count.parse::<u64>().is_ok(),
                "counts stay verbatim: {}",
                s.count
            );
        }
        assert!(is_count("2.6M") && is_count("108.2M") && is_count("56K") && is_count("420"));
        assert!(!is_count("Views") && !is_count("") && !is_count("2.6MB"));
    }

    /// The tweet's own picture rides in its blocks, so the reader draws it under the ordinary
    /// image policy and page info counts it. It is not the avatar and must not fill that slot.
    #[test]
    fn the_tweets_picture_is_a_block_and_not_the_avatar() {
        let media = r#"<div class="media"><a aria-label="Image"
                         href="/erling/status/9001/photo/1">
                         <img src="/media/pic.jpg" alt=""></a></div>"#;
        let html = page(&tweet_markup("erling", 9001, "Well well well", media));
        let (tweets, _) = detect(&html, &base());
        let p = &tweets.expect("found")[0];
        let figures: Vec<&str> = p
            .blocks
            .iter()
            .filter_map(|b| match b {
                ir::Block::Figure { image, .. } => Some(image.src.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(figures, vec!["https://example.invalid/media/pic.jpg"]);
        assert!(p.avatar.as_ref().is_some_and(|a| a.src.contains("/pfp/")));
    }

    /// A comment has a name and a date and no counted actions. `thread` reads those, and a
    /// detector that took them would replace a discussion with a list of one-line tweets.
    #[test]
    fn a_tweet_without_counted_actions_is_not_a_tweet() {
        let html = page(
            r#"<article><a href="https://example.invalid/x">Real Name</a>
                 <a href="https://example.invalid/x">@x</a>
                 <div dir="auto">Some words that were said in reply to something else.</div>
                 <a href="/x/status/3">Jul 5, 2026</a></article>"#,
        );
        let (tweets, why) = detect(&html, &base());
        assert!(tweets.is_none());
        assert_eq!(
            why.candidates[0].rejected,
            Some("fewer than two counted actions")
        );
    }

    /// An article page carries a share row. It has counts, and it has no at-name and no date
    /// anchor over the body, so it must not be redrawn as somebody's status.
    #[test]
    fn an_article_with_a_share_row_is_not_a_tweet() {
        let html = page(
            r#"<article><h1>A headline about a thing</h1>
                 <p>Body prose long enough to clear any floor, and then some more of it.</p>
                 <div><button aria-label="Like"><span>1.2K</span></button>
                      <button aria-label="Bookmark"><span>340</span></button></div>
               </article>"#,
        );
        let (tweets, why) = detect(&html, &base());
        assert!(tweets.is_none(), "{why:?}");
        assert_eq!(why.candidates[0].rejected, Some("no name over it"));
    }

    #[test]
    fn a_tweet_inside_chrome_is_rejected() {
        let html = Html::parse_document(&format!(
            "<html><body><footer>{}</footer></body></html>",
            tweet_markup("erling", 9001, "Well well well", "")
        ));
        let (tweets, why) = detect(&html, &base());
        assert!(tweets.is_none());
        assert_eq!(why.candidates[0].rejected, Some("inside chrome"));
    }

    /// A permalink draws its own tweet twice — once in the column, once at the head of the
    /// conversation. Two copies of one tweet is not a tweet and a reply.
    #[test]
    fn a_tweet_rendered_twice_is_kept_once() {
        let one = tweet_markup("erling", 9001, "Well well well", "");
        let html = page(&format!("{one}{one}"));
        let (tweets, why) = detect(&html, &base());
        let tweets = tweets.expect("found");
        assert_eq!(tweets.len(), 1, "{why:?}");
        assert_eq!(why.kept, 1);
    }

    /// The tweet the address names leads, whatever order the page drew things in, and everything
    /// else on the page is a reply to it.
    #[test]
    fn the_focal_tweet_leads_and_the_rest_reply_at_depth_one() {
        let html = page(&format!(
            "{}{}{}",
            tweet_markup("someone", 7000, "An earlier unrelated tweet", ""),
            tweet_markup("erling", 9001, "Well well well", ""),
            tweet_markup("third", 7002, "A reply to it", ""),
        ));
        let (tweets, _) = detect(&html, &base());
        let tweets = tweets.expect("found");
        assert_eq!(tweets.len(), 3);
        assert_eq!(tweets[0].handle.as_deref(), Some("@erling"));
        assert_eq!(tweets[0].depth, 0);
        assert!(tweets[1..].iter().all(|p| p.depth == 1));
    }

    /// A timeline: nothing on it is the address. The first tweet leads at depth 0 and the rest
    /// follow at 1, rather than the detector giving up on a page with no focal tweet.
    #[test]
    fn a_timeline_with_no_focal_tweet_reads_top_to_bottom() {
        let html = page(&format!(
            "{}{}",
            tweet_markup("someone", 7000, "An earlier unrelated tweet", ""),
            tweet_markup("third", 7002, "A later one", ""),
        ));
        let home = Url::parse("https://example.invalid/home").expect("a literal URL");
        let (tweets, why) = detect(&html, &home);
        let tweets = tweets.expect("found");
        assert_eq!(tweets.len(), 2, "{why:?}");
        assert_eq!(tweets[0].handle.as_deref(), Some("@someone"));
        assert_eq!(tweets[0].depth, 0);
        assert_eq!(tweets[1].depth, 1);
        assert_eq!(why.weighed, 2);
        assert_eq!(why.kept, 2);
    }

    /// The reader arrived by a shared link, query and all. The page's own permalink wears no
    /// query, and the focal tweet is still the one the path names.
    #[test]
    fn a_tracking_query_on_the_address_does_not_lose_the_focal_tweet() {
        let html = page(&format!(
            "{}{}",
            tweet_markup("someone", 7000, "An earlier unrelated tweet", ""),
            tweet_markup("erling", 9001, "Well well well", ""),
        ));
        let shared = Url::parse("https://example.invalid/erling/status/9001?s=20&t=abc")
            .expect("a literal URL");
        let (tweets, _) = detect(&html, &shared);
        let tweets = tweets.expect("found");
        assert_eq!(tweets[0].handle.as_deref(), Some("@erling"));
        assert_eq!(tweets[0].depth, 0);
    }

    /// A pinned tweet drawn as the sticky and again in the list, with its date printed and not
    /// linked, so neither copy has a permalink. The name, the date and the words still say it is
    /// one tweet.
    #[test]
    fn a_permalinkless_tweet_rendered_twice_is_kept_once() {
        let one = r#"<article>
             <a href="/erling"><img alt="@erling" src="/pfp/erling.jpg"></a>
             <a href="https://example.invalid/erling">Real Name</a>
             <a href="https://example.invalid/erling">@erling</a>
             <div dir="auto"><span>Well well well</span></div>
             <time>10:47 PM · Jul 5, 2026</time>
             <div><button aria-label="Repost"><span>166K</span></button>
                  <button aria-label="Like"><span>2.6M</span></button></div>
           </article>"#;
        let html = page(&format!("{one}{one}"));
        let (tweets, why) = detect(&html, &base());
        let tweets = tweets.expect("found");
        assert_eq!(tweets.len(), 1, "{why:?}");
        assert!(tweets[0].permalink.is_none());
        assert_eq!(
            tweets[0].timestamp.as_deref(),
            Some("10:47 PM · Jul 5, 2026")
        );
        assert_eq!(why.weighed, 2);
        assert_eq!(why.kept, 1);
    }

    /// Two tweets by one author at one minute are two tweets. Only a copy that matches in every
    /// printed part is folded.
    #[test]
    fn two_permalinkless_tweets_by_one_author_stay_two() {
        let one = |words: &str| {
            format!(
                r#"<article>
                 <a href="https://example.invalid/erling">Real Name</a>
                 <a href="https://example.invalid/erling">@erling</a>
                 <div dir="auto"><span>{words}</span></div>
                 <time>Jul 5, 2026</time>
                 <div><button aria-label="Repost"><span>166K</span></button>
                      <button aria-label="Like"><span>2.6M</span></button></div>
               </article>"#
            )
        };
        let html = page(&format!(
            "{}{}",
            one("Well well well"),
            one("Something else entirely")
        ));
        let (tweets, _) = detect(&html, &base());
        assert_eq!(tweets.expect("found").len(), 2);
    }

    /// The name is read from an at-name and from nothing else. A container attributed only by
    /// a class the thread detector would recognise is not a tweet here: the decision may not
    /// rest on a class name, and a forum entry belongs to `thread`.
    #[test]
    fn a_name_carried_only_by_a_class_name_does_not_make_a_tweet() {
        let html = page(
            r#"<article><span class="author">Real Name</span>
                 <div dir="auto">Some words that were said.</div>
                 <a href="/x/status/3">Jul 5, 2026</a>
                 <div><button aria-label="Repost"><span>166K</span></button>
                      <button aria-label="Like"><span>2.6M</span></button></div>
               </article>"#,
        );
        let (tweets, why) = detect(&html, &base());
        assert!(tweets.is_none(), "{why:?}");
        assert_eq!(why.candidates[0].rejected, Some("no name over it"));
    }

    /// A `<time>` element dates a tweet without linking it: that is a tag saying what the thing
    /// is, not a class name. A date carried only by a class does not.
    #[test]
    fn an_unlinked_date_is_read_from_a_time_element_and_not_from_a_class() {
        let with = |stamp: &str| {
            page(&format!(
                r#"<article>
                 <a href="https://example.invalid/erling">Real Name</a>
                 <a href="https://example.invalid/erling">@erling</a>
                 <div dir="auto"><span>Well well well</span></div>
                 {stamp}
                 <div><button aria-label="Repost"><span>166K</span></button>
                      <button aria-label="Like"><span>2.6M</span></button></div>
               </article>"#
            ))
        };
        let (tweets, _) = detect(&with("<time>Jul 5, 2026</time>"), &base());
        let p = tweets.expect("found").swap_remove(0);
        assert_eq!(p.timestamp.as_deref(), Some("Jul 5, 2026"));
        assert!(p.permalink.is_none());
        let (tweets, why) = detect(
            &with(r#"<span class="timestamp">Jul 5, 2026</span>"#),
            &base(),
        );
        assert!(tweets.is_none(), "{why:?}");
        assert_eq!(why.candidates[0].rejected, Some("no date under it"));
    }

    /// A container holding the tweet plus a login prompt is the page's layout. Taking it would
    /// duplicate the tweet and drag the prompt into the reading column.
    #[test]
    fn a_wrapper_around_a_tweet_is_not_a_second_tweet() {
        let html = page(&format!(
            "<article><div>{}</div><div>Log in or sign up</div></article>",
            tweet_markup("erling", 9001, "Well well well", "")
        ));
        let (tweets, why) = detect(&html, &base());
        assert_eq!(tweets.expect("found").len(), 1);
        assert!(
            why.candidates
                .iter()
                .any(|c| c.rejected == Some("wraps another candidate")),
            "{why:?}"
        );
    }

    #[test]
    fn a_date_is_told_from_a_sentence() {
        assert!(looks_like_time("10:47 PM · Jul 5, 2026"));
        assert!(looks_like_time("Jul 5"));
        assert!(looks_like_time("2026-07-05"));
        assert!(looks_like_time("5h"));
        assert!(looks_like_time("2 days ago"));
        assert!(!looks_like_time("Well well well"));
        assert!(!looks_like_time("56K"));
        assert!(!looks_like_time(
            "A sentence of prose that happens to mention 5 things and runs long"
        ));
    }

    #[test]
    fn an_at_name_is_told_from_an_email_and_a_sentence() {
        assert!(is_handle("@erling"));
        assert!(is_handle("@a_b.c-d"));
        assert!(!is_handle("erling"));
        assert!(!is_handle("@"));
        assert!(!is_handle("@someone and then some words"));
    }
}
