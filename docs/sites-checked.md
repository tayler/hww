# Sites checked

A ledger of the hosts the extractor has been measured against, so the work is not repeated:
before triaging a host, look here; after, add or update its row. These are public sites chosen
for the audit, not anyone's browsing history, which is the line `docs/findings.md` keeps.
Dates are when the pages were fetched. "Clean"
means the article read well with no host-specific work; generic fixes found on a host are in
`triage/sheet.md` (gitignored) and the findings doc, not here.

A row older than ninety days is due a re-check the next time its host comes up: the same
three pages (front, section, article) through `hww --why` and the text render, the profile's
fixture still matching the live markup, and the date moved. A host whose markup has changed
under its profile shows as `strip matched nothing` in `--why`, which is the other trigger.

| host | checked | looked at | outcome |
|---|---|---|---|
| abc.net.au | 2026-08-22 | front, section, article | "See more ABC coverage in your search results", left |
| abcnews.go.com | 2026-08-22 | front, article | clean |
| aljazeera.com | 2026-08-22 | front, article | clean |
| apnews.com | 2026-08-22 | front, section, article | profile: strip `.CarouselOverlay-carousel` on `/article/` |
| arstechnica.com | 2026-08-22 | front, article | related entries at the tail; fine |
| axios.com | 2026-08-22 | front, section, article | article 403 (Cloudflare) |
| bbc.com | 2026-08-22 | front, section, article | clean |
| bloomberg.com | 2026-08-22 | front, section, article | newsletter page read; paywall elsewhere |
| bostonglobe.com | 2026-08-22 | front, section | front read; no article picked |
| breitbart.com | 2026-08-22 | front | connection refused |
| bsky.app | 2026-09-03 | profile | JS shell, body emits 0 chars; nothing for `tweet.rs` to read |
| businessinsider.com | 2026-08-22 | front, article | clean |
| cbc.ca | 2026-08-22 | front, article | clean |
| cbsnews.com | 2026-08-22 | front, section, article | clean |
| chicagotribune.com | 2026-08-22 | front, article | Trinity Audio line, fixed generically (tts hint) |
| clarin.com | 2026-08-22 | front, article | clean |
| cnbc.com | 2026-08-22 | front, section, article | clean |
| cnn.com | 2026-08-22 | front, section, article | profile: strip follow-topics bar |
| corriere.it | 2026-08-22 | front, article | profile: strip newsletter link |
| dailymail.co.uk | 2026-08-22 | front, article | clean |
| dailymaverick.co.za | 2026-08-22 | front, article | profile: strip comments container |
| dailywire.com | 2026-08-22 | front, article | tip box, hashed classes, left |
| dw.com | 2026-08-22 | front, article | clean |
| economist.com | 2026-08-22 | front, article | clean |
| elmundo.es | 2026-08-22 | front, article | profile: strip comments panel |
| eltiempo.com | 2026-08-22 | front, section, article | "Exclusivo suscriptores" line, left |
| eluniversal.com.mx | 2026-08-22 | front, article | badge images at the tail, fixed generically |
| emol.com | 2026-08-22 | front, article | clean |
| en.wikipedia.org | 2026-08-22 | front, article | clean |
| engadget.com | 2026-08-22 | front, section, article | clean |
| english.elpais.com | 2026-08-22 | front, section, article | clean |
| espn.com | 2026-08-22 | front | 202 shell, nothing to read |
| folha.uol.com.br | 2026-08-22 | front, article | profile: strip subscriber modal, sign-in |
| forbes.com | 2026-08-22 | front, section, article | article 403 |
| foxnews.com | 2026-08-22 | front, section, article | profile: strip listen banner, onelink link |
| france24.com | 2026-08-22 | front | 403 |
| ft.com | 2026-08-22 | front | front read; no article picked |
| g1.globo.com | 2026-08-22 | front, article | clean |
| html.duckduckgo.com | 2026-08-24 | search | results read; 202 + CAPTCHA after ~4 rapid queries; default engine |
| huffpost.com | 2026-08-22 | front, section, article | Outbrain row, fixed generically |
| independent.co.uk | 2026-08-22 | front, article | profile: strip commenting-forum pitch |
| irishtimes.com | 2026-08-22 | front, section, article | clean |
| jornada.com.mx | 2026-08-22 | front, article | article 403 (Cloudflare) |
| kyivindependent.com | 2026-08-22 | front, section, article | clean |
| lanacion.com.ar | 2026-08-22 | front, section, article | date/time/read-time list under the headline, left |
| latimes.com | 2026-08-22 | front, section, article | clean |
| lefigaro.fr | 2026-08-22 | front, section, article | profile: strip consent banner (root walked with hints off) |
| lemonde.fr | 2026-08-22 | front, section, article | clean |
| lwn.net | 2026-08-22 | front, article | mailing-list page read; fine |
| mastodon.social | 2026-09-03 | profile | JS shell, body emits 0 chars; nothing for `tweet.rs` to read |
| medium.com | 2026-08-22 | front | front gave the picker no article; not read |
| mg.co.za | 2026-08-22 | front, article | clean |
| milenio.com | 2026-08-22 | front, section, article | clean |
| mojeek.com | 2026-08-24 | search | results read; no challenge across repeats |
| msnbc.com | 2026-08-22 | front, section, article | clean |
| nation.africa | 2026-08-22 | front, section, article | article 403 (Cloudflare) |
| nationalreview.com | 2026-08-22 | front | front gave the picker no article; not read |
| nature.com | 2026-08-22 | front, section | front gave the picker no article; not read |
| nbcnews.com | 2026-08-22 | front, article | "Listen with a free profile" line, hashed classes, left |
| news.yahoo.com | 2026-08-22 | front, section, article | clean |
| news24.com | 2026-08-22 | front, section, article | profile: strip gift counter, public-editor box |
| newsweek.com | 2026-08-22 | front, section, article | Trust Project line, hashed classes, left |
| newyorker.com | 2026-08-22 | front, article | clean |
| nhk.or.jp | 2026-08-22 | front | front is a JS shell |
| nos.nl | 2026-08-22 | front, article | clean |
| npr.org | 2026-08-22 | front, section, article | clean |
| nypost.com | 2026-08-22 | front, section, article | Google preferred-sources box, fixed generically |
| nytimes.com | 2026-08-22 | front, section, article | front reads; articles 403 (JS gate) |
| old-search.marginalia.nu | 2026-08-24 | search | results read; 200 countdown page when rate-limited |
| on.substack.com | 2026-08-22 | front | front gave the picker no article; not read |
| people.com | 2026-08-22 | front, section, article | rate-limits after a few requests (403) |
| politico.com | 2026-08-22 | front | front gave the picker no article; not read |
| politiken.dk | 2026-08-22 | front, section, article | games widget under a paywalled page, left |
| premiumtimesng.com | 2026-08-22 | front, section, article | picker landed on a category page; not read |
| pressgazette.co.uk | 2026-08-22 | front | 403 (Varnish, UA gate) |
| punchng.com | 2026-08-22 | front, section, article | clean |
| quantamagazine.org | 2026-08-22 | front, article | clean |
| reason.com | 2026-08-22 | front, article | clean |
| reddit.com | 2026-09-03 | subreddit comment page | JS shell, 0 chars; its server-rendered alternate host now 302s to a login form, 0 chars. The rewrite to it was removed; see findings, Phase 2 |
| reuters.com | 2026-08-22 | front | 401 |
| sciencedaily.com | 2026-08-22 | front, section, article | "related stories" thread false positive, fixed generically |
| scmp.com | 2026-08-22 | front, section, article | profile: strip TTS speed menu |
| search.brave.com | 2026-08-24 | search | results read via data- attributes; class names carry a build hash |
| slate.com | 2026-08-22 | front, article | clean |
| spiegel.de | 2026-08-22 | front, section, article | clean |
| straitstimes.com | 2026-08-22 | front, section, article | profile: strip newsletter line |
| sueddeutsche.de | 2026-08-22 | front, section, article | clean |
| tagesschau.de | 2026-08-22 | front, article | clean |
| techcrunch.com | 2026-08-22 | front, section, article | clean |
| telegraph.co.uk | 2026-08-22 | front | 402 |
| theatlantic.com | 2026-08-22 | front, article | clean |
| theglobeandmail.com | 2026-08-22 | front, section, article | clean |
| theguardian.com | 2026-08-22 | front, section, article | "Most viewed" tail, fixed generically |
| thehill.com | 2026-08-22 | front, section, article | related-card remnant at the tail, left |
| thehindu.com | 2026-08-22 | front, section, article | clean |
| theverge.com | 2026-08-22 | front, article | native-ad block at the foot: tail trimmer handles it (profile tried and removed) |
| threads.com | 2026-09-03 | profile | JS shell, body emits 0 chars; nothing for `tweet.rs` to read |
| time.com | 2026-08-22 | front, section, article | clean |
| timesofindia.indiatimes.com | 2026-08-22 | front, section, article | clean |
| usatoday.com | 2026-08-22 | front, section, article | clean |
| vox.com | 2026-08-22 | front, section, article | native-ad block at the foot: tail trimmer handles it (profile tried and removed) |
| washingtonpost.com | 2026-08-22 | front | h2 reset on every request |
| wired.com | 2026-08-22 | front, article | clean |
| wsj.com | 2026-08-22 | front | 401 |
| wyborcza.pl | 2026-08-22 | front, article | adblock wall in place of the article |
| x.com | 2026-09-03 | status permalink, profile | permalink server-rendered whole; article scoring lost it (139 chars, false JS caution). `tweet.rs` added, named for this host: no rewrite, no profile. Profile timeline reads as one article (288 chars), tweet run dropped |
