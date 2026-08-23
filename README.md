# hww: a quiet-web client

[![CI](https://github.com/tayler/hww/actions/workflows/ci.yml/badge.svg)](https://github.com/tayler/hww/actions/workflows/ci.yml)

A reading client for the non-app web: stripped HTML, feeds, gemini, gopher, and local Markdown.
A browser without JavaScript, CSS, ads, third-party requests, or cookies.

It is a *second* browser, not a Chrome replacement. Single-page apps render blank, and that is
expected for now.

## Why

A quiet reading experience.

## Status

Working end to end for HTML, as a CLI that prints text and as a keyboard-driven reading GUI:

```
cargo run -- [--why] [--no-rewrite] [--no-profile] <url>       # text to stdout
cargo run --features gui --bin hww-gui -- [--no-rewrite] <url> # the reader
```

| Piece | State |
|---|---|
| `fetch` | Privacy rules enforced in code; no cookie jar exists at compile time |
| `sites` | Per-site rules: a rewrite table and a profile table, builtin only, every application reported |
| `html` | Parse, article extraction, IR mapping |
| `thread` | Forum/discussion extraction (structural, not heuristic) |
| `cards` | Story-card detection: a front page or section page as a run of entries, in place |
| `session` | The pipeline in one place; the rewrite notice reported before dispatch |
| `render` | Plain text |
| `ir` | Document / Block / Inline / Thread / Comment / Entries / Entry |
| `reader` | Reading model: inline runs, outline, history, threads, image decoding, notices |
| GUI | egui reader: links, back/forward, outline, find, threads, images. **Linux verified**; macOS and Windows are believed to build and are not tested |
| feeds, gemini, gopher, markdown | not started |
| TUI, archive | not started |

## Running

A stable Rust toolchain new enough for edition 2024 (1.85 or later) is the only prerequisite.
Every native library in the graph is opened at runtime rather than linked at build time, so
there is no system package to install first.

```
cargo run --features gui --bin hww-gui                          # the reader, URL bar focused
cargo run --features gui --bin hww-gui -- example.com/article   # straight to a page
cargo run -- https://example.com/article                        # the same page as text
```

### Diagnosing a site

Two tools say why a page came out the way it did, one for extraction and one for rendering:

```
cargo run -- --why https://example.com/article                  # the extractor's account
cargo run --features gui --bin hww-shot -- --url https://example.com/article --out /tmp/look
```

`--why` fetches the page through the same pipeline as a normal load and prints, instead of the
article, what `html::extract` did with it: every content-root candidate it weighed (origin,
tag, id, class, raw text, link density, and the emitted text the choice is actually made on)
with the winner marked; what the thread detector saw (the largest sibling groups, how many
members carried an author or timestamp, and why each was rejected or chosen, and whether the
thread replaced or joined the document); where each metadata field came from; whether the
`<body>` fallback fired. It is built by the same pass that builds the document, so it cannot
describe a decision the extractor did not take. The second command photographs the reader on
the page, so the rendering can be judged rather than guessed at; `--keys "space space"` scrolls
first, `--theme dark` checks the other palette, and `--out` keeps an ad-hoc look out of the
regression baseline.

The reader's URL argument is optional; without one it opens empty with the URL bar focused,
`g` reopens that bar later, and `?` lists every key. It also accepts a bare host and fills in
`https://`. The `hww` CLI wants a full URL, and prints the article to stdout with its
provenance on stderr.

Both binaries take `--no-rewrite` and `--no-profile` to switch off a per-site table, and
`--show-rewrites` / `--show-profiles` to print one and exit. The reader lives behind the `gui` feature because it adds one network
capability the CLI does not have, loading an image subresource (article pictures on click, and
the page favicon); in the `hww` binary
that code is not compiled at all.

For everyday use, build once and run the binary:

```
cargo build --release --features gui
./target/release/hww-gui example.com/article
```

## Measured

Against 150 real URLs sampled from browsing history ([full findings](docs/phase0-findings.md)):

- **72%** of document-shaped pages are readable with zero JavaScript
- **59%** of pages extract at or above trafilatura's output, 21% below
- On a threaded discussion page: **19,180 chars extracted vs trafilatura's 341**;
  article extraction cannot represent a thread, so threads get their own algorithm
- On the one JS-gated discussion site in that sample: **0 chars, then 8,331** once a per-site
  rule reaches the operator's own server-rendered domain

## Site rules

Some sites serve a fully server-rendered alternate of themselves. Phase 0 measured one such
site among its JS-only losses, which is why per-site rules are a feature rather than a hack.
`cargo run -- --show-rewrites` prints the table; `--no-rewrite` turns it off.

The table is compiled in and deliberately tiny. A rule ships only if it is the **same
operator** as the host it replaces, **cites** a measured failure in the findings doc, stays
**offline** (never fetched, never auto-updated), and is **reported** on stderr before the
request goes out; hww never silently contacts a host you did not name, and a fetch that fails
or hangs cannot swallow the notice. Third-party frontend proxies are expressible but
will never be builtin: they move your reading to an unrelated operator, which is the thing
this client exists not to do.

The second table is **profiles**: what a host's pages need beyond the generic extractor, as
data. Today that is one CSS selector per host, `strip`, for in-body chrome the extractor cannot
tell from prose (a "listen to this article" banner, a "see all topics" link, a lead gallery's
duplicate captions), measured on the top-ten US news sites after every generic fix had landed.
A profile is applied to the bytes that arrived, before anything is scored; it is refused when
it would remove most of the page; and every field reports what it did, on stderr after the
page, in `--why`, and in the page-info panel, so a profile that has gone stale says so. Each
shipped profile carries a fixture that the tests run, and `docs/sites-checked.md` lists every
host the extractor has been measured against, with the outcome, so the same host is not
triaged twice. `--show-profiles` prints the table,
`--no-profile` turns it off, and `Shift+R` in the reader reloads bare: no rewrite, no profile.

`src/sites.rs` is the only file in the crate that contains a hostname, and the module graph
keeps it that way: it imports no extractor, and nothing imports it but `src/session.rs`, the
one function that owns the pipeline and reports the rewrite before it dispatches.

## Layout

    src/ir.rs      the IR (no styling)
    src/fetch.rs   HTTP with privacy as code
    src/profile.rs a site profile as data: selectors, outcomes, a report; no hostname
    src/sites.rs   the two per-site tables: rewrites (entry URL) and profiles (final URL)
    src/session.rs the pipeline: rewrite -> fetch -> decode -> extract, plus provenance
    src/html.rs    HTML -> IR (article path)
    src/thread.rs  HTML -> IR (discussion path)
    src/cards.rs   story-card detection; the walker emits `Entries` in place
    src/render.rs  IR -> text
    src/reader/    IR -> reading model (pure, tested) + reader/ui (egui, feature-gated)
    src/bin/hww-gui thin main: argv -> reader::ui::run
    src/bin/corpus build a test corpus from local browser history. Checks how many of the sites you browse will work on hww.
    src/bin/bench  calibration harness vs a reference extractor (needs a local cache)
    src/bin/dbg    the `--why` account over one cached corpus page

## The reader

    cargo run --features gui --bin hww-gui -- <url>

Keyboard first, but nothing is keyboard-only: `?` lists the keys, and the status strip along
the bottom carries back/forward, a clickable URL, and a circled `i` so a mouse alone can
navigate. That strip never hides: the URL bar, outline, find bar, and help all dismiss on
`Esc`, but a rewrite notice has to be on screen *before* the request goes out, so the strip
stays.

What the reader has to say about a page divides on whether it can wait. Anything that would be
mis-read otherwise is a notice: an infobar docked under the top chrome, where no page can be,
with a severity icon, one sentence naming who did what, and a close, the shape every browser
uses for it. A 4xx body shown anyway, an extraction that came back thin, a body that arrived
truncated, a rewrite rule applied or landed on the wrong host are all bars; a failed navigation
is an error page; and none of them is ever dressed as prose.

The rest is accounting, and it lives behind the circled `i` (or `p`): encoding, bytes
downloaded against characters extracted, the redirect chain, cookies discarded, images loaded
and their hosts. Facts worth being able to ask for, not worth a permanent ribbon of six-point
text beside the URL.

One centred reading column, measured in characters (`[` and `]`). Zoom is egui's own, so
`Ctrl`+`+`/`-`/`0`, `Ctrl`+wheel, and trackpad pinch behave the way a browser does. Themes are
light, sepia, dark, and follow-the-system (`d`).

**Images are placeholders that name their host** (`[image] load from cdn.example.net`), and
load only on an explicit click. They are never loaded automatically, on hover, or in advance —
except the page favicon beside the masthead eyebrow, which is fetched as soon as the document
names one. Each load is counted alongside cookie attempts in the page-info panel, and nothing is
written to disk. Image requests, and only image requests, carry an origin-only `Referer`;
documents carry none. See AGENTS.md for why that one header is worth its exception.

## Building a corpus

    cargo run --features tools --bin corpus -- --limit 150 --per-host 3 --out corpus.jsonl

Finds a Firefox profile, stages a copy (the DB is locked while the browser runs), and samples
document-shaped URLs. **Sampled by recency, never by `visit_count`**; frequency ranking
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
