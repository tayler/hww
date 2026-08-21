# Phase 0 findings — is the web readable without JS?

Measured 2026-08-21 against a real browser history: 4,843 distinct URLs, 6,731 visits across
four months, 527 hosts. Sample: 150 URLs across 78 hosts, recency-ordered, max 3 per host.

Sites are described by structure rather than name throughout. The identity of the corpus was
what made the go/no-go decision trustworthy; it is not what makes the findings reusable, and it
does not belong in a permanent public record.

## Headline

**72% of document-shaped pages are readable with zero JavaScript** (63/87). Gate was ≥70%.

| Bucket | n | % |
|---|---:|---:|
| READABLE (>1000 visible chars) | 63 | 72.4% |
| EMPTY (needs JS) | 10 | 11.5% |
| NON-200 (404/400) | 6 | 6.9% |
| BLOCKED (bot detection) | 5 | 5.7% |
| THIN (200–1000 chars) | 3 | 3.4% |

Readable docs: median 9,668 visible chars, median link-density 0.18, spanning 39 hosts.

## Coverage: what browsing actually consists of

Of 4,843 distinct URLs (link/typed/bookmark visits only — no embeds, redirects, downloads):

| class | % |
|---|---:|
| SEARCH (engines + on-site search) | 42.6% |
| CANDIDATE (document-shaped) | 41.4% |
| MEDIA (video platforms) | 7.5% |
| ROOT (bare homepages) | 4.8% |
| APP (mail, chat, calendar) | 3.3% |

**Two numbers, honestly:** ~41% of browsing is document-shaped; ~72% of those read fine with no JS.

## Confirmed: ranking by visit count is the wrong sample

The highest visit counts belong to search engines, a video platform, webmail, and web apps —
none of them documents. Articles are read once and sit at `visit_count = 1`. Ranking that way
produces a sample containing almost nothing readable. Sample by recency from the *visits*
table, not by frequency from the *places* table. This is the single most important
methodological note here.

## Correction: the "cheap structured paths" do not exist

The plan assumed structured data would materially raise the hit rate. Measured availability:

| Signal | availability |
|---|---:|
| JSON-LD `articleBody` | **0.0%** |
| `rel=amphtml` | **0.0%** |
| `rel=alternate` RSS/Atom | 13.9% |
| `og:type` present | 39.3% (only 13 are `article`) |

AMP is dead, and `articleBody` is a news-publisher convention this corpus barely touched.
**Content scoring is the primary path, not the fallback.** `og:type` survives as a weak
document-class hint; `rel=alternate` is worth keeping for the feed layer.

## Encoding: a non-finding

100% UTF-8. Zero legacy encodings. The prescan work is correct but was not urgent; this
corpus skews modern. It still matters for the older web the client is aimed at.

## The real JS-only losses

Ten pages, and not all of them are reading material: two threaded-discussion pages, two pages
of a JS-rendered documentation site, three pages of one unusual personal site, two WebGL demos,
and one browser-based app. The demos and the app are applications and correctly out of scope.

So roughly **seven genuine reading losses out of 87**. One of the discussion sites serves a
fully server-rendered alternate domain, which is the canonical argument that **per-site rewrite
rules are a first-class feature, not a hack**.

## Bot detection is real but narrow

Five 403s plus eleven read-timeouts, concentrated in four commerce and ticketing hosts. Note
the failure mode: **a silent read-timeout, not a 403**. The client must treat a hang as a block
signal rather than a network fault.

## Privacy, observed

**56% of responses (84/150) attempted to set a cookie.** All discarded. 26 responses
redirected. Zero `Referer` sent throughout.

## Extraction quality

trafilatura 2.2.0 and readability-lxml 0.8.4.1, run offline from the byte cache. Of 76 document-shaped HTML 200s:

| Bucket | n | % |
|---|---:|---:|
| GOOD | 58 | 76.3% |
| EMPTY (needs JS) | 10 | 13.2% |
| PARTIAL (thin vs page) | 7 | 9.2% |
| FAILED | 1 | 1.3% |

**88% GOOD of pages that had any text** (58/66). Median extraction 3,775 chars.

The two agreed on length (within 25%) on only **28/49 = 57%** of pages where both produced
output — a caution against treating any single one as ground truth. trafilatura won
consistently and became the porting reference, contradicting the plan's choice of Readability.js.

Hand-read 14 GOOD extractions across distinct hosts. Article-shaped pages are genuinely clean:
long-form journalism, newsletter posts, engineering blogs, documentation, and personal blogs all
start at the right place with no nav or footer bleed. Commerce and marketing pages extract
"successfully" but are sales copy, not reading material — they inflate the GOOD number without
being anything anyone wants to read.

## The forum problem is the headline finding

**8/8 threaded-discussion pages broken. 5.6% of visible content recovered (13,703 / 243,351 chars).**

| page type | visible | extracted | recovered |
|---|---:|---:|---:|
| table-layout discussion site (×3) | 201,524 | 10,519 | 5.2% |
| deal-aggregator forum (×3) | 41,827 | 3,184 | 7.6% |
| JS-gated discussion site (×2) | 0 | 0 | JS-gated |

The failure is worse than the numbers suggest. On the table-layout site trafilatura returns
**4,358 characters of pure comment metadata** — usernames, timestamps, reply links — and drops
every comment body. It scores as a *successful* extraction.

This is the "plausible-looking failure" the plan warned about. Trusting the automated buckets
would have meant reporting 88% GOOD while a heavily-used site was 100% broken.

**Consequence:** thread extraction is a first-class feature, not a follow-up, and it needs a
different algorithm — repeated-sibling-structure detection with per-post author, depth, and body
records — not best-single-subtree scoring. The IR needs `Thread`/`Comment` block types.

## Verdict

**Build it.** 72% of documents readable with no JS; 88% of those extract cleanly. But the
product is only as good as its discussion story, and that was 0%.

Priority order: article extraction (works) → **thread extraction (did not)** → per-site rewrite
rules → feeds → gemini/gopher.

---

# Phase 1 — Rust extractor

Benchmarked offline against the same 76-page byte cache, using trafilatura's per-URL output
as the baseline.

| | first cut | + div scoring | + threads | final |
|---|---:|---:|---:|---:|
| ≥75% of trafilatura | 43% | 50% | 62%* | **59%** |
| <45% (regression) | 33% | 29% | 13%* | **21%** |

\* The 62% was inflated. The thread detector was misfiring on article pages, capturing sibling
`<p>` tags as posts by "anon" — raising the character count while destroying the document.
Requiring that a majority of posts carry an author or timestamp fixed it and *lowered* the
score. **A metric that improves when a bug is introduced is a warning about the metric.**

## Four bugs worth recording

**1. `comment` as a chrome hint.** Readability strips `class="comment"` to clean up articles.
On a discussion page that *is* the content — it is why both trafilatura and the
first cut returned pure usernames and timestamps. Comment-stripping belongs to the article path
only, never to the parser.

**2. Best-single-subtree is structurally wrong for threads.** Instrumenting content selection
showed it choosing one `div.comment` — 814 chars out of 88 posts. There is no single subtree
that is "the content" in a discussion; it is N sibling subtrees. No tuning fixes this, which is
why thread extraction is a separate algorithm.

**3. Semantic containers cannot be trusted over scoring.** `<article>` selected an author-bio
card while the real body sat in a `<section>` — which additionally contained a complete nested
`<!DOCTYPE html><html><body>`. html5ever recovered from it per spec; the selector logic did not.
Candidates now compete rather than short-circuit. This is the clearest argument in the project
for not hand-rolling the HTML parser.

**4. Rank candidates by what the walker emits, not by raw text.** A container can be dense with
text yet yield almost nothing once chrome and empty wrappers are dropped. Ranking by raw text
cost one newsletter post 90% of its body to a higher-scoring ancestor.

## Where it beats trafilatura

| page type | trafilatura | hww |
|---|---:|---:|
| large threaded discussion | 341 | **113,010** |
| threaded discussion | 341 | **19,180** |
| personal blog article | 7,868 | **11,959** |
| engineering blog post | 5,145 | **6,092** |
| long-form product review | 21,092 | **22,241** |

## Where it still loses

A code-hosting file view (content lives in a `<textarea>`), a university policy page, an
authenticated app page, and two marketing pages. Mostly app-web and sales copy — but the policy
page at 20 chars vs 1,191 is a genuine article failure and remains unexplained.

One deal-aggregator forum is still not detected; its sibling group fails the attribution test.
One discussion site remains JS-gated and needs the per-site rewrite layer to reach its
server-rendered alternate domain.

---

# Phase 2 — per-site rules

The layer Phase 0 called for: a host swap that reaches a JS-gated site's own server-rendered
alternate domain. Measured on one comment page of that site:

| | requested host | alternate host |
|---|---:|---:|
| bytes | 8,434 | 246,619 |
| extracted | **0** | **8,331** |

That is the whole feature. It converts on the order of two pages of 87 — small, and the only
mechanism that can reach them at all.

## The hint that measured worse than no hint

The plan had a second half: per-site extraction hints, starting with `force_thread` on
comment pages. A comment page *is* a discussion, so forcing the thread path looked obviously
right, and it was justified as belt-and-braces — the merge in `html::extract` would pick the
thread anyway; the hint only removes the dependence on a length comparison.

Measured on the same page, on identical bytes:

| | extracted |
|---|---:|
| `force_thread` on | 2,254 |
| `force_thread` off | **8,331** |

The thread detector finds 13 of the page's 61 comments, because replies nest in child
containers rather than sitting as siblings. The article path picks up the whole comment tree,
and the unhinted merge then *appends* the detected thread to it. Forcing the thread throws the
larger result away.

**A hint that looked principled destroyed 73% of the content.** This is the same
"plausible-looking failure" recorded at the top of this document, one layer up: the failure was
invisible from the outside, because the page still rendered as a clean, correctly attributed
discussion — just a much shorter one.

Two consequences:

1. **The hint machinery was removed, not just switched off.** With its one candidate rule
   measured worse and nothing else in the corpus asking for one, keeping the plumbing would
   have been mechanism with no caller — the same standard that had already cut two other hints
   during planning. This layer rewrites URLs and does nothing else. `html.rs` and `thread.rs`
   are untouched by it.
2. **The forum problem is not solved by a rewrite.** Reaching a server-rendered domain gets the
   bytes; the 13-of-61 figure says thread extraction still loses nested replies. That is a
   `thread.rs` problem and it remains open.

## What shipped

Host matching at a label boundary — `evil-example.com` is not `example.com` — with rewriting
confined to the entry URL, never to redirect hops. Non-web schemes pass through untouched
because gemini and gopher are next. A URL carrying credentials is never rewritten: moving
userinfo to another host would leak it.

There is no config file, and no fallback when a rewrite fails. Instead, a request that does not
end on the host it was rewritten to is reported as a dead rule on stderr — the early warning
that stands in for a retry until one is measured to be needed. The test is leaving the
alternate, not returning to the exact host typed: the realistic bounce is to a sibling of it.
The swap itself is reported before the request goes out, so a fetch that fails or hangs cannot
swallow it.

`src/rewrite.rs` is the only file in the crate that contains a hostname. The module graph keeps
it that way: `rewrite` imports no extractor, and nothing imports `rewrite` except `main.rs`.
