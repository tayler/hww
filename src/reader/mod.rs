//! The reading experience — the pure half compiles always, the egui half behind `gui`.
//!
//! The split is deliberate and is not "gui vs non-gui files": everything decidable without a
//! `Context` lives in its own module, so inline flattening, outline building, history, title
//! de-duplication, thread reconstruction, and image decoding are all exercised by `cargo test`
//! rather than only by opening a window. What is confined to `ui/` is what genuinely needs a
//! `Context`: chrome, widget dispatch, and texture upload — the code that can only be
//! compiled, never tested headlessly.
//!
//! [`image_decode`] is the sharpest case. It is the only module in the project that parses
//! untrusted binary input, and putting it under `ui/` would have made the highest-risk code
//! the one code no test ever runs. It needs the `image` crate, so it compiles under `gui` —
//! but it takes `&[u8]` and returns pixels, so `cargo test --features gui` covers the format
//! matrix, the ceilings, and the partial-decode paths with fixture bytes and no window.
//!
//! The module is named `reader` rather than `gui` because the pure half is exactly what a
//! future TUI reuses.

pub mod history;
pub mod inline;
pub mod opts;
pub mod outline;
pub mod settings;
pub mod thread_tree;
pub mod title;

/// Needs the `image` crate, which rides with `gui` — but no `Context`, so its tests run
/// headless. See the module doc for why it is not under `ui/`.
#[cfg(feature = "gui")]
pub mod image_decode;

#[cfg(feature = "gui")]
pub mod ui;
