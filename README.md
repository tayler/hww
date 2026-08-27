# hww: a quiet-web client

[![CI](https://github.com/tayler/hww/actions/workflows/ci.yml/badge.svg)](https://github.com/tayler/hww/actions/workflows/ci.yml)

A reading client for the non-app web: stripped HTML, feeds, gemini, gopher, and local Markdown.
A browser without JavaScript, CSS, ads, third-party requests, or cookies.

It is a *second* browser, not a Chrome replacement. Single-page apps render blank, and that is
expected for now.

## Why

A quiet reading experience.

## Status

Working end to end for HTML, as a keyboard-driven reading GUI with a headless `--why` for triage:

```
cargo run --features gui -- [--no-rewrite] <url> # the reader
cargo run --features gui -- --why <url>          # the extractor's account, no window
```

| Piece | State |
|---|---|
| `fetch` | Privacy rules enforced in code; no cookie jar exists at compile time |
| `sites` | Per-site rules: a rewrite table, a profile table, and a search-engine table, builtin only, every application reported |
| `html` | Parse, article extraction, IR mapping |
| `thread` | Forum/discussion extraction (structural, not heuristic) |
| `cards` | Story-card detection: a front page or section page as a run of entries, in place |
| `search` | Web search: typed words become a query URL, a result page becomes entries; four engines, user-selectable |
| `session` | The pipeline in one place; the rewrite and search notices reported before dispatch |
| `render` | Plain text |
| `ir` | Document / Block / Inline / Thread / Comment / Entries / Entry |
| `reader` | Reading model: inline runs, outline, history, threads, image decoding, notices, the menu bar and every setting as data |
| GUI | egui reader: links, back/forward, outline, find, threads, images, menu bar, settings panel. **Linux, Windows, and macOS verified**, all three with a published binary |
| feeds, gemini, gopher, markdown | not started |
| archive | not started |

## Installing on Ubuntu

Stable 64-bit Ubuntu packages are published on the
[Releases page](https://github.com/tayler/hww/releases). Download the `.deb` and
`SHA256SUMS` files from the latest release, then verify and install them:

```
sha256sum -c SHA256SUMS
gh attestation verify hww_*_amd64.deb --repo tayler/hww
sudo apt install ./hww_*_amd64.deb
```

The package installs the application, its desktop entry, and its icons. Open `hww` from the
Ubuntu application menu or run `hww` at a shell. Installing a newer package with the same
command upgrades the existing installation.

Each successful push to `main` also attaches a temporary development package to its
[CI workflow run](https://github.com/tayler/hww/actions/workflows/ci.yml). The
`hww-linux-amd64` artifact expires after 14 days and is intended for testing unreleased code,
not as a stable download. Unzip it before installing it.

Remove the package and its one saved settings file completely with:

```
sudo apt purge hww
rm -rf "${XDG_CONFIG_HOME:-$HOME/.config}/hww"
```

If `HWW_CONFIG_DIR` was set when hww ran, remove that directory instead of the default config
directory. The downloaded ZIP and `.deb` are ordinary files and can be deleted separately.
hww writes no page content or reading history to disk.

## Installing on Windows

Stable 64-bit Windows builds are published on the same
[Releases page](https://github.com/tayler/hww/releases). Download
`hww-<version>-x86_64-windows.zip` and `SHA256SUMS`, then check the archive against the
published hash before extracting it. PowerShell has no `sha256sum`, so compare the two lines
this prints:

```
$zip = Get-Item hww-*-x86_64-windows.zip
(Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
Select-String -Path SHA256SUMS -Pattern $zip.Name
```

With the [GitHub CLI](https://cli.github.com) installed, one further check is available. It is
optional, and it answers a different question than the hash does: not whether the file matches
the release, but whether the release was built by this repository's own workflow from this
repository's own source.

```
gh attestation verify $zip --repo tayler/hww
```

Extract the archive and run `hww.exe` from the folder it creates. There is nothing to install:
the binary links the C runtime statically, so no Visual C++ Redistributable is needed, and
settings are the only thing it ever writes, to `%APPDATA%\hww`.

The binary is not code-signed, so the first run raises SmartScreen's *"Windows protected your
PC"*. Choose **More info**, then **Run anyway**. A certificate that would remove that prompt
costs money and identifies a publisher; the hash and the attestation above answer the same
question without one.

Each successful push to `main` also attaches a temporary `hww-windows-amd64` artifact to its
[CI workflow run](https://github.com/tayler/hww/actions/workflows/ci.yml), on the same 14-day
expiry as the Linux one. GitHub wraps every artifact in a ZIP of its own, so that download is a
ZIP containing the release ZIP.

Remove hww by deleting the extracted folder and `%APPDATA%\hww`. As on Linux, nothing else was
written: no page content, no reading history.

One flag behaves differently here. `hww.exe` is a GUI-subsystem executable, which is what keeps
a console window from opening behind the reader, and Windows gives such a process no console to
write to. `--why`, `--show-rewrites`, `--show-profiles`, and `--show-engines` therefore print
nothing on Windows and only their exit codes survive. Use a Linux build or `cargo run` for
triage.

## Installing on macOS

Stable Apple Silicon builds are published on the same
[Releases page](https://github.com/tayler/hww/releases). They are `aarch64` only: an Intel Mac
is not covered, and `cargo install --locked --features gui` is the route there. macOS 11 Big Sur
or later is required, which the disk image itself enforces.

Download `hww-<version>-aarch64-macos.dmg` and `SHA256SUMS`, then check the image against the
published hash before opening it. macOS ships `shasum` rather than `sha256sum`, so compare the
two lines this prints:

```
shasum -a 256 hww-*-aarch64-macos.dmg
grep aarch64-macos SHA256SUMS
```

With the [GitHub CLI](https://cli.github.com) installed, one further check is available. It is
optional, and it answers a different question than the hash does: not whether the file matches
the release, but whether the release was built by this repository's own workflow from this
repository's own source.

```
gh attestation verify hww-*-aarch64-macos.dmg --repo tayler/hww
```

Open the image and drag `hww.app` onto the `Applications` shortcut beside it, then eject the
image. There is nothing else to install: every font is compiled into the binary, and settings
are the only thing hww ever writes, to `~/Library/Application Support/hww`.

The app is ad-hoc signed, which is the minimum an Apple Silicon binary needs to run at all, and
is not a Developer ID. Gatekeeper therefore refuses the first launch with *"hww Not Opened —
Apple could not verify..."*. Open **System Settings › Privacy & Security**, scroll to the
message naming hww, and choose **Open Anyway**; it appears there for about an hour after the
refusal. The old Control-click → **Open** shortcut no longer works on Sequoia and later. If you
would rather not visit System Settings, `xattr -d com.apple.quarantine /Applications/hww.app`
strips the download flag the check reads and does the same job in one command. A certificate
that would remove that prompt costs money and identifies a publisher; the hash and the
attestation above answer the same question without one.

Each successful push to `main` also attaches a temporary `hww-macos-arm64` artifact to its
[CI workflow run](https://github.com/tayler/hww/actions/workflows/ci.yml), on the same 14-day
expiry as the Linux and Windows ones. As with those, GitHub wraps every artifact in a ZIP of its
own, so that download is a ZIP containing the disk image.

Remove hww by dragging the app to the Trash and running
`rm -rf ~/Library/Application\ Support/hww`. As on the other two platforms, nothing else was
written: no page content, no reading history.

The flags that print nothing on Windows do print here. `#![windows_subsystem = "windows"]` is
inert on this target, so the app is an ordinary console binary as well as a bundle, and the
triage commands work by naming the executable inside it:

```
/Applications/hww.app/Contents/MacOS/hww --why https://example.com/article
```

## Running

A stable Rust toolchain new enough for edition 2024 (1.85 or later), and a C toolchain. Every
native library in the graph is opened at runtime rather than linked at build time, so no
graphics, font, or windowing package has to be installed first; the C compiler is for
`aws-lc-sys`, which arrives under rustls and builds vendored sources rather than linking a
system TLS library. On Windows that target also assembles, so NASM must be on PATH; on macOS the
Xcode Command Line Tools supply the compiler and no separate assembler is needed, since
`aws-lc-sys` takes its perlasm path there.

```
cargo run --features gui                          # the reader, URL bar focused
cargo run --features gui -- example.com/article   # straight to a page
```

### Diagnosing a site

Two tools say why a page came out the way it did, one for extraction and one for rendering:

```
cargo run --features gui -- --why https://example.com/article                 # the extractor's account
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
`https://`. `--why` prints the extractor's account to stdout with its provenance on stderr and
opens no window; every terminal print runs through `render::sanitize_for_terminal`, because
untrusted page text on a terminal is a stream of commands, not glyphs.

`hww` takes `--no-rewrite` and `--no-profile` to switch off a per-site table, and
`--show-rewrites` / `--show-profiles` to print one and exit. The application lives behind the
`gui` feature so core tests compile without the GUI dependency graph or its image subresource
path.

For everyday use, build once and run the binary:

```
cargo build --release --features gui
./target/release/hww example.com/article
```

## Measured

Against 150 real URLs sampled from browsing history ([full findings](docs/findings.md)):

- **72%** of document-shaped pages are readable with zero JavaScript
- **59%** of pages extract at or above trafilatura's output, 21% below
- On a threaded discussion page: **19,180 chars extracted vs trafilatura's 341**;
  article extraction cannot represent a thread, so threads get their own algorithm
- On the one JS-gated discussion site in that sample: **0 chars, then 8,331** once a per-site
  rule reaches the operator's own server-rendered domain

## Site rules

Some sites serve a fully server-rendered alternate of themselves. Phase 0 measured one such
site among its JS-only losses, which is why per-site rules are a feature rather than a hack.
`cargo run --features gui -- --show-rewrites` prints the table; `--no-rewrite` turns it off.

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
`--no-profile` turns it off, and `Shift+R` in the reader reloads bare: no rewrite, no profile,
no search.

## Search

Type words rather than an address and hww searches. Which engine it asks is a setting: DuckDuckGo
(the default), Mojeek, Brave Search, or Marginalia. `--show-engines` prints the table and
`--no-search` reads a result page as an ordinary page instead.

The results are a real list, not the engine's page flattened into prose: hww reads the result
markup into the same entries a front page produces, and follows the destination directly.
DuckDuckGo wraps every result in its own redirect, so hww unwraps it and the click reaches the
site rather than the click tracker.

The rewrite charter's first condition, *same operator*, cannot apply here — you named words,
not a host — so the fourth does the whole job. Before the request leaves, hww prints the engine
**and the terms**: `[searching html.duckduckgo.com for "rust ownership"]`. Nothing is written
to disk; queries live in the session's history and nowhere else.

An engine that declines to search is reported as such rather than as a site that did not
answer, and offers you another engine: DuckDuckGo answers several quick queries with an HTTP
202 and a CAPTCHA, and hww says so. Marginalia's countdown is not one of these — it carries 214
characters against a thin-page threshold of 200, so it falls through to the extractor and you
read the engine's own page. An engine that searched and found nothing says that instead, and
an engine that redesigned its results falls back to the ordinary extractor rather than claiming
your words found nothing. Which case it is comes from the engine's own result frame and how
much text is in it, not from a guess: see `docs/findings.md`, Phase 4.

`src/sites.rs` is the only file in the crate that contains a hostname, tests included, and it
imports no extractor. Everything else names a *row*, never a host: `src/search.rs` holds the
result-page selectors with no domain among them, and the settings panel stores an engine id, so
a preference file records `"mojeek"` and not an address. `src/session.rs` is still the one place
that turns a row into a request, and it reports the rewrite and the search before dispatching.

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
    src/bin/hww    thin main: argv -> reader::ui::run
    src/bin/shot   the screenshot catalog: named scenes, baseline hashes

## The reader

    cargo run --features gui -- <url>

Keyboard first, but nothing is keyboard-only: `?` lists the keys, a menu bar across the top
carries every command with the key that already does it, and the status strip along the bottom
carries back/forward, a clickable URL, and an italic `i` so a mouse alone can navigate. That
strip never hides: the URL bar, outline, find bar, menus, and help all dismiss on `Esc`, but a
rewrite notice has to be on screen *before* the request goes out, so the strip stays. The bar
itself can go (`View` ▸ `Menu bar`), and `F10` brings it back and focuses it.

What the reader has to say about a page divides on whether it can wait. Anything that would be
mis-read otherwise is a notice: an infobar docked under the top chrome, where no page can be,
with a severity icon, one sentence naming who did what, and a close, the shape every browser
uses for it. A 4xx body shown anyway, an extraction that came back thin, a body that arrived
truncated, a rewrite rule applied or landed on the wrong host are all bars; a failed navigation
is an error page; and none of them is ever dressed as prose.

The rest is accounting for the page on screen, and it lives behind the italic `i` (or `p`):
encoding, bytes downloaded against characters extracted, the redirect chain, cookies discarded,
images loaded and their hosts. Facts worth being able to ask for, not worth a permanent ribbon of six-point
text beside the URL.

Back and forward re-open the document hww still has rather than asking the site for it again,
so returning to a page you were reading costs no request for it and lands where you left it. What is
kept is bounded by the back stack — a page whose entry is gone is gone with it — and by a text
budget past which the oldest are dropped and fetched again. Pictures are not kept: their
textures and the tally of hosts they came from belong to the page as it was read, and a Back
that quietly re-contacted every one of them is the thing the image policy exists to refuse. A
restored page says so as the first row of its page-info panel, above the fetch it is still
reporting, because every other row there describes a request this visit did not make. Nothing
about any of it touches disk; it is gone with the process, like the history stack it follows.

One centred reading column, measured in characters (`[` and `]`). Zoom is egui's own, so
`Ctrl`+`+`/`-`/`0`, `Ctrl`+wheel, and trackpad pinch behave the way a browser does. Themes are
light, sepia, dark, and follow-the-system (`d`), plus a high-contrast pair that the `d` cycle
leaves out and the settings panel reaches.

**Images are placeholders that name their host** (`[image] load from cdn.example.net`), and by
default load only on an explicit click. They are never loaded on hover or in advance — except
the page favicon beside the masthead eyebrow, which is fetched as soon as the document names
one. Four policies. *Ask first* is the default and offers to load each picture; *Load
automatically* fetches the pictures near what you are reading and roughly a screenful ahead,
quietly, leaving the rest of the page until you scroll towards it;
*Article images off* marks where each one was and offers nothing; *No image requests* fetches
nothing at all, the site icon included, which is what separates it from the one above. Each
load is counted alongside cookie attempts in the page-info panel, and nothing is written to
disk. Image requests, and only image requests, carry an origin-only `Referer`; documents carry
none. `fetch::Referer::PageOrigin` documents why that one header is worth its exception.

The automatic policy is bounded by the layout band rather than by the document, and by a
ceiling on outstanding requests (`reader::autoload::MAX_OUTSTANDING`), because the worker pool
is shared with navigation: a page that queued two hundred pictures would leave the next click
waiting behind them.

## Settings

`,` or `Edit` ▸ `Settings...` opens a panel over the page, and everything the reader has is in
it: line width, text size, line and paragraph spacing, typeface, theme, image policy, link
addresses, zoom, scroll step, reply indent and its depth limit, the menu bar, and whether an
image request names the site it is loading for. Each carries a sentence saying what it does and
the key that also does it; each group has a `reset`, and the panel has a `reset all`.

Thirteen settings existed before the panel did and three of them were reachable. The rest
needed a text editor and the knowledge that there was a file to edit, which nothing in the
reader or this README mentioned. The list is data, and a test walks it against the settings
struct, so a setting that exists and cannot be reached fails the build rather than going
quietly missing.

The file is the only thing that survives a run:

    ${XDG_CONFIG_HOME:-~/.config}/hww/settings.json     # ~/Library/Application Support/hww on
                                                        # macOS, %APPDATA%\hww on Windows

It stays hand-editable, and the panel names its path at the foot. A value it cannot parse costs
that one setting rather than the file: an unknown theme or image policy falls back to the
default, everything else is kept, and a number outside its range is clamped instead of
believed.

## Tests

    cargo test                              # default features; egui never compiled
    cargo test --features gui               # adds the reader's pure modules

Covers IR mapping, relative-URL resolution, malformed-markup recovery, nested-document
recovery, encoding prescan bounds, host matching at a label boundary (`evil-example.com` is
not `example.com`), inline flattening, thread reconstruction, outline and fragment matching,
history, title de-duplication, image decoding and its ceilings, and the origin-only `Referer`.

Regressions worth keeping named: comment bodies stripped as chrome; styling leaking into the
IR; a whitespace-only node between two inline elements welding two words together; a setting
the panel cannot reach; a menu item the help card does not list; a reader-facing string needing
a glyph no shipped face carries; and the text renderer's markdown output, which is checked
against a verbatim copy of the walker it replaced.

## Releasing

Stable releases use annotated `vMAJOR.MINOR.PATCH` tags. Set the package version in
`Cargo.toml`, run `cargo check` to update `Cargo.lock`, and commit both files. Merge the change
to `main` and run all five CI commands listed in `AGENTS.md`. The update command deliberately
omits `--locked`; the subsequent `--locked` CI commands verify that the committed lockfile
matches the manifest. Tag that merged commit:

```
git switch main
git pull --ff-only
git tag -a v0.1.0 -m "hww 0.1.0"
git push origin v0.1.0
```

The tag workflow checks the tag against `Cargo.toml`, verifies that the commit belongs to
`main`, then builds on three runners: the Debian package on Ubuntu 22.04, which pins its glibc
floor, the Windows archive on a Windows runner, and the macOS disk image on an Apple Silicon
runner, whose floor comes from `MACOSX_DEPLOYMENT_TARGET` in `.cargo/config.toml` rather than
from the host and is asserted against the linked binary. All three are installed or run where
they are built. One `SHA256SUMS` covers the set, one attestation names all of them, and the
release publishes them together, so a tag that fails on any platform publishes nothing.

Every distribution carries the same license texts, staged by `packaging/licenses.sh`, which
both build scripts source: the crate's `LICENSE-MIT` and `LICENSE-APACHE` where the reader lands,
and the font licenses under `fonts/` beside them. The binary embeds every face in `fonts/`, so a
package without them redistributes those fonts without their terms. A new packaging format calls
`hww_install_licenses` instead of listing files, and a face added without its license text fails
the build.

After it succeeds, advance `Cargo.toml` and `Cargo.lock` on `main` to the next intended release
version. Development packages use `~git` versions below that future release, so upgrades
preserve Debian version ordering; the Windows archive spells the same version `-git`, since the
tilde is a dpkg ordering rule and means nothing to a ZIP.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for
inclusion in this crate by you, as defined in the Apache-2.0 license, shall be dual licensed
as above, without any additional terms or conditions.

The embedded faces keep their own terms: Atkinson Hyperlegible Next, IBM Plex, and Noto under
the SIL Open Font License, DejaVu Serif under the Bitstream Vera license. The texts are in
`fonts/` and ship in every package.
