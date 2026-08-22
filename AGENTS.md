# CLAUDE.md

Guidance for Claude Code and other agents working in this repository.

`README.md` carries the status table, module layout, and Phase 0/2 measurements; this file
carries what it does not: the CI gates, the extraction order, the invariants, and the
decisions measurement has already closed. Reference symbols (`html::extract`), not line
numbers: the files move.

## What this is

A reading client for the non-app web: stripped HTML, feeds, gemini, gopher, and local Markdown.
JavaScript, the CSS cascade, ads, third-party requests, and cookies are all absent. It is a
*second* browser, not a Chrome replacement; single-page apps render blank, and that is
expected. The HTML path works end to end, in a CLI that prints text and in a keyboard-driven
reading GUI; feeds, gemini, gopher, Markdown, TUI, and archive are not started.

The archive (saving a page and reading it back offline) is deliberately **not** started. It
is a second doctrine: reading history at rest, a directory to protect, an eviction policy,
and a schema to version. Nothing in the reader writes page content to disk, and the only
thing that survives a run is `settings.json`.

## Commands

    cargo run -- [--no-rewrite] [--show-rewrites] <url>   # fetch, extract, render one page
    cargo run --features gui --bin hww-gui -- <url>       # the reading GUI
    cargo test                                            # synthetic fixtures; no network

CI gates (`.github/workflows/ci.yml`). Two jobs run five commands, and all five must pass:

    cargo fmt --all -- --check
    cargo clippy --all-targets --locked -- -D warnings    # warnings are errors
    cargo test --locked
    cargo clippy --all-targets --locked --features gui,tools -- -D warnings
    cargo test --locked --features gui,tools

The first job never compiles egui; keep it that way, and keep it fast. The second uses
**explicit features, never `--all-features`**; that would silently opt into whatever anyone
adds later, the opposite of the auditability posture. No test may call `eframe::run_native`.
Neither job installs an apt package, and neither should need to: every native library in this
graph is opened at runtime rather than linked at build time.

Local-only tools. These need gitignored inputs and are **not** run by CI:

    cargo run --features tools --bin corpus -- --limit 150 --per-host 3 --out corpus.jsonl
    cargo run --bin dbg -- <signals.jsonl> <cache-dir> <url-substring> [--text]
    cargo run --bin bench -- <signals.jsonl> <baseline.json> <cache-dir>

## Architecture

Everything funnels through the IR:

    main.rs ───┐
               ├─> session.rs ─> rewrite.rs (entry URL only) ─> fetch.rs (bytes + decode)
    reader/ ───┘        │                                              │
                        └── Notice, before dispatch                    │
                              ┌────────────────────────────────────────┴─┐
                              │                                          │
                    html.rs (article path)              thread.rs (discussion path)
                              └──────────────> ir::Document <────────────┘
                                                    │
                              ┌─────────────────────┴──────────────────┐
                        render.rs (text)                       reader/ (GUI)

`src/ir.rs` is the contract. Every future format (gemtext, RSS, gopher, Markdown) maps *into*
it and every future renderer (TUI, GUI, speech, e-ink) consumes *only* it. Adding a format is
a new module producing `ir::Document`; adding a renderer is a new module consuming it. Neither
touches the other. That is what makes a multi-protocol reader one program instead of five.
`reader/` is the second renderer, and it cashed that promise in: it consumes `ir::Document`
and `session::Loaded` and touches no extractor.

`src/reader/` splits by *what a test can reach*, not by "gui vs non-gui files":

    reader/opts.rs         ReadOpts: the GUI's TextOpts. No egui types.
    reader/inline.rs       Inline tree -> flat Vec<Run>. The one styled flatten.
    reader/outline.rs      headings -> outline; level normalization; fragment slugs
    reader/history.rs      back/forward over a cursor
    reader/thread_tree.rs  flat Vec<Comment> + depth -> tree; CommentKey
    reader/title.rs        title / blocks[0] de-duplication
    reader/notice.rs       what the reader reports about a page, and how loud. No egui types
    reader/settings.rs     the config-path triple, atomic write, never fatal
    reader/image_decode.rs sniff, ceilings, partial handling, downscale. &[u8] -> pixels
    reader/ui/             everything that needs an egui::Context, and nothing else
    reader/ui/notice_ui.rs the one place the reader draws its own text: Notice -> a band

`image_decode` is the sharp case: it is the only module in the project that parses untrusted
binary input, and putting it under `ui/` would have made the highest-risk code the one code no
test ever runs.

Text length is the shared scoring currency: `Document::text_len`, `ir::blocks_text_len`, and
`thread::comments_text_len`. Reuse these rather than adding a fourth counter. `ir::THIN_TEXT`
is the one threshold on that currency, and it has consumers at both ends of the pipeline:
`html::extract` falls back to `<body>` below it, `reader::notice` warns about a page that came
out under it. Same event, two ends; a second copy of the number lets the reader stop warning
about exactly the pages the extractor gave up on. The same rule now
covers *text itself*: `ir::plain_text` is the one presentation-neutral flatten, and
`reader::inline::flatten` is the one styled one, with `render::inline_text` built on top of it.
Four walkers disagreeing about where an emphasis boundary falls is a bug nobody finds.

**Extraction order in `html::extract`.** Easy to break, and the merge is load-bearing:

1. Score for a content root, map it to blocks.
2. Run `thread::extract_thread`. The thread *replaces* the document if it is longer; it is
   *appended* if it exceeds 200 chars. The append branch is what recovers comment trees the
   thread detector under-counts (see Closed decisions).
3. Fall back to `<body>` only if the result is still under 200 chars. This bleeds navigation,
   but a thin page beats a blank one.

## Invariants

**No styling in the IR.** No color, font, width, alignment, or spacing fields in `src/ir.rs`,
ever. Wanting one means the answer is a renderer setting: `render::TextOpts` for the text
renderer, `reader::opts::ReadOpts` for the GUI. `reader/ui/theme.rs` is the only file in the
crate where a `Color32` appears, and that is a property worth keeping greppable. Same for
`Block::Table`: simple grid only, no colspan/rowspan, because layout tables are precisely what
this client refuses to reproduce. And no dimensions on `ir::Image`: the extractor does not
capture them, so the reader reserves layout from what it has actually decoded instead.

**The reading column contains page content and nothing else, and a notice shares no colour role
or layout idiom with the page.** This is the no-styling rule one layer out. Everything the
reader reports *about* a page is a band drawn by `reader::ui::notice_ui::band`, full-bleed across
the central panel, which is the one property an `ir::Block` cannot have: the column is a fixed
measure and a band is not. That is the load-bearing signal, because it survives a screenshot, a
monochrome eye, and a theme nobody has designed yet; `notice::MARK` is the second. Colour is
support, and `theme::notice_fill_clears_every_page_colour` holds the floor, because exact
inequality means nothing when two roles sit five units apart, which is what `chrome_bg` and
`code_bg` do. A band borrows no page idiom: not `heading_font`, not `body_font`, not the quote
rule, not the code frame. The three inline exceptions (`[image]`, `Block::Embed`, "(N replies
hidden)") are *positional*, marking one thing at one place, which a full-bleed band cannot do;
they carry the bracket convention instead. Wording lives in `src/reader/notice.rs`, outside
`ui/`, so the fast CI job tests it.

**Privacy is compile-time absence, not policy.** `reqwest` is built with
`default-features = false`, so no cookie jar exists: the capability is absent, not merely
unused. Alongside it in `src/fetch.rs`: `referer(false)`, manual redirect handling so every
hop is inspected, https→http downgrade refused, 5 MB body cap, 5 redirect cap, and 15 s
timeout. Cookie attempts are counted for reporting and never stored. Do not add a dependency
or feature flag that restores any of these capabilities.

**A read-timeout is a bot-block signal, not a network fault.** Phase 0 measured bot detection
failing as a silent hang rather than a 403. Keep `FetchError::LikelyBlocked` distinct from
generic transport errors, and keep it distinct from `FetchError::Timeout`, which is the same
event on a *subresource*, where the finding does not apply. 10 MB inside 15 s needs 5.3 Mbit/s
sustained; calling a slow CDN a bot-block is the same mistake pointing the other way.
`Limits::timeout_is_block` is where the two part company.

**Images are a third-party request, allowed on exactly one argument.** Article images live on
CDN hosts constantly, and "no third-party requests" is in the first paragraph of both README
and this file. The capability lives behind the `gui` feature, so the `hww` CLI retains
compile-time absence of it, and inside the reader it is allowed only because: the placeholder
**names the host before the click**, the click is a deliberate user action, the load is
counted in the status strip alongside cookie attempts, and nothing is ever auto-loaded,
prefetched, or loaded on hover. `I` (load all) still names the distinct hosts first. No image
touches disk.

**`referer(false)` remains.** `reqwest` never attaches a Referer on its own, on any request or
any redirect hop. Image subresource requests, and only those, set an explicit origin-only
`Referer` naming the page being read: scheme and host, never the path, built from
`Url::origin()` so no code path can emit a full URL even by mistake. **Document fetches carry
no Referer, ever.**

This buys hotlink checks, which the client cannot otherwise pass, and the reason is a failure
it cannot otherwise detect: hotlink protection that answers 403 degrades safely (no picture,
visible failure), while hotlink protection that answers **200 with a substitute image** is
indistinguishable from the real one. No status code, decode check, or heuristic available to
us can tell them apart, and a reader that shows a hostile picture and calls it the article's
photograph has failed worse than one that shows no picture. Sending the origin removes the
trigger; sending it only after a 403 cannot, because there is no 403 to react to. The origin
adds nearly nothing to what the request already discloses (the CDN inferred the referring
site from the path before we said a word), while the *path*, which is the real leak, is never
sent. Reported in the status strip, and disableable in the settings file, accepting the
breakage.

**`src/rewrite.rs` is the only file in the crate that contains a hostname.** `html.rs` scores
subtrees, `thread.rs` groups siblings by class signature, `bin/corpus.rs` classifies by URL
shape; none of them know a domain name. Nothing imports `rewrite` except `src/session.rs`, and
`rewrite` imports no extractor. `session::load` owns the whole pipeline, so the notice is a value
it hands back before it dispatches rather than an `eprintln!` a second driver could forget; the
rule is structural, not a convention. A builtin rule ships only if all four hold:

1. **Same operator.** The target host is run by the same entity as the source. A third-party
   frontend proxy relocates the reader onto an unrelated party, exactly the "no third-party
   requests" line this client exists to hold. Never builtin.
2. **Cited.** The rule names a measured failure in `docs/phase0-findings.md`. No speculative
   entries, no curated directory of the web.
3. **Offline.** Compiled in, never fetched, never auto-updated: a remotely updated rule list
   is a channel that reports what you read.
4. **Reported.** Every applied rule prints on stderr *before* the request goes out, so a fetch
   that fails or hangs cannot swallow it. hww never silently contacts a host the user did not
   name.

Mechanically: host matching at a label boundary (`evil-example.com` is not `example.com`),
the entry URL only and never redirect hops, non-web schemes and credential-carrying URLs
passed through untouched, first match wins and applies exactly once.

**Never commit corpus output.** `corpus.jsonl`, `urls.jsonl`, `fetched.jsonl`, `signals.jsonl`,
`extract.json`, `cache/`, and `extracted/` are somebody's real browsing history. They are
gitignored and must stay that way. Do not un-ignore them, and do not paste their contents
into code, tests, docs, or commit messages, or name hosts drawn from them. The one builtin
rewrite rule is the deliberate, charter-gated exception: a rule cited in the findings doc, not
a host lifted from a sample.

**Sample by recency, never `visit_count`.** Frequency ranking surfaces search engines, webmail,
and dashboards (the pages a reader never needs), while the articles under test are read once
and sit at `visit_count = 1`.

**`Cargo.lock` is committed.** This crate ships binaries.

## Closed decisions

Measured already. Do not re-propose without new measurement.

**Per-site extraction hints (`Hints`, `extract_with`, `force_thread`) were built and removed.**
Forcing the thread path on a comment page looks obviously right (a comment page *is* a
discussion), and it measured 2,254 chars against 8,331 unhinted on identical bytes. The thread
detector finds 13 of 61 comments, because replies nest in child containers rather than sitting
as siblings; the article path picks up the whole comment tree, and the unhinted merge then
appends the detected thread to it. Forcing the thread throws the larger result away, and the
failure is invisible from outside: the page still renders as a clean, correctly attributed
discussion, just a much shorter one. The machinery was deleted rather than switched off:
mechanism with no caller. The per-site layer rewrites URLs and does nothing else.
`docs/phase0-findings.md`, Phase 2.

**Nested replies in `thread.rs` are an open problem.** That 13-of-61 is not fixed by a URL
rewrite: a rewrite gets the bytes, extraction still loses the tree.

**No config file, and no fallback when a rewrite fails.** A request that does not end on the
host it was rewritten to is reported as a dead rule on stderr: the early warning that stands
in for a retry until one is measured to be needed.

## Testing

Regression cover is synthetic `#[cfg(test)]` fixtures inside each module (`src/html.rs`,
`src/fetch.rs`, `src/rewrite.rs`, and every module under `src/reader/` that is not `ui/`, plus
`ui/theme.rs`, whose `palette` takes no `egui::Context` and is therefore reachable): no
network and no corpus, so CI can run them. The reader's split exists for this: everything
decidable without an `egui::Context` is tested, and only chrome, widget dispatch, and texture
upload are merely compiled. Tests worth keeping named:

- `ir_carries_no_styling`: the executable form of the no-styling invariant. A change that
  makes this test awkward is a signal, not a test to relax.
- `comment_class_is_not_treated_as_chrome`: comment bodies stripped as chrome.
- `lookalike_host_does_not_match`, `rule_host_as_a_prefix_does_not_match`: the label-boundary
  safety property, which is the whole safety story of the rewrite layer.
- `builtin_table_is_sound` guards the shipped rule table against silent rot: normalized
  hosts, no scheme downgrade, idempotence, and no unreachable rules.
- `flatten_reproduces_the_old_markdown_exactly` (`src/render.rs`) holds a verbatim copy of
  the recursive `inline_text` that `reader::inline::flatten` replaced, and runs both over every
  inline shape the extractor produces. It is the entire safety net for building the text
  renderer on top of the GUI's flatten, and `the_only_divergences_are_the_documented_ones`
  pins the two normalizations that are deliberate.
- `referer_is_origin_only_and_never_carries_a_path` (`src/fetch.rs`): the executable form of
  the Referer amendment. If it fails, the client is telling a CDN which article is being read.
- `a_jpeg_truncated_at_half_still_renders_and_says_so` and
  `truncated_png_gif_and_webp_refuse_rather_than_half_decode` (`src/reader/image_decode.rs`):
  the partial-render matrix as a test rather than a paragraph.
- `whitespace_between_inline_elements_is_a_word_boundary` (`src/html.rs`): markup indentation
  puts a whitespace-only text node between adjacent inline elements constantly, and dropping it
  welded two words into one.
- `no_notice_contains_an_arrow` (`src/reader/notice.rs`): the executable form of the
  embedded-font gap. The prose rule below existed the whole time and two strings shipped with
  `→` anyway, one of them a heading, tofu from the day it was written. Prose does not fail a
  build.
- `notice_fill_clears_every_page_colour` (`src/reader/ui/theme.rs`): the executable form of the
  notice/page separation, and **the one test under `ui/`**. Argued rather than assumed:
  `theme::palette` takes no `egui::Context`, unlike `apply`, `measure_px`, and `system_is_dark`,
  so it is reachable by the directory's own "split by what a test can reach" rule.
- `a_pathologically_deep_thread_builds_and_traverses_completely`
  (`src/reader/thread_tree.rs`): a hostile thread is untrusted input, so the traversal is
  iterative.

`rewrite`'s mechanism tests run against a `FIXTURE` table rather than the shipped `RULES`, so
they keep testing the mechanism no matter what does or does not ship.

`bench` is offline calibration against a *local* reference-extractor baseline that is
deliberately absent from this repo. It is not a test and CI does not run it.

## Portability

Linux, macOS, and Windows from one source tree, but **only Linux is verified**, and the gap
between "portable by construction" and a claim worth making is recorded here so it stays a
deferral rather than becoming an assumption.

Portable by construction, and not by accident: `reqwest` uses `rustls`, so there is no OpenSSL
build to lose a day to on Windows; `rusqlite`'s bundled C SQLite is behind `tools`, so neither
the default nor the `gui` build touches a C toolchain; `default_fonts` embeds egui's own faces
rather than probing system fonts, so text renders identically everywhere; `accesskit` has real
backends on all three; `wgpu` is Metal, DX12, and Vulkan natively rather than one API stretched
across three platforms; and winit gates its Wayland dependencies behind
`cfg(all(unix, not(macos)))`, so the Linux-looking feature flags resolve to nothing elsewhere
rather than to an error.

**CI stays Linux-only, deliberately.** The obvious addition is a compile-only
`macos-latest`/`windows-latest` matrix running clippy with no test step. Deferred: it costs CI
minutes on every push to guard platforms nobody is running yet, and the honest sequence is to
make the Linux build good first. The cost is that a portability break lands silently and is
found late, by whoever first tries to build on a Mac; acceptable only while that person is
nobody. Revisit the moment anyone runs hww off Linux.

**`Modifiers::COMMAND`, never a literal Ctrl.** It resolves to Cmd on macOS and Ctrl
everywhere else. Back and forward are the one place the platforms genuinely disagree, so both
`Alt`+arrows and `Cmd`+`[`/`]` are bound rather than picking a winner; the mouse side buttons
(`PointerButton::Extra1`/`Extra2`) work identically everywhere.

## Known gaps

Real, and worth knowing before re-discovering them:

- **egui's embedded fonts have no arrows.** `→` renders as tofu, though `←`, `↑`, and `↓` do
  not, which is why the help table is asymmetric on purpose. Prime marks (`′ ″`) are missing
  too, so coordinates on some pages read as boxes. The fix is the same embedded-face work that
  `Strong` needs, and until then no chrome string may contain `→`. That rule is now enforced by
  `no_notice_contains_an_arrow` rather than remembered: it was stated here in prose while
  two strings shipped with `→` in them, one of them a failure heading.
- **`Strong` is colour, not weight.** `default_fonts` ship no bold face and egui synthesizes
  none. `theme::Palette::strong` is the whole implementation, and it is the one line that
  changes when a real face is embedded.
- **Nested replies in `thread.rs` are still under-counted** (see Closed decisions). The reader
  reconstructs whatever tree extraction hands it; it cannot recover replies extraction lost.
- **No justification, and no `justify` setting.** One `Label` per segment cannot justify. The
  setting is deliberately absent rather than present and inert: if justification is ever wanted
  badly enough it arrives together with a single-galley renderer, and the two are one decision.

## Style

Edition 2024; let-chains (`if let ... && let ...`) are in use. Module-level `//!` doc comments
carry the *why* and are the primary record; extend them rather than duplicating rationale
here. Clippy runs with `-D warnings`, so land code clean.
