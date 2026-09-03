# Phase 0: is the web readable without JS?

Measured 2026-08-21 against a real browser history: 4,843 distinct URLs, 6,731 visits
across four months, and 527 hosts. Sample: 150 URLs across 78 hosts, recency-ordered, max 3
per host.

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

Readable docs: median 9,668 visible chars and median link-density 0.18, spanning 39 hosts.

## Coverage: what browsing actually consists of

Of 4,843 distinct URLs (link/typed/bookmark visits only; no embeds, redirects, downloads):

| class | % |
|---|---:|
| SEARCH (engines + on-site search) | 42.6% |
| CANDIDATE (document-shaped) | 41.4% |
| MEDIA (video platforms) | 7.5% |
| ROOT (bare homepages) | 4.8% |
| APP (mail, chat, calendar) | 3.3% |

**Two numbers, honestly:** ~41% of browsing is document-shaped; ~72% of those read fine with no JS.

## Confirmed: ranking by visit count is the wrong sample

The highest visit counts belong to search engines, a video platform, webmail, and web apps,
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
output: a caution against treating any single one as ground truth. trafilatura won
consistently and became the porting reference, contradicting the plan's choice of Readability.js.

Hand-read 14 GOOD extractions across distinct hosts. Article-shaped pages are genuinely clean:
long-form journalism, newsletter posts, engineering blogs, documentation, and personal blogs all
start at the right place with no nav or footer bleed. Commerce and marketing pages extract
"successfully" but are sales copy, not reading material; they inflate the GOOD number without
being anything anyone wants to read.

## The forum problem is the headline finding

**8/8 threaded-discussion pages broken. 5.6% of visible content recovered (13,703 / 243,351 chars).**

| page type | visible | extracted | recovered |
|---|---:|---:|---:|
| table-layout discussion site (×3) | 201,524 | 10,519 | 5.2% |
| deal-aggregator forum (×3) | 41,827 | 3,184 | 7.6% |
| JS-gated discussion site (×2) | 0 | 0 | JS-gated |

The failure is worse than the numbers suggest. On the table-layout site trafilatura returns
**4,358 characters of pure comment metadata** (usernames, timestamps, reply links) and drops
every comment body. It scores as a *successful* extraction.

This is the "plausible-looking failure" the plan warned about. Trusting the automated buckets
would have meant reporting 88% GOOD while a heavily-used site was 100% broken.

**Consequence:** thread extraction is a first-class feature, not a follow-up, and it needs a
different algorithm: repeated-sibling-structure detection with per-post author, depth, and body
records, not best-single-subtree scoring. The IR needs `Thread`/`Comment` block types.

## Verdict

**Build it.** 72% of documents readable with no JS; 88% of those extract cleanly. But the
product is only as good as its discussion story, and that was 0%.

Priority order: article extraction (works) → **thread extraction (did not)** → per-site rewrite
rules → feeds → gemini/gopher.

---

# Phase 1: Rust extractor

Benchmarked offline against the same 76-page byte cache, using trafilatura's per-URL output
as the baseline.

| | first cut | + div scoring | + threads | final |
|---|---:|---:|---:|---:|
| ≥75% of trafilatura | 43% | 50% | 62%* | **59%** |
| <45% (regression) | 33% | 29% | 13%* | **21%** |

\* The 62% was inflated. The thread detector was misfiring on article pages, capturing sibling
`<p>` tags as posts by "anon", raising the character count while destroying the document.
Requiring that a majority of posts carry an author or timestamp fixed it and *lowered* the
score. **A metric that improves when a bug is introduced is a warning about the metric.**

## Four bugs worth recording

**1. `comment` as a chrome hint.** Readability strips `class="comment"` to clean up articles.
On a discussion page that *is* the content, it is why both trafilatura and the
first cut returned pure usernames and timestamps. Comment-stripping belongs to the article path
only, never to the parser.

**2. Best-single-subtree is structurally wrong for threads.** Instrumenting content selection
showed it choosing one `div.comment` (814 chars out of 88 posts). There is no single subtree
that is "the content" in a discussion; it is N sibling subtrees. No tuning fixes this, which is
why thread extraction is a separate algorithm.

**3. Semantic containers cannot be trusted over scoring.** `<article>` selected an author-bio
card while the real body sat in a `<section>`, which additionally contained a complete nested
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
authenticated app page, and two marketing pages. Mostly app-web and sales copy, but the policy
page at 20 chars vs 1,191 is a genuine article failure and remains unexplained.

One deal-aggregator forum is still not detected; its sibling group fails the attribution test.
One discussion site remains JS-gated and needs the per-site rewrite layer to reach its
server-rendered alternate domain.

---

# Phase 2: per-site rules

The layer Phase 0 called for: a host swap that reaches a JS-gated site's own server-rendered
alternate domain. Measured on one comment page of that site:

| | requested host | alternate host |
|---|---:|---:|
| bytes | 8,434 | 246,619 |
| extracted | **0** | **8,331** |

That is the whole feature. It converts on the order of two pages of 87. That is small, and the
only mechanism that can reach them at all.

## The hint that measured worse than no hint

The plan had a second half: per-site extraction hints, starting with `force_thread` on
comment pages. A comment page *is* a discussion, so forcing the thread path looked obviously
right, and it was justified as belt-and-braces: the merge in `html::extract` would pick the
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
discussion, just a much shorter one.

Two consequences:

1. **The hint machinery was removed, not just switched off.** With its one candidate rule
   measured worse and nothing else in the corpus asking for one, keeping the plumbing would
   have been mechanism with no caller, the same standard that had already cut two other hints
   during planning. This layer rewrites URLs and does nothing else. `html.rs` and `thread.rs`
   are untouched by it.
2. **The forum problem is not solved by a rewrite.** Reaching a server-rendered domain gets the
   bytes; the 13-of-61 figure says thread extraction still loses nested replies. That is a
   `thread.rs` problem and it remains open.

## What shipped

Host matching at a label boundary (`evil-example.com` is not `example.com`), with rewriting
confined to the entry URL, never to redirect hops. Non-web schemes pass through untouched
because gemini and gopher are next. A URL carrying credentials is never rewritten: moving
userinfo to another host would leak it.

There is no config file, and no fallback when a rewrite fails. Instead, a request that does not
end on the host it was rewritten to is reported as a dead rule on stderr: the early warning
that stands in for a retry until one is measured to be needed. The test is leaving the
alternate, not returning to the exact host typed: the realistic bounce is to a sibling of it.
The swap itself is reported before the request goes out, so a fetch that fails or hangs cannot
swallow it.

The rewrite table is the only place in the crate that contains a hostname, and the module graph
keeps it that way: it imports no extractor, and no extractor imports it. (It has since moved to
`src/sites.rs`, where Phase 3's profile table joined it under the same host matcher.)

## The one rule, removed

Re-checked 2026-09-03, on a subreddit comment page of the same site:

| | requested host | alternate host |
|---|---:|---:|
| response | 200, JavaScript shell | 302 to the alternate's own `/login/?reason=lor2&dest=…` |
| bytes | 8,421 | 352,462 |
| extracted | **0** | **0** |

The alternate host now answers every page hww asks for with its login form. A generic
User-Agent gets a 403 "blocked due to a network policy" from both hosts instead. Nothing hww
can fetch on either side reads, so the rule traded one blank page for another and announced
the swap on stderr while doing it. It is gone, and `RULES` ships empty: `--show-rewrites` says
so. The layer stays. Its charter is the argument for the next rule, `--no-rewrite` and the
bare reload still mean something, and the mechanism tests run against a fixture rather than
the shipped table, so none of it goes untested while the table is empty.

One gap the re-check exposed, recorded and not changed: the dead-rule report fires when a
response *leaves* the alternate host, and a bounce to a login page on that same host passes
it. A rule that dies this way is silent until somebody reads the blank page. With no rule
shipping, a same-host detector would be mechanism with no caller, the standard this phase
already applied once.


---

# Phase 3: from okay to good, measured on the top-ten US news sites

Measured 2026-08-22 on the top ten US news sites by monthly visits (a public ranking, not a
browsing history), three pages each: the front page, one section page, one article. Eight
hosts were reachable; two answer bot gates (a JS "please enable" 403, an h2 reset) and are
out of scope. The instrument was `hww --why`, built first: the extractor's own account of each
page (root candidates with the emitted length the choice is made on, the thread detector's
verdict, the card detector's verdict, metadata sources, the fallback), produced by the one
pass that builds the document so it cannot disagree with it, paired with `hww-shot --url` for
the rendering.

## What the triage found

Article bodies were already good. Of 24 defects recorded, nine were generic extractor bugs,
five were the shape of a front page, a few were rendering, and five were in-body chrome on
articles that only a selector could name. Two bug classes dominated:

| | before | after |
|---|---:|---:|
| articles with a fake "discussion" appended (captions, carousels, related cards) | 6 of 8 | 0 |
| fronts replaced outright by a 3–4 post "thread" | 2 of 8 | 0 |
| a wire service's front page, chars emitted | 2,371 of 16,673 | 16,442 |
| a network's front page, chars emitted | 945 | 11,798 |
| a portal's front page, chars emitted | 1,233 | 13,799 |

The first row was one substring: `age` inside `image` attributed every caption container as a
post. The third was another: `promo` inside the class a wire service gives every story card.
Hint matching is at token boundaries now (`src/hint.rs`), a thread group whose members are
mostly links is a card list, and a chrome hint that would delete more than half a root is
ignored for that root and says so in `--why`.

## Front pages are a shape

No front page was good under article scoring (a legal notice out-scored the page on two of
them) and none is a discussion. The card detector (`src/cards.rs`) reuses the thread detector's
sibling groups with a different acceptance test (a headline-length link, few distinct hrefs,
median text over 40), and the walker emits `Block::Entries` in place, so section headings stay
between card lists. Entries per front page went from 0 everywhere to between 13 and 106; a
card stream kept outside `<main>` brings the body in. Measured first, as a verdict in `--why`,
before the IR block existed: the detector found the real card groups on every page and the only
false positives were short nav lists.

## Where profiles were needed

After three rounds of generic fixes and the front-page stage, what remained that only a
selector could name: a "listen to this article" banner and an app-download link mid-article
(one network), a "see all topics" link under the hero (another), and a lead gallery's overlay
repeating every caption above the byline (a wire service). Three public hosts, one to five
junk paragraphs each, on articles otherwise clean. That opened the gate the plan had set (three
hosts), and it opened onto a vocabulary of one field: `strip`. Nothing asked for a root, a
metadata source, or a thread shape, and the previous per-site layer was removed for being
mechanism without a caller, so the profile has the field with callers and grows when a page
asks. A portal's "Return to Homepage" on a close button turned out generic (screen-reader-only
labels are chrome) and left the pile.

## A second, wider sample

Fifty-seven more hosts across the political spectrum and across kinds of site (national and
international news, magazines, business, tech, science, reference), forty-four articles read
head and tail. Bodies were good throughout. What repeated was generic and landed as such: link
rubble after the last paragraph (tag lists, "See all", "Most viewed", a "Comments" heading) on
ten hosts is trimmed; recirculation and audio-player vendors share class names across hundreds
of hosts and are chrome hints; a dated "related stories" block under a chrome container is not
a thread. A native-ad slot's placeholder copy at the foot of two hosts of one publisher started
as a profile and ended as the tail trimmer's job; one more host needed a selector (a
text-to-speech speed menu set as prose). Later samples (ten non-US sites; twenty across Europe,
South America, Mexico, and Africa) added eight more: newsletter lines, comments pitches,
subscriber modals, a gift counter, a consent banner on a page walked with chrome hints off. The
table spans the spectrum and four continents rather than naming a side of either, and is kept
sorted by host.

## What a profile is

Data, not code: one CSS selector per field, in `src/sites.rs`, the only file in the crate that
contains a hostname, keyed on the final URL at a label boundary with the same matcher as the
rewrite table. Applied before anything is scored, refused when it would remove more than half
the page, and reported field by field (matched *n*, matched nothing, rejected) on stderr after
the page, in `--why`, and in the page-info panel. Each shipped profile carries a synthetic
fixture and `every_profile_earns_its_keep` runs it with and without the profile. `--no-profile`
and a bare reload switch it off.

---

# Phase 4: search

Search was 42.6% of the sampled corpus, the largest class and larger than the CANDIDATE
documents the extractor was built for, and none of it was reachable: the URL bar refused
anything containing a space, so a session could only begin with a URL found in another browser.
Eight engines were measured on 2026-08-24 against the shipped fetcher.

**The address a user would type cannot be read.** `duckduckgo.com/?q=` returns 24 KB with
**zero result links in the HTML**; the results arrive by XHR. Every engine that ships a modern
SERP behaves the same way, so scraping "the search page" is not a thing that exists. What can be
read are the no-JavaScript endpoints some engines still serve.

| Engine | Result | Read as |
|---|---|---|
| Mojeek | 200, stable across repeats | `.serp-results` frame, `ul.results-standard > li` |
| Marginalia | 200 | `#results` frame, `section.search-result` |
| DuckDuckGo | 200, then 202 | `#links` frame, `div.result`, links wrapped in `?uddg=` |
| Brave | 200, 292 KB | `#search-page` frame, `div.snippet[data-type="web"]` |
| Google | interstitial hiding every element (`table,div,span,p{display:none}`) | not expressible |
| Startpage | proof-of-work challenge | not expressible |
| searx.be | "Verifying your browser…" antibot | not expressible |
| Bing | JS-heavy; `?format=rss` returns clean XML | ToS limits it to personal RSS use; not shipped |

**Refusal is a 2xx, not a timeout.** DuckDuckGo answers four rapid queries and then returns
**HTTP 202 with a 123-character CAPTCHA** ("select all squares containing a duck"), reproduced at
run 5 of 6 on two separate occasions. Marginalia's rate limiter answers **HTTP 200** with a
one-second countdown page. Neither is a read-timeout, so `FetchError::LikelyBlocked` — which is
a document read-timeout and nothing else — cannot see either, and must not be widened to try:
the distinction between a host that hangs and a host that answers is the one Phase 0 paid for.

**A frame selector is what separates "found nothing" from "did not search".** Both produce zero
parsed results, and guessing between them blames the wrong party in one direction or the other.
Every engine renders its own result frame whether or not it found anything — Mojeek's
`.serp-results` is present on a nonsense query and absent on the interstitial — so the frame
answers the question that result counts cannot. Four outcomes follow: results; frame present and
holding less text than `THIN_TEXT` (the engine answered, and found nothing — "no results for …"
is a sentence); no frame and either a 2xx that is not 200 or a body under `THIN_TEXT` (the
engine declined); anything else falls back to the generic extractor, including a frame that is
*full* of text no shape could name, which is an engine that kept its container and renamed the
rows inside it.

**Thresholds were not tuned to fit a page.** Marginalia's wait page extracts to 214 characters
against a `THIN_TEXT` of 200, so it lands in the fallback and the reader sees the engine's own
countdown text and its manual link. Moving the floor to capture it would be fitting the metric
to one case, which is the failure this document already records once.

**Brave's class names carry a build hash** (`svelte-1qq4ge7`) that rotates on every deploy, and
41 distinct ones appear on a single result page. Its results do carry stable `data-pos` and
`data-type` attributes, and `no_shape_depends_on_a_build_hash` fails the build on a selector that
names a hash.

**No API key.** Bing's Web Search API was decommissioned 2025-08-11 and Brave's free tier ended
for new signups in February 2026, so a keyed backend means a paid signup before search works at
all. It would also be the first secret this project has ever stored, in a plaintext
`settings.json`, for a feature that works without one.

**Result links are unwrapped.** DuckDuckGo wraps every result in `//duckduckgo.com/l/?uddg=…`.
Pulling the destination out is a parsing requirement and a privacy improvement in the same move:
the click reaches the site rather than the engine's click tracker. Only `http` and `https`
destinations are accepted, through the same `classify_link` gate a clicked link uses.

**Disclosure, not "same operator".** The rewrite charter's first condition cannot apply to a
search: the user named words, not a host, so there is no operator to stay within. Condition four
does the whole job instead. `Rewrite::Searched` names the host **and the terms** before the
request is dispatched, because "hww went somewhere you did not type" is only half the disclosure
when what it carried was your query.

# Phase 5: feeds

`README.md` listed feeds as not started, but a feed fetched today was never refused —
`session::is_web_page` already passed `application/rss+xml`, `application/atom+xml`, and
`application/xml` — it was handed to `html::extract`, which reads XML as HTML soup. That is the
confident wrong diagnosis the content-type gate exists to prevent, one level deeper: the page
came out near-empty and the reader was told it might need JavaScript. Measured 2026-08-28 over
six feeds chosen for format and payload-encoding coverage, then a wider sample of 27 for the
parse-rate question. Nothing scoped to a host, so `docs/sites-checked.md` needs nothing.

| feed | format | items | payload chars | payload encoding |
|---|---|---|---|---|
| Daring Fireball | Atom | 48 | 121,466 | CDATA, `xml:base` |
| Hacker News | RSS | 30 | 2,040 | CDATA |
| LWN | RSS | 15 | 14,050 | entity-escaped |
| Rust Blog | Atom | 10 | 119,761 | entity-escaped, `xml:base` |
| WordPress News | RSS | 10 | 271,965 | CDATA, `content:encoded` |
| xkcd | Atom | 4 | 1,567 | entity-escaped |

**html5ever cannot read feeds.** It parses `<![CDATA[…]]>` as a bogus comment, which runs to the
first `>` inside the payload and swallows the prose into it. On WordPress News' first item
`scraper` returns **0 characters** for `<description>` where `roxmltree` returns **3,251**, and
**2,451** for the `content:encoded` beside it — 800 characters gone with nothing said. How much
is lost depends on where that first `>` happens to fall, so the failure is silent and
data-dependent: the entry simply comes out short. Over the whole of that feed, `html::extract`
recovers **13,735** characters and `feed::read` **63,480** from the same bytes.

Two further divergences, both reproduced on a four-element fixture. html5ever treats `<link>` as
void, so an RSS item's URL is not the link element's text (`""`) but a loose sibling text node.
And an unknown self-closing element nests its siblings underneath itself: after
`<media:thumbnail/>`, the `<description>` that followed it parses as its child.

**roxmltree reads real feeds: 26 of 28**, across WordPress, Ghost, Hugo, Jekyll, Blogger and
hand-rolled generators, both formats, 1 to 71 items. It recovers CDATA and entity-escaped
payloads alike and resolves `content:encoded` and `xml:base` by namespace. One package added to
the lockfile (`memchr` was already there), no `unsafe`, no RustSec advisories, an explicit
no-panic policy.

**Neither miss is the parser's.** One feed is larger than the 5 MB transfer cap, so what arrives
is a document cut mid-element; it opened `<?xml` rather than naming a format, so it is
`NotAFeed` and falls through, and the reader gets the truncation caution, which is the accurate
thing to say about it. The other is served as `application/octet-stream`, so
`session::is_web_page` refuses it before any of this runs. Widening that gate to admit
`octet-stream` would admit every PDF and archive with it, which is the failure the gate exists
for; the reader is told which type was sent and that hww has no reader for it, and that is true.

**Its strictness is real and is not a practical risk.** Constructed defects that kill the whole
document: `&nbsp;` (`unknown entity reference`), an undeclared namespace prefix, a raw `&`, a
control character. None occurred in the 27-feed sample, because generators emit well-formed XML —
every other feed reader would break too. The answer is disclosure when it happens
(`feed::Outcome::Unreadable`, which becomes a page remark carrying the parser's message and
position), not leniency. A document that only ever opened `<?xml` and then failed is
`NotAFeed` instead: plenty of XML is not a feed, and blaming a publisher for a document hww was
not asked to read is the same wrong confidence in the other direction.

**`allow_dtd` stays at its default of false.** Both measured consequences are wanted: a
billion-laughs entity bomb is refused at the parse rather than expanded, and an HTML page is
refused for its `<!DOCTYPE html>` before any element is inspected. Turning it on re-opens entity
expansion on untrusted input.

**The mapping needed no IR change.** `ir::Block::Entries` says so at the variant itself — *"A
feed maps onto the same block"* — and both renderers already draw it. Each payload through
`Html::parse_fragment` and the existing `html::blocks_from_public` produces real IR: Daring
Fireball's first entry comes out `Paragraph, Quote, Paragraph, Paragraph`.

**The chrome hints have to be off inside a summary.** They exist to drop nav, promo and footer
subtrees from a whole page. A feed summary is already the content, so a publisher who wraps a
post body in a `share`- or `promo`-classed element has it deleted with nothing on screen saying
why — the `PagePromo` failure this document already records, one level in.
`html::blocks_from_public_unhinted` is the entry point that does not apply them.

**1,000 bytes is the entry budget.** It leaves Hacker News, xkcd and LWN entries whole and turns
the two full-text feeds into lead-ins. Without one the WordPress feed is a single
272,971-character block and one of its items is 127,328 on its own. The cut is a **floor** and
overshoots by one block, so it always falls on a block boundary; it lives in `reader::budget`
rather than in `feed.rs`, because a lossy extractor is not recoverable and the plain renderer has
no width pressure. It is bytes and is named bytes: `ir::inlines_text_len` sums `s.len()`, so a
budget written as characters would cut a CJK or Cyrillic feed at roughly a third of its visible
length.

**xkcd is why the disclosure field exists.** Its summaries are a single `<img>` each, so every
entry is one `Block::Figure`, and `ir::block_text_len` counts a figure as its caption alone. The
whole document scores under `ir::THIN_TEXT`, so `reader::notice::about_page` fired *"hww found
little text on this page, which may need JavaScript to show its content"* and
`reader::pageinfo::rows` repeated it on the `Article text` row. Both are false about a feed that
parsed perfectly. The IR must not change to fix this — text length has one currency — so
`Provenance::feed` carries the fact that the page was read as a feed, and one field serves the
caution, the page-info row, and `--why`.

**An XML declaration names an encoding that no charset prescan can see.** `fetch::decode`
searched a 4 KiB window for the literal `charset=`; an XML declaration writes `encoding="…"`, so
a legacy-encoded feed with no HTTP charset decoded as UTF-8 and reached the parser as replacement
characters. It has to be fixed at decode: `roxmltree` takes a `&str`, so by then the decision is
already made, and it ignores the declaration. The scan is the declaration itself and not a
window, because `encoding=` is an ordinary word that appears in prose and in a feed's own payload.

**RFC 822 dates are layout damage, not cosmetics.** A feed writes
`Thu, 27 Aug 2026 17:17:57 +0000`, about 31 characters. `blocks::entries_ui` measures that stamp
with `layout_no_wrap` and subtracts its width from the headline's, floored at half the available
width, so every feed headline in a card was clamped to that floor. `reader::title::short_date`
now reformats it to `2026-08-27`, which changes that function's contract from truncating to
reformatting and gives dates one shape everywhere in the reader. `render.rs` is left verbatim:
the text renderer has no width pressure and no reason to restate what the page said.

# Phase 6: the hot paths

Two paths were read end to end and measured: extraction, which runs once per navigation over
untrusted DOM, and the draw loop, which runs while the reader scrolls. Everything below holds
what hww extracts, draws, remembers, and requests identical except where it says otherwise.

Measured 2026-08-29 on the crate's own fixtures, since there is no benchmark harness and no
corpus on disk. Two figures, both `cargo test --release --lib -- --test-threads=1`, which is the
build that ships:

| suite | before | after |
|---|---|---|
| `html::` — extraction over every fixture | 0.32 s | 0.27 s |
| `reader::archive` — filling all three caps | 2.03 s | 0.09 s |

The archive figure is the headline and it is not about tests. `Archive::same_page` parsed the
stored string with `Url::parse` per item, and the three tenants share one vector: a navigation
scanned every visited row and then every bookmark, so a reader at the caps paid up to seven
thousand URL parses on the frame a page installed, and `Ctrl+D` paid three times over. Every
string this build stores came out of `Url::to_string`, so the text before the first `#` is
already the normalized fragment-stripped form and a byte comparison answers the same question.
It answers differently for exactly one input — a row hand-written into `archive.json`, which
`settings::archive_path` documents as the way to carry a 0.3 `library.json` across the rename —
and that trade is argued at the function and pinned by
`same_page_is_the_parse_it_replaced`.

**Root selection was not deterministic.** `html::score_best` reads its scores out of a `HashMap`
and `max_by` keeps the last maximum, so a page whose containers score equally chose its root by
the process's hash seed. The 6,000-deep chain in `deeply_nested_text_extracts_in_linear_time`
extracted 53k, 107k, and 168k characters on three consecutive runs of one binary — the same page,
the same build, three different documents, and a reader reloading such a page could be handed a
different article each time. A linear chain of wrappers is exactly the shape that ties, because
each one holds the same prose. The tie now goes to the earlier node, which in an `ego_tree` index
is the outer container: that is the one `choose_root` prefers on the emitted-text ranking that
follows, so the two tie-breaks agree rather than fight.

**A debug build measures this backwards.** `nested_story_cards_do_not_walk_off_the_stack` runs
40% *slower* after the change under `cargo test` and 11% faster under `cargo test --release`.
`thread::text_len` stopped materializing a subtree's text to take its length and counts
non-whitespace bytes and words instead (`html::WsSummary`, which already did that arithmetic for
the scoring pass); the counting walk is iterator-shaped where the old one was a `push_str`, and
an unoptimized build does not inline it. Read release numbers for anything in this phase.

**The list views laid out every row, every frame.** `blocks::entries_ui` had no viewport check,
and `hww:history` holds up to `MAX_VISITS` rows — so the block-level window rule in `ready_screen`
could never help it, because the whole page is one `Block::Entries`. The row-level rule is
`thread_ui`'s, and getting it right needed two corrections that are worth recording because
both are invisible in a screenshot of a short list: a row's remembered height is **how far the
cursor moved, not how tall its rect was** (a `Ui`'s rect excludes the trailing space under its
last paragraph), and `allocate_space` adds the layout's item spacing back, so the gap comes off
before the height is stored. Each error is a couple of points per row and neither shows on a page
that fits the window; on sixty rows they are a page that ends ten rows early, with `Shift+G`
landing above the last story.

**A long list is measured wrong on the frame it arrives.** The chrome font's metrics are not
ready on the first frame a page is drawn, so `entry_ui` measures a narrower timestamp, gives the
headline more width, and the row comes out a line shorter than it will ever be again — and the
first frame is the only one on which every row is laid out. Rows near the window correct
themselves; rows the reader has not scrolled to would keep that height for as long as the page is
open. So a drawn row that measures differently from what was remembered throws away the whole
run's table rather than just its own entry: the rows it cannot see are wrong for the same reason
it was, and being drawn is the only way they can be re-measured.

## Recorded, not changed

- **`thread::link_density` and `html::TextMetrics::link_density` compute the same quantity and
  disagree.** Different text semantics (`thread::text_of` skips only `script|style|svg` and
  inserts no block spacing; `WsSummary` reproduces `inner_text`, which skips the whole `NOISE`
  list and spaces every block element), and `thread`'s early `1.0` for an element that *is* an
  anchor, which `select("a")` cannot see. That case is pinned by
  `a_list_of_bylined_cards_is_not_a_thread` and `cards::a_card_that_is_itself_the_anchor_counts`.
  Unifying them would move `MAX_LINK_DENSITY`, which Phase 3 measured. A question for the next
  triage, not for a change that claims to move nothing.
- **`thread::depth_of`'s indent search walks the whole subtree**, which under `with_nested`
  includes nested posts, so a nested reply's `indent` can answer for its parent. `body_of` and
  `find_hint_skipping` use `descendants_skipping` for exactly this reason. Fixing it needs a
  fixture that nests *and* carries `indent` — no existing one does both — and a decision about
  which answer is right.
- **Candidate roots are scored under a dummy base** (`root_candidates` uses `https://x.invalid/`)
  while the winner is walked under the real one, and the walk is base-dependent through
  `all_same_document_links`. Passing the real base in would let the winner's blocks be reused
  instead of rebuilt, saving a full walk, and would also move `emitted` and therefore candidate
  ranking on pages with absolute self-links. There is no version of this that is free.
- **`cards::detect`'s `"fewer than 3 members"` rejection is unreachable.** `sibling_groups`
  already filters `g.len() >= MIN_SIBLINGS`, and both constants are 3, so that string can never
  appear in a `--why` report. Either the branch is dead or the two constants are meant to be able
  to differ and silently cannot.
- **`settings_round_trip_and_survive_a_corrupt_file` is flaky.** It `set_var`s `HWW_CONFIG_DIR`,
  which is process-global, while the rest of the suite runs in parallel threads; its `SAFETY`
  comment argues single-threadedness *within the test*, which is not the condition that matters.
  Observed failing once in about a dozen runs, on a tree that does not touch settings.

# Phase 7: a tweet is a shape

Measured 2026-09-02 against one status permalink on x.com, requested with hww's own client.

## The page was never the problem

| | |
|---|---:|
| status | 200 `text/html` |
| bytes | 192,483 |
| redirects | 0 |
| cookie attempts discarded | 1 |
| extracted | **139** |

hww's UA and Accept headers get byte-identical bytes to a plain `curl`, and the response
server-renders the whole tweet: the display name, the at-name, the words, the picture with a full
`srcset`, the date, the view count, four engagement counts, and three replies. No JavaScript is
required to read any of it, and **no rewrite is warranted** — the operator serves the content at
the address the reader typed. The four-condition charter in `src/sites.rs` is never reached, and
`sites.rs` was not touched.

## What broke, and why it is the worst kind of break

```
root candidates: none cleared the floor
thread: no thread
cards: 1 group(s) look like story cards; 0 chars outside chrome
body fallback: fired (body would emit 139)
result: 1 blocks, 139 chars
```

139 is under `ir::THIN_TEXT`, so `reader::notice::about_page` told the reader the page "may need
JavaScript to show its content". Not a blank screen — a confident wrong diagnosis, about a page
hww had complete in memory. That is the failure this document keeps returning to, and it is the
reason the fix had to be a shape rather than a loosened floor.

Each detector lost for its own structural reason, and none of them was wrong to:

| detector | why it lost |
|---|---|
| content root | `score = raw_text × (1 − link_density)`; a tweet is short **and** nearly all links, so the product is small twice over and clears no floor |
| thread | needs three same-signature siblings over `MIN_MEDIAN_TEXT` and under `MAX_LINK_DENSITY`; a permalink shows one tweet, and its text is mostly inside anchors |
| cards | needs a link of `MIN_HEADLINE` length; a tweet's longest link is a handle |

## The shape, and whose it is

An article is one subtree, a discussion is N sibling subtrees, a front page is N sibling cards.
A tweet is a fourth shape: one short, link-dense container carrying an attributed message and a
row of counts. `src/tweet.rs` accepts a container with a name over it, a date under it, and **two
or more counted actions** beside it. The counts are what separate a tweet from a comment; the name
and the date are what separate it from a card.

The module is named `tweet` and the block `Block::Tweets`, for X and not for social posts in
general, because X is the only social host the shape was built from or has been measured
against. Checked 2026-09-03 with `--why`: mastodon.social, bsky.app, and threads.com each answer
a client without JavaScript with a shell whose body emits 0 characters, so no tweet, no article,
and no thread can be read from any of them, and nothing on those networks has ever exercised
this code. The fixtures reproduce X's server-rendered markup — `aria-label="Reply"` beside
`data-icon="icon-reply-stroke"`, `dir="auto"` on the message, `/status/<id>` on the date, an
`<img alt="@name">` inside the profile link — and the six `ir::StatKind`s are X's engagement
row and no other network's. Where the code names another network's word (`boost`, `renote`,
`favourite`), it is on paper only. A Mastodon at-name of the `@user@host` form fails
`is_handle`, a Reddit post carries no at-name at all, and a Facebook post counts reactions and
shares, which no kind names. The first draft called this a social-post detector; the name was
narrowed to what it fits, and it widens when a second social host earns a ledger row.

Nothing in the detector is a class name and nothing is a host. The hooks are `<article>`,
`dir`, `aria-label`, `data-icon`, `<img alt>`, `<time>`, and the arithmetic of anchors — the parts
of the markup a redesign does not rotate. The first draft let the name and the date fall back to
the class-name hints `thread` keeps; that fallback could decide acceptance on a class alone and
was removed, so a forum entry attributed only by `.author` stays the thread detector's. Where a page stops carrying them the failure is graceful and
total: no tweet, and the page reads exactly as it did before.

## A candidate with a floor, not an override

The tweet run takes the document **only where the article and thread paths came out under
`ir::THIN_TEXT`**. It rescues a page nothing else could read; it never competes with one that
reads. A forum whose posts each carry a like count stays a thread, because the thread carried
text. A news article with a share row stays an article, for the same reason, and
`an_article_that_reads_is_never_replaced_by_a_tweet` pins it.

The `<body>` fallback then has to be skipped on a page that installed tweets. Without that guard
the correct answer is overwritten by the wrong one on every tweet page, because a tweet is under
the fallback's floor having parsed perfectly.

## Two floors that had to learn about shortness

- **`flush_pending`'s `MIN_LOOSE_TEXT`** dropped every run under 40 characters that carried no
  link. The measured tweet is nine words. `html::blocks_from_tweet` lifts the floor for a tweet
  body, on the argument `blocks_from_public_unhinted` already makes for a feed summary: the
  container is known to be the content, so a guess about *page* text does not apply to it.
- **`notice::about_page`'s thin-text caution** would still fire on a tweet that parsed perfectly.
  Exempted through `Provenance::tweets`, exactly as a picture-only feed is exempted through
  `Provenance::feed`. **The IR did not change**: text length has one currency, and inflating a
  tweet's would hide a real extraction failure on every other page in the corpus.

## Recorded, not changed

- **Counts are the page's own strings.** The markup carries `2.6M` and `108.2M`. The exact
  integers (`favorite_count:2616339`, `view_count:"108174045"`) exist only inside a `<script>` as
  a serialized JavaScript — not JSON — object store. `script` is in `html::NOISE`, and this crate
  parses no JavaScript. `ir::Stat::count` is therefore a `String`; expanding `2.6M` back to
  2,600,000 would print a number nobody published.
- **`og:description` carries the tweet text verbatim**, and hww reads it nowhere, before this
  change or after. Reading it would hand every article a duplicate of its own opening, and the
  tweet path does not need it. This is unchanged from Phase 0's finding that the cheap structured
  paths do not pay.
- **A localised count suffix fails `is_count`** and the stat is dropped rather than mislabelled.
- **A profile timeline on x.com reads as an article, not as tweets.** Checked 2026-09-03: the
  page server-renders three tweets, the detector weighs 47 containers and keeps 3, and the run
  is dropped as `the page already read` because the article path found 288 characters in the
  first `<article>`. That is the rescue floor doing what it says — a page that reads is never
  redrawn — and the cost is a timeline drawn as one article of three bylines. Recorded; the
  floor is not tuned for it.
- **The avatar is content, not identity.** A face on a fetched page is a third-party picture and
  answers `ImagePolicy` like any other; it is not the site-mark exception, which covers marks hww
  draws on its own surfaces. Under the default policy a tweet shows a name, a handle, and an empty
  square until the reader asks.
