//! The semantic IR: the whole point of the project.
//!
//! Every source format (HTML, gemtext, RSS, gopher, Markdown) maps *into* this, and every
//! renderer consumes *only* this. That is what makes a multi-protocol reader one program
//! instead of five, and what lets a GUI, TUI, speech, or e-ink backend be interchangeable.
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
    /// represent reply structure. See docs/phase0-findings.md.
    Thread(Vec<Comment>),
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
    }
}

fn inlines_text_len(inlines: &[Inline]) -> usize {
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
}
