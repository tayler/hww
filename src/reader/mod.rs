//! The reading experience — the pure half compiles always, the egui half behind `gui`.
//!
//! The split is deliberate and is not "gui vs non-gui files": everything decidable without a
//! `Context` lives in the modules below, so the default CI job — which never compiles egui —
//! still exercises inline flattening, outline building, history, title de-duplication, thread
//! reconstruction, and image decoding. What only compiles under `gui` is `ui/`: chrome, widget
//! dispatch, and texture upload.
//!
//! It is named `reader` rather than `gui` because the pure half is exactly what a future TUI
//! reuses.

pub mod opts;

#[cfg(feature = "gui")]
pub mod ui;
