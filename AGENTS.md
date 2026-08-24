# Agent guidance

`README.md` carries product status, commands, module layout, and measurements.
Module-level `//!` comments carry subsystem rationale and are the primary design record.
`docs/phase0-findings.md` records measured decisions; `docs/sites-checked.md` is the host
ledger. This file contains only rules useful across tasks. Reference symbols, not line numbers.

## What this is

hww is a second browser for stripped, non-app HTML, with text and GUI renderers. Feeds,
gemini, gopher, Markdown, and archive are not started. Nothing writes page content to disk;
only `settings.json` survives a run.

Everything maps through `ir::Document`. Formats produce the IR and renderers consume it.
Never put colour, fonts, dimensions, alignment, spacing, or other presentation in `src/ir.rs`.
Use `render::TextOpts` or `reader::opts::ReadOpts`.

## CI

All five gates in `.github/workflows/ci.yml` must pass:

    cargo fmt --all -- --check
    cargo clippy --all-targets --locked -- -D warnings
    cargo test --locked
    cargo clippy --all-targets --locked --features gui,tools -- -D warnings
    cargo test --locked --features gui,tools

The default job never compiles egui. The second job uses explicit `gui,tools` features, never
`--all-features`. Tests do not call `eframe::run_native`. CI installs no apt packages; native
libraries are opened at runtime rather than linked at build time.

`Cargo.lock` is committed because this crate ships binaries.

## Diagnosis and visual checks

Diagnose a site with `cargo run --features gui --bin hww -- --why <url>` before reading extractor source.
`html::extract_traced` produces the document and explanation in one pass. For rendering, use
`cargo run --features gui --bin hww-shot -- --url URL --out DIR` or name only the scenes being
changed. Write to `./shots` only when deliberately re-baselining.

The full screenshot catalog is a separate regression run:

    cargo run --features gui --bin hww-shot -- --all
    cargo run --features gui --bin hww-shot -- --all --check

It owns the display and its window must remain visible. Do not run it as an incidental check.

Scenes drive real paths through four public hooks: `ReaderApp::new`, `present`, `follow_link`,
and `run_command`. A synthetic Tab does not reliably land focus, so anything reachable only by
tabbing to it, a link in the page or a menu item with no key of its own, needs a hook rather
than key steps; key steps photograph an open popup and never what the item does.

## Extraction

The merge order in `html::extract_traced` is load-bearing. Read its module documentation before
changing cards, content-root scoring, thread extraction, or body fallback.

Text length has one currency: `Document::text_len`, `ir::blocks_text_len`, and
`thread::comments_text_len`. Reuse them. `ir::THIN_TEXT` is the shared thin-document threshold.
`ir::plain_text` is the presentation-neutral flatten; `reader::inline::flatten` is the styled
flatten used by `render::inline_text`.

Per-site boolean extraction overrides were measured and removed. Profiles are data with a
floor and a report; do not add `force_thread` or equivalent. Nested replies extend the generic
thread detector. See `docs/phase0-findings.md` before reopening either decision.

## Reader boundaries

`reader/` splits by what a test can reach. Logic that does not need an `egui::Context` stays
outside `reader/ui/` and is tested. `reader/image_decode` parses untrusted binary input and must
not move under `ui/`. `reader/face` chooses family names; `reader/ui/fonts` registers bytes.

The reading column contains page content only. Page remarks are infobars, error pages, or
toasts. Positional markers such as `[image]`, `Block::Embed`, and `notice::PENDING` are the
exceptions.

Reader-facing wording stays outside `ui/` so the default job tests it: page remarks in
`reader/notice.rs`, menu and help rows in `reader/menu.rs`, setting labels and notes in
`reader/prefs.rs`. Use only glyphs the embedded faces carry and check `fonts/`, not the desktop
font. `ⓘ`, `⏵`, `▸`, `☰`, and `⚙` are absent; they are hand-painted or substituted.

Every setting is reachable from the panel. A field added to `Settings` or `ReadOpts` must also be
added to `prefs::fields()` and to both exhaustive matches in `prefs::get` and `prefs::set`;
`every_setting_is_exposed` walks the serialized form, so a struct field claimed by no `Field`
fails the build. Values handed to egui go through their accessor rather than the raw field, and
`Settings::zoom_factor` must span at least egui's own zoom range, or opening the panel rezooms
the window untouched.

A serialized enum that can gain a variant needs an unknown-value fallback (`theme_or_system`,
`images_or_default`): `#[serde(default)]` covers a missing field, not an unparseable one, and
without it an older binary loses every other setting in the file on the next write. Ask policy
enums a named question (`ImagePolicy::offers_loading`, `allows_any_request`), never equality
against one variant, which silently treats a new variant as the most permissive answer.

`reader::measure` skips off-band blocks. Anything that walks widgets, including find, Tab,
focus, selection, and AccessKit, must retain the guards in `ReaderApp::lays_out_whole_page`.
Height caches invalidate when layout-affecting state changes. Use `allocate_space`, not
`add_space`, for skipped blocks.

A navigation keeps the outgoing page visible until commit. `navigate` creates a request;
`ReaderApp::commit` creates the arriving page and owns document resets such as textures,
collapsed threads, find position, and scroll offset. Code describing the visible page goes
through `ReaderApp::shown`.

Only `reader/ui/theme.rs` may contain `Color32`. Form owns layout and type; `Palette` owns
colours and `dark_mode`. Chrome uses `theme::chrome_font`; page text uses the page family.
Do not weaken the contrast tests.

## Privacy and network

Privacy is compile-time absence. `reqwest` keeps `default-features = false`; do not add a
dependency or feature that restores a cookie jar. Preserve manual redirect inspection,
HTTPS-to-HTTP downgrade refusal, byte and redirect caps, and timeouts in `src/fetch.rs`.

Keep `FetchError::LikelyBlocked` distinct from generic transport errors. Only a document
read-timeout is the measured bot-block signal; a subresource timeout is `FetchError::Timeout`.

Article images are a third-party capability behind `gui`. They load only after a control names
the host and the user acts, are counted in page info, and never touch disk. Never auto-load,
prefetch, or load on hover. The favicon is the one automatic exception and is chrome identity.

Keep `referer(false)` on the client. Document requests carry no Referer. Image requests alone
may set an origin-only Referer built from `Url::origin()`, never a path. The setting remains
disableable and the disclosure remains reported.

## Site rules

`src/sites.rs` is the only file in the crate that contains a hostname. Rewrites match the entry
URL; profiles match the final URL and are data from `src/profile.rs`. Extractors do not know
domains. `session::load` owns the pipeline and reports a rewrite before dispatch.

A builtin rewrite ships only when all four conditions hold:

1. **Same operator.** Never route through a third-party frontend.
2. **Cited.** Name a measured failure in `docs/phase0-findings.md`.
3. **Offline.** Compile it in; never fetch or auto-update the table.
4. **Reported.** Report it before the request so failure cannot hide the destination.

Match hosts at a label boundary. Leave non-web schemes and credential-bearing URLs untouched.
Apply a rule once, to the entry URL only. Keep profile tables sorted and update
`docs/sites-checked.md` after triage. A stale profile must fail visibly through its floor/report,
not silently override extraction. A dead rewrite reports on stderr; there is no automatic retry.

## Corpus safety

Never commit, unignore, quote, or paste corpus output. `corpus.jsonl`, `urls.jsonl`,
`fetched.jsonl`, `signals.jsonl`, `extract.json`, `cache/`, and `extracted/` are real browsing
history. Do not name hosts learned from them in code, tests, docs, or commit messages.

Corpus sampling is by recency, never `visit_count`.

## Testing

Regression tests use synthetic `#[cfg(test)]` fixtures with no network and no corpus. Test
everything decidable without an `egui::Context`; compile UI dispatch and texture upload, and use
named `hww-shot` scenes for visual behavior. A pure function under `ui/` may be tested when its
failure is otherwise invisible, but make that argument at the test rather than citing precedent.

Do not relax tests that enforce IR purity, hostname boundaries, profile soundness, origin-only
Referer, parser word boundaries, shared inline flattening, font registration, glyph coverage,
contrast, settings exposure, menu and help agreement, iterative thread traversal, or layout-band
boundaries.

## Portability and style

Only Linux is verified. Use `Modifiers::COMMAND`, never a literal Ctrl; it maps to Cmd on macOS
and Ctrl elsewhere. Back/forward support both Alt+arrows and Cmd+brackets.

Rust edition 2024 and let-chains are in use. Extend module `//!` comments when rationale changes
instead of adding it here. Clippy runs with `-D warnings`; land code clean.
