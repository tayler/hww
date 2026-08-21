//! hww: a quiet-web client.
//!
//! A reading client for the non-app web: stripped HTML, feeds, gemini, gopher, and local Markdown.
//! JavaScript, the CSS cascade, ads, third-party requests, and cookies are all absent.
//!
//! Architecture: every source format maps into [`ir::Document`], and every renderer consumes
//! only that. See `docs/phase0-findings.md` for the measurements this design rests on.

pub mod fetch;
pub mod html;
pub mod ir;
pub mod reader;
pub mod render;
pub mod rewrite;
pub mod session;
pub mod thread;
