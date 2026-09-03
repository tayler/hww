//! The reading experience: the pure half compiles always, the egui half behind `gui`.
//!
//! The split is deliberate and is not "gui vs non-gui files": everything decidable without a
//! `Context` lives in its own module, so inline flattening, outline building, history, title
//! de-duplication, thread reconstruction, and image decoding are all exercised by `cargo test`
//! rather than only by opening a window. What is confined to `ui/` is what genuinely needs a
//! `Context`: chrome, widget dispatch, and texture upload, the code that can only be
//! compiled, never tested headlessly.
//!
//! [`image_decode`] is the sharpest case. It is the only module in the project that parses
//! untrusted binary input, and putting it under `ui/` would have made the highest-risk code
//! the one code no test ever runs. It needs the `image` crate, so it compiles under `gui`,
//! but it takes `&[u8]` and returns pixels, so `cargo test --features gui` covers the format
//! matrix, the ceilings, and the partial-decode paths with fixture bytes and no window.
//!
//! The module is named `reader` rather than `gui` because the pure half is renderer-agnostic:
//! a future speech or e-ink backend reuses it unchanged.

/// The archive: what hww is allowed to remember, and the file it remembers it in. No egui
/// types, so the fast CI job tests it. Read its module doc before adding anything that writes to
/// disk — the doctrine is there and it is the point of the module.
pub mod archive;
/// Which pictures `ImagePolicy::Auto` asks for. No egui types, so the fast CI job tests it,
/// which is the point: the mistakes are set arithmetic and none of them show in a screenshot.
pub mod autoload;
/// Where an entry's summary stops, for the draw and for the image walk alike. No egui types,
/// so the fast CI job tests it, which is the point: a budget the two disagreed about would
/// fetch pictures the reader never sees.
pub mod budget;
/// What the desktop asks a `Theme::System` reader for, and how that answer changes while the
/// window is open. No egui types, so the fast CI job tests the reply parsing.
pub mod desktop;
/// Which face an inline style is set in. No egui types, so the fast CI job tests it.
pub mod face;
pub mod history;
/// Site-icon bytes on disk, for addresses the archive already names. No egui types and no
/// network, so the fast CI job tests it. Read `archive`'s doctrine and then this module's own
/// before writing anything else to disk: this is the one exception to the rule that nothing hww
/// fetches is kept, and it argues for itself there.
pub mod iconcache;
/// Which picture formats this reader declines, and the three claims that name one. No egui
/// types and no `image` crate, so the fast CI job tests it — which is the point, since one of
/// the three is asked while building the IR in the default build. [`image_decode`] holds the
/// fourth answer, the byte sniff, and the test that keeps them in step.
pub mod image_formats;

pub mod inline;
/// The link macOS hands a bundled application, which it sends as an Apple Event rather than
/// putting in `argv`. macOS only, and the one module in the crate no CI job on another platform
/// compiles; it decides nothing about the address it receives, for that reason.
#[cfg(target_os = "macos")]
pub mod mac_open;
/// How tall each block of the page was last time it was laid out, so a page longer than the
/// window costs a window's worth of layout. No egui types, so the fast CI job tests it.
pub mod measure;
/// The menu bar as data, and the help card's table. No egui types, so the fast CI job tests
/// both, which is why the help table moved out of `ui::app` to live here.
pub mod menu;
/// What the reader has to say about a page, and how loud. No egui types, so the fast CI
/// job tests it.
pub mod notice;
pub mod opts;
pub mod outline;
/// Which pages Back and Forward can open without asking the site again. No egui types, so the
/// fast CI job tests it, which is the point: the mistakes are eviction arithmetic and none of
/// them show in a screenshot.
pub mod pagecache;
/// How a page arrived, as labelled rows for the page-info panel. No egui types, so the fast
/// CI job tests it. `notice`'s quiet half: what does not rise to interrupting the column.
pub mod pageinfo;
/// Every setting as a labelled control with a sentence about it, for the settings panel. No
/// egui types, so the fast CI job tests it, and the test that matters walks the serialized
/// `Settings` so a field added without a control fails the build.
pub mod prefs;
pub mod settings;
pub mod thread_tree;
pub mod title;

/// Needs the `image` crate, which rides with `gui`, but no `Context`, so its tests run
/// headless. See the module doc for why it is not under `ui/`.
#[cfg(feature = "gui")]
pub mod image_decode;

#[cfg(feature = "gui")]
pub mod ui;
