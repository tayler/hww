//! hww — a quiet-web client.
//!
//! A reading client for the non-app web: stripped HTML, feeds, gemini, gopher, local Markdown.
//! No JavaScript, no CSS cascade, no ads, no third-party requests, no cookies.
//!
//! Architecture: every source format maps into [`ir::Document`], and every renderer consumes
//! only that. See `docs/phase0-findings.md` for the measurements this design rests on.

pub mod fetch;
pub mod html;
pub mod ir;
pub mod render;
pub mod rewrite;
pub mod thread;

/// Length of the text carried by a list of inlines. Shared by the extractors.
pub(crate) fn html_inlines_len(blocks: &[ir::Block]) -> usize {
    ir::Document {
        url: String::new(),
        title: None,
        byline: None,
        published: None,
        site_name: None,
        lang: None,
        blocks: blocks.to_vec(),
    }
    .text_len()
}
