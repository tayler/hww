# CLAUDE.md

Guidance for Claude Code and other agents working in this repository.

`README.md` carries the status table, module layout, and Phase 0/2 measurements; this file
carries what it does not — the CI gates, the extraction order, the invariants, and the
decisions measurement has already closed. Reference symbols (`html::extract`), not line
numbers: the files move.

## What this is

A reading client for the non-app web: stripped HTML, feeds, gemini, gopher, local Markdown.
No JavaScript, no CSS cascade, no ads, no third-party requests, no cookies. It is a *second*
browser, not a Chrome replacement — single-page apps render blank, and that is expected. The
HTML path works end to end; feeds, gemini, gopher, Markdown, TUI, archive, and GUI are not
started.

## Commands

    cargo run -- [--no-rewrite] [--show-rewrites] <url>   # fetch, extract, render one page
    cargo test                                            # synthetic fixtures; no network

CI gates (`.github/workflows/ci.yml`) — all three must pass:

    cargo fmt --all -- --check
    cargo clippy --all-targets --locked -- -D warnings    # warnings are errors
    cargo test --locked

Local-only tools. These need gitignored inputs and are **not** run by CI:

    cargo run --bin corpus -- --limit 150 --per-host 3 --out corpus.jsonl
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

Text length is the shared scoring currency: `Document::text_len`, `ir::blocks_text_len`, and
`thread::comments_text_len`. Reuse these rather than adding a fourth counter. The same rule now
covers *text itself*: `ir::plain_text` is the one presentation-neutral flatten, and
`reader::inline::flatten` is the one styled one, with `render::inline_text` built on top of it.
Four walkers disagreeing about where an emphasis boundary falls is a bug nobody finds.

**Extraction order in `html::extract`** — easy to break, and the merge is load-bearing:

1. Score for a content root, map it to blocks.
2. Run `thread::extract_thread`. The thread *replaces* the document if it is longer; it is
   *appended* if it exceeds 200 chars. The append branch is what recovers comment trees the
   thread detector under-counts — see Closed decisions.
3. Fall back to `<body>` only if the result is still under 200 chars. This bleeds navigation,
   but a thin page beats a blank one.

## Invariants

**No styling in the IR.** No color, font, width, alignment, or spacing fields in `src/ir.rs`,
ever. Wanting one means the answer is a renderer setting — see `render::TextOpts`. Same for
`Block::Table`: simple grid only, no colspan/rowspan, because layout tables are precisely what
this client refuses to reproduce.

**Privacy is compile-time absence, not policy.** `reqwest` is built with
`default-features = false`, so no cookie jar exists — the capability is absent, not merely
unused. Alongside it in `src/fetch.rs`: `referer(false)`, manual redirect handling so every
hop is inspected, https→http downgrade refused, 5 MB body cap, 5 redirect cap, 15 s timeout.
Cookie attempts are counted for reporting and never stored. Do not add a dependency or feature
flag that restores any of these capabilities.

**A read-timeout is a bot-block signal, not a network fault.** Phase 0 measured bot detection
failing as a silent hang rather than a 403. Keep `FetchError::LikelyBlocked` distinct from
generic transport errors.

**`src/rewrite.rs` is the only file in the crate that contains a hostname.** `html.rs` scores
subtrees, `thread.rs` groups siblings by class signature, `bin/corpus.rs` classifies by URL
shape — none of them know a domain name. Nothing imports `rewrite` except `src/session.rs`, and
`rewrite` imports no extractor. `session::load` owns the whole pipeline, so the notice is a value
it hands back before it dispatches rather than an `eprintln!` a second driver could forget — the
rule is structural, not a convention. A builtin rule ships only if all four hold:

1. **Same operator.** The target host is run by the same entity as the source. A third-party
   frontend proxy relocates the reader onto an unrelated party — exactly the "no third-party
   requests" line this client exists to hold. Never builtin.
2. **Cited.** The rule names a measured failure in `docs/phase0-findings.md`. No speculative
   entries, no curated directory of the web.
3. **Offline.** Compiled in, never fetched, never auto-updated — a remotely updated rule list
   is a channel that reports what you read.
4. **Reported.** Every applied rule prints on stderr *before* the request goes out, so a fetch
   that fails or hangs cannot swallow it. hww never silently contacts a host the user did not
   name.

Mechanically: host matching at a label boundary (`evil-example.com` is not `example.com`),
the entry URL only and never redirect hops, non-web schemes and credential-carrying URLs
passed through untouched, first match wins and applies exactly once.

**Never commit corpus output.** `corpus.jsonl`, `urls.jsonl`, `fetched.jsonl`, `signals.jsonl`,
`extract.json`, `cache/`, and `extracted/` are somebody's real browsing history. They are
gitignored and must stay that way — do not un-ignore them, and do not paste their contents
into code, tests, docs, or commit messages, or name hosts drawn from them. The one builtin
rewrite rule is the deliberate, charter-gated exception: a rule cited in the findings doc, not
a host lifted from a sample.

**Sample by recency, never `visit_count`.** Frequency ranking surfaces search engines, webmail,
and dashboards — the pages a reader never needs — while the articles under test are read once
and sit at `visit_count = 1`.

**`Cargo.lock` is committed.** This crate ships binaries.

## Closed decisions

Measured already. Do not re-propose without new measurement.

**Per-site extraction hints (`Hints`, `extract_with`, `force_thread`) were built and removed.**
Forcing the thread path on a comment page looks obviously right — a comment page *is* a
discussion — and it measured 2,254 chars against 8,331 unhinted on identical bytes. The thread
detector finds 13 of 61 comments, because replies nest in child containers rather than sitting
as siblings; the article path picks up the whole comment tree, and the unhinted merge then
appends the detected thread to it. Forcing the thread throws the larger result away, and the
failure is invisible from outside: the page still renders as a clean, correctly attributed
discussion, just a much shorter one. The machinery was deleted rather than switched off —
mechanism with no caller. The per-site layer rewrites URLs and does nothing else.
`docs/phase0-findings.md`, Phase 2.

**Nested replies in `thread.rs` are an open problem.** That 13-of-61 is not fixed by a URL
rewrite: a rewrite gets the bytes, extraction still loses the tree.

**No config file, and no fallback when a rewrite fails.** A request that does not end on the
host it was rewritten to is reported as a dead rule on stderr — the early warning that stands
in for a retry until one is measured to be needed.

## Testing

Regression cover is synthetic `#[cfg(test)]` fixtures inside each module (`src/html.rs`,
`src/fetch.rs`, `src/rewrite.rs`) — no network and no corpus, so CI can run them. Tests worth
keeping named:

- `ir_carries_no_styling` — the executable form of the no-styling invariant. A change that
  makes this test awkward is a signal, not a test to relax.
- `comment_class_is_not_treated_as_chrome` — comment bodies stripped as chrome.
- `lookalike_host_does_not_match`, `rule_host_as_a_prefix_does_not_match` — the label-boundary
  safety property, which is the whole safety story of the rewrite layer.
- `builtin_table_is_sound` — guards the shipped rule table against silent rot: normalized
  hosts, no scheme downgrade, idempotence, no unreachable rules.

`rewrite`'s mechanism tests run against a `FIXTURE` table rather than the shipped `RULES`, so
they keep testing the mechanism no matter what does or does not ship.

`bench` is offline calibration against a *local* reference-extractor baseline that is
deliberately absent from this repo. It is not a test and CI does not run it.

## Style

Edition 2024; let-chains (`if let ... && let ...`) are in use. Module-level `//!` doc comments
carry the *why* and are the primary record — extend them rather than duplicating rationale
here. Clippy runs with `-D warnings`, so land code clean.
