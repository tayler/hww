# hww — a quiet-web client

[![CI](https://github.com/tayler/hww/actions/workflows/ci.yml/badge.svg)](https://github.com/tayler/hww/actions/workflows/ci.yml)

A reading client for the non-app web: stripped HTML, feeds, gemini, gopher, local Markdown.
A browser without JavaScript, CSS, ads, third-party requests, or cookies.

It is a *second* browser, not a Chrome replacement. Single-page apps render blank, and that is
expected for now.

## Why

A quiet reading experience.

## Status

Working end to end for HTML, as a CLI that prints text and as a keyboard-driven reading GUI:

```
cargo run -- [--no-rewrite] [--show-rewrites] <url>            # text to stdout
cargo run --features gui --bin hww-gui -- [--no-rewrite] <url> # the reader
```

| Piece | State |
|---|---|
| `fetch` | Privacy rules enforced in code; no cookie jar exists at compile time |
| `rewrite` | Per-site rules; builtin table only, every rewrite reported on stderr |
| `html` | Parse, article extraction, IR mapping |
| `thread` | Forum/discussion extraction (structural, not heuristic) |
| `session` | The pipeline in one place; the rewrite notice reported before dispatch |
| `render` | Plain text |
| `ir` | Document / Block / Inline / Thread / Comment |
| `reader` | Reading model: inline runs, outline, history, threads, image decoding |
| GUI | egui reader — links, back/forward, outline, find, threads, images. **Linux verified**; macOS and Windows are believed to build and are not tested |
| feeds, gemini, gopher, markdown | not started |
| TUI, archive | not started |

## Measured

Against 150 real URLs sampled from browsing history ([full findings](docs/phase0-findings.md)):

- **72%** of document-shaped pages are readable with zero JavaScript
- **59%** of pages extract at or above trafilatura's output, 21% below
- On a threaded discussion page: **19,180 chars extracted vs trafilatura's 341** —
  article extraction cannot represent a thread, so threads get their own algorithm
- On the one JS-gated discussion site in that sample: **0 chars, then 8,331** once a per-site
  rule reaches the operator's own server-rendered domain

## Rewrite rules

Some sites serve a fully server-rendered alternate of themselves. Phase 0 measured one such
site among its JS-only losses, which is why per-site rules are a feature rather than a hack.
`cargo run -- --show-rewrites` prints the table; `--no-rewrite` turns it off.

The table is compiled in and deliberately tiny. A rule ships only if it is the **same
operator** as the host it replaces, **cites** a measured failure in the findings doc, stays
**offline** (never fetched, never auto-updated), and is **reported** on stderr before the
request goes out — hww never silently contacts a host you did not name, and a fetch that fails
or hangs cannot swallow the notice. Third-party frontend proxies are expressible but
will never be builtin: they move your reading to an unrelated operator, which is the thing
this client exists not to do.

`src/rewrite.rs` is the only file in the crate that contains a hostname, and the module graph
keeps it that way — it imports no extractor, and nothing imports it but `src/session.rs`, the
one function that owns the pipeline and reports the rewrite before it dispatches.

## Layout

    src/ir.rs      the IR — no styling
    src/fetch.rs   HTTP with privacy as code
    src/rewrite.rs per-site rules (a JS-gated host -> its own server-rendered alternate)
    src/session.rs the pipeline: rewrite -> fetch -> decode -> extract, plus provenance
    src/html.rs    HTML -> IR (article path)
    src/thread.rs  HTML -> IR (discussion path)
    src/render.rs  IR -> text
    src/reader/    IR -> reading model (pure, tested) + reader/ui (egui, feature-gated)
    src/bin/hww-gui thin main: argv -> reader::ui::run
    src/bin/corpus build a test corpus from local browser history. Checks how many of the sites you browse will work on hww.
    src/bin/bench  calibration harness vs a reference extractor (needs a local cache)
    src/bin/dbg    inspect extraction of one cached page

## The reader

    cargo run --features gui --bin hww-gui -- <url>

Keyboard first, but nothing is keyboard-only: `?` lists the keys, and the status strip along
the bottom carries back/forward and a clickable URL so a mouse alone can navigate. That strip
never hides — the URL bar, outline, find bar, and help all dismiss on `Esc`, but a rewrite
notice has to be on screen *before* the request goes out, so the strip stays.

One centred reading column, measured in characters (`[` and `]`). Zoom is egui's own, so
`Ctrl`+`+`/`-`/`0`, `Ctrl`+wheel, and trackpad pinch behave the way a browser does. Themes are
light, sepia, dark, and follow-the-system (`d`).

**Images are placeholders that name their host** — `[image] load from cdn.example.net` — and
load only on an explicit click, never automatically, never on hover, never prefetched. Each
load is counted in the status strip alongside cookie attempts, and nothing is written to disk.
Image requests, and only image requests, carry an origin-only `Referer`; documents carry none.
See AGENTS.md for why that one header is worth its exception.

## Building a corpus

    cargo run --features tools --bin corpus -- --limit 150 --per-host 3 --out corpus.jsonl

Finds a Firefox profile, stages a copy (the DB is locked while the browser runs), and samples
document-shaped URLs. **Sampled by recency, never by `visit_count`** — frequency ranking
surfaces search engines, webmail, and dashboards, while the articles under test are read once.

Its output is real browsing history. It is gitignored, and it should stay that way.

## Tests

    cargo test                              # default features; egui never compiled
    cargo test --features gui,tools         # adds the reader's pure modules

Covers IR mapping, relative-URL resolution, malformed-markup recovery, nested-document
recovery, encoding prescan bounds, host matching at a label boundary (`evil-example.com` is
not `example.com`), inline flattening, thread reconstruction, outline and fragment matching,
history, title de-duplication, image decoding and its ceilings, and the origin-only `Referer`.

Regressions worth keeping named: comment bodies stripped as chrome; styling leaking into the
IR; a whitespace-only node between two inline elements welding two words together; and the
text renderer's markdown output, which is checked against a verbatim copy of the walker it
replaced.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for
inclusion in this crate by you, as defined in the Apache-2.0 license, shall be dual licensed
as above, without any additional terms or conditions.
