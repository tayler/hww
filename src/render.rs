//! IR -> plain text. The first of several renderers; deliberately the dumbest one.
//!
//! Presentation decisions live here, never in the IR. Width, indentation, and how a link
//! is shown are this renderer's business alone.

use crate::ir::{Block, Document, Inline};

pub struct TextOpts {
    pub width: usize,
    pub show_links: bool,
}

impl Default for TextOpts {
    fn default() -> Self {
        Self { width: 78, show_links: false }
    }
}

pub fn to_text(doc: &Document, opts: &TextOpts) -> String {
    let mut out = String::new();
    if let Some(t) = &doc.title {
        out.push_str(t);
        out.push('\n');
        out.push_str(&"=".repeat(t.chars().count().min(opts.width)));
        out.push_str("\n\n");
    }
    if let Some(b) = &doc.byline {
        out.push_str(&format!("by {b}\n\n"));
    }
    render_blocks(&doc.blocks, opts, 0, &mut out);
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
                let marker = if *ordered { format!("{}. ", i + 1) } else { "- ".to_string() };
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
            let cap = caption.as_ref().map(|c| inline_text(c, o)).unwrap_or_default();
            out.push_str(&format!("{pad}[{alt}]{}\n\n", if cap.is_empty() { String::new() } else { format!(" — {cap}") }));
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
                out.push_str(&format!("{}{who} {when}\n", " ".repeat(indent + c.depth as usize * 2)));
                render_blocks(&c.blocks, o, indent + c.depth as usize * 2, out);
            }
        }
    }
}

fn inline_text(inlines: &[Inline], o: &TextOpts) -> String {
    let mut s = String::new();
    for i in inlines {
        match i {
            Inline::Text(t) => s.push_str(t),
            Inline::Code(t) => s.push_str(&format!("`{t}`"),),
            Inline::Emph(v) => s.push_str(&format!("_{}_", inline_text(v, o))),
            Inline::Strong(v) => s.push_str(&format!("*{}*", inline_text(v, o))),
            Inline::Link { href, inlines } => {
                let t = inline_text(inlines, o);
                if o.show_links { s.push_str(&format!("{t} <{href}>")) } else { s.push_str(&t) }
            }
            Inline::Image(img) => s.push_str(&format!("[{}]", img.alt.as_deref().unwrap_or("image"))),
            Inline::Break => s.push('\n'),
        }
    }
    s
}

fn wrap(text: &str, width: usize, pad: &str) -> String {
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
