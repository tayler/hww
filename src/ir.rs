//! The semantic IR: the whole point of the project.
//!
//! Every source format (HTML, gemtext, RSS, gopher, Markdown) maps *into* this, and every
//! renderer consumes *only* this. That is what makes a multi-protocol reader one program
//! instead of five, and what lets a GUI, speech, or e-ink backend be interchangeable.
//!
//! **Invariant: the IR carries no styling.** No colors, fonts, widths, alignment, or spacing.
//! Presentation belongs entirely to the renderer, and therefore to the reader. If you ever
//! feel the need to add a `color` or `width` field here, the answer is a renderer setting.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub url: String,
    pub title: Option<String>,
    pub byline: Option<String>,
    pub published: Option<String>,
    pub site_name: Option<String>,
    /// Absolute URL of the page's favicon, when the markup named one or the well-known
    /// `/favicon.ico` fallback applies. A URL, not pixels: fetching and decoding are the
    /// GUI reader's job, and headless extraction never does either.
    pub favicon: Option<String>,
    pub lang: Option<String>,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Block {
    Heading {
        level: u8,
        inlines: Vec<Inline>,
    },
    Paragraph(Vec<Inline>),
    List {
        ordered: bool,
        items: Vec<Vec<Block>>,
    },
    Quote {
        blocks: Vec<Block>,
        cite: Option<String>,
    },
    Code {
        lang: Option<String>,
        text: String,
    },
    Figure {
        image: Image,
        caption: Option<Vec<Inline>>,
    },
    /// Simple grid only. No colspan/rowspan: a table that needs them is a layout table,
    /// and layout tables are exactly what this client is refusing to reproduce.
    Table {
        headers: Vec<Vec<Inline>>,
        rows: Vec<Vec<Vec<Inline>>>,
    },
    Rule,
    /// Never auto-loaded. A placeholder the reader may choose to act on.
    Embed {
        kind: EmbedKind,
        url: String,
    },
    /// Threaded discussion. Added after Phase 0 measurement: forum pages were 8/8 broken
    /// under article extraction (5.6% of content recovered), and a flat `Vec<Block>` cannot
    /// represent reply structure. See docs/findings.md.
    Thread(Vec<Comment>),
    /// A run of story cards: the shape of a front page, a section page, or a feed. Added
    /// after the Phase 3 triage: every front page in a top-ten news sample was either a legal
    /// notice (article scoring) or a four-post discussion (thread detection), and a renderer
    /// handed linked paragraphs cannot tell a headline from a dek. A feed maps onto the same
    /// block. See docs/findings.md.
    Entries(Vec<Entry>),
    /// A tweet and the replies the page showed under it: the shape X serves on a status
    /// permalink, and named for X because no second social host has been measured (see
    /// `tweet.rs`). Added after the Phase 7 triage: a tweet is a handful of words wrapped in
    /// links and buttons, so article scoring puts it under the floor and the reader is told the
    /// page needs JavaScript when the server sent the whole thing. Flat with a `depth`, as
    /// `Thread` is. See docs/findings.md.
    Tweets(Vec<Tweet>),
}

/// One tweet in a `Tweets` run: who wrote it, what they said, and what the page said about how
/// it was received. `depth` is reply nesting, 0 for the tweet the page is about. The one producer,
/// `tweet::assemble`, writes 0 and 1 and nothing deeper, because a page carries no reply chain
/// it can read; the field is a `u16` so the renderers indent it with the arithmetic they
/// already have for `Comment::depth`, not because a deeper value is ever produced.
///
/// Not an `Entry` and not a `Comment`. An `Entry` is a card pointing at something to read; this
/// *is* the thing read. A `Comment` has nowhere to put a picture or a count, and a reply here
/// carries both.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tweet {
    /// Display name, as the page wrote it.
    pub author: Option<String>,
    /// The at-name, `@` included, when the page carried one distinct from the display name.
    pub handle: Option<String>,
    /// The tweet's own address, from the anchor its timestamp wore.
    pub permalink: Option<String>,
    /// The words the page printed, unparsed: "10:47 PM · Jul 5, 2026", or just "Jul 5". A date
    /// is what the page said it was; reformatting it would be this client inventing precision.
    pub timestamp: Option<String>,
    /// The author's picture. A URL and not pixels, like every other [`Image`], and drawn under
    /// the reader's image policy like every other picture: nothing here is fetched by itself.
    pub avatar: Option<Image>,
    pub depth: u16,
    pub blocks: Vec<Block>,
    pub stats: Vec<Stat>,
}

/// One engagement count, exactly as the page printed it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stat {
    pub kind: StatKind,
    /// Verbatim: "56K", "2.6M", "108.2M". Kept a string on purpose. The page rounds these and
    /// the exact figures are not in the markup, so parsing "2.6M" back to 2,600,000 would print
    /// a number nobody published.
    pub count: String,
}

/// What a count counts. Naming the kind is what separates a labelled stats row from four bare
/// numbers, which is the whole reason a tweet is a block rather than a run of paragraphs.
///
/// The six are the row X draws and no other network's: a share, an upvote, or a reaction has
/// no kind here, which is one reason the block is named for the site. A new kind is a variant,
/// a word in `label`, and a row in `every_stat_kind_has_its_own_word`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StatKind {
    Replies,
    Reposts,
    Likes,
    Quotes,
    Bookmarks,
    Views,
}

impl StatKind {
    /// The word a renderer prints beside the count. Here rather than in a renderer because
    /// every renderer must agree on what the number means; how it is laid out is theirs.
    pub fn label(self) -> &'static str {
        match self {
            StatKind::Replies => "replies",
            StatKind::Reposts => "reposts",
            StatKind::Likes => "likes",
            StatKind::Quotes => "quotes",
            StatKind::Bookmarks => "bookmarks",
            StatKind::Views => "views",
        }
    }
}

/// One card in an `Entries` run: a headline, where it leads, and what the card said about it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub title: Vec<Inline>,
    pub href: Option<String>,
    /// The dek and anything else the card carried, minus the headline and the thumbnail.
    pub summary: Vec<Block>,
    pub published: Option<String>,
    /// The address to show under the headline, for a list whose rows are hard to tell apart
    /// without it. `None` on every entry a page produced: a card or a search result already says
    /// where it goes in its own words, and printing the target of every headline on a front page
    /// is what `ReadOpts::show_link_urls` is for.
    ///
    /// Semantics and not presentation, as `published` is: this says the address is part of what
    /// the row *says*, and the renderer decides how it looks. See `reader::archive`, whose
    /// history view is the one thing that sets it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    /// The mark of the site this row leads to: an absolute URL, drawn small in a column to the
    /// left of the headline. `None` on every entry a page produced, as `address` is — a front
    /// page's cards all lead to the same site, so a column of one identical mark per row would
    /// be twenty requests to say one thing. `reader::archive`'s bookmarks view is what sets it.
    ///
    /// Separate from `image`, which is the card's own picture and is drawn as a thumbnail under
    /// the reader's image policy. These are different objects: one is what the row is *about*,
    /// the other is whose page it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub image: Option<Image>,
}

/// One post in a `Thread`. `depth` is reply nesting, 0 for top level.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comment {
    pub author: Option<String>,
    pub timestamp: Option<String>,
    pub depth: u16,
    pub id: Option<String>,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EmbedKind {
    Video,
    Audio,
    Iframe,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Inline {
    Text(String),
    Emph(Vec<Inline>),
    Strong(Vec<Inline>),
    Code(String),
    Link { href: String, inlines: Vec<Inline> },
    Image(Image),
    Break,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Image {
    pub src: String,
    pub alt: Option<String>,
}

impl Document {
    /// Total visible text length. Used by the extraction harness to score coverage.
    pub fn text_len(&self) -> usize {
        blocks_text_len(&self.blocks)
    }
}

/// The text length below which a document is treated as having failed to extract.
///
/// One constant, two consumers on opposite sides of the pipeline: `html::extract` falls back
/// to `<body>` below it, and `reader::notice` warns about a page that came out under it. Those
/// are the same event seen from each end, so they cannot be allowed to drift; it lives here
/// because text length is this crate's shared scoring currency and both sides already depend
/// on the IR. It is a threshold, not a style, so the no-styling invariant is untouched.
pub const THIN_TEXT: usize = 200;

/// Visible text length of a block list. Text length is the shared scoring currency of this
/// crate; reuse this rather than adding another counter.
pub fn blocks_text_len(blocks: &[Block]) -> usize {
    blocks.iter().map(block_text_len).sum()
}

/// Collapse every run of whitespace to one space, with the ends trimmed.
///
/// One implementation for the seven places that wanted it. The shape it replaces —
/// `split_whitespace().collect::<Vec<_>>().join(" ")` — allocates a vector of borrows before the
/// string it actually wanted; this writes the words straight into the output. It lives here
/// beside [`plain_text`] for the same reason that does: title de-duplication, the outline, a
/// stored title, a feed's title, and both extractors have to agree on what the words are, and
/// whitespace is not styling.
pub fn normalize_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for word in s.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

/// Inlines as plain text, with no renderer's sigils, indentation, or link decoration.
///
/// Presentation-neutral by construction, which is why it belongs here beside [`text_len`]
/// rather than in any one renderer: title de-duplication, find-in-page, and outline slugs all
/// need *the words*, and they must all agree on what the words are.
pub fn plain_text(inlines: &[Inline]) -> String {
    let mut out = String::new();
    push_plain(inlines, &mut out);
    out
}

fn push_plain(inlines: &[Inline], out: &mut String) {
    for i in inlines {
        match i {
            Inline::Text(t) | Inline::Code(t) => out.push_str(t),
            Inline::Emph(v) | Inline::Strong(v) => push_plain(v, out),
            Inline::Link { inlines, .. } => push_plain(inlines, out),
            Inline::Image(img) => out.push_str(img.alt.as_deref().unwrap_or("")),
            Inline::Break => out.push('\n'),
        }
    }
}

fn block_text_len(b: &Block) -> usize {
    match b {
        Block::Heading { inlines, .. } | Block::Paragraph(inlines) => inlines_text_len(inlines),
        Block::List { items, .. } => items.iter().map(|i| blocks_text_len(i)).sum(),
        Block::Quote { blocks, .. } => blocks_text_len(blocks),
        Block::Code { text, .. } => text.len(),
        Block::Figure { caption, .. } => caption.as_deref().map_or(0, inlines_text_len),
        Block::Table { headers, rows } => {
            headers.iter().map(|c| inlines_text_len(c)).sum::<usize>()
                + rows
                    .iter()
                    .flatten()
                    .map(|c| inlines_text_len(c))
                    .sum::<usize>()
        }
        Block::Rule | Block::Embed { .. } => 0,
        Block::Thread(cs) => cs.iter().map(|c| blocks_text_len(&c.blocks)).sum(),
        Block::Entries(es) => es
            .iter()
            .map(|e| inlines_text_len(&e.title) + blocks_text_len(&e.summary))
            .sum(),
        // Bodies only, as the `Thread` arm above counts comment bodies only: an author, a
        // date, and a row of counts are attribution, and letting them score would make a
        // stats row look like prose to every caller that measures coverage.
        Block::Tweets(ps) => ps.iter().map(|p| blocks_text_len(&p.blocks)).sum(),
    }
}

/// Visible text length of an inline run. The inline half of [`blocks_text_len`], and the same
/// currency: reuse this rather than measuring `plain_text(..).len()`, which is a different
/// number (it counts an image's alt text and a break's newline, and this counts neither).
pub fn inlines_text_len(inlines: &[Inline]) -> usize {
    inlines
        .iter()
        .map(|i| match i {
            Inline::Text(s) | Inline::Code(s) => s.len(),
            Inline::Emph(v) | Inline::Strong(v) => inlines_text_len(v),
            Inline::Link { inlines, .. } => inlines_text_len(inlines),
            Inline::Image(_) | Inline::Break => 0,
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Inline {
        Inline::Text(s.to_owned())
    }

    #[test]
    fn plain_text_drops_markup_and_keeps_words() {
        let inlines = vec![
            t("a "),
            Inline::Emph(vec![t("b")]),
            t(" "),
            Inline::Link {
                href: "https://example.com/".into(),
                inlines: vec![Inline::Strong(vec![t("c")])],
            },
            Inline::Code("d".into()),
        ];
        assert_eq!(plain_text(&inlines), "a b cd");
    }

    #[test]
    fn plain_text_uses_image_alt_and_nothing_for_missing_alt() {
        let with_alt = Inline::Image(Image {
            src: "x".into(),
            alt: Some("a cat".into()),
        });
        let without = Inline::Image(Image {
            src: "x".into(),
            alt: None,
        });
        assert_eq!(plain_text(&[with_alt]), "a cat");
        assert_eq!(plain_text(&[without]), "");
    }

    #[test]
    fn blocks_text_len_sums_nested_structure() {
        let blocks = vec![
            Block::Paragraph(vec![t("1234")]),
            Block::Quote {
                blocks: vec![Block::Paragraph(vec![t("567")])],
                cite: None,
            },
        ];
        assert_eq!(blocks_text_len(&blocks), 7);
    }

    /// A tweet measures its message and not its attribution, exactly as a `Thread` measures its
    /// comments' bodies and not their authors. The counter's job is deciding whether extraction
    /// failed, and a name, a date, and a row of counts are what a JavaScript shell renders when
    /// it has nothing: letting those score would make an empty shell look like content.
    #[test]
    fn a_tweet_measures_its_message_and_not_its_attribution() {
        let words = "a".repeat(40);
        let b = Block::Tweets(vec![Tweet {
            author: Some("A Writer With A Long Name".into()),
            handle: Some("@a_rather_long_handle".into()),
            permalink: Some("https://example.test/a/b".into()),
            timestamp: Some("10:47 PM \u{b7} Jul 5, 2026".into()),
            avatar: Some(Image {
                src: "https://example.test/pfp.jpg".into(),
                alt: None,
            }),
            depth: 0,
            blocks: vec![Block::Paragraph(vec![Inline::Text(words.clone())])],
            stats: vec![
                Stat {
                    kind: StatKind::Likes,
                    count: "2.6M".into(),
                },
                Stat {
                    kind: StatKind::Views,
                    count: "108.2M".into(),
                },
            ],
        }]);
        assert_eq!(blocks_text_len(std::slice::from_ref(&b)), words.len());
    }

    /// Every kind prints a word, and no two kinds print the same one, or a stats row would say
    /// "2.6M likes \u{b7} 44K likes" and the reader could not tell which was which.
    #[test]
    fn every_stat_kind_has_its_own_word() {
        let kinds = [
            StatKind::Replies,
            StatKind::Reposts,
            StatKind::Likes,
            StatKind::Quotes,
            StatKind::Bookmarks,
            StatKind::Views,
        ];
        let mut seen: Vec<&str> = kinds.iter().map(|k| k.label()).collect();
        assert!(seen.iter().all(|l| !l.is_empty()));
        seen.sort_unstable();
        let before = seen.len();
        seen.dedup();
        assert_eq!(seen.len(), before, "two kinds share a word");
    }
}
