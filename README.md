<img src="assets/logo/hww.svg" alt="hww" width="128">

# A new faster, safer, accessible, and more private browser

[![CI](https://github.com/tayler/hww/actions/workflows/ci.yml/badge.svg)](https://github.com/tayler/hww/actions/workflows/ci.yml)

I guess every browser makes those claims, but hww is very different than what you might be imagining. 

hww fetches a web page, parses the HTML, then displays it using hww's own renderer. It removes so much surface area this way. Speed, safety, accessibility, and more privacy are very cheap after that. And the browser maintainer is not playing whack-a-mole with JS engine vulnerabilities. Multiple classes of bugs and vulnerabilities are not possible in hww.

- **Speed** is gained by skipping CSS and JS parsing and either skipping image loading or loading as needed (configurable).
- **Safety** is increased because tomorrow's browser exploit will never execute in hww. 
- **Accessibility** is nearly automatic on every website hww supports because hww controls the output.
- **More privacy** because there are no cookies or JavaScript engine. Multiple fingerprinting techniques are not possible: Pages cannot probe the `<canvas>` element, WebGL, audio, installed fonts, screen size, timezone, storage, or WebRTC. Servers still see your IP address, requested URLs, and hww's HTTP/TLS fingerprint.

## Features

- **Page readibility.** Readability-shaped scoring with a recall bias, tuned to find the prose in pages. Work to support more sites is ongoing.
- **No cookies.** The capability is missing from the build, along with JavaScript, page CSS, ads, and third-party requests.
- **Choose when images load.** Load them automatically, ask first (the default), turn off article images, or block every image request (including favicons).
- **Limited sharing when loading images.** An image's server can see which website the image appears on, but not the full page address. You can switch this off.
- **Ubuntu, Windows, and macOS,** binaries (See #Install below). More Linux distros planned.
- **Keyboard navigation** Every action has a keyboard shortcut (uses common browser defaults where possible): `Ctrl+L`, `Ctrl+F`, `Ctrl+R`, `Ctrl+H`, `Ctrl+D`, `Alt+Left`. Press `?` for the card.
- **A library, a reading list, and a history you can switch off.** `Ctrl+D` (`Cmd+D` on macOS) keeps the page you are reading and `Ctrl+Shift+B` opens the list; `Ctrl+H` (`Cmd+Y` on macOS) opens the pages hww has drawn for you, which it writes down as it goes. Each has its own switch in Settings and its own button to empty it, and none of them takes another with it. The history records the address and the title, once per page rather than once per visit, and nothing about what you did on the page. Page info tells you whether the page on screen is kept.
- **Line up what to read next.** Tab to a link and press `Shift+L`, or right-click it and choose **Read later**, and it waits on a list `l` opens. hww does not fetch any of them until you open one, so a reading list of thirty links makes no requests and costs no network — it is a list of addresses and the words the links were wearing, nothing else. Each row carries a **remove** button, and **forget all** under the title empties the list; `Shift+L` on a link already listed takes it off too.
- **Six themes.** Follow-the-desktop (the default), light, sepia, dark, and a high-contrast AAA pair.
- **Four typefaces.** IBM Plex Sans, Serif, and Mono, plus Atkinson Hyperlegible Next for low vision. All of them are compiled into the binary.
- **Screen-reader support.** An AccessKit tree, with the off-screen layout skip suspended while an assistive technology is attached.
- **RSS and Atom feeds.** Open a feed's address and it is read as a list of entries with their summaries, not as a wall of XML. Nothing behind a headline is requested until you follow it, and how much of each entry to show is a setting.
- **Per-site rules, compiled in and reported.** A rewrite table, a profile table, a search-engine table. Every application is announced before the request goes out, and `Ctrl+Shift+R` reloads bare.

## Install

Binaries live on the [Releases page](https://github.com/tayler/hww/releases). Download the file for your platform along with `SHA256SUMS`, and verify it before installing. With the [GitHub CLI](https://cli.github.com) you can also check that a download was built by this repository's own workflow:

```
gh attestation verify <file> --repo tayler/hww
```

### Ubuntu

Ubuntu 22.04 or newer, for `libc6 >= 2.35`.

#### Install

Download `hww_*_amd64.deb` and the checksum file `SHA256SUMS` from the latest [release](https://github.com/tayler/hww/releases), then verify and install:

```
sha256sum --ignore-missing -c SHA256SUMS
sudo apt install ./hww_*_amd64.deb
```

The package installs the application, its desktop entry, and its icons. Open `hww` from the application menu or run `hww` at a shell; installing a newer package the same way upgrades it.

#### Uninstall

```
sudo apt purge hww
rm -rf "${XDG_CONFIG_HOME:-$HOME/.config}/hww"
```

### macOS

Apple Silicon, macOS 11 Big Sur or later, which the disk image enforces. Download `hww-*-aarch64-macos.dmg` and the checksum file `SHA256SUMS` from the latest [release](https://github.com/tayler/hww/releases). macOS ships `shasum` rather than `sha256sum`, so compare the two lines this prints:

```
shasum -a 256 hww-*-aarch64-macos.dmg
grep aarch64-macos SHA256SUMS
```

#### Install

Open the image, drag `hww.app` onto the `Applications` shortcut beside it, and eject the image. The app is ad-hoc signed and carries no Developer ID, so Gatekeeper refuses the first launch. `xattr -d com.apple.quarantine /Applications/hww.app` clears it in one command, or open **System Settings › Privacy & Security** and choose **Open Anyway** beside the message naming hww.

An Intel Mac has no published binary and builds from source, which needs the Xcode Command Line Tools and a stable Rust toolchain:

```
cargo install --git https://github.com/tayler/hww --locked --features gui --bin hww
```

#### Uninstall

Drag the app to the Trash, then:

```
rm -rf ~/Library/Application\ Support/hww
```

### Windows

64-bit Windows. Download `hww-*-x86_64-windows.zip` and the checksum file `SHA256SUMS` from the latest [release](https://github.com/tayler/hww/releases). PowerShell has no `sha256sum`, so compare the two lines this prints:

```
$zip = Get-Item hww-*-x86_64-windows.zip
(Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
Select-String -Path SHA256SUMS -Pattern $zip.Name
```

#### Install

Extract the archive and run `hww.exe` from the `hww-<version>` folder it creates. There is nothing to install: the binary links the C runtime statically, so no Visual C++ Redistributable is needed. The first run raises SmartScreen's *"Windows protected your PC"*, since the binary is not code-signed; choose **More info**, then **Run anyway**.

#### Uninstall

Delete the `hww-<version>` folder and `%APPDATA%\hww`.

### Opening links in hww

The Ubuntu package and the macOS bundle register hww as a handler for `http` and `https`, so it appears in **Open with…** on Ubuntu and **Open With** in Finder, and in the default-browser list on both. That makes hww **choosable, not default.** Do not set it as your default browser: single-page applications render blank in hww by design, and every link you click anywhere on the machine would land in a reader that cannot run them. Choose hww for the link you want to read in it.

Windows has no door yet. It ships as a zip with no installer, and registering a browser there means writing under `HKLM\Software\Clients\StartMenuInternet`, which is an installer's job. Run `hww.exe <url>` instead.

### On every platform

`HWW_CONFIG_DIR` overrides where settings, the library, and the history are kept; if it was set when hww ran, remove that directory instead of the one named above. Page content and images are never saved to disk. The one file that outlives a run besides `settings.json` is `library.json`, and it holds three lists: the pages you asked hww to keep, the links you marked to read later, and — unless you switch it off — the address and title of each page hww has shown you. **Settings › Library › forget everything** empties the first, **Settings › Reading list › forget the reading list** the second, and **Settings › History › forget history** the third; each leaves the other two alone, and the uninstall commands above delete the file with the rest of the directory.

## Main files

| File | Role |
| --- | --- |
| `src/bin/hww.rs` | Parses command-line flags and starts the reader |
| `src/reader/` | Reading logic and settings; `reader/ui` drives the egui window |
| `src/reader/archive.rs` | The library, the reading list, and the history, and the doctrine for anything hww is allowed to remember |
| `src/session.rs` | Runs rewrite → fetch → decode → extract for a URL |
| `src/sites.rs` | Builtin host rewrites and per-site extraction profiles |
| `src/search.rs` | Turns a query into a URL and a result page into entries |
| `src/feed.rs` | Parses RSS 2.0 and Atom into `Document` |
| `src/fetch.rs` | HTTP client with no cookie jar, redirect inspection, and request limits |
| `src/html.rs` | Parses HTML into `Document`: article scoring, cards, thread hooks |
| `src/thread.rs` | Detects comment lists from repeated siblings with the same classes |
| `src/ir.rs` | Semantic document intermediate representation (IR); extractors write the IR, renderers read it |

## Developing

A stable Rust toolchain new enough for edition 2024 (1.85 or later), and a C toolchain for `aws-lc-sys`, which arrives under rustls and builds vendored sources rather than linking a system TLS library. No graphics, font, or windowing package has to be installed first, because every native library in the graph is opened at runtime. Windows also needs NASM on PATH; macOS needs only the Xcode Command Line Tools.

```
cargo run --features gui                          # the reader, URL bar focused
cargo run --features gui -- example.com/article   # straight to a page
cargo build --release --features gui              # ./target/release/hww
```

The application lives behind the `gui` feature, so core tests compile without the egui dependency graph or the image subresource path. `hww` takes `--no-rewrite`, `--no-profile`, `--no-search`, and `--no-feed` to switch off one way of reading a response for a single run, `--show-rewrites`, `--show-profiles`, and `--show-engines` to print a table and exit, and `--why` for triage. In the reader, `Ctrl+Shift+R` (`Cmd+Shift+R` on macOS) reloads with all four off at once.

### Diagnosing a site

```
cargo run --features gui -- --why https://example.com/article
cargo run --features gui --bin hww-shot -- --url https://example.com/article --out /tmp/look
```

`--why` fetches through the same pipeline as a normal load and prints, instead of the article, what the extractor did with it: every content-root candidate it weighed with the winner marked, what the thread detector saw and why each group was rejected or chosen, where each metadata field came from, and whether the `<body>` fallback fired. It is built by the same pass that builds the document, so it cannot describe a decision the extractor did not take. Every terminal print runs through `render::sanitize_for_terminal`, because untrusted page text on a terminal is a stream of commands.

`hww-shot` photographs the reader on a page, so you can look at the rendering instead of guessing at it. `--keys "space space"` scrolls first, `--theme dark` checks the other palette, and `--out` keeps an ad-hoc look out of the regression baseline. Its `--all` catalog is a separate regression run that owns the display for minutes at a time.

### CI

Five commands must pass:

```
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo clippy --all-targets --locked --features gui -- -D warnings
cargo test --locked --features gui
```

The first three run only on Linux. The last two run again on Windows and macOS, which block the same merges. Each push to `main` also attaches a development package to its [workflow run](https://github.com/tayler/hww/actions/workflows/ci.yml), kept for 14 days and meant for testing unreleased code.

`Cargo.lock` is committed because this crate ships binaries.

## License

Licensed under the GNU Affero General Public License, version 3 or later ([LICENSE](LICENSE) or <https://www.gnu.org/licenses/agpl-3.0.html>).

Section 13 does not arise for a desktop client, which serves nobody over a network; it is here for the shape this code would take if it ever did. The extraction pipeline is the part of hww worth lifting, and a hosted version of it is the one derivative that plain GPL would let stay closed.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this crate by you shall be licensed as above, without any additional terms or conditions.

The embedded faces keep their own terms: Atkinson Hyperlegible Next, IBM Plex, and Noto under the SIL Open Font License, DejaVu Serif under the Bitstream Vera license. The texts are in `fonts/` and ship in every package.

### Name and logo

The license grants rights in the code and none in the name `hww` or the logo under `assets/logo/`. Sections 7(d) and 7(e) of the AGPL provide for a notice of this kind, so this is an additional term under the license rather than a request standing beside it. It is stated in [NOTICE](NOTICE), which ships in every package: a term that reached only this file would reach nobody holding the `.deb` or the `.zip`, who are the people it is addressed to.

Referring to the project needs no permission: call a fork "a fork of hww", say that a program reads hww's settings file, name it in a comparison, write about it. What needs permission is using the name or the logo as the identity of something distributed — a modified build published as `hww`, a package or executable named for it, or any presentation implying that this project produced or endorsed the result. Fork under the license and give the fork its own name.
