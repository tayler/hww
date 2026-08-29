//! `hww-shot`: drive the reader into a named state, photograph it, and report the photograph
//! in far fewer bytes than the photograph.
//!
//! # Why a tool and not a screenshot key
//!
//! The states worth looking at are the ones that are hard to arrange: a body cut at the 5 MB
//! cap, a rewrite rule that landed on the wrong host, a refused https to http downgrade, a
//! server that answers 404 with the article still in the markup. Waiting for the live web to
//! serve one of those is not a workflow. Every such scene here is built offline: fixture markup
//! goes through the real `html::extract`, a hand-built [`Provenance`] carries the fact that
//! makes the scene, and [`ReaderApp::present`] hands the pair to the same `settle` a real
//! navigation runs. Nothing downstream of that call knows the page never came off a socket.
//!
//! The three scenes that are genuinely *about* the network keep the network: `loading`,
//! `fail-transport`, and the image scenes drive real requests against a fixture server bound to
//! a loopback port in this process. They run last, because a stalled document fetch keeps a
//! worker until the 15 s timeout.
//!
//! # Why it reports the way it does
//!
//! A screenshot read back by an agent costs tokens by the pixel, so the run prints text and
//! writes pixels. Each scene gets one line: dimensions, a content hash, and `same` / `changed`
//! / `new` against the baseline recorded on the previous run. A run where nothing moved is
//! thirty lines and no image needs opening at all; a run where three scenes moved names those
//! three, and only those get read. `--ascii` adds a luminance sketch, which is enough to see
//! that a band appeared or a panel opened without loading the picture. `--max-dim` bounds the
//! longest edge of what is written, because a HiDPI capture is four times the pixels and none
//! of the extra ones say anything.
//!
//! `HWW_CONFIG_DIR` is pointed at a directory under `--out` before the window opens: `d`, `[`,
//! and `]` mark settings dirty, and the reader saves them. A screenshot tool must not be able
//! to edit the settings of the person running it.

use eframe::egui::{self, Key, Modifiers};
use hww::fetch::{FetchError, Truncation};
use hww::html;
use hww::reader::menu::Command;
use hww::reader::opts::Theme;
use hww::reader::settings::Settings;
use hww::reader::ui::{Launch, ReaderApp};
use hww::session::{LoadError, Loaded, Provenance};
use image::codecs::png::PngEncoder;
use image::{ImageEncoder, RgbaImage};
use std::collections::BTreeMap;
use std::io::{BufRead, BufWriter, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use url::Url;

// ---------------------------------------------------------------- scenes

/// One step of a scene, applied one per frame so the state it sets has a frame to take effect.
enum Step {
    /// A page the reader is handed rather than fetching.
    Page {
        url: String,
        rewrite: Option<String>,
        body: String,
        tweak: fn(&mut Provenance),
    },
    /// A failure screen the reader is handed rather than earning.
    Fail {
        url: String,
        error: LoadError,
    },
    /// A real navigation, typed into the URL bar exactly as a reader would type it.
    Nav(String),
    /// A real navigation started from a link in the page, as Enter on it does. What a click
    /// does, without needing to know where the link is drawn.
    Follow(String),
    /// Mark one link to read later, as `Shift+L` on it and the context menu's "Read later" both
    /// do.
    ///
    /// [`Step::Follow`]'s argument exactly, one action over, and it is the reason
    /// `ReaderApp::read_later` is public. Both routes to it need a *focused* link, and a
    /// synthetic Tab does not reliably land focus — so a scene built from key steps would mark
    /// nothing, photograph an empty reading list, and pass.
    ReadLater {
        href: String,
        title: String,
    },
    /// One menu command, as choosing it from the bar does.
    ///
    /// The same argument as [`Step::Follow`], one menu over: a synthetic Tab does not
    /// reliably land focus inside an egui popup, so an item with no key of its own cannot be
    /// reached by pressing keys at the window. Driving F10 and four Tabs photographs the
    /// route as far as the open menu and then stops, which is a picture of the menu rather
    /// than of what the item does.
    Run(Command),
    Key(Modifiers, Key),
    Type(String),
    Wait(u32),
}

struct Scene {
    name: String,
    what: &'static str,
    steps: Vec<Step>,
    /// Extra milliseconds before the shutter. A picture has to be fetched and decoded first.
    settle_ms: u64,
    /// The picture cannot be the same twice: the loading notice counts the seconds it has been
    /// waiting, which is the point of it. Photographed, never compared, never baselined.
    volatile: bool,
}

fn scene(name: &str, what: &'static str, steps: Vec<Step>) -> Scene {
    Scene {
        name: name.to_owned(),
        what,
        steps,
        settle_ms: 0,
        volatile: false,
    }
}

fn page(url: &str, body: &str) -> Step {
    Step::Page {
        url: url.to_owned(),
        rewrite: None,
        body: body.to_owned(),
        tweak: |_| {},
    }
}

fn page_with(url: &str, body: &str, tweak: fn(&mut Provenance)) -> Step {
    Step::Page {
        url: url.to_owned(),
        rewrite: None,
        body: body.to_owned(),
        tweak,
    }
}

fn fail(url: &str, error: LoadError) -> Step {
    Step::Fail {
        url: url.to_owned(),
        error,
    }
}

fn key(key: Key) -> Step {
    Step::Key(Modifiers::NONE, key)
}

fn shift(key: Key) -> Step {
    Step::Key(Modifiers::SHIFT, key)
}

const ARTICLE_URL: &str = "https://example.com/2026/the-quiet-web";
const THREAD_URL: &str = "https://example.org/thread/1181";

/// Fixtures are markup, not IR: they go through the real extractor, so a scene cannot show a
/// document shape `html::extract` would never produce.
/// A front page: two card lists under section headings, a lead with a dek, thumbnails never
/// loaded. Through the real extractor, so what is photographed is `cards::detect` finding the
/// groups and the walker emitting `Block::Entries` in place.
const FRONT_URL: &str = "https://example.com/";
const FRONT: &str = r#"
<html lang="en"><head><title>The Example Times - Front Page</title>
<meta property="og:site_name" content="The Example Times"></head><body>
<nav><a href="/">home</a> <a href="/world">world</a> <a href="/local">local</a></nav>
<main>
<h2>Top stories</h2>
<div class="card"><a href="/s/1"><img src="/t1.jpg" alt="A harbour at dusk"></a>
<h3><a href="/s/1">Council approves the harbour plan after a long night of amendments</a></h3>
<p class="dek">The vote came at two in the morning, with the gallery still full and the dissenters
still talking. What was agreed, and what was left for the autumn.</p><span class="time">2 hours ago</span></div>
<div class="card"><a href="/s/2"><img src="/t2.jpg" alt="A rail yard"></a>
<h3><a href="/s/2">Rail strike called off as both sides accept the mediator's figure</a></h3>
<p class="dek">Services resume at dawn. The figure is not the one either side asked for.</p><span class="time">3 hours ago</span></div>
<div class="card"><a href="/s/3"><img src="/t3.jpg" alt="A field"></a>
<h3><a href="/s/3">Drought order extended to the whole of the valley for a second month</a></h3>
<p class="dek">Farmers say the river has not been this low since the records began.</p><span class="time">5 hours ago</span></div>
<h2>World</h2>
<ul>
<li class="item"><h4><a href="/w/1">Talks resume in the capital after a week of silence from both delegations</a></h4><span class="time">1 hour ago</span></li>
<li class="item"><h4><a href="/w/2">Election count enters its third day with the margin still under a point</a></h4><span class="time">4 hours ago</span></li>
<li class="item"><h4><a href="/w/3">A volcano that had slept for four centuries is venting again, and the town below is calm</a></h4><span class="time">6 hours ago</span></li>
<li class="item"><h4><a href="/w/4">Central bank holds rates and says very little about the next meeting</a></h4><span class="time">9 hours ago</span></li>
</ul>
</main>
<footer><a href="/about">about</a> <a href="/contact">contact</a></footer>
</body></html>
"#;

/// A front page long enough that most of it is off-screen: what the row-level window rule in
/// `blocks::entries_ui` is for.
///
/// `FRONT` has seven cards and fits in about two screens, so every row of it is inside the band
/// and the skip never fires. This one is sixty, which at 900x800 leaves the great majority of
/// them clear of the window in either direction once the page is scrolled — so the `front-long`
/// scene photographs rows drawn *below* a run of skipped ones, which is where an error in the
/// remembered heights shows up as text in the wrong place.
fn front_long() -> String {
    let mut s = String::from(
        "<html lang=\"en\"><head><title>The Example Times - Every Story</title>\n\
         <meta property=\"og:site_name\" content=\"The Example Times\"></head><body>\n\
         <main><h2>Every story</h2>\n",
    );
    for i in 1..=60 {
        s.push_str(&format!(
            "<div class=\"card\"><h3><a href=\"/s/{i}\">Story number {i}: the council, the \
             harbour, and the long night of amendments</a></h3>\n\
             <p class=\"dek\">The vote came at two in the morning, with the gallery still full \
             and the dissenters still talking.</p><span class=\"time\">{i} hours ago</span></div>\n"
        ));
    }
    s.push_str("</main></body></html>\n");
    s
}

/// The article with one more link in its first paragraph, to `href`: what `pending-link`
/// follows. A fixture of its own rather than a change to `ARTICLE`, which a dozen scenes share.
fn article_linking(href: &str) -> String {
    ARTICLE.replacen(
        "left as text.",
        &format!("left as text, <a href=\"{href}\">the slow page</a> notwithstanding."),
        1,
    )
}

/// The two documents the page-cache scene navigates between, served by `serve` at `/one` and
/// `/two`. Both carry a link to the other, so the scene can be a real Follow rather than a
/// second typed URL, and both clear `ir::THIN_TEXT` so no extraction bar joins the picture.
/// The two pages the bookmarks scene keeps, served at `/marked-one` and `/marked-two`.
///
/// Their own documents rather than the injected `FRONT` and `THREAD` the scene used before the
/// bookmarks grew an icon column: a mark is a real request for a real image, so photographing the
/// column at all means keeping pages hww fetched from a server that answers for their icons.
/// Two different icons, because the column's job is to tell one site from another, and a
/// picture in which every row carries the same mark would not show that it does.
const MARKED_ONE: &str = r#"
<html lang="en"><head><title>A page worth keeping</title>
<link rel="icon" href="/mark-a.png"></head><body>
<article>
<h1>A page worth keeping</h1>
<p>Ctrl+D writes this page's address, its title, and the address of the mark above into the one
file that outlives the run. Nothing else about it is stored, and nothing of what it says.</p>
<p>The mark is fetched when the bookmarks draws it, not when the page is kept: a list of icons on
disk would be page content on disk, which is the line this reader does not cross.</p>
</article></body></html>
"#;

const MARKED_TWO: &str = r#"
<html lang="en"><head><title>The second page in the list</title>
<link rel="icon" href="/mark-b.png"></head><body>
<article>
<h1>The second page in the list</h1>
<p>Two pages from one host would carry one mark between them, so this fixture names a second
icon and the column has something to distinguish. The bookmarks lists most recently kept first,
which puts this row above the other one.</p>
<p>Every row's mark is fetched under the same policy as the icon beside the masthead, and the
page-info panel names the hosts it came from even here, where nothing else was requested.</p>
</article></body></html>
"#;

const KEPT_ONE: &str = r#"
<html lang="en"><head><title>The page you came back to</title></head><body>
<article>
<h1>The page you came back to</h1>
<p>Back and forward re-open the document the reader still has, so returning to this page costs
no request at all. The page-info panel says so in as many words, because every other row in it
describes a fetch and this visit did not make one.</p>
<p>What is not kept is the pictures. Their textures and the tally of hosts they were fetched
from both belong to the page as it was read, and a Back that quietly re-contacted every one of
them would be the opposite of what the image policy promises.</p>
<p><a href="/two">Onwards to the second page</a></p>
</article></body></html>
"#;

const KEPT_TWO: &str = r#"
<html lang="en"><head><title>The page you went to</title></head><body>
<article>
<h1>The page you went to</h1>
<p>Following this link pushed an entry onto the stack and left the page before it in memory,
filed under the entry it belongs to. Pressing Back now moves the cursor and finds it there
instead of asking the site a second time.</p>
<p>The document that comes back opens where it was left, which it did before this cache existed
too: the offset is remembered by the history entry and not by the page.</p>
</article></body></html>
"#;

const ARTICLE: &str = r#"
<html lang="en"><head><title>Reading without the rest of it</title>
<meta name="author" content="A. Reader"></head><body>
<nav><a href="/">home</a> <a href="/archive">archive</a></nav>
<article>
<h1>Reading without the rest of it</h1>
<p class="byline">By A. Reader, 14 March 2026</p>
<p>The page you are reading arrived as markup and left as text. Nothing between those two
points ran a script, consulted a stylesheet, or asked a third party for anything at all. What
is left is the part that was worth keeping, which on most pages is a good deal smaller than
what was sent.</p>
<h2>What survives the trip</h2>
<p>Headings, paragraphs, lists, quotations, tables, and code all map onto a small intermediate
form, and every renderer reads only that. A <a href="https://example.net/spec">specification</a>
of eight block kinds is enough for the reading web, and the discipline of keeping it at eight is
what makes a second renderer cheap.</p>
<ul><li>Text, with emphasis and <em>emphasis within emphasis</em>.</li>
<li>Links, which say where they go before they are followed.</li>
<li>Images, which are named and not fetched.</li></ul>
<blockquote><p>A reader that shows a hostile picture and calls it the article's photograph has
failed worse than one that shows no picture at all.</p></blockquote>
<p>Code keeps its spacing, because <code>whitespace</code> is the content:</p>
<pre><code>cargo run --features gui --bin hww -- example.com
cargo run --features gui --bin hww-shot -- --all</code></pre>
<h2>Measurements</h2>
<table><tr><th>Path</th><th>Pages</th><th>Recovered</th></tr>
<tr><td>Article</td><td>150</td><td>94%</td></tr>
<tr><td>Discussion</td><td>8</td><td>5.6%</td></tr></table>
<hr>
<p><strong>The point of the exercise</strong> is that the second browser does not have to be
the first one with the features removed. It can be a different thing that happens to read the
same wire format, and the wire format is the only part anyone agreed on anyway.</p>
</article>
<footer><p>Copyright nobody. Take it.</p></footer>
</body></html>
"#;

const THREAD: &str = r#"
<html><head><title>Discussion: the second browser</title></head><body>
<h1>Discussion: the second browser</h1>
<div class="comments">
  <div class="comment"><span class="author">first</span> <span class="age">3 hours ago</span>
    <div class="commtext">The interesting claim here is not that the web is too heavy. It is
    that a client which refuses the whole apparatus is still useful on most of the pages people
    actually read, which is a measurable claim rather than an aesthetic one.</div>
    <div class="comment"><span class="author">second</span> <span class="age">2 hours ago</span>
      <div class="commtext">Measurable, and measured badly by everyone who has tried. The
      numbers people quote are for their own reading list, which is exactly the sample that
      makes the answer come out the way they wanted.</div>
      <div class="comment"><span class="author">third</span> <span class="age">1 hour ago</span>
        <div class="commtext">Sampling by recency rather than by visit count is the whole trick.
        Frequency ranking surfaces webmail and dashboards, which nobody reads in a reader.</div>
      </div>
    </div>
  </div>
  <div class="comment"><span class="author">fourth</span> <span class="age">2 hours ago</span>
    <div class="commtext">Nested replies are where every one of these projects loses the plot.
    Detect the siblings, miss the children, and report a thread of thirteen when the page has
    sixty one of them sitting right there in the markup.</div>
  </div>
  <div class="comment"><span class="author">fifth</span> <span class="age">40 minutes ago</span>
    <div class="commtext">A discussion page is not an article with comments appended. It is N
    sibling subtrees and no best one, and scoring will happily hand back a single reply out of
    eighty eight without noticing anything went wrong.</div>
  </div>
</div>
</body></html>
"#;

const THIN: &str = r#"
<html><head><title>Dashboard</title></head><body>
<div id="root"></div><noscript>You need to enable JavaScript to run this app.</noscript>
</body></html>
"#;

const RTL: &str = r#"
<html><head><title>A page with Hebrew</title></head><body>
<article>
<h1>A page with Hebrew</h1>
<p>A Latin paragraph long enough to be a paragraph on its own merits, then a Hebrew run that
this reader cannot lay out: שלום עולם. The rest of this paragraph exists so the extracted
text clears the floor: a thin page would raise a second caution about JavaScript, and the
layout remark would share the panel with a diagnosis that is not this page's.</p>
<p>Another paragraph of Latin, still about nothing in particular, so the character count
cannot be mistaken for a page that failed to extract.</p>
</article></body></html>
"#;

/// Two images from a loopback host: one that answers with a picture, one that answers 404.
const IMAGES: &str = r#"
<html><head><title>The photograph problem</title></head><body>
<article>
<h1>The photograph problem</h1>
<p>An article image is a third party request, and the placeholder names the host before the
click rather than after it. Nothing here is fetched until someone asks for it, and asking for
all of them names the distinct hosts first.</p>
<figure><img src="http://127.0.0.1:{PORT}/photo.png" alt="A test gradient, 640 by 360">
<figcaption>A gradient, standing in for the photograph that would otherwise be here.</figcaption>
</figure>
<p>Hotlink protection that answers 403 degrades safely: no picture, and a visible failure. The
kind that answers 200 with a substitute picture does not degrade safely at all, and no status
code available to the client can tell the two apart.</p>
<figure><img src="http://127.0.0.1:{PORT}/missing.png" alt="An image the server does not have">
<figcaption>The same request, against a path the server answers 404 for.</figcaption></figure>
<p>Both requests carry an origin only Referer naming this page's host, never its path, and both
are counted in the page info panel next to the cookies nobody stored.</p>
</article></body></html>
"#;

/// A feed: a masthead with no site of its own, entries with real dates, and one summary long
/// enough for the reading budget to cut. Through the real `feed::read`, so what is photographed
/// is the XML path rather than the extractor's idea of XML.
const FEED_URL: &str = "https://example.com/feed.xml";
const FEED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:content="http://purl.org/rss/1.0/modules/content/"
     xmlns:atom="http://www.w3.org/2005/Atom">
  <channel>
    <atom:link href="https://example.com/feed.xml" rel="self"/>
    <title>The Example Times</title>
    <link>https://example.com/</link>
    <language>en</language>
    <lastBuildDate>Thu, 27 Aug 2026 17:17:57 +0000</lastBuildDate>
    <item>
      <title>A quiet week for the human-wide web</title>
      <link>https://example.com/2026/quiet-week</link>
      <pubDate>Thu, 27 Aug 2026 09:12:00 +0000</pubDate>
      <content:encoded><![CDATA[<div class="post-promo"><p>Three publishers moved back to
      hand-written HTML this month, and none of them announced it. The pages are half the size
      they were, they read the same in every client anyone has tried them in, and not one of the
      three has had to touch a build step since. That is the whole argument, and it keeps
      arriving as a footnote to something else.</p>
      <p>The wrapper this paragraph sits in is classed as promotional, which on a whole page is
      the sort of thing the chrome hints exist to delete. Inside a feed summary it is the post,
      and deleting it would leave the entry short with nothing on screen saying why. It also
      carries a &gt; greater-than sign, which is exactly where an HTML parser reading this CDATA
      section as a bogus comment would have stopped swallowing it, losing however much of the
      entry happened to sit before the next one, which is a different amount on every feed and
      is never announced anywhere the reader could see it.</p>
      <blockquote><p>The web was readable before it was interactive, and where anyone has left
      it alone, it is readable still.</p></blockquote>
      <p>This fourth paragraph is past the reading budget. It is on the page, hww has it, and
      the renderer stops before it — which is why the picture below the quotation goes straight
      to the next headline.</p></div>]]></content:encoded>
    </item>
    <item>
      <title>Notes from a slow reader</title>
      <link>/2026/slow-reader</link>
      <pubDate>Wed, 26 Aug 2026 18:40:00 +0000</pubDate>
      <description>&lt;p&gt;A short entry, entity-escaped rather than wrapped in CDATA, which
      is the other half of what a feed reader has to handle.&lt;/p&gt;</description>
    </item>
    <item>
      <link>https://example.com/2026/no-headline</link>
      <pubDate>Tue, 25 Aug 2026 07:05:00 +0000</pubDate>
      <description>An item with no title element at all, which is how a link blog writes one,
      and whose headline is therefore the first line of what it does carry.</description>
    </item>
  </channel>
</rss>"#;

fn catalog(port: u16) -> Vec<Scene> {
    let images = IMAGES.replace("{PORT}", &port.to_string());
    let stall = format!("http://127.0.0.1:{port}/stall");
    let mut scenes = vec![
        scene(
            "idle",
            "nothing asked for yet: the splash and the URL bar",
            vec![],
        ),
        scene(
            "article",
            "a page that arrived clean: no bars, no extra chrome",
            vec![page(ARTICLE_URL, ARTICLE)],
        ),
        scene(
            "front",
            "a front page: card lists as entries under their section headings",
            vec![page(FRONT_URL, FRONT)],
        ),
        Scene {
            settle_ms: 400,
            volatile: true,
            ..scene(
                "bookmarks",
                "the bookmarks: two kept pages, each under its site's mark",
                // Two real keeps and a real open, through `run_command`, which is one of the
                // four hooks `AGENTS.md` names for this: both commands are reachable that way,
                // so neither needs key steps nor depends on a synthetic Tab landing focus. It
                // photographs `Ready::built`, `Provenance::built`, and `archive::document` on the
                // path a reader takes rather than a page injected to look like one.
                //
                // Two real *navigations* as well, which injected pages cannot replace since the
                // icon column grew: a mark is a third-party image, so the only way to photograph
                // the column with anything in it is to keep pages a server will answer for. The
                // waits are the requests — the document, then the mark the built view asks for
                // once it is drawn.
                //
                // Volatile for the reason the loading scenes are, and now for the fixture
                // server's ephemeral port as well: an entry says the day it was kept, so the
                // picture carries today's date by construction. Neither page is `ARTICLE_URL`,
                // so the page-info scenes that photograph that article still show it as one the
                // reader has not kept, whatever order these run in.
                vec![
                    Step::Nav(format!("http://127.0.0.1:{port}/marked-one")),
                    Step::Wait(30),
                    Step::Run(Command::BookmarkPage),
                    Step::Nav(format!("http://127.0.0.1:{port}/marked-two")),
                    Step::Wait(30),
                    Step::Run(Command::BookmarkPage),
                    Step::Run(Command::OpenBookmarks),
                    Step::Wait(30),
                ],
            )
        },
        Scene {
            settle_ms: 400,
            volatile: true,
            ..scene(
                "reading-list",
                "the reading list: links marked to read later, addresses under their words",
                // Three real marks and a real open, through `ReaderApp::read_later` — the hook
                // that exists because both routes to this need a focused link and a synthetic Tab
                // does not reliably land one. Key steps here would mark nothing and photograph an
                // empty list, which is a scene that passes while the feature is broken.
                //
                // The first two wear the words a link usually wears, which is the whole argument
                // for drawing the address under each row: without it these are two identical
                // "Read more"s. The third carries none at all, so the picture also covers
                // `Item::label`'s fallback to the address and `address_under`'s refusal to print
                // it twice.
                //
                // Volatile for the bookmarks scene's reason: a row says the day it was marked.
                vec![
                    page(FRONT_URL, FRONT),
                    Step::ReadLater {
                        href: ARTICLE_URL.to_owned(),
                        title: "Read more".to_owned(),
                    },
                    Step::ReadLater {
                        href: THREAD_URL.to_owned(),
                        title: "Read more".to_owned(),
                    },
                    Step::ReadLater {
                        href: "https://example.net/untitled".to_owned(),
                        title: String::new(),
                    },
                    // One row on the fixture server, so the picture carries a mark that really
                    // arrived. The other three point at hosts that do not resolve, which is the
                    // ordinary case for this list and is worth photographing beside it: a guess
                    // that comes to nothing leaves the column empty and the row where it was.
                    Step::ReadLater {
                        href: format!("http://127.0.0.1:{port}/marked-one"),
                        title: "A page worth keeping".to_owned(),
                    },
                    Step::Run(Command::OpenReadingList),
                    Step::Wait(30),
                ],
            )
        },
        Scene {
            volatile: true,
            ..scene(
                "history",
                "the pages hww has written down, most recent first",
                // Volatile, and for one more reason than the bookmarks scene: an entry carries the
                // day it was drawn, and the list is whatever this run of the catalog has visited
                // by the time it gets here, so it changes when a scene is added ahead of it.
                // Photographed, never compared.
                vec![
                    page(FRONT_URL, FRONT),
                    page(THREAD_URL, THREAD),
                    Step::Run(Command::OpenHistory),
                ],
            )
        },
        scene(
            "feed",
            "an RSS feed: entries with calendar dates, summaries cut at a block boundary",
            vec![page(FEED_URL, FEED)],
        ),
        scene(
            "thread",
            "a discussion: authors, timestamps, nested replies",
            vec![page(THREAD_URL, THREAD)],
        ),
        scene(
            "thread-collapsed",
            "the same discussion after Shift+Z",
            vec![page(THREAD_URL, THREAD), shift(Key::Z)],
        ),
        scene(
            "thin",
            "caution: little content extracted, may require JavaScript",
            vec![page("https://example.com/app", THIN)],
        ),
        scene(
            "rtl",
            "caution: right-to-left text this reader cannot lay out",
            vec![page("https://example.com/he", RTL)],
        ),
        scene(
            "status-404",
            "caution: the server answered 404, content shown anyway",
            vec![page_with(ARTICLE_URL, ARTICLE, |p| p.status = 404)],
        ),
        scene(
            "truncated-cap",
            "caution: body stopped at the 5 MB cap",
            vec![page_with(ARTICLE_URL, ARTICLE, |p| {
                p.truncation = Truncation::AtCap(5_000_000);
                p.bytes = 5_000_000;
            })],
        ),
        scene(
            "truncated-timeout",
            "caution: body still arriving when the clock ran out",
            vec![page_with(ARTICLE_URL, ARTICLE, |p| {
                p.truncation = Truncation::Timeout(Duration::from_secs(15));
            })],
        ),
        scene(
            "truncated-dropped",
            "caution: the connection dropped partway",
            vec![page_with(ARTICLE_URL, ARTICLE, |p| {
                p.truncation = Truncation::Incomplete(41_820);
            })],
        ),
        scene(
            "dead-rule",
            "caution: a rewrite rule landed somewhere else, and is dead",
            vec![page_with(ARTICLE_URL, ARTICLE, |p| {
                p.rewritten_to = Some(url("https://text.example.com/2026/the-quiet-web"));
                p.rule_appears_dead = true;
                p.final_url = url("https://www.example.com/2026/the-quiet-web?src=redirect");
                p.hops = vec![url("https://text.example.com/2026/the-quiet-web")];
            })],
        ),
        scene(
            "stacked-cautions",
            "rewrite bar and three cautions: dead rule, truncation, and 404",
            vec![page_with(ARTICLE_URL, ARTICLE, |p| {
                p.rewritten_to = Some(url("https://text.example.com/2026/the-quiet-web"));
                p.rule_appears_dead = true;
                p.final_url = url("https://www.example.com/2026/the-quiet-web");
                p.status = 404;
                p.truncation = Truncation::MaybeAtCap(5_000_000);
            })],
        ),
        scene(
            "fail-blocked",
            "failure: a silent hang, read as bot detection",
            vec![fail(
                ARTICLE_URL,
                LoadError::Fetch(FetchError::LikelyBlocked),
            )],
        ),
        scene(
            "fail-timeout",
            "failure: out of time on a subresource, bandwidth not a block",
            vec![fail(
                ARTICLE_URL,
                LoadError::Fetch(FetchError::Timeout(Duration::from_secs(15))),
            )],
        ),
        scene(
            "fail-downgrade",
            "failure: refused an https to http downgrade mid-redirect",
            vec![fail(
                ARTICLE_URL,
                LoadError::Fetch(FetchError::SchemeDowngrade(url("http://example.com/plain"))),
            )],
        ),
        scene(
            "fail-redirects",
            "failure: past the redirect cap, with the chain as detail",
            vec![fail(
                ARTICLE_URL,
                LoadError::Fetch(FetchError::TooManyRedirects(vec![
                    url("https://example.com/consent?to=/2026/the-quiet-web"),
                    url("https://consent.example.com/?return=1"),
                    url("https://example.com/2026/the-quiet-web"),
                    url("https://example.com/consent?to=/2026/the-quiet-web&again=1"),
                    url("https://consent.example.com/?return=2"),
                ])),
            )],
        ),
        scene(
            "fail-no-location",
            "failure: a 302 that did not say where",
            vec![fail(
                ARTICLE_URL,
                LoadError::Fetch(FetchError::RedirectWithoutLocation(302)),
            )],
        ),
        scene(
            "fail-bad-location",
            "failure: a redirect to something unfetchable",
            vec![fail(
                ARTICLE_URL,
                LoadError::Fetch(FetchError::BadLocation(
                    "app://open/article/1181".to_owned(),
                )),
            )],
        ),
        scene(
            "fail-not-web-page",
            "failure: the response was not a web page",
            vec![fail(
                "https://example.com/paper.pdf",
                LoadError::NotWebPage {
                    content_type: "application/pdf".to_owned(),
                },
            )],
        ),
        scene(
            "outline",
            "the outline panel over an article",
            vec![
                page(ARTICLE_URL, ARTICLE),
                Step::Key(Modifiers::COMMAND | Modifiers::SHIFT, Key::O),
            ],
        ),
        scene(
            "help",
            "the help overlay",
            vec![page(ARTICLE_URL, ARTICLE), key(Key::Questionmark)],
        ),
        scene(
            "about",
            "the About card: every line of it, not the first one in a toast",
            // `Step::Run`, not F10 and a row of Tabs: Help, About hww is the item's only
            // route, and a synthetic Tab does not land focus inside an egui popup, so the key
            // route photographs an open menu and never the card. The card used to be a
            // six-second toast carrying `ABOUT[0]` alone, which is why it has cover now: the
            // two sentences that say what hww refuses to do were the unreachable ones.
            vec![page(ARTICLE_URL, ARTICLE), Step::Run(Command::ShowAbout)],
        ),
        scene(
            "settings",
            "the settings panel: every setting, with a sentence about each",
            vec![
                page(ARTICLE_URL, ARTICLE),
                Step::Key(Modifiers::COMMAND, Key::Comma),
            ],
        ),
        scene(
            "menubar",
            "the View menu open: commands, their keys, and the live settings",
            // F10 focuses the first title, Tab walks to View, Enter opens it. egui menus read
            // no arrow keys, so this is the whole keyboard route and photographing it is also
            // what checks that the route exists.
            vec![
                page(ARTICLE_URL, ARTICLE),
                key(Key::F10),
                key(Key::Tab),
                key(Key::Tab),
                key(Key::Enter),
            ],
        ),
        scene(
            "toast",
            "Copied URL as a pill above the strip",
            vec![page(ARTICLE_URL, ARTICLE), key(Key::Y)],
        ),
        scene(
            "pageinfo",
            "the page info panel: hops, cookies, rewrite, profile, referer",
            vec![
                page_with(ARTICLE_URL, ARTICLE, |p| {
                    p.profile = Some(hww::profile::ProfileReport {
                        host: "example.com".to_owned(),
                        fields: vec![
                            hww::profile::FieldReport {
                                field: "strip",
                                outcome: hww::profile::Outcome::Matched(2),
                            },
                            hww::profile::FieldReport {
                                field: "strip",
                                outcome: hww::profile::Outcome::Dead,
                            },
                        ],
                    });
                    p.rewritten_to = Some(url("https://text.example.com/2026/the-quiet-web"));
                    p.final_url = url("https://text.example.com/2026/the-quiet-web");
                    p.hops = vec![
                        url("https://example.com/2026/the-quiet-web"),
                        url("https://text.example.com/2026/the-quiet-web"),
                    ];
                    p.cookie_attempts = 3;
                    p.encoding = "windows-1252";
                    p.bytes = 184_320;
                }),
                Step::Key(Modifiers::COMMAND, Key::I),
            ],
        ),
        scene(
            "pageinfo-thin",
            "page info on a thin page: the extraction floor is in the record",
            vec![
                page("https://example.com/app", THIN),
                Step::Key(Modifiers::COMMAND, Key::I),
            ],
        ),
        scene(
            "pageinfo-rtl",
            "page info on an RTL page: layout stays in the record",
            vec![
                page("https://example.com/he", RTL),
                Step::Key(Modifiers::COMMAND, Key::I),
            ],
        ),
        scene(
            "find",
            "the find bar, with every match on the page highlighted",
            vec![
                page(ARTICLE_URL, ARTICLE),
                Step::Key(Modifiers::COMMAND, Key::F),
                Step::Wait(2),
                Step::Type("read".to_owned()),
            ],
        ),
        scene(
            "urlbar",
            "the URL bar, summoned over a page",
            vec![
                page(ARTICLE_URL, ARTICLE),
                Step::Key(Modifiers::COMMAND, Key::L),
            ],
        ),
        scene(
            "focus-link",
            "the focus ring, three tab stops in",
            vec![
                page(ARTICLE_URL, ARTICLE),
                key(Key::Tab),
                Step::Wait(1),
                key(Key::Tab),
                Step::Wait(1),
                key(Key::Tab),
            ],
        ),
        scene(
            "scrolled",
            "the foot of an article, after Shift+G",
            vec![page(ARTICLE_URL, ARTICLE), shift(Key::G)],
        ),
        Scene {
            settle_ms: 1200,
            ..scene(
                "entries-scrolled",
                "the foot of a sixty-card front page: rows drawn under a run of skipped ones",
                vec![
                    page("https://example.com/every-story", &front_long()),
                    // Before the key, not after: `Shift+G` scrolls to the bottom the *previous*
                    // frame measured, and a list this long does not reach its final height on
                    // the frame it arrives. Without the wait the scene lands somewhere in the
                    // forties and somewhere else next run — which it did before the row-level
                    // window rule existed too. It is the scene racing the layout, not the reader
                    // seeing anything move.
                    Step::Wait(1200),
                    shift(Key::G),
                ],
            )
        },
        scene(
            "images-placeholder",
            "image placeholders, naming the host before the click",
            vec![page("https://example.com/photographs", &images)],
        ),
        Scene {
            settle_ms: 2500,
            ..scene(
                "images-loaded",
                "the same page after Shift+I, pictures fetched and decoded",
                vec![
                    page("https://example.com/photographs", &images),
                    shift(Key::I),
                ],
            )
        },
        Scene {
            settle_ms: 400,
            volatile: true,
            ..scene(
                "loading",
                "a navigation in flight with nothing on screen under it",
                // The `fail` is not decoration. Every scene shares one `ReaderApp` and nothing
                // resets it between them, and a navigation now keeps the page it is replacing,
                // so a bare `Nav` here photographs whichever article ran before it. A failure
                // owns the viewport, so navigating from one is how this state is reached without
                // a hook the reader would otherwise not have to give up.
                vec![
                    fail(ARTICLE_URL, LoadError::Fetch(FetchError::LikelyBlocked)),
                    Step::Nav(stall.clone()),
                ],
            )
        },
        Scene {
            settle_ms: 400,
            volatile: true,
            ..scene(
                "pending-link",
                "a link followed and its page still in flight: the [loading] mark on the link",
                // The mark that `notice::PENDING` exists for, photographed at last: the page
                // stays, the followed link carries the bracket, the strip counts. Driven by
                // `Step::Follow`, which does what Enter on the focused link does, because a
                // synthetic Tab does not reliably land focus in this harness.
                vec![
                    page(ARTICLE_URL, &article_linking(&stall)),
                    Step::Follow(stall.clone()),
                ],
            )
        },
        Scene {
            settle_ms: 400,
            volatile: true,
            ..scene(
                "loading-over-page",
                "a navigation in flight while the outgoing page is still being read",
                // The picture the commit-on-arrival change exists for: the article still
                // readable, the strip counting, and the progress rule crossing the strip.
                //
                // Driven from the URL bar rather than by following a link, which would be the
                // truer scenario and would also catch `notice::PENDING` in the same frame. A
                // link needs Tab and Enter, and synthetic Tab focus does not reliably land in
                // this harness: `focus-link` photographs the article with no focus ring on the
                // same machines this does. Worth revisiting with a pointer-click step, which
                // would be a better way to drive both scenes.
                vec![page(ARTICLE_URL, ARTICLE), Step::Nav(stall)],
            )
        },
        // Two real navigations, not injected pages: `present` resets the history to a single
        // entry, so the only way to photograph a page that came back from memory is to go
        // somewhere and press Back. `Command::Back` rather than a key, for the reason
        // `Step::Run` exists.
        //
        // Volatile, and not for the usual reason. Nothing in this picture counts seconds —
        // `pageinfo::ago` floors at a minute precisely so it could be compared — but the
        // fixture server binds an ephemeral port, and the panel's Address row prints the whole
        // URL, so the hash moves on every run. What the scene is worth is the path: `Back`
        // through `run_command` into `restore` and out at a drawn page, which no unit test
        // reaches. The row's group, its position ahead of every Response row, its wording and
        // its ages are all asserted in `reader::pageinfo`, where `cargo test` can read them.
        Scene {
            settle_ms: 400,
            volatile: true,
            ..scene(
                "pageinfo-restored",
                "page info on a page put back from memory rather than fetched again",
                vec![
                    Step::Nav(format!("http://127.0.0.1:{port}/one")),
                    Step::Wait(30),
                    // Absolute: `session::classify_link` parses the href, and the reader
                    // only ever sees absolutised ones because `html::extract` joins them.
                    Step::Follow(format!("http://127.0.0.1:{port}/two")),
                    Step::Wait(30),
                    Step::Run(Command::Back),
                    Step::Wait(4),
                    Step::Run(Command::TogglePageInfo),
                ],
            )
        },
        Scene {
            settle_ms: 400,
            ..scene(
                "fail-transport",
                "failure: a real connection refused, in the reader's own words",
                vec![Step::Nav("http://127.0.0.1:1/".to_owned())],
            )
        },
    ];
    scenes.sort_by_key(|s| network_last(&s.name));
    scenes
}

/// The scenes that use the network run last: a stalled document fetch holds a worker until the
/// 15 s timeout, and everything after it would queue behind that.
///
/// Three of them now hold one each, against `net::WORKERS`, which is four. There is room for
/// exactly no more.
fn network_last(name: &str) -> u8 {
    u8::from(matches!(
        name,
        "loading" | "loading-over-page" | "pending-link" | "fail-transport"
    ))
}

fn url(s: &str) -> Url {
    Url::parse(s).expect("a fixture URL parses")
}

// ---------------------------------------------------------------- the driver

struct Config {
    out: PathBuf,
    size: [f32; 2],
    max_dim: u32,
    /// Milliseconds between the last step of a scene and the shutter. Time, not frames: egui
    /// fades scrollbars and grows a window to its content over a wall-clock interval, so a
    /// frame count photographs whatever the machine happened to reach and hashes differently
    /// on the next run.
    settle_ms: u64,
    ascii: bool,
    json: bool,
    /// Seconds of no captured scene before the run gives up. A compositor that stops sending
    /// frame callbacks to an occluded window stops the whole state machine, and a screenshot
    /// tool that hangs forever is worse than one that says it stalled.
    timeout_s: u64,
}

struct Shot {
    name: String,
    what: &'static str,
    volatile: bool,
    /// Pixels that differ from the picture this scene had before, and the count is in the
    /// report line. `None` when there was nothing to compare against.
    moved: Option<u64>,
    w: u32,
    h: u32,
    hash: String,
    path: PathBuf,
    ascii: Option<String>,
}

enum Stage {
    Steps,
    /// Two conditions, both necessary: a few frames so the state is drawn at all, and a
    /// deadline so every fade and sizing pass has finished before the shutter.
    Settle {
        frames: u32,
        until: Instant,
    },
    Shutter,
    /// Frames left to wait for the reply before giving up on this scene.
    Await(u32),
}

struct Runner {
    app: ReaderApp,
    scenes: Vec<Scene>,
    scene: usize,
    step: usize,
    wait: u32,
    stage: Stage,
    cfg: Arc<Config>,
    baseline: Arc<BTreeMap<String, String>>,
    shots: Arc<Mutex<Vec<Shot>>>,
    /// When the last scene landed. The watchdog reads it; see [`Config::timeout_s`].
    progress: Arc<Mutex<Instant>>,
    /// When the previous frame ran. A long gap means the compositor stopped asking for frames
    /// and has just started again, which is not a moment to photograph anything.
    last_frame: Instant,
    frames: u64,
}

impl Runner {
    fn advance(&mut self) {
        self.scene += 1;
        self.step = 0;
        self.wait = 0;
        self.stage = Stage::Steps;
    }

    /// A page arriving clears the chrome first: the launch opens the URL bar over an empty
    /// reader, and every scene after the first would otherwise inherit it.
    /// Frames enough to draw the state at all, and then a wall clock long enough for every
    /// fade and sizing pass to finish. Both, because either alone photographs a page in motion.
    fn settle_stage(&self) -> Stage {
        let ms = self.cfg.settle_ms + self.scenes[self.scene].settle_ms;
        Stage::Settle {
            frames: 6,
            until: Instant::now() + Duration::from_millis(ms),
        }
    }

    fn apply(&mut self, ctx: &egui::Context, step: Step) {
        if matches!(step, Step::Page { .. } | Step::Fail { .. }) {
            press(ctx, Modifiers::NONE, Key::Escape);
        }
        match step {
            Step::Page {
                url,
                rewrite,
                body,
                tweak,
            } => {
                let url = Url::parse(&url).expect("a fixture URL parses");
                let loaded = build(&url, &body, tweak);
                self.app.present(url, rewrite, Ok(loaded));
            }
            Step::Fail { url, error } => {
                let url = Url::parse(&url).expect("a fixture URL parses");
                self.app.present(url, None, Err(error));
            }
            Step::Nav(_)
            | Step::Follow(_)
            | Step::ReadLater { .. }
            | Step::Run(_)
            | Step::Key(..)
            | Step::Type(_)
            | Step::Wait(_) => {
                unreachable!("handled against the context")
            }
        }
    }

    fn drive(&mut self, ctx: &egui::Context) {
        // A frame that arrives long after the one before it means the window was frozen, and
        // the first frame back is mid-animation: a scroll still easing to the top, a window
        // still growing to its content. The settle is a wall clock, so a freeze runs it out
        // and the shutter fires on exactly that frame. Start it again instead.
        let gap = self.last_frame.elapsed();
        self.last_frame = Instant::now();
        if gap > Duration::from_millis(250)
            && let Stage::Settle { .. } = self.stage
        {
            trace(&format!("settle restarted after a {gap:?} gap"));
            self.stage = self.settle_stage();
        }
        match self.stage {
            Stage::Steps => {
                if self.wait > 0 {
                    self.wait -= 1;
                    return;
                }
                let i = self.scene;
                if self.step < self.scenes[i].steps.len() {
                    let step =
                        std::mem::replace(&mut self.scenes[i].steps[self.step], Step::Wait(0));
                    self.step += 1;
                    match step {
                        Step::Wait(n) => self.wait = n,
                        Step::Key(mods, k) => press(ctx, mods, k),
                        Step::Type(text) => {
                            ctx.input_mut(|i| i.events.push(egui::Event::Text(text)))
                        }
                        Step::Follow(href) => {
                            self.app.follow_link(&href);
                            self.wait = 2;
                        }
                        Step::ReadLater { href, title } => {
                            self.app.read_later(&href, &title);
                            self.wait = 2;
                        }
                        Step::Run(command) => {
                            self.app.run_command(ctx, command);
                            self.wait = 2;
                        }
                        Step::Nav(target) => {
                            press(ctx, Modifiers::COMMAND, Key::L);
                            self.wait = 2;
                            // The bar opens prefilled with the current URL, so replace rather
                            // than append: this is what Ctrl+A then typing does.
                            self.scenes[i]
                                .steps
                                .insert(self.step, Step::Key(Modifiers::COMMAND, Key::A));
                            self.scenes[i]
                                .steps
                                .insert(self.step + 1, Step::Type(target));
                            self.scenes[i]
                                .steps
                                .insert(self.step + 2, Step::Key(Modifiers::NONE, Key::Enter));
                        }
                        other => self.apply(ctx, other),
                    }
                    return;
                }
                trace(&format!("{}: steps done", self.scenes[i].name));
                self.stage = self.settle_stage();
            }
            Stage::Settle { frames, until } => {
                self.stage = if frames == 0 && Instant::now() >= until {
                    Stage::Shutter
                } else {
                    Stage::Settle {
                        frames: frames.saturating_sub(1),
                        until,
                    }
                };
            }
            Stage::Shutter => {
                trace(&format!("{}: shutter", self.scenes[self.scene].name));
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
                self.stage = Stage::Await(120);
            }
            Stage::Await(n) => {
                let captured = ctx.input(|i| {
                    i.events.iter().find_map(|e| match e {
                        egui::Event::Screenshot { image, .. } => Some(image.clone()),
                        _ => None,
                    })
                });
                match captured {
                    Some(image) => {
                        trace(&format!("{}: captured", self.scenes[self.scene].name));
                        let scene = &self.scenes[self.scene];
                        match save(&image, scene, &self.cfg) {
                            // Reported as it lands rather than in a summary at the end: a run
                            // that stalls on scene twenty has still told you about nineteen.
                            Ok(shot) => {
                                report(&shot, &self.baseline, &self.cfg);
                                self.shots.lock().expect("no poisoned lock").push(shot);
                            }
                            Err(e) => eprintln!("{}: {e}", scene.name),
                        }
                        *self.progress.lock().expect("no poisoned lock") = Instant::now();
                        self.advance();
                    }
                    None if n == 0 => {
                        eprintln!("{}: no screenshot came back", self.scenes[self.scene].name);
                        self.advance();
                    }
                    None => self.stage = Stage::Await(n - 1),
                }
            }
        }
    }
}

impl eframe::App for Runner {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // Nothing here is driven by user input, so nothing would ask for the next frame.
        ctx.request_repaint();
        self.frames += 1;
        if self.frames.is_multiple_of(120) {
            trace(&format!("frame {} scene {}", self.frames, self.scene));
        }
        if self.scene >= self.scenes.len() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else {
            self.drive(&ctx);
        }
        self.app.ui(ui, frame);
    }

    fn on_exit(&mut self) {
        self.app.on_exit();
    }
}

/// Stage transitions on stderr when `HWW_SHOT_TRACE` is set: the state machine is a window
/// that stops advancing, and there is nothing else to look at when it does.
fn trace(msg: &str) {
    if std::env::var_os("HWW_SHOT_TRACE").is_some() {
        eprintln!("[shot] {msg}");
    }
}

/// A key press and its release, as a real keyboard sends them. `consume_key` reads the press;
/// the release keeps `keys_down` honest for anything watching it.
///
/// Tab needs a second half, and it is the one key that does. Everything the reader binds it
/// reads for itself out of the frame's `InputState`, which is what a pushed event lands in.
/// Focus is not the reader's: egui derives the move from the *raw* input in
/// `Memory::begin_pass`, which has run before any step gets to push, and which resets the
/// direction on every pass. So an injected Tab was read by nobody, and `focus-link`
/// photographed a plain article for as long as the scene existed, matching `article` hash for
/// hash in the baseline that was supposed to catch exactly this. `drive` runs before
/// `app.ui`, so a direction set here is consumed by the widgets drawn after it, in this pass.
/// The match mirrors egui's own: bare Tab forward, Shift+Tab back, anything else not a move.
fn press(ctx: &egui::Context, modifiers: Modifiers, key: Key) {
    ctx.input_mut(|i| {
        for pressed in [true, false] {
            i.events.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers,
            });
        }
    });
    let direction = match key {
        Key::Tab if !modifiers.any() => Some(egui::FocusDirection::Next),
        Key::Tab if modifiers.shift_only() => Some(egui::FocusDirection::Previous),
        _ => None,
    };
    if let Some(direction) = direction {
        ctx.memory_mut(|m| m.move_focus(direction));
    }
}

/// Fixture markup through the real reader, with the provenance the scene needs.
///
/// `feed::read` first and `html::extract` second, which is the order `session::load` asks them
/// in. A feed scene then photographs the path a feed really takes, and `prov.feed` is set the
/// same way the pipeline sets it, so the page-info row and the suppressed thin-text caution are
/// the real ones.
fn build(url: &Url, body: &str, tweak: fn(&mut Provenance)) -> Loaded {
    let (doc, feed) = match hww::feed::read(body, url) {
        hww::feed::Outcome::Feed(doc, report) => (doc, Some(report)),
        _ => (html::extract(body, url), None),
    };
    let mut prov = Provenance {
        requested: url.clone(),
        rewritten_to: None,
        rule_appears_dead: false,
        status: 200,
        encoding: "UTF-8",
        final_url: url.clone(),
        bytes: body.len(),
        chars: doc.text_len(),
        hops: Vec::new(),
        cookie_attempts: 0,
        truncation: Truncation::Complete,
        profile: None,
        feed,
    };
    tweak(&mut prov);
    Loaded { doc, prov }
}

// ---------------------------------------------------------------- pixels out, text back

fn save(image: &egui::ColorImage, scene: &Scene, cfg: &Config) -> std::io::Result<Shot> {
    let (name, what) = (scene.name.as_str(), scene.what);
    let [w, h] = [image.size[0] as u32, image.size[1] as u32];
    let mut raw = Vec::with_capacity((w * h * 4) as usize);
    for px in &image.pixels {
        raw.extend_from_slice(&px.to_array());
    }
    let full = RgbaImage::from_raw(w, h, raw).expect("the capture is w*h RGBA");
    let long = w.max(h);
    let img = if cfg.max_dim > 0 && long > cfg.max_dim {
        let scale = f64::from(cfg.max_dim) / f64::from(long);
        let (nw, nh) = (
            ((f64::from(w) * scale) as u32).max(1),
            ((f64::from(h) * scale) as u32).max(1),
        );
        image::imageops::resize(&full, nw, nh, image::imageops::FilterType::Lanczos3)
    } else {
        full
    };

    let path = cfg.out.join(format!("{name}.png"));
    // Against the picture already on disk, before it is overwritten. A line saying a scene
    // changed does not say what moved; this does, and it is the one image worth opening.
    let diff_path = cfg.out.join(format!("{name}.diff.png"));
    let _ = std::fs::remove_file(&diff_path);
    let moved = match diff(&path, &img) {
        Some((count, marked)) => {
            let file = BufWriter::new(std::fs::File::create(&diff_path)?);
            PngEncoder::new(file)
                .write_image(
                    marked.as_raw(),
                    marked.width(),
                    marked.height(),
                    image::ExtendedColorType::Rgba8,
                )
                .map_err(std::io::Error::other)?;
            Some(count)
        }
        None => None,
    };
    let file = BufWriter::new(std::fs::File::create(&path)?);
    PngEncoder::new(file)
        .write_image(
            img.as_raw(),
            img.width(),
            img.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(std::io::Error::other)?;

    Ok(Shot {
        name: name.to_owned(),
        what,
        volatile: scene.volatile,
        moved,
        w: img.width(),
        h: img.height(),
        hash: hash(img.as_raw()),
        path,
        ascii: cfg.ascii.then(|| sketch(&img)),
    })
}

/// The previous picture for this scene against the new one: how many pixels moved, and where.
///
/// The where is the point. A hash says a scene changed and a full screenshot says what it looks
/// like now; neither says which band grew a line or which glyph shifted a pixel. The marked
/// image is the old one dimmed to grey with every changed pixel in red, which reads at a glance
/// and at a fraction of the tokens of comparing two screenshots by eye.
fn diff(previous: &Path, now: &RgbaImage) -> Option<(u64, RgbaImage)> {
    let old = image::ImageReader::open(previous)
        .ok()?
        .decode()
        .ok()?
        .to_rgba8();
    if old.dimensions() != now.dimensions() {
        return None;
    }
    let mut count = 0u64;
    let mut marked = RgbaImage::new(now.width(), now.height());
    for (x, y, px) in marked.enumerate_pixels_mut() {
        let (a, b) = (old.get_pixel(x, y).0, now.get_pixel(x, y).0);
        if a == b {
            let grey = (u32::from(a[0]) + u32::from(a[1]) + u32::from(a[2])) / 3;
            // Toward the middle, so both dark and light pages fall back behind the marks.
            let flat = (grey as u8 / 2).saturating_add(96);
            *px = image::Rgba([flat, flat, flat, 255]);
        } else {
            count += 1;
            *px = image::Rgba([220, 30, 30, 255]);
        }
    }
    (count > 0).then_some((count, marked))
}

/// FNV-1a over the saved pixels. Change detection, not cryptography: the question it answers is
/// "is this the same picture as last time", and the answer decides whether an image is worth
/// opening at all.
fn hash(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// A luminance sketch: enough to see a band appear or a panel open, at a thousandth of the
/// tokens of the picture it describes.
fn sketch(img: &RgbaImage) -> String {
    const COLS: u32 = 48;
    const ROWS: u32 = 24;
    const RAMP: &[u8] = b"@%#*+=-:. ";
    let (w, h) = (img.width(), img.height());
    let mut cells = Vec::with_capacity((COLS * ROWS) as usize);
    for row in 0..ROWS {
        for col in 0..COLS {
            let (x0, x1) = (
                col * w / COLS,
                ((col + 1) * w / COLS).max(col * w / COLS + 1),
            );
            let (y0, y1) = (
                row * h / ROWS,
                ((row + 1) * h / ROWS).max(row * h / ROWS + 1),
            );
            let mut sum = 0u64;
            let mut n = 0u64;
            for y in y0..y1.min(h) {
                for x in x0..x1.min(w) {
                    let p = img.get_pixel(x, y).0;
                    sum += u64::from(p[0]) * 2 + u64::from(p[1]) * 5 + u64::from(p[2]);
                    n += 8;
                }
            }
            cells.push(sum.checked_div(n).unwrap_or(255) as u32);
        }
    }
    // Stretched to the extremes actually present, because the thing worth seeing is a band or
    // a panel against the page, and a caution band differs from paper by a few percent of
    // luminance. Absolute levels would render the whole page as one shade of nothing.
    let lo = cells.iter().copied().min().unwrap_or(0);
    let hi = cells.iter().copied().max().unwrap_or(255).max(lo + 1);
    let mut out = String::with_capacity(((COLS + 3) * ROWS) as usize);
    for (i, cell) in cells.iter().enumerate() {
        if (i as u32).is_multiple_of(COLS) {
            out.push_str("  ");
        }
        let level = (cell - lo) as usize * (RAMP.len() - 1) / (hi - lo) as usize;
        out.push(RAMP[level] as char);
        if i as u32 % COLS == COLS - 1 {
            out.push('\n');
        }
    }
    out
}

// ---------------------------------------------------------------- the fixture server

/// A loopback server for the scenes that are genuinely about the network.
///
/// It exists so `images-loaded`, `loading` and `pageinfo-restored` are real: a real request, a
/// real decode, a real hang, a real Back. Bound to 127.0.0.1 on a port the OS picks, serving the
/// paths matched below and nothing else.
fn serve() -> std::io::Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let photo = Arc::new(gradient_png());
    let mark_a = Arc::new(mark_png([206, 86, 66]));
    let mark_b = Arc::new(mark_png([54, 108, 168]));
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let photo = Arc::clone(&photo);
            let mark_a = Arc::clone(&mark_a);
            let mark_b = Arc::clone(&mark_b);
            std::thread::spawn(move || {
                let mut stream = stream;
                let mut buf = [0u8; 2048];
                let n = stream.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let path = req.split_whitespace().nth(1).unwrap_or("/").to_owned();
                let _ = match path.as_str() {
                    "/photo.png" => ok(&mut stream, "image/png", &photo),
                    // The two site marks the bookmarks scene photographs. Small and square,
                    // because that is what a favicon is, and two colours apart, because the
                    // column exists to tell one site from another.
                    "/mark-a.png" => ok(&mut stream, "image/png", &mark_a),
                    // The reading list's marks are guesses at this path, because hww has
                    // never fetched the pages those rows point at and so has no declared
                    // address to have kept. Serving it is what makes that scene photograph a
                    // real request rather than an empty column. See `archive::well_known_icon`.
                    "/favicon.ico" => ok(&mut stream, "image/png", &mark_a),
                    "/mark-b.png" => ok(&mut stream, "image/png", &mark_b),
                    "/marked-one" => ok(
                        &mut stream,
                        "text/html; charset=utf-8",
                        MARKED_ONE.as_bytes(),
                    ),
                    "/marked-two" => ok(
                        &mut stream,
                        "text/html; charset=utf-8",
                        MARKED_TWO.as_bytes(),
                    ),
                    // Two ordinary documents, for the one scene that needs two *real*
                    // navigations rather than injected pages: `present` resets the history to a
                    // single entry, so a page cache can only be photographed by actually going
                    // somewhere and coming back. They answer 200 with prose over the extraction
                    // floor, or the thin-page and status bars would join the picture.
                    "/one" | "/two" => {
                        let body = if path == "/one" { KEPT_ONE } else { KEPT_TWO };
                        ok(&mut stream, "text/html; charset=utf-8", body.as_bytes())
                    }
                    // The stall is the point: a document fetch that never answers is what the
                    // Loading state, and eventually LikelyBlocked, are made of.
                    "/stall" => {
                        std::thread::sleep(Duration::from_secs(60));
                        Ok(())
                    }
                    _ => stream.write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: 9\r\nConnection: close\r\n\r\nnot here\n",
                    ),
                };
            });
        }
    });
    Ok(port)
}

/// One 200 with a body, so no fixture path hand-rolls a response head of its own.
fn ok(stream: &mut TcpStream, content_type: &str, body: &[u8]) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(head.as_bytes())
        .and_then(|()| stream.write_all(body))
}

/// One site mark: a rounded-off square of one colour, at the size a favicon actually is.
///
/// Not `gradient_png` scaled down. That one is 640x360, and a wide picture in a square box is
/// what `images::site_icon` fits rather than stretches, so using it here would photograph the
/// fitting and never the ordinary case.
fn mark_png(rgb: [u8; 3]) -> Vec<u8> {
    const N: u32 = 32;
    let img = RgbaImage::from_fn(N, N, |x, y| {
        // A disc, so the picture shows a shape and not a block of colour that could be a
        // background fill.
        let (dx, dy) = (x as f32 - 15.5, y as f32 - 15.5);
        let inside = dx * dx + dy * dy <= 15.0 * 15.0;
        let a = if inside { 255 } else { 0 };
        image::Rgba([rgb[0], rgb[1], rgb[2], a])
    });
    let mut out = Vec::new();
    PngEncoder::new(&mut out)
        .write_image(img.as_raw(), N, N, image::ExtendedColorType::Rgba8)
        .expect("encoding a generated image cannot fail");
    out
}

fn gradient_png() -> Vec<u8> {
    let img = RgbaImage::from_fn(640, 360, |x, y| {
        let t = (x * 255 / 640) as u8;
        let u = (y * 255 / 360) as u8;
        image::Rgba([t, 90u8.saturating_add(u / 2), 255 - t, 255])
    });
    let mut out = Vec::new();
    PngEncoder::new(&mut out)
        .write_image(img.as_raw(), 640, 360, image::ExtendedColorType::Rgba8)
        .expect("encoding a generated image cannot fail");
    out
}

// ---------------------------------------------------------------- baseline and reporting

/// One line per scene: what it is called, how big the picture is, whether it is the picture
/// that was there last time. The hash is the whole point of the line, because `same` means the
/// image does not need opening.
fn report(shot: &Shot, baseline: &BTreeMap<String, String>, cfg: &Config) -> bool {
    let status = match baseline.get(&shot.name) {
        _ if shot.volatile => "volatile",
        None => "new",
        Some(h) if h == &shot.hash => "same",
        Some(_) => "changed",
    };
    if cfg.json {
        println!(
            "{}",
            serde_json::json!({
                "scene": shot.name, "what": shot.what, "w": shot.w, "h": shot.h,
                "hash": shot.hash, "status": status, "path": shot.path, "moved_px": shot.moved,
            })
        );
    } else {
        let moved = match shot.moved {
            Some(n) => format!(" {n}px moved"),
            None => String::new(),
        };
        println!(
            "{:<20} {:>4}x{:<4} {} {:<8} {}{}",
            shot.name,
            shot.w,
            shot.h,
            &shot.hash[..8],
            status,
            shot.path.display(),
            moved
        );
    }
    if let Some(a) = &shot.ascii {
        print!("{a}");
    }
    let _ = std::io::stdout().flush();
    status != "same"
}

fn read_baseline(path: &Path) -> BTreeMap<String, String> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_baseline(path: &Path, shots: &[Shot], previous: &BTreeMap<String, String>) {
    let mut map = previous.clone();
    for s in shots.iter().filter(|s| !s.volatile) {
        map.insert(s.name.clone(), s.hash.clone());
    }
    if let Ok(text) = serde_json::to_string_pretty(&map) {
        let _ = std::fs::write(path, text);
    }
}

/// Nudge the event loop, and give up out loud rather than hanging.
///
/// Two failures, one thread. The nudge is for a compositor that has stopped asking for frames;
/// the deadline is for the case where nudging does not help, because a screenshot tool that
/// spins forever with no output is indistinguishable from a broken one. Scenes already
/// photographed have already been printed, so exiting here loses only the summary.
fn watchdog(ctx: &egui::Context, progress: Arc<Mutex<Instant>>, w: Watch) {
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(500));
            ctx.request_repaint();
            let idle = progress.lock().expect("no poisoned lock").elapsed();
            if idle > Duration::from_secs(w.timeout_s) {
                eprintln!(
                    "stalled: no scene captured in {}s (the compositor stopped asking this \
                     window for frames)",
                    w.timeout_s
                );
                // What has already been photographed is already reported, and the baseline
                // has to record it or the supervisor's next attempt calls every one of those
                // scenes changed.
                let shots = w.shots.lock().expect("no poisoned lock");
                if !w.check {
                    write_baseline(&w.baseline_path, &shots, &w.baseline);
                }
                std::process::exit(3);
            }
        }
    });
}

/// What the watchdog needs to leave the run in a reportable state before it gives up.
struct Watch {
    timeout_s: u64,
    check: bool,
    baseline_path: PathBuf,
    baseline: Arc<BTreeMap<String, String>>,
    shots: Arc<Mutex<Vec<Shot>>>,
}

/// Run the scenes in a child process, and restart it where it left off if it stalls.
///
/// A Wayland compositor stops sending frame callbacks to a surface it considers hidden, and
/// eframe runs no egui pass without one: the window is alive, the process spins, and nothing
/// advances. Nothing inside the process can talk it round, but a fresh window gets a fresh
/// configure and a fresh callback, so the recovery is a new process for the scenes that did
/// not land. Each attempt has to photograph something, or the run gives up rather than
/// restarting forever.
fn supervise(args: &[String], names: Vec<String>, check: bool) -> ExitCode {
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("cannot find my own binary: {e}");
            return ExitCode::from(2);
        }
    };
    let flags: Vec<&String> = {
        // Everything but the scene names, which the supervisor decides per attempt.
        let mut out = Vec::new();
        let mut takes_value = false;
        for a in args {
            if takes_value {
                out.push(a);
                takes_value = false;
                continue;
            }
            if a.starts_with('-') {
                out.push(a);
                takes_value = matches!(
                    a.as_str(),
                    "--out"
                        | "--url"
                        | "--keys"
                        | "--max-dim"
                        | "--settle"
                        | "--timeout"
                        | "--measure"
                        | "--theme"
                        | "--size"
                );
            }
        }
        out
    };

    let mut left = names;
    let mut moved: Vec<String> = Vec::new();
    let mut done = 0usize;
    for attempt in 0.. {
        if left.is_empty() {
            break;
        }
        if attempt > 4 {
            eprintln!(
                "giving up after five stalls, {} scene(s) unphotographed",
                left.len()
            );
            return ExitCode::from(3);
        }
        let mut cmd = std::process::Command::new(&exe);
        cmd.arg("--child")
            .args(&flags)
            .args(&left)
            .stdout(std::process::Stdio::piped());
        let mut proc = match cmd.spawn() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("cannot start the window: {e}");
                return ExitCode::from(2);
            }
        };
        let mut landed: Vec<String> = Vec::new();
        if let Some(out) = proc.stdout.take() {
            for line in std::io::BufReader::new(out).lines().map_while(Result::ok) {
                println!("{line}");
                let _ = std::io::stdout().flush();
                if let Some((name, status)) = parse_report(&line) {
                    landed.push(name.clone());
                    if status == "changed" || status == "new" {
                        moved.push(name);
                    }
                }
            }
        }
        let code = proc.wait().ok().and_then(|s| s.code()).unwrap_or(3);
        done += landed.len();
        left.retain(|n| !landed.contains(n));
        if left.is_empty() {
            break;
        }
        if landed.is_empty() {
            eprintln!("the window landed no scenes; stopping rather than restarting");
            return ExitCode::from(if code == 0 { 3 } else { code as u8 });
        }
        eprintln!("restarting for {} remaining scene(s)", left.len());
    }

    if moved.is_empty() {
        println!("{done} scene(s), none moved");
    } else {
        println!(
            "{done} scene(s), {} moved: {}",
            moved.len(),
            moved.join(" ")
        );
    }
    if check && !moved.is_empty() {
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

/// A child's report line back into (scene, status). Both output shapes put the name first and
/// the status where this can find it; anything else the child prints is passed through.
fn parse_report(line: &str) -> Option<(String, String)> {
    if let Some(rest) = line.strip_prefix('{') {
        let v: serde_json::Value = serde_json::from_str(&format!("{{{rest}")).ok()?;
        return Some((
            v.get("scene")?.as_str()?.to_owned(),
            v.get("status")?.as_str()?.to_owned(),
        ));
    }
    let mut f = line.split_whitespace();
    let name = f.next()?.to_owned();
    let _size = f.next()?;
    let _hash = f.next()?;
    let status = f.next()?;
    matches!(status, "new" | "same" | "changed" | "volatile").then_some((name, status.to_owned()))
}

// ---------------------------------------------------------------- main

fn main() -> ExitCode {
    let mut cfg = Config {
        out: PathBuf::from("shots"),
        size: [900.0, 800.0],
        max_dim: 900,
        settle_ms: 600,
        ascii: false,
        json: false,
        timeout_s: 60,
    };
    let mut wanted: Vec<String> = Vec::new();
    let mut all = false;
    let mut theme = Theme::Light;
    let mut measure: Option<f32> = None;
    let mut check = false;
    let mut list = false;
    let mut ad_hoc: Option<String> = None;
    let mut extra_keys: Option<String> = None;
    let mut child = false;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let value = |i: &mut usize| -> String {
            *i += 1;
            args.get(*i).cloned().unwrap_or_else(|| {
                eprintln!("{a} needs a value");
                std::process::exit(2);
            })
        };
        match a {
            "--list" => list = true,
            "--child" => child = true,
            "--all" => {
                all = true;
                wanted.clear();
            }
            "--check" => check = true,
            "--ascii" => cfg.ascii = true,
            "--json" => cfg.json = true,
            "--out" => cfg.out = PathBuf::from(value(&mut i)),
            "--url" => ad_hoc = Some(value(&mut i)),
            "--keys" => extra_keys = Some(value(&mut i)),
            "--max-dim" => cfg.max_dim = value(&mut i).parse().unwrap_or(cfg.max_dim),
            "--settle" => cfg.settle_ms = value(&mut i).parse().unwrap_or(cfg.settle_ms),
            "--timeout" => cfg.timeout_s = value(&mut i).parse().unwrap_or(cfg.timeout_s),
            "--measure" => measure = value(&mut i).parse().ok(),
            "--theme" => {
                let name = value(&mut i);
                theme = match name.as_str() {
                    "light" => Theme::Light,
                    "dark" => Theme::Dark,
                    "sepia" => Theme::Sepia,
                    "system" => Theme::System,
                    "contrast-light" => Theme::ContrastLight,
                    "contrast-dark" => Theme::ContrastDark,
                    // Not a fallback to Light. This used to be `_ => Theme::Light`, which meant
                    // `--theme contrast-dark` exited 0 and photographed Light: a tool whose
                    // whole job is catching a change nobody noticed cannot have a silent
                    // fallback in its own argument parser.
                    other => {
                        eprintln!("--theme: {other} is not a theme");
                        std::process::exit(2);
                    }
                }
            }
            "--size" => {
                let v = value(&mut i);
                let (w, h) = v.split_once(['x', 'X']).unwrap_or(("900", "800"));
                cfg.size = [w.parse().unwrap_or(900.0), h.parse().unwrap_or(800.0)];
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            s if s.starts_with('-') => {
                eprintln!("unknown flag: {s}\n{USAGE}");
                return ExitCode::from(2);
            }
            s => wanted.push(s.to_owned()),
        }
        i += 1;
    }

    if list {
        for s in catalog(0) {
            println!("{:<20} {}", s.name, s.what);
        }
        return ExitCode::SUCCESS;
    }

    if let Err(e) = std::fs::create_dir_all(&cfg.out) {
        eprintln!("cannot write to {}: {e}", cfg.out.display());
        return ExitCode::from(2);
    }
    if !child {
        // A bare invocation used to mean every scene, which is how a run nobody asked for
        // takes the display for minutes: `--all` cannot share a screen, because a window the
        // compositor stops sending frame callbacks to stops the run. The scene set is
        // therefore always explicit, and the expensive one has to be named.
        if wanted.is_empty() && !all && ad_hoc.is_none() {
            eprintln!(
                "name the scenes to photograph, or ask for --all.\n\
                 --all owns the display until it finishes and the window has to stay visible \
                 the whole time, so it is never what you get by default.\n\
                 {USAGE}"
            );
            return ExitCode::from(2);
        }
        let names: Vec<String> = if ad_hoc.is_some() {
            vec!["url".to_owned()]
        } else {
            let all = catalog(0);
            let names: Vec<String> = all.into_iter().map(|s| s.name).collect();
            if wanted.is_empty() {
                names
            } else {
                let unknown: Vec<&String> = wanted.iter().filter(|w| !names.contains(w)).collect();
                if !unknown.is_empty() {
                    eprintln!("no such scene: {unknown:?} (--list to see them)");
                    return ExitCode::from(2);
                }
                names.into_iter().filter(|n| wanted.contains(n)).collect()
            }
        };
        return supervise(&args, names, check);
    }
    // A scene that presses `d` or `[` marks the settings dirty and the reader saves them. Not
    // into the settings of whoever is running this.
    let scratch = cfg.out.join(".config");
    let _ = std::fs::create_dir_all(&scratch);
    // The bookmarks, unlike the settings, is cleared between runs. The catalog builds its own
    // through `Command::BookmarkPage`, so a file left behind by the previous run would leave the
    // bookmarks scene showing a list that grows every time the catalog is run.
    let _ = std::fs::remove_file(scratch.join("archive.json"));
    // Safety: before any thread that reads the environment exists.
    unsafe { std::env::set_var("HWW_CONFIG_DIR", &scratch) };

    let port = match serve() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("cannot bind the fixture server: {e}");
            return ExitCode::from(2);
        }
    };

    let mut scenes: Vec<Scene> = match &ad_hoc {
        // `{PORT}` reaches the fixture server, which is how the ad-hoc path is exercised
        // without asking the live web for anything.
        Some(target) => vec![Scene {
            settle_ms: 4000,
            volatile: true,
            ..scene(
                "url",
                "an ad-hoc target",
                vec![Step::Nav(target.replace("{PORT}", &port.to_string()))],
            )
        }],
        None => catalog(port),
    };
    if !wanted.is_empty() {
        scenes.retain(|s| wanted.iter().any(|w| w == &s.name));
        let missing: Vec<&String> = wanted
            .iter()
            .filter(|w| !scenes.iter().any(|s| &s.name == *w))
            .collect();
        if !missing.is_empty() {
            eprintln!("no such scene: {missing:?} (--list to see them)");
            return ExitCode::from(2);
        }
    }
    if let Some(keys) = &extra_keys {
        let steps = parse_keys(keys);
        for s in &mut scenes {
            s.steps.extend(steps.iter().map(clone_step));
        }
    }
    if scenes.is_empty() {
        eprintln!("nothing to shoot");
        return ExitCode::from(2);
    }

    let mut settings = Settings::default();
    settings.read.theme = theme;
    if let Some(m) = measure {
        settings.read.measure_chars = m;
    }

    let cfg = Arc::new(cfg);
    let baseline_path = cfg.out.join("baseline.json");
    let baseline = Arc::new(read_baseline(&baseline_path));
    let shots: Arc<Mutex<Vec<Shot>>> = Arc::new(Mutex::new(Vec::new()));
    let progress = Arc::new(Mutex::new(Instant::now()));
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("hww-shot")
            .with_inner_size(cfg.size)
            // A window an occluded compositor stops sending frame callbacks to is a state
            // machine that stops advancing, so the window that is being photographed stays on
            // top of whatever is running underneath it.
            .with_always_on_top()
            .with_app_id("hww"),
        ..Default::default()
    };
    let launch = (
        Arc::clone(&cfg),
        Arc::clone(&baseline),
        Arc::clone(&shots),
        Arc::clone(&progress),
    );
    let run = eframe::run_native(
        "hww-shot",
        options,
        Box::new(move |cc| {
            let (cfg, baseline, shots, progress) = launch;
            watchdog(
                &cc.egui_ctx,
                Arc::clone(&progress),
                Watch {
                    timeout_s: cfg.timeout_s,
                    check,
                    baseline_path: cfg.out.join("baseline.json"),
                    baseline: Arc::clone(&baseline),
                    shots: Arc::clone(&shots),
                },
            );
            let app = ReaderApp::new(
                cc,
                Launch {
                    start: None,
                    settings,
                    settings_note: None,
                    opts: hww::session::LoadOptions::default(),
                },
            )?;
            Ok(Box::new(Runner {
                app,
                scenes,
                scene: 0,
                step: 0,
                wait: 0,
                stage: Stage::Steps,
                cfg,
                baseline,
                shots,
                progress,
                last_frame: Instant::now(),
                frames: 0,
            }))
        }),
    );
    if let Err(e) = run {
        eprintln!("the window failed: {e}");
        return ExitCode::from(2);
    }

    // The tally and the exit code are the supervisor's: this process may be one attempt of
    // several, and only the supervisor has seen all of them.
    let shots = shots.lock().expect("no poisoned lock");
    if !check {
        write_baseline(&baseline_path, &shots, &baseline);
    }
    ExitCode::SUCCESS
}

fn clone_step(s: &Step) -> Step {
    match s {
        Step::Key(m, k) => Step::Key(*m, *k),
        Step::Type(t) => Step::Type(t.clone()),
        Step::Wait(n) => Step::Wait(*n),
        Step::Nav(u) => Step::Nav(u.clone()),
        Step::Follow(u) => Step::Follow(u.clone()),
        Step::ReadLater { href, title } => Step::ReadLater {
            href: href.clone(),
            title: title.clone(),
        },
        Step::Run(c) => Step::Run(*c),
        Step::Page {
            url,
            rewrite,
            body,
            tweak,
        } => Step::Page {
            url: url.clone(),
            rewrite: rewrite.clone(),
            body: body.clone(),
            tweak: *tweak,
        },
        Step::Fail { url, .. } => Step::Fail {
            url: url.clone(),
            error: LoadError::NotWebPage {
                content_type: "unclonable".to_owned(),
            },
        },
    }
}

/// `--keys "t wait:10 / 'quiet ctrl+l"`: keys as a reader would press them, `'` for text to
/// type, `wait:n` for frames to let something settle.
fn parse_keys(spec: &str) -> Vec<Step> {
    let mut out = Vec::new();
    for token in spec.split_whitespace() {
        if let Some(text) = token.strip_prefix('\'') {
            out.push(Step::Type(text.to_owned()));
            continue;
        }
        if let Some(n) = token.strip_prefix("wait:") {
            out.push(Step::Wait(n.parse().unwrap_or(10)));
            continue;
        }
        let mut mods = Modifiers::NONE;
        let mut name = token;
        loop {
            match name.split_once('+') {
                Some(("ctrl" | "cmd", rest)) => {
                    mods |= Modifiers::COMMAND;
                    name = rest;
                }
                Some(("shift", rest)) => {
                    mods |= Modifiers::SHIFT;
                    name = rest;
                }
                Some(("alt", rest)) => {
                    mods |= Modifiers::ALT;
                    name = rest;
                }
                _ => break,
            }
        }
        let key = match name {
            "?" => Some(Key::Questionmark),
            "/" => Some(Key::Slash),
            "[" => Some(Key::OpenBracket),
            "]" => Some(Key::CloseBracket),
            " " | "space" => Some(Key::Space),
            other => {
                let mut c = other.chars();
                let capitalized: String = c
                    .next()
                    .map(|f| f.to_ascii_uppercase().to_string() + c.as_str())
                    .unwrap_or_default();
                Key::from_name(&capitalized)
            }
        };
        match key {
            Some(k) => out.push(Step::Key(mods, k)),
            None => eprintln!("--keys: ignoring {token}"),
        }
        out.push(Step::Wait(1));
    }
    out
}

const USAGE: &str = "\
hww-shot: open the reader in a named state and photograph it.

    hww-shot [options] [scene ...]

    --list             the scene catalog, one line each
    --all              every scene: minutes, and the window must stay visible throughout
    --url URL          one ad-hoc scene: fetch URL for real
    --keys \"t wait:5 / 'quiet\"  extra steps, appended to every scene
    --theme light|dark|sepia|system|contrast-light|contrast-dark
                       (default light: system follows the desktop)
    --measure N        reading column, in characters
    --size WxH         window size in points (default 900x800)
    --max-dim N        longest edge of the saved PNG (default 900, 0 for native)
    --settle MS        milliseconds between the last step and the shutter (default 600)
    --timeout S        give up if no scene lands for S seconds (default 60, exit 3)
    --out DIR          where the PNGs and baseline.json go (default ./shots)
    --ascii            print a luminance sketch under each line
    --json             one JSON object per scene instead of a table
    --check            compare against the baseline, do not update it, exit 1 on a change
";
