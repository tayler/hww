//! IR -> plain text. The first of several renderers; deliberately the dumbest one.
//!
//! Presentation decisions live here, never in the IR. Width, indentation, and how a link
//! is shown are this renderer's business alone.
//!
//! Inline styling is *not* re-derived here: `inline_text` maps `reader::inline::flatten`'s
//! output to markdown sigils, so the text renderer and the GUI cannot disagree about where an
//! emphasis boundary falls. See that module for the two normalizations flattening introduces,
//! and `render`'s tests for the old recursive walker kept as their oracle.

use crate::ir::{Block, Document, Inline};
use crate::reader::inline::{Run, Style, flatten};

pub struct TextOpts {
    pub width: usize,
    pub show_links: bool,
    /// Levels of reply indent before nesting stops accumulating.
    ///
    /// `Comment::depth` is page-controlled (`thread::depth_of` parses an `indent` attribute
    /// as a `u16` and does not clamp it), and the pad is emitted once per *word*, because a
    /// pad wider than `width` drives the wrap width to zero. Uncapped, one comment carrying
    /// `indent="60000"` and a 240-character body renders 5.9 MB of spaces. The GUI has always
    /// capped this via [`crate::reader::opts::ReadOpts::max_thread_indent`]; this is the same
    /// bound for the same reason.
    pub max_thread_indent: u16,
}

impl Default for TextOpts {
    fn default() -> Self {
        Self {
            width: 78,
            show_links: false,
            max_thread_indent: 8,
        }
    }
}

impl TextOpts {
    /// Columns of indent for a reply at `depth`.
    fn indent_for(&self, depth: u16) -> usize {
        usize::from(depth.min(self.max_thread_indent)) * 2
    }
}

pub fn to_text(doc: &Document, opts: &TextOpts) -> String {
    let mut out = String::new();
    if let Some(t) = crate::reader::title::display(doc) {
        out.push_str(&t);
        out.push('\n');
        out.push_str(&"=".repeat(t.chars().count().min(opts.width)));
        out.push_str("\n\n");
    }
    if let Some(b) = &doc.byline {
        out.push_str(&format!("by {b}\n\n"));
    }
    // The same masthead skip the reader applies: the headline the title line already shows,
    // and the "By Name" the byline line already shows. `reader::title` owns the decision.
    let masthead = crate::reader::title::masthead(doc);
    let body: Vec<Block> = doc
        .blocks
        .iter()
        .enumerate()
        .filter(|(i, _)| !masthead.skips(*i))
        .map(|(_, b)| b.clone())
        .collect();
    render_blocks(&body, opts, 0, &mut out);
    out
}

fn render_blocks(blocks: &[Block], o: &TextOpts, indent: usize, out: &mut String) {
    for b in blocks {
        render_block(b, o, indent, out);
    }
}

fn render_block(b: &Block, o: &TextOpts, indent: usize, out: &mut String) {
    let pad = " ".repeat(indent);
    match b {
        Block::Heading { level, inlines } => {
            let t = inline_text(inlines, o);
            out.push_str(&format!("\n{pad}{} {t}\n\n", "#".repeat(*level as usize)));
        }
        Block::Paragraph(inlines) => {
            let t = inline_text(inlines, o);
            if !t.trim().is_empty() {
                out.push_str(&wrap(&t, o.width.saturating_sub(indent), &pad));
                out.push('\n');
            }
        }
        Block::List { ordered, items } => {
            for (i, item) in items.iter().enumerate() {
                let marker = if *ordered {
                    format!("{}. ", i + 1)
                } else {
                    "- ".to_string()
                };
                let mut buf = String::new();
                render_blocks(item, o, 0, &mut buf);
                let body = buf.trim();
                let mut lines = body.lines();
                if let Some(first) = lines.next() {
                    out.push_str(&format!("{pad}{marker}{first}\n"));
                    for l in lines {
                        out.push_str(&format!("{pad}   {l}\n"));
                    }
                }
            }
            out.push('\n');
        }
        Block::Quote { blocks, .. } => {
            let mut buf = String::new();
            render_blocks(blocks, o, 0, &mut buf);
            for l in buf.trim().lines() {
                out.push_str(&format!("{pad}> {l}\n"));
            }
            out.push('\n');
        }
        Block::Code { text, .. } => {
            for l in text.lines() {
                out.push_str(&format!("{pad}    {l}\n"));
            }
            out.push('\n');
        }
        Block::Figure { image, caption } => {
            let alt = image.alt.as_deref().unwrap_or("image");
            let cap = caption
                .as_ref()
                .map(|c| inline_text(c, o))
                .unwrap_or_default();
            out.push_str(&format!(
                "{pad}[{alt}]{}\n\n",
                if cap.is_empty() {
                    String::new()
                } else {
                    format!(" — {cap}")
                }
            ));
        }
        Block::Table { headers, rows } => {
            if !headers.is_empty() {
                let h: Vec<String> = headers.iter().map(|c| inline_text(c, o)).collect();
                out.push_str(&format!("{pad}{}\n", h.join(" | ")));
            }
            for r in rows {
                let c: Vec<String> = r.iter().map(|c| inline_text(c, o)).collect();
                out.push_str(&format!("{pad}{}\n", c.join(" | ")));
            }
            out.push('\n');
        }
        Block::Rule => out.push_str(&format!("{pad}{}\n\n", "-".repeat(20))),
        Block::Embed { kind, url } => out.push_str(&format!("{pad}[{kind:?}: {url}]\n\n")),
        Block::Thread(comments) => {
            for c in comments {
                let who = c.author.as_deref().unwrap_or("anon");
                let when = c.timestamp.as_deref().unwrap_or("");
                let depth = indent + o.indent_for(c.depth);
                out.push_str(&format!("{}{who} {when}\n", " ".repeat(depth)));
                render_blocks(&c.blocks, o, depth, out);
            }
        }
        Block::Entries(entries) => {
            for e in entries {
                let title = inline_text(&e.title, o);
                match (&e.href, o.show_links) {
                    (Some(h), true) => out.push_str(&format!("{pad}* {title} <{h}>\n")),
                    _ => out.push_str(&format!("{pad}* {title}\n")),
                }
                let mut buf = String::new();
                render_blocks(&e.summary, o, 0, &mut buf);
                for l in buf.trim().lines() {
                    out.push_str(&format!("{pad}  {l}\n"));
                }
                if let Some(p) = &e.published {
                    out.push_str(&format!("{pad}  {p}\n"));
                }
                out.push('\n');
            }
        }
    }
}

fn inline_text(inlines: &[Inline], o: &TextOpts) -> String {
    runs_text(&flatten(inlines), o)
}

fn runs_text(runs: &[Run], o: &TextOpts) -> String {
    let mut s = String::new();
    for r in runs {
        match r {
            Run::Text { text, style } => s.push_str(&marked(text, *style)),
            Run::Link { href, runs } => {
                let t = runs_text(runs, o);
                if o.show_links {
                    // The href goes where the anchor's own trailing space was, so that space
                    // has to come back out the other side: without it `closure </a>Updated`
                    // renders as `closure <https://...>Updated`.
                    s.push_str(&link_text(&t, href));
                } else {
                    s.push_str(&t)
                }
            }
            Run::Image(img) => s.push_str(&format!("[{}]", img.alt.as_deref().unwrap_or("image"))),
            Run::Break => s.push('\n'),
        }
    }
    s
}

/// Sigils, innermost first: code, then strong, then emph, the order the recursive walker
/// produced for the nestings that actually occur.
fn marked(text: &str, style: Style) -> String {
    let mut t = text.to_owned();
    if style.code {
        t = format!("`{t}`");
    }
    if style.strong {
        t = format!("*{t}*");
    }
    if style.emph {
        t = format!("_{t}_");
    }
    t
}

/// Wrap to `width`, never narrower than this.
///
/// A wrap width of zero puts every word on its own line, each carrying the full pad, so a
/// wide indent does not merely look bad, it multiplies the output by the word count. The
/// indent cap in [`TextOpts::indent_for`] is the primary bound; this is the floor that holds
/// when indents nest or `width` is set very small.
const MIN_WRAP: usize = 20;

/// `anchor <href>`, keeping whatever word boundary the anchor's own text carried.
fn link_text(t: &str, href: &str) -> String {
    let tail = if t.ends_with(char::is_whitespace) {
        " "
    } else {
        ""
    };
    format!("{} <{href}>{tail}", t.trim_end())
}

fn wrap(text: &str, width: usize, pad: &str) -> String {
    let width = width.max(MIN_WRAP);
    let mut out = String::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split_whitespace() {
            if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
                out.push_str(pad);
                out.push_str(&line);
                out.push('\n');
                line.clear();
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        if !line.is_empty() {
            out.push_str(pad);
            out.push_str(&line);
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::Image;

    /// The walker `inline_text` replaced, kept verbatim as an oracle.
    ///
    /// This is the whole safety net for building the text renderer on top of `flatten`: the
    /// rewrite is only sound if it produces the same bytes, and the only honest way to check
    /// that is to run both.
    fn old_inline_text(inlines: &[Inline], o: &TextOpts) -> String {
        let mut s = String::new();
        for i in inlines {
            match i {
                Inline::Text(t) => s.push_str(t),
                Inline::Code(t) => s.push_str(&format!("`{t}`")),
                Inline::Emph(v) => s.push_str(&format!("_{}_", old_inline_text(v, o))),
                Inline::Strong(v) => s.push_str(&format!("*{}*", old_inline_text(v, o))),
                Inline::Link { href, inlines } => {
                    let t = old_inline_text(inlines, o);
                    if o.show_links {
                        s.push_str(&link_text(&t, href));
                    } else {
                        s.push_str(&t)
                    }
                }
                Inline::Image(img) => {
                    s.push_str(&format!("[{}]", img.alt.as_deref().unwrap_or("image")))
                }
                Inline::Break => s.push('\n'),
            }
        }
        s
    }

    fn t(s: &str) -> Inline {
        Inline::Text(s.to_owned())
    }

    fn link(href: &str, inner: Vec<Inline>) -> Inline {
        Inline::Link {
            href: href.to_owned(),
            inlines: inner,
        }
    }

    /// Every inline shape the extractor produces, checked against the old walker byte for
    /// byte, with links shown and hidden.
    #[test]
    fn flatten_reproduces_the_old_markdown_exactly() {
        let cases: Vec<Vec<Inline>> = vec![
            vec![],
            vec![t("plain prose")],
            vec![t("a "), Inline::Emph(vec![t("b")]), t(" c")],
            vec![Inline::Strong(vec![t("bold")])],
            vec![Inline::Code("f(x)".into())],
            vec![Inline::Strong(vec![Inline::Code("f(x)".into())])],
            vec![Inline::Emph(vec![Inline::Strong(vec![t("both")])])],
            vec![link("https://example.com/a", vec![t("anchor")])],
            vec![
                t("see "),
                link("https://example.com/a", vec![Inline::Emph(vec![t("this")])]),
                t(" now"),
            ],
            vec![
                link("https://example.com/a", vec![t("one")]),
                t(", "),
                link("https://example.com/b", vec![t("two")]),
            ],
            vec![t("line"), Inline::Break, t("next")],
            vec![Inline::Image(Image {
                src: "x.png".into(),
                alt: Some("a cat".into()),
            })],
            vec![Inline::Image(Image {
                src: "x.png".into(),
                alt: None,
            })],
            vec![
                t("mixed "),
                Inline::Strong(vec![t("b")]),
                Inline::Code("c".into()),
                link("https://example.com/", vec![t("d")]),
                Inline::Break,
            ],
            // The boundary a page puts *inside* the anchor: `<a>headline </a>Updated`.
            vec![
                link("https://example.com/a", vec![t("headline ")]),
                t("Updated"),
            ],
        ];
        for show_links in [false, true] {
            let o = TextOpts {
                show_links,
                ..TextOpts::default()
            };
            for case in &cases {
                assert_eq!(
                    inline_text(case, &o),
                    old_inline_text(case, &o),
                    "diverged on {case:?} (show_links={show_links})"
                );
            }
        }
    }

    /// The two documented normalizations, asserted rather than left as a surprise.
    #[test]
    fn the_only_divergences_are_the_documented_ones() {
        let o = TextOpts::default();

        // 1. Adjacent equal-styled segments merge: same words, fewer sigils.
        let adjacent = vec![Inline::Emph(vec![t("a")]), Inline::Emph(vec![t("b")])];
        assert_eq!(old_inline_text(&adjacent, &o), "_a__b_");
        assert_eq!(inline_text(&adjacent, &o), "_ab_");

        // 2. Emphasis nesting order is a set, so both orders render the same.
        let outer_strong = vec![Inline::Strong(vec![Inline::Emph(vec![t("x")])])];
        let outer_emph = vec![Inline::Emph(vec![Inline::Strong(vec![t("x")])])];
        assert_eq!(old_inline_text(&outer_strong, &o), "*_x_*");
        assert_eq!(inline_text(&outer_strong, &o), "_*x*_");
        assert_eq!(inline_text(&outer_emph, &o), inline_text(&outer_strong, &o));
    }

    #[test]
    fn a_whole_document_still_renders_the_way_it_did() {
        let doc = Document {
            url: "https://example.com/a".into(),
            title: Some("Title".into()),
            byline: Some("A Writer".into()),
            published: None,
            site_name: None,
            favicon: None,
            lang: None,
            blocks: vec![
                Block::Heading {
                    level: 2,
                    inlines: vec![t("A heading")],
                },
                Block::Paragraph(vec![
                    t("Prose with "),
                    Inline::Strong(vec![t("weight")]),
                    t(" and a "),
                    link("https://example.com/b", vec![t("link")]),
                    t("."),
                ]),
                Block::List {
                    ordered: false,
                    items: vec![vec![Block::Paragraph(vec![t("one")])]],
                },
            ],
        };
        let out = to_text(&doc, &TextOpts::default());
        assert!(out.starts_with("Title\n=====\n\nby A Writer\n\n"));
        assert!(out.contains("## A heading"));
        assert!(out.contains("Prose with *weight* and a link."));
        assert!(out.contains("- one"));
    }

    /// `Comment::depth` comes from a page-controlled `indent` attribute with no clamp, and the
    /// pad is emitted once per *word* once it exceeds the wrap width. Uncapped, this one
    /// comment rendered 5.9 MB of spaces.
    #[test]
    fn a_hostile_reply_depth_cannot_inflate_the_output() {
        let body = "word ".repeat(48).trim_end().to_owned();
        let doc = Document {
            url: String::new(),
            title: None,
            byline: None,
            published: None,
            site_name: None,
            favicon: None,
            lang: None,
            blocks: vec![Block::Thread(vec![crate::ir::Comment {
                author: Some("a".into()),
                timestamp: None,
                depth: u16::MAX,
                id: None,
                blocks: vec![Block::Paragraph(vec![t(&body)])],
            }])],
        };
        let out = to_text(&doc, &TextOpts::default());
        assert!(
            out.len() < 2_000,
            "one comment rendered {} bytes",
            out.len()
        );
        // Capped, not dropped: the reply is still indented as deeply as the cap allows.
        let o = TextOpts::default();
        assert!(out.contains(&" ".repeat(o.indent_for(o.max_thread_indent))));
    }

    /// The text renderer shares the reader's masthead: a headline the title line already
    /// shows and a "By Name" the byline line already shows are not printed again.
    #[test]
    fn the_masthead_is_not_printed_twice() {
        let doc = crate::html::extract(
            "<html><head><title>The Headline</title><meta name=\"author\" content=\"Jane Writer\"></head>\
             <body><article><h1>The Headline</h1><p class=\"byline\">By Jane Writer</p>\
             <p>Body prose long enough to be a paragraph in its own right, and then some more.</p>\
             </article></body></html>",
            &url::Url::parse("https://example.test/a").unwrap(),
        );
        let text = to_text(&doc, &TextOpts::default());
        assert_eq!(text.matches("The Headline").count(), 1, "{text}");
        assert_eq!(text.matches("Jane Writer").count(), 1, "{text}");
        assert!(text.contains("Body prose"));
    }

    #[test]
    fn entries_render_as_a_headline_its_summary_and_its_time() {
        let doc = Document {
            url: "https://example.test/".into(),
            title: None,
            byline: None,
            published: None,
            site_name: None,
            favicon: None,
            lang: None,
            blocks: vec![Block::Entries(vec![crate::ir::Entry {
                title: vec![Inline::Text("A headline of some length".into())],
                href: Some("https://example.test/s".into()),
                summary: vec![Block::Paragraph(vec![Inline::Text("The dek.".into())])],
                published: Some("2 hours ago".into()),
                image: None,
            }])],
        };
        let plain = to_text(&doc, &TextOpts::default());
        assert_eq!(
            plain,
            "* A headline of some length\n  The dek.\n  2 hours ago\n\n"
        );
        let linked = to_text(
            &doc,
            &TextOpts {
                show_links: true,
                ..TextOpts::default()
            },
        );
        assert!(linked.starts_with("* A headline of some length <https://example.test/s>\n"));
    }
}
