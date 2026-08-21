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
pub mod session;
pub mod thread;
