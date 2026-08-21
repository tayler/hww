# hww — a quiet-web client

[![CI](https://github.com/tayler/hww/actions/workflows/ci.yml/badge.svg)](https://github.com/tayler/hww/actions/workflows/ci.yml)

A reading client for the non-app web: stripped HTML, feeds, gemini, gopher, local Markdown.
A browser without JavaScript, CSS, ads, third-party requests, or cookies.

It is a *second* browser, not a Chrome replacement. Single-page apps render blank, and that is
expected for now.

## Why

A quiet reading experience.

## Status

Phase 1. Working end to end for HTML:

```
cargo run -- [--no-rewrite] [--show-rewrites] <url>
```

| Piece | State |
|---|---|
| `fetch` | Privacy rules enforced in code; no cookie jar exists at compile time |
| `rewrite` | Per-site rules; builtin table only, every rewrite reported on stderr |
| `html` | Parse, article extraction, IR mapping |
| `thread` | Forum/discussion extraction (structural, not heuristic) |
| `render` | Plain text |
| `ir` | Document / Block / Inline / Thread / Comment |
| feeds, gemini, gopher, markdown | not started |
| TUI, archive, GUI | not started |

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
keeps it that way — it imports no extractor, and nothing imports it but `main.rs`.

## Layout

    src/ir.rs      the IR — no styling
    src/fetch.rs   HTTP with privacy as code
    src/rewrite.rs per-site rules (a JS-gated host -> its own server-rendered alternate)
    src/html.rs    HTML -> IR (article path)
    src/thread.rs  HTML -> IR (discussion path)
    src/render.rs  IR -> text
    src/bin/corpus build a test corpus from local browser history. Checks how many of the sites you browse will work on hww.
    src/bin/bench  calibration harness vs a reference extractor (needs a local cache)
    src/bin/dbg    inspect extraction of one cached page

## Building a corpus

    cargo run --bin corpus -- --limit 150 --per-host 3 --out corpus.jsonl

Finds a Firefox profile, stages a copy (the DB is locked while the browser runs), and samples
document-shaped URLs. **Sampled by recency, never by `visit_count`** — frequency ranking
surfaces search engines, webmail, and dashboards, while the articles under test are read once.

Its output is real browsing history. It is gitignored, and it should stay that way.

## Tests

    cargo test

Covers IR mapping, relative-URL resolution, malformed-markup recovery, nested-document
recovery, encoding prescan bounds, host matching at a label boundary (`evil-example.com` is
not `example.com`), and two regressions worth keeping named: comment bodies being stripped as
chrome, and styling leaking into the IR.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for
inclusion in this crate by you, as defined in the Apache-2.0 license, shall be dual licensed
as above, without any additional terms or conditions.
