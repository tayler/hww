//! What the reader has to say about a page, and how loud. **No egui types, no colours.**
//!
//! `src/ir.rs` keeps the page's *styling* out of the document. This module is the same fence
//! one layer out: it keeps the reader's own *wording* out of `ui/`, so the text hww puts on
//! screen is decided where `cargo test` can read it. That matters more than it looks. Two strings
//! in the old error screen contained `→`, which the faces egui embedded by default did not carry,
//! and prose review did not catch that both rendered as tofu. The former
//! `no_notice_contains_an_arrow` test is gone because `reader::ui::fonts` now embeds faces that
//! carry arrows in every role. What the episode leaves behind is the reason this module sits
//! outside `ui/` at all.
//!
//! # Severity, not colour
//!
//! [`Severity`] is how loud a notice is. `theme::notice_ink` turns a severity into paint, and
//! it lives under `ui/` because that is the only file in the crate where a `Color32` appears.
//! Splitting them is what lets the words be tested without a window and the palette be retuned
//! without touching a call site.
//!
//! # Two kinds, because there are two situations
//!
//! Notices divide on whether there is a page to read. A `LoadError` means there is not, so the reader
//! owns the viewport and gets a heading, prose, and actions. A 4xx that still carried an
//! article, or an extraction that came back nearly empty, means the page *is* there and the notice
//! is only annotating it; dressing that as an error screen would be a lie, because the prose is
//! readable immediately below. [`about_page`] returns the second kind and is empty for a page
//! that arrived clean, which is most of them.
//!
//! # Who did what
//!
//! Every sentence here names its actor as the grammatical subject: *hww* for what the reader
//! did (a built-in rewrite rule, the 5 MB cap), the *host* for what the server did (a redirect,
//! a 404). "Opened text.example.com instead of example.com" reads as a choice the browser made,
//! and it was; "text.example.com sent the request on to www.example.com" reads as the server's,
//! and it was. Neither hides behind a passive, and a reader deciding whether to trust the page
//! can tell which side of the wire each fact came from.
//!
//! # The shape the words are drawn in
//!
//! The reader no longer prints a `[hww]` mark or the severity word: a notice about a loaded
//! page is an **infobar** under the top chrome, in the form every browser shares (an icon, a
//! sentence, the actions as buttons, a close), and a failure is an **error page**. The severity
//! survives as the icon's shape and as [`Severity::word`], which is the icon's accessible name
//! and hover text, so a screen reader and a monochrome eye both still get it. `[image]` and
//! `[loading]` stay inline on purpose: they mark one thing at one place in the document.
//!
//! # Why this is not under `ui/`
//!
//! Two reasons. It is decidable without a `Context`, so the fast CI job that never compiles
//! egui tests it. And headless `--why` prints a bare `LoadError` today while the GUI has
//! considered wording for the same eight cases; [`failure`] stays in the testable layer if
//! another non-window caller needs it.

use crate::fetch::Truncation;
use crate::reader::pageinfo::Arrival;
use crate::session::{LoadError, Provenance};
use std::time::Duration;
use url::Url;

/// The extracted-text length below which a page is treated as having failed to extract.
///
/// Re-exported rather than restated: this *is* `html::extract`'s `<body>` fallback threshold,
/// and a page that took the fallback is exactly the page worth warning about. A copy of the
/// number here would let the two drift silently, with the reader quietly ceasing to warn.
pub use crate::ir::THIN_TEXT;

/// The deadline a document fetch is held to.
///
/// Re-exported for the same reason as [`THIN_TEXT`] just above: it is the denominator of
/// [`elapsed_fraction`], and a copy of the number here would let the progress rule go on
/// measuring against 15 seconds after `fetch` had moved to something else.
pub use crate::fetch::TIMEOUT;

/// The mark on a link whose page is being fetched.
///
/// A bracket, not a colour or a second underline. Everything the reader says *inside* the reading
/// column carries this convention (`[image]` is the other) because a bracket survives a
/// screenshot, a monochrome eye, and a theme nobody has designed yet, and because a coloured
/// underline on a link would be the reader borrowing the page's own idiom to speak in, which is
/// the one thing the column rule forbids. It is positional, which is what admits it inline at
/// all: it marks one link at one place, and a bar docked above the page cannot point.
pub const PENDING: &str = "[loading]";

/// What the reader says when a key asks for a picture the image setting has ruled out.
///
/// `i` and `Shift+I` are bound whatever the setting says, and the page under a declining
/// policy draws a dim `[image] not shown` rather than a control, so there is nothing on screen
/// for the key to be refused *by*. A keypress that does nothing and says nothing reads as a
/// broken binding; this says which setting decided, in the reader's own vocabulary, and it
/// lives out here with the rest of the wording so the fast job tests it.
pub const IMAGES_ARE_OFF: &str = "hww is set not to load images; change Images in Settings.";

/// Why a site mark is not drawn, when hww asked once and the site served nothing.
///
/// Never on screen as a sentence: neither `images::site_icon` nor `images::favicon` draws
/// anything but a ready texture, so a mark that is not there is a gap in a column and no words at
/// all. It is written here anyway rather than as a literal at the call site, because it is the
/// *reason* a `Failure` carries, every other one of those is the reader's words, and a string
/// that becomes visible the day either of those two grows a placeholder must not be the day
/// somebody notices it was never written for a reader.
pub const NO_SITE_ICON: &str = "this site served no icon at that address.";

/// What the reader says when a key asks for pictures the page has, but in a format this
/// renderer declines.
///
/// Distinct from "no images on this page", which is false and reads as a page hww saw
/// differently than the reader did, and from [`IMAGES_ARE_OFF`], which blames a setting the
/// reader can change. Nothing was contacted, so nothing may be named: this says why, and stops
/// there.
pub const IMAGES_ARE_DECLINED: &str =
    "the images on this page are in a format hww does not display.";

/// What the reader says when every picture left to ask for has already failed for good.
///
/// The same rule as [`IMAGES_ARE_DECLINED`] one step later: nothing will be contacted, so
/// nothing may be named, and "loading 3 image(s) from …" would be a claim about the network
/// that no request backs. Each of those pictures still carries its own reason where it sits;
/// this only answers why the key did nothing.
pub const IMAGES_ALL_FAILED: &str = "every image on this page has already failed to load.";

/// What the reader says when the two above are true of the same page, each of some of it.
///
/// Neither one alone may stand in for it. [`IMAGES_ARE_DECLINED`] would say a picture that was
/// asked for and refused was never opened, and [`IMAGES_ALL_FAILED`] would say one that hww
/// never requested had failed — a claim about the network with no request behind it, which is
/// the thing the whole set of these exists to avoid. Same rule: nothing was contacted by this
/// press, so nothing is named.
pub const IMAGES_ARE_DECLINED_OR_FAILED: &str =
    "some images here are in a format hww does not display; the rest have already failed.";

/// What the URL bar says when it is empty.
///
/// One line where there used to be a label and a hint. `https://…` described only half of what
/// Enter accepts now that words are a search, and a "URL" label in front of the field
/// contradicted the other half; the field is wide enough to say both itself. The wording is
/// Chrome's, deliberately: most readers have read it already, and a bar that behaves like every
/// other bar should not be asking them to learn a new phrasing for it.
pub const URL_BAR_HINT: &str = "Search or type a URL";

/// What a bare reload turns off, named after the key has done it.
///
/// Everything that reads a response as other than prose, in the order `session::load` consults
/// them, because a reader diagnosing a page needs to know which of them was in play. Search
/// joined the list when result pages became readable and the feed reader joined it next:
/// `LoadOptions::BARE` is the documented way to see what a host actually sent, so a message
/// that named three of the four would leave the reader wondering why the entries stopped being
/// entries. The last is not a table — a feed is recognised from its own first bytes rather than
/// from a host — and it is here because what it turns off is the same kind of thing.
///
/// `menu::HELP` says the same four; `the_bare_reload_says_the_same_thing_twice` holds them
/// together.
pub const RELOADED_BARE: &str =
    "reloaded bare: no rewrite rule, no site profile, no search shape, no feed reader";

/// The bookmarks' masthead, and so also its window title in the outline and the status strip.
///
/// "Your", because the whole doctrine of `reader::archive` is that this holds what the reader
/// named and nothing hww decided to collect. A bare "Bookmarks" would read as a catalogue the
/// application maintains. The noun is the one every other browser uses, which is the point of
/// it: a second browser gets to be unusual about what it does, not about what things are called.
pub const BOOKMARKS_TITLE: &str = "Your bookmarks";

/// The one line an empty bookmarks page shows.
///
/// A function and not a const, because it names a key and the key is `cfg`-selected: a Mac
/// reader told to press Ctrl+D is told to press a key nothing binds. Not an error page, either:
/// see `archive::bookmarks_document` for why an engine that found nothing and a reader who has
/// not saved anything yet are different situations.
pub fn bookmarks_are_empty() -> String {
    format!(
        "No bookmarks yet. {} saves the page you are reading, and hww never bookmarks a page you \
         did not ask it to.",
        crate::reader::menu::BOOKMARK_PAGE
    )
}

/// The reading list's masthead, and so also its window title and its line in the status strip.
///
/// "Your", for [`BOOKMARKS_TITLE`]'s reason. "Read next" rather than "Reading list" because the
/// list is a queue and the title is the instruction: these are the things the reader said they
/// would come back to.
pub const READING_LIST_TITLE: &str = "Read next";

/// The one line an empty reading list shows, which is two different sentences.
///
/// Two, for [`history_is_empty`]'s reason: an empty list either means nothing has been marked yet
/// or means hww has been told not to keep one, and a reader who switched it off would otherwise
/// meet a blank page that looks like a bug.
///
/// The first arm has a job the bookmarks' and the history's do not, and it is why this is the
/// longest of the three. Bookmarking a page and opening the history act on the page in front of
/// the reader, so their empty
/// states can assume the reader already knows what to press. This list is filled from a *link*,
/// which is a thing the reader has to be told how to aim at — so the sentence names both routes,
/// the key and the right-click, and a list that can only be filled by a gesture nobody mentioned
/// is a list that stays empty.
pub fn reading_list_is_empty(marking: bool) -> String {
    if marking {
        format!(
            "Nothing to read next yet. Tab to a link and press {}, or right-click it and \
             choose Read later. hww keeps it here until you open it.",
            crate::reader::menu::READ_LATER
        )
    } else {
        "hww is set not to keep a reading list, so this stays empty. Turn on Keep a reading \
         list in Settings › Reading list to start one."
            .to_owned()
    }
}

/// What the reader is told when a link goes on the reading list.
///
/// It names the link for [`kept`]'s reason, and here the reason is sharper: the reader was aiming
/// at one of many links on a page, and the only confirmation that they hit the right one is being
/// told which.
pub fn read_later(label: &str) -> String {
    format!("hww will keep {label} on your reading list; l opens it")
}

/// What the reader is told when a link comes off the reading list.
pub const READ_LATER_REMOVED: &str = "hww took that link off your reading list.";

/// The store's answer when the link is already listed. Not a failure, for [`ALREADY_BOOKMARKED`]'s
/// reason, and pre-empted by the same toggle.
pub const ALREADY_READ_LATER: &str = "that link is already on your reading list.";

/// A link whose address is longer than `archive::MAX_URL`. [`ADDRESS_TOO_LONG`] one tenant over,
/// and a separate string because it has to name the right list.
pub const ADDRESS_TOO_LONG_TO_READ_LATER: &str =
    "that link's address is too long for hww to write down; it was not added.";

/// A link this file will not write down whatever the switch says: not a web address, or one
/// carrying a credential. `Archive::add_next`'s own answer, which `ReaderApp` normally pre-empts
/// with the better sentence `session::classify_link` gives.
pub const CANNOT_READ_LATER: &str = "hww only puts web links on your reading list.";

/// The reading list is at its cap. It says the number and says nothing was dropped, for
/// [`bookmarks_are_full`]'s reason.
pub fn reading_list_is_full(cap: usize) -> String {
    format!(
        "your reading list holds {cap} links, which is all hww keeps; nothing was removed to \
         make room."
    )
}

/// A link was marked while the reading list is switched off. Said rather than swallowed, for
/// [`BOOKMARKING_IS_OFF`]'s reason.
pub const READING_LIST_IS_OFF: &str =
    "hww is set not to keep a reading list; change Keep a reading list in Settings › Reading list.";

/// What `Shift+L` says when nothing on the page is focused.
///
/// A function, because it names two `cfg`-selected keys. It does not stop at reporting the
/// failure: the near-miss this catches is a reader who meant to save the page they are on, so it
/// names the key that does that rather than leaving them to guess which of the two they wanted.
pub fn no_link_to_read_later() -> String {
    format!(
        "hww has no link to read later: none is focused. Tab picks one, and {} keeps the page \
         you are reading.",
        crate::reader::menu::BOOKMARK_PAGE
    )
}

/// What "forget the reading list" reports. The count, for [`bookmarks_forgotten`]'s reason.
pub fn reading_list_forgotten(n: usize) -> String {
    match n {
        0 => "your reading list was already empty.".to_owned(),
        1 => "hww forgot the one link on your reading list.".to_owned(),
        n => format!("hww forgot all {n} links on your reading list."),
    }
}

/// The label on the control that takes one link off the reading list, drawn at the end of every
/// row of `hww:reading-list`.
///
/// Unbracketed, because it is drawn as a framed button rather than as one of the page's marks.
/// Brackets are how a mark set in the page's own text says it is hww's — `[image]`, `[Video]`,
/// `notice::PENDING` — and a control that already has a border around it does not need the
/// typographic version of one. Lower case, like every button in the settings panel.
///
/// A control at all, rather than leaving the key to do it, because this is the one view whose
/// rows are things the reader chose and may now unchoose, and the key that removes one is
/// `Shift+L` — the same key that added it, which is a toggle nobody arriving here would guess at.
/// What it presses is a door that removes and never adds; see `ReaderApp::forget_next_link`.
pub const READING_LIST_REMOVE: &str = "remove";

/// The label on the reading list's own empty-it control, drawn under the masthead whenever there
/// is anything to empty.
///
/// Shorter than the panel's "forget the reading list" because the page under it has already said
/// which list this is; a button that repeats its own masthead reads as a button that must
/// therefore do something else.
pub const READING_LIST_FORGET_ALL: &str = "forget all";

/// What that control says on hover, and what the panel's button says: one sentence, because they
/// are one act reached from two places.
///
/// It names what survives, for the reason the panel has three buttons instead of one: a reader
/// clearing the links they lined up is not asking to lose the pages they kept or the trail hww
/// wrote down.
pub const READING_LIST_FORGET_ALL_HOVER: &str = "Remove every link you marked to read later. This cannot be undone, and it leaves the pages \
     you have kept and your history alone.";

/// The history's masthead, and so also its window title and its line in the status strip.
///
/// "Your", for the reason [`BOOKMARKS_TITLE`] is: hww wrote this list, but it is a record of the
/// reader's own reading and calling it anything else would make it sound like the application's
/// property.
pub const HISTORY_TITLE: &str = "Pages you have visited";

/// The one line an empty history shows, which is two different sentences.
///
/// An empty list means one of two things, and they are not the same news: nothing has been read
/// yet, or hww has been told not to write any of it down. A reader who switched recording off
/// and later opened this page would otherwise be shown a blank list that looks exactly like a
/// bug. It names the setting rather than a key, because there is no key for the switch.
pub fn history_is_empty(recording: bool) -> String {
    if recording {
        "No pages yet. hww writes down each page it draws for you, and this is where they \
         appear; you can switch that off in Settings › History."
            .to_owned()
    } else {
        "hww is set not to remember the pages you visit, so this list stays empty. Turn on \
         Remember pages you visit in Settings › History to start it."
            .to_owned()
    }
}

/// What "forget history" reports. The count, for the reason [`bookmarks_forgotten`] gives: a button
/// whose effect is invisible is a button nobody trusts.
pub fn history_forgotten(n: usize) -> String {
    match n {
        0 => "your history was already empty.".to_owned(),
        1 => "hww forgot the one page in your history.".to_owned(),
        n => format!("hww forgot all {n} pages you had visited."),
    }
}

/// What the reader is told when a page is kept.
///
/// It names the page rather than saying "kept", because `Ctrl+D` is one keystroke away from
/// `Ctrl+F` and a reader who hit the wrong one deserves to see which page just went in.
pub fn bookmarked(title: &str) -> String {
    format!(
        "hww bookmarked {title}; {} opens your bookmarks",
        crate::reader::menu::BOOKMARKS
    )
}

/// The store's answer when the page is already there. Not a failure: the page is bookmarked,
/// which is what was wanted.
///
/// `ReaderApp::bookmark_page` pre-empts it, because the menu row is a check mark and a second press
/// removes rather than repeats. This is `Archive::bookmark`'s own answer, and the exhaustive arm that
/// receives it says it in the reader's words rather than inventing a sentence under `ui/`.
pub const ALREADY_BOOKMARKED: &str = "hww has this page bookmarked already.";

/// What a key that cannot keep anything says: on the splash, on an error screen, and on hww's own
/// pages, where the menu row is greyed out instead.
///
/// `Ctrl+D` is bound whatever is on screen, and a press that does nothing and says nothing reads
/// as a broken binding — the same argument as [`IMAGES_ARE_OFF`], one key over.
pub const NOTHING_TO_BOOKMARK: &str = "there is nothing here for hww to bookmark.";

/// What the reader is told when a page stops being bookmarked.
pub const FORGOTTEN: &str = "hww removed this page from your bookmarks.";

/// The bookmarks are at their cap. It says the number, and it says that nothing was dropped,
/// because the alternative every cache takes is to evict quietly and this one deliberately does
/// not.
pub fn bookmarks_are_full(cap: usize) -> String {
    format!("you have {cap} bookmarks, which is all hww holds; nothing was removed to make room.")
}

/// The archive could not be written. Rare, and worth saying out loud rather than swallowing:
/// something hww was going to remember was not written down.
///
/// It names neither tenant. One file holds both, the failure is the same failure, and the two
/// callers are a keypress and a page arriving — a sentence about the bookmarks after a navigation
/// would be a wrong answer to a question nobody asked.
pub fn archive_not_saved(reason: &str) -> String {
    format!("hww could not save what it remembers: {reason}")
}

/// What a forget button says when the file it has just written was one hww could not read.
///
/// `bookmarks_forgotten(0)` would answer "your bookmarks was already empty", which is what hww saw
/// and not what happened: the file was there, it could not be parsed, and pressing this replaced
/// it. A count is the wrong sentence about a file whose contents were never known.
pub const UNREADABLE_FILE_REPLACED: &str =
    "hww could not read that file when it started; it has now been replaced by what hww holds.";

/// What the settings panel says under the Bookmarks and History groups when there is no
/// configuration directory at all — no `HWW_CONFIG_DIR`, no `HOME`, no `XDG_CONFIG_HOME`.
///
/// The place the file's path would otherwise be printed. A group that named no file at all
/// would leave two switches promising to remember things and no surface saying where, or that
/// nothing is being written; `archive::save` refuses in that state and says so on the keypress,
/// and this is the standing half of the same disclosure.
pub const NO_ARCHIVE_PATH: &str = "Not saved: hww has no configuration directory to write to.";

/// The same sentence for the site marks, under the two groups whose lists draw a column.
///
/// Its own string rather than [`NO_ARCHIVE_PATH`] repeated, because the two say different things
/// about what is lost: with no directory the archive stops remembering pages the reader chose,
/// while the marks merely go on being fetched every time a list opens, which is what hww did
/// before this store existed and is a smaller thing to be told.
pub const NO_ICON_PATH: &str = "Site icons are fetched each time: there is nowhere to keep them.";

/// A keep was asked for on a page whose address is longer than `archive::MAX_URL`.
///
/// Said rather than swallowed, for [`BOOKMARKING_IS_OFF`]'s reason: the key is bound, the reader
/// pressed it, and a press that did nothing and said nothing reads as a broken binding. It names
/// the address as the thing that was refused, because nothing about the page is wrong and
/// reaching it by a shorter link would work.
pub const ADDRESS_TOO_LONG: &str =
    "that page's address is too long for hww to write down; it was not kept.";

/// A keep was asked for while keeping is switched off. The key is bound whatever the setting
/// says, so a press that did nothing and said nothing would read as a broken binding — the same
/// argument as [`IMAGES_ARE_OFF`], one setting over.
pub const BOOKMARKING_IS_OFF: &str =
    "hww is set not to keep bookmarks; change Keep bookmarks in Settings › Bookmarks.";

/// What "forget everything" reports. The count, because a button whose effect is invisible is a
/// button nobody trusts, and no bookmarks after it looks identical to none before.
pub fn bookmarks_forgotten(n: usize) -> String {
    match n {
        0 => "hww found no bookmarks to forget.".to_owned(),
        1 => "hww forgot your one bookmark.".to_owned(),
        n => format!("hww forgot all {n} of your bookmarks."),
    }
}

/// How long a transient status stays up: "Copied URL", "loading 2 images from …", "no link
/// focused". The Material range is four to ten seconds; six, because the one message in the set
/// that matters is the disclosure of which hosts `I` is about to contact, and a reader looking
/// at the page rather than the pill needs a moment to notice it. It used to be twelve in the
/// status strip, where a line of dim text needs longer to be found than a pill does.
pub const TOAST_FOR: Duration = Duration::from_secs(6);

/// How loud a notice is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Information: something the reader did on the page's behalf and should own up to. An
    /// information bar; nothing is wrong.
    Quiet,
    /// Something about the page the reader would otherwise mis-read. A caution bar over a
    /// readable page.
    Caution,
    /// There is no page. An error page, which owns the viewport.
    Failure,
}

impl Severity {
    /// The severity, as a word: the accessible name and hover text of the painted icon.
    ///
    /// Colour is the obvious way to say how loud a notice is and the one that fails quietest: a
    /// hue can be missed, screenshotted into greyscale, or read by an eye that does not separate
    /// rust from brown. The icon's *shape* carries it for a sighted reader (a triangle, a circled
    /// `i`, a crossed circle); this word carries it for a screen reader, which is told nothing
    /// by a shape, and for the pointer that rests on it.
    ///
    /// Wording, so it lives here rather than in `ui/` for the same reason every other notice
    /// string does: the fast CI job never compiles egui, and this is where a test can read it.
    pub fn word(self) -> &'static str {
        match self {
            Severity::Quiet => "note",
            Severity::Caution => "caution",
            Severity::Failure => "failure",
        }
    }
}

/// What a notice offers to do about itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Retry,
    RetryWithoutRewrite,
    /// Open the settings panel. On a search that was refused or came back empty, changing the
    /// engine is the remedy, and it is two keystrokes away from a page that cannot say so.
    OpenSettings,
    CopyUrl,
}

impl Button {
    /// "Reload", which is the word on the button every browser puts on this page, rather than
    /// "Retry", which is the word in the code.
    pub fn label(self) -> &'static str {
        match self {
            Button::Retry => "Reload",
            Button::RetryWithoutRewrite => "Reload without rewrite",
            Button::OpenSettings => "Choose another engine",
            Button::CopyUrl => "Copy URL",
        }
    }
}

/// One thing the reader has to say, with no idea how it will be drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub severity: Severity,
    /// Failures only. A caution is one line and a heading would inflate it into a screen.
    pub heading: Option<&'static str>,
    pub body: String,
    /// Set dimmer under the body: the URL, the redirect chain.
    pub detail: Vec<String>,
    pub actions: &'static [Button],
    /// Failures only: the short name of what went wrong, the way an error page prints an
    /// `ERR_` code under the prose. For the reader who files a bug, and for nobody else.
    pub code: Option<&'static str>,
}

impl Notice {
    fn new(severity: Severity, body: impl Into<String>) -> Self {
        Self {
            severity,
            heading: None,
            body: body.into(),
            detail: Vec::new(),
            actions: &[],
            code: None,
        }
    }

    /// What a bar is dismissed by: its own words. A notice is rebuilt from the provenance every
    /// frame, so the identity has to be something the words themselves carry, and two notices
    /// with the same words about the same page are the same remark.
    pub fn key(&self) -> &str {
        &self.body
    }
}

/// A built-in rewrite rule was applied: the reader asked a different host than the one named.
///
/// An information bar and not a caution, because nothing is wrong: the rule is compiled in,
/// cited, and reported. But it is hww's doing and not the site's, and the bar says so in those
/// words, with the one action a reader might want beside it. The status strip still prints the
/// terse version before the request goes out (charter point 4); this is the readable one once
/// the page is up, and it lasts as long as the page does.
pub fn rewrite(prov: &Provenance) -> Option<Notice> {
    let to = prov.rewritten_to.as_ref()?;
    let aimed = to.host_str().unwrap_or("another host");
    let asked = prov.requested.host_str().unwrap_or("the host you named");
    let mut n = Notice::new(
        Severity::Quiet,
        format!(
            "hww asked {aimed} for this page instead of {asked}, by a built-in rule for this site."
        ),
    );
    n.actions = &[Button::RetryWithoutRewrite];
    Some(n)
}

/// Everything worth saying *about* a page that loaded, loudest first.
///
/// Empty for a page that arrived clean, so the column costs nothing on the pages that are fine.
/// Takes no fragment argument: an unresolved `#fragment` is a navigation event, not a fact
/// about the page's content, and it is reported once in the status strip.
///
/// The first two are here rather than in the status strip because neither is accounting. A
/// dead rule means the article on screen may not be the article asked for, and a truncated
/// body means it stops early with nothing to say so: both are things the reader would
/// otherwise mis-read, which is what [`Severity::Caution`] is for. They were a dim fragment at
/// the right-hand end of a truncated strip row and a twelve-second flash respectively, so the
/// first was often not drawn at all and the second could not be recalled once it faded.
pub fn about_page(prov: &Provenance, text_len: usize, arrival: Arrival) -> Vec<Notice> {
    let mut out = Vec::new();
    // A page hww built has nothing to be cautioned about: no rule chose it, no body was cut, no
    // server answered, and its length is whatever the reader has kept. Without this, a two-item
    // bookmarks trips the thin-extraction caution and hww tells the reader their own saved list
    // "may need JavaScript to show its content".
    //
    // Passed in rather than left to `app.rs` to skip the call, for the reason `loading` states
    // outright: the caller is `app.rs`, the fast CI job never compiles egui, and a decision made
    // there is a decision no test reads.
    if arrival == Arrival::Built {
        return out;
    }
    if prov.rule_appears_dead {
        // Not an error: the fetch succeeded. But it succeeded somewhere else, and no rewrite
        // is ever retried, so this is the whole early-warning system.
        let aimed = prov
            .rewritten_to
            .as_ref()
            .and_then(Url::host_str)
            .unwrap_or("the rewritten host");
        let landed = prov.final_url.host_str().unwrap_or("somewhere else");
        let mut n = Notice::new(
            Severity::Caution,
            format!(
                "{aimed} sent the request on to {landed}. The rule that chose {aimed} appears \
                 dead, so this may not be the page you asked for."
            ),
        );
        n.detail = vec![format!("asked for {}", prov.requested)];
        out.push(n);
    }
    if let Some(body) = truncation_body(prov.truncation) {
        out.push(Notice::new(Severity::Caution, body));
    }
    if prov.status >= 400 {
        // Many 404s are readable and many paywalls answer 403 with the article still in the
        // markup, so this is a remark *over* the content, never instead of it.
        let host = prov.final_url.host_str().unwrap_or("The server");
        out.push(Notice::new(
            Severity::Caution,
            format!(
                "{host} answered {}. Showing what it sent anyway.",
                prov.status
            ),
        ));
    }
    // Not on a feed. `ir::block_text_len` counts a `Block::Figure` as its caption alone, so a
    // feed whose entries are each a single picture scores under the floor having parsed
    // perfectly, and this caution would tell its reader that a page hww read correctly needs
    // JavaScript. The IR is not the place to fix that — text length has one currency — so the
    // pipeline says the page was a feed and the caution reads `prov.feed`.
    if text_len < THIN_TEXT && prov.feed.is_none() {
        out.push(Notice::new(
            Severity::Caution,
            "hww found little text on this page, which may need JavaScript to show its content.",
        ));
    }
    out
}

/// The bar prose for a body that did not arrive whole.
///
/// One string per variant, hedging exactly where the evidence hedges: `len >= cap` alone
/// cannot tell a body that was cut from one that happened to end on the boundary, so
/// `MaybeAtCap` says "may" and `AtCap` does not. The cap and the clock are hww's, and the
/// sentences say so. The terser phrasing for the page-info panel is
/// [`crate::reader::pageinfo`]'s; this one is the alarm, that one is the record.
fn truncation_body(t: Truncation) -> Option<String> {
    Some(match t {
        Truncation::Complete => return None,
        Truncation::AtCap(n) => format!(
            "hww stopped reading at its {} MB cap. The rest of this page was not fetched.",
            n / 1_000_000
        ),
        Truncation::MaybeAtCap(n) => format!(
            "hww may have cut this page short at its {} MB cap.",
            n / 1_000_000
        ),
        Truncation::Timeout(d) => format!(
            "hww stopped waiting after {}s with the body still arriving. This article is \
             incomplete.",
            d.as_secs()
        ),
        Truncation::Incomplete(_) => {
            "hww lost the connection partway. This article is incomplete.".to_owned()
        }
    })
}

/// Whether the document carries a run in the RTL blocks. Public so the page-info panel can
/// keep the record after the caution is closed: dismissing the bar must not be the way the
/// fact disappears.
pub fn carries_rtl(doc: &crate::ir::Document) -> bool {
    fn has_rtl(s: &str) -> bool {
        s.chars().any(|c| ('\u{0590}'..='\u{08FF}').contains(&c))
    }
    /// The inline tree, scanned rather than flattened.
    ///
    /// This used to be `has_rtl(&ir::plain_text(inlines))`, which builds a `String` per run to
    /// look at its characters and throw it away. The answer is the same and the walk is the
    /// same; only the buffer is gone. An image's alt text and a `Break`'s newline are what
    /// `plain_text` would have put in it, so the alt is scanned and the newline is not a
    /// right-to-left character.
    fn inlines_have_rtl(inlines: &[crate::ir::Inline]) -> bool {
        use crate::ir::Inline;
        inlines.iter().any(|i| match i {
            Inline::Text(t) | Inline::Code(t) => has_rtl(t),
            Inline::Emph(v) | Inline::Strong(v) => inlines_have_rtl(v),
            Inline::Link { inlines, .. } => inlines_have_rtl(inlines),
            Inline::Image(img) => img.alt.as_deref().is_some_and(has_rtl),
            Inline::Break => false,
        })
    }
    fn blocks_have_rtl(blocks: &[crate::ir::Block]) -> bool {
        use crate::ir::Block;
        blocks.iter().any(|b| match b {
            Block::Heading { inlines, .. } | Block::Paragraph(inlines) => inlines_have_rtl(inlines),
            Block::List { items, .. } => items.iter().any(|i| blocks_have_rtl(i)),
            Block::Quote { blocks, .. } => blocks_have_rtl(blocks),
            Block::Code { text, .. } => has_rtl(text),
            Block::Figure { caption, .. } => caption.as_deref().is_some_and(inlines_have_rtl),
            Block::Table { headers, rows } => {
                headers.iter().any(|c| inlines_have_rtl(c))
                    || rows.iter().flatten().any(|c| inlines_have_rtl(c))
            }
            Block::Thread(cs) => cs.iter().any(|c| blocks_have_rtl(&c.blocks)),
            Block::Entries(es) => es
                .iter()
                .any(|e| inlines_have_rtl(&e.title) || blocks_have_rtl(&e.summary)),
            Block::Rule | Block::Embed { .. } => false,
        })
    }
    doc.title.as_deref().is_some_and(has_rtl) || blocks_have_rtl(&doc.blocks)
}

/// A page carrying right-to-left text. epaint shapes but does not reorder, so Hebrew and
/// Arabic lay out in logical order, backwards, or as tofu (the tail face is chosen to carry no
/// RTL script so the failure is visible rather than plausible). The honest completion of that
/// choice is to say so. `U+0590..=U+08FF` is Hebrew, Arabic, Syriac, Thaana, NKo, Samaritan,
/// Mandaic, and Arabic Supplement/Extended-A: the RTL blocks in one span.
pub fn rtl(doc: &crate::ir::Document) -> Option<Notice> {
    carries_rtl(doc).then(rtl_notice)
}

/// The remark [`rtl`] makes, for a caller that has already asked [`carries_rtl`].
///
/// The reader asks it once per page, at `ReaderApp::settle`, rather than once per frame: the
/// walk allocates the plain text of every run it looks at and stops at the first right-to-left
/// character, so the page that pays for all of it is the page that has none.
pub fn rtl_notice() -> Notice {
    Notice::new(
        Severity::Caution,
        "hww cannot lay out the right-to-left text on this page: a Hebrew or Arabic run \
         is drawn in the wrong order, or as missing glyphs."
            .to_owned(),
    )
}

/// The screen a failed navigation gets.
///
/// Each error gets its own words and its own actions: a wrong-but-confident diagnosis is the
/// worst kind, because it blames the site.
pub fn failure(error: &LoadError, url: &Url) -> Notice {
    use crate::fetch::FetchError;

    let (heading, body, actions, code): (&'static str, String, &'static [Button], &'static str) =
        match error {
            LoadError::Fetch(FetchError::LikelyBlocked) => (
                "This site did not answer.",
                // Phase 0 measured bot detection failing as a silent hang rather than a 403.
                "The request timed out with no reply. On a document fetch that is usually bot \
             detection failing silently rather than a network fault."
                    .to_owned(),
                &[Button::Retry, Button::RetryWithoutRewrite, Button::CopyUrl],
                "LikelyBlocked",
            ),
            LoadError::Fetch(FetchError::Timeout(d)) => (
                "Timed out.",
                format!(
                    "No reply within {}s. That is bandwidth rather than a block.",
                    d.as_secs()
                ),
                // No retry-without-rewrite: bandwidth is not a rewrite rule's fault.
                &[Button::Retry, Button::CopyUrl],
                "Timeout",
            ),
            LoadError::Fetch(FetchError::SchemeDowngrade(to)) => (
                "Refused an https -> http downgrade.",
                format!(
                    "A redirect tried to send this request to {} without encryption. \
                 hww does not follow that.",
                    to.host_str().unwrap_or("an unnamed host")
                ),
                // No retry: refusing is correct, and a bypass would be a policy hole.
                &[Button::CopyUrl],
                "SchemeDowngrade",
            ),
            LoadError::Fetch(FetchError::TooManyRedirects(_)) => (
                "Too many redirects.",
                "The request bounced past the redirect limit. A loop like this is usually a \
             consent wall."
                    .to_owned(),
                &[Button::Retry, Button::RetryWithoutRewrite, Button::CopyUrl],
                "TooManyRedirects",
            ),
            LoadError::Fetch(FetchError::RedirectWithoutLocation(status)) => (
                "The server sent a broken redirect.",
                format!(
                    "It answered {status}, which means \"go somewhere else\", but did not say \
                 where. There is nothing to follow."
                ),
                &[Button::Retry, Button::CopyUrl],
                "RedirectWithoutLocation",
            ),
            LoadError::Fetch(FetchError::BadLocation(to)) => (
                "The server redirected somewhere unusable.",
                format!("It pointed at {to}, which is not an address hww can fetch."),
                &[Button::Retry, Button::CopyUrl],
                "BadLocation",
            ),
            // Answered promptly, and not with results: a puzzle, or a countdown. Saying "did
            // not answer" here would be the confident wrong diagnosis this function exists to
            // avoid, and it would send the reader to reload, which makes a rate limit worse.
            LoadError::SearchBlocked { engine } => (
                "The search did not run.",
                format!(
                    "{engine} replied with a check or a wait rather than results, which usually \
                     follows several searches in quick succession. hww cannot answer either \
                     one: there is no JavaScript here to run it and no cookie to remember it. \
                     Waiting a moment often clears it, and another engine clears it now."
                ),
                &[Button::OpenSettings, Button::Retry, Button::CopyUrl],
                "SearchBlocked",
            ),
            // Not a failure of anything. The reading column carries page content only, so the
            // absence of content has to be said here rather than drawn as a page.
            LoadError::SearchEmpty { engine, terms } => (
                "No results.",
                format!("{engine} found nothing for {terms}."),
                &[Button::OpenSettings, Button::CopyUrl],
                "SearchEmpty",
            ),
            // Parsed, and carried nothing. Same shape as `SearchEmpty` and for the same reason:
            // the reading column holds page content only, so the absence of content is said
            // here rather than drawn as an empty run of entries.
            LoadError::FeedEmpty { title } => (
                "This feed is empty.",
                format!(
                    "{title} is a feed hww can read, and it listed no entries. Either nothing \
                     has been posted to it or everything has been taken down."
                ),
                &[Button::Retry, Button::CopyUrl],
                "FeedEmpty",
            ),
            // It said it was a feed and would not parse. Falling through to the extractor
            // instead would draw a near-blank page under a caution blaming JavaScript, which is
            // the confident wrong diagnosis in its purest form: the reason hww reads feeds with
            // an XML parser at all is that the HTML extractor gets nothing out of them.
            // Two documents that fail at the same parser call for opposite reasons, and the
            // difference is whose fault it is. A feed hww cut at the transfer cap or the read
            // clock ends mid-element and cannot parse; telling that reader a generator is
            // broken and a reload will not help is a confident wrong diagnosis about the one
            // party who did nothing wrong, and it is the reload that actually stands a chance.
            // Its own heading and its own code, because it is its own failure.
            LoadError::FeedUnreadable {
                truncated: true, ..
            } => (
                "This feed arrived incomplete.",
                "hww reads feeds with an XML parser, which needs the whole document. This one \
                 stopped arriving partway, so it breaks off mid-element and none of it can be \
                 read. That cut is hww's own limit rather than a fault in the feed, and \
                 reloading may well get all of it."
                    .to_owned(),
                &[Button::Retry, Button::CopyUrl],
                "FeedTruncated",
            ),
            LoadError::FeedUnreadable { .. } => (
                "This feed is not well-formed.",
                "hww reads feeds with an XML parser, which is strict where an HTML parser \
                 guesses. Something in this document is not valid XML, so none of it can be \
                 read. That is the publisher's generator rather than anything here, and \
                 reloading will not change it until they fix it."
                    .to_owned(),
                // Reload, because the fix is a redeploy at the other end and a reader who comes
                // back to it wants one press. Not `RetryWithoutRewrite`: no rewrite rule chose
                // this document's contents.
                &[Button::Retry, Button::CopyUrl],
                "FeedUnreadable",
            ),
            LoadError::NotWebPage { content_type } => (
                "Not a web page.",
                format!("The server sent {content_type}, which hww has no reader for."),
                &[Button::CopyUrl],
                "NotWebPage",
            ),
            // Every remaining transport failure, and the arm that keeps this match exhaustive.
            // It was a bare `other =>`, which meant a new `LoadError` variant compiled straight
            // onto the generic transport screen with the code `"Transport"` and a Retry button:
            // the confident wrong diagnosis this function exists to prevent, arriving silently.
            // `every_failure_gets_its_own_words` walks a hand-written list rather than the enum,
            // so it would not have caught it either. Naming `Fetch` here makes the next variant
            // a compile error instead.
            LoadError::Fetch(other) => (
                "The request failed.",
                other.to_string(),
                &[Button::Retry, Button::CopyUrl],
                "Transport",
            ),
        };

    let mut detail = vec![url.to_string()];
    if let LoadError::Fetch(FetchError::TooManyRedirects(hops)) = error {
        // A redirect loop is usually a consent wall, and the chain is what shows it.
        detail.extend(hops.iter().map(|h| format!("  -> {h}")));
    }
    if let LoadError::FeedUnreadable { why, .. } = error {
        // The parser's own message and position: which entity, which prefix, which line. It is
        // publisher-controlled text, capped by `feed::read` before it gets here.
        detail.push(why.clone());
    }

    Notice {
        severity: Severity::Failure,
        heading: Some(heading),
        body,
        detail,
        actions,
        code: Some(code),
    }
}

/// The idle screen: the one screen where the reader is the page.
///
/// A wordmark, one line of what this is, and the two keys that matter, each with its meaning.
/// The waveform that sits beside the wordmark is paint, not wording, so it lives in
/// `ui::notice_ui` with the rest of how this screen is drawn. Not a notice: nothing has
/// happened yet, so there is nothing to report, and a Quiet band that said "press Ctrl+L" was
/// the reader describing its own chrome in the column. The URL bar is open and focused over
/// it (`ReaderApp::new`), which is where the typing goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Splash {
    pub wordmark: &'static str,
    pub tagline: &'static str,
    /// `(keys, what they do)`, set as keycaps by the reader.
    pub keys: &'static [(&'static str, &'static str)],
}

pub fn idle() -> Splash {
    Splash {
        wordmark: "hww",
        tagline: "A new browser for the human-wide web.",
        keys: &[
            (crate::reader::menu::OPEN_LOCATION, "Search or open a URL"),
            ("?", "Keyboard shortcuts"),
        ],
    }
}

/// A navigation in flight, when there is nothing on screen to read while it runs: one dim line
/// in the middle of an empty page, naming what is being fetched.
///
/// **`None` when a page is still up.** A fetch used to replace the article the moment the link
/// was clicked, which on a fast connection is a line that flashes for two frames and cannot be
/// read, and on a slow one is a blank screen that reads as having arrived somewhere empty rather
/// than as waiting. The outgoing page stays now, and while it does the reader reports the errand
/// from the chrome instead: the strip counts the seconds and `ui::notice_ui::progress_rule` draws
/// what is left of the budget. A browser paints nothing on a page that has not arrived and moves
/// an indicator in the chrome; this is that, plus the one line, because fifteen seconds of blank
/// with a hairline moving at the bottom reads as a hang to anyone not watching the hairline.
///
/// This returns the `Option` rather than letting the caller decide, because the caller is
/// `app.rs`: the fast CI job never compiles egui, so a decision made there is a decision no test
/// reads.
///
/// `elapsed` is shown because a 15 s bot-block hang and a slow CDN look identical for the
/// first ten seconds, and a number that keeps moving is the difference between waiting and
/// wondering whether it is stuck.
pub fn loading(url: &Url, elapsed: Duration, over_a_page: bool) -> Option<String> {
    if over_a_page {
        return None;
    }
    Some(format!(
        "loading {} … {:.1}s",
        host_and_path(url),
        elapsed.as_secs_f32()
    ))
}

/// How much of the fetch deadline a navigation has spent, as `0.0..=1.0`.
///
/// The number behind the progress rule on the status strip. Determinate rather than an
/// indeterminate sweep, and the denominator is honest: [`TIMEOUT`] bounds the *whole* fetch
/// including every redirect hop, which `fetch::the_timeout_bounds_the_whole_fetch_not_each_hop`
/// is what pins. So the far edge means the request is at its deadline, rather than meaning
/// nothing at all.
///
/// Approximate at both ends, and clamped for it. `Page::Loading::started` is stamped when the job
/// is dispatched, while the fetcher's own deadline starts when a worker picks it up, and
/// `session::load` rewrites and extracts *after* the body is in. Neither gap is worth a second
/// clock; a bar that saturates a little early on a page that was always going to time out costs
/// nothing.
pub fn elapsed_fraction(elapsed: Duration) -> f32 {
    (elapsed.as_secs_f32() / TIMEOUT.as_secs_f32()).clamp(0.0, 1.0)
}

/// The name in the window title, and the title of a window with no page in it. Also what
/// `ui::run` opens the window under, so the first frame does not rename it.
pub const WINDOW_NAME: &str = "hww";

/// What the window itself is called: the name a task list, macOS's Window menu, the app
/// switcher, and a screenshot of the frame all read.
///
/// It was `hww` from launch to quit, which is a window a reader cannot tell from the other two
/// they have open, and comments here and in `search` were already written as though the page's
/// title reached it. Every browser puts the page there; this does the same, with the name after
/// it, so the window is still identifiable as hww's when the page has no useful title.
///
/// `None` is the idle window and the bare name. So is a title that is only spaces, which is a
/// `<title>` a page can write.
///
/// **The page's own words, leaving the reader.** This is the one string hww hands to somebody
/// else's chrome, drawn by a window manager with none of the guards the reading column has, so
/// [`one_line`] takes control characters and the bidi overrides out: a title that reorders the
/// text *around* it in a task list is a page writing on furniture it does not own. The cap is
/// the same argument in the other direction — a window title is one line in a layout hww does
/// not control, and a page can write a paragraph into it.
pub fn window_title(page: Option<&str>) -> String {
    match page.map(one_line).filter(|line| !line.is_empty()) {
        Some(line) => format!("{line} \u{2014} {WINDOW_NAME}"),
        None => WINDOW_NAME.to_owned(),
    }
}

/// How much of a page's title the window keeps. Long enough for a headline, short enough that
/// what follows it — the name, and whatever the desktop adds — survives the truncation the
/// task list does next.
const WINDOW_TITLE_MAX: usize = 80;

/// One line of untrusted text: no control characters, no bidi overrides, runs of whitespace
/// collapsed to one space, and capped with an ellipsis.
///
/// The bidi marks go rather than being balanced, because there is nothing here to balance them
/// against: the string is dropped into a strip of other applications' titles, and an unclosed
/// override runs until something else closes it.
fn one_line(s: &str) -> String {
    // A control character becomes the space it stood for and a bidi mark is dropped, which is
    // the difference between what each one is: a newline in a `<title>` separates two words and
    // deleting it runs them together, while an override separates nothing and means only what
    // it does to the text after it.
    let kept: String = s
        .chars()
        .filter(|c| !is_bidi_control(*c))
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let mut line = kept.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() > WINDOW_TITLE_MAX {
        line = line.chars().take(WINDOW_TITLE_MAX).collect::<String>();
        line = line.trim_end().to_owned();
        line.push('\u{2026}');
    }
    line
}

/// The bidirectional marks, embeddings, overrides, and isolates. `char::is_control` does not
/// cover them: they are formatting characters, which is exactly what makes them a problem in a
/// string somebody else lays out.
fn is_bidi_control(c: char) -> bool {
    matches!(
        c,
        '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
    )
}

/// `example.com/a/b`, or bare `example.com` for a root path.
///
/// A wording decision, so it lives with the other wording decisions rather than in `ui/`.
pub fn host_and_path(url: &Url) -> String {
    // hww's own views (`hww:bookmarks`) have no authority component, so there is no host to
    // shorten and the path is a bare word. Shortening one anyway spells `hwwbookmarks`, which is
    // not an address anybody typed; the whole thing is already short, so say it as written.
    if url.cannot_be_a_base() {
        return url.as_str().to_owned();
    }
    let host = url.host_str().unwrap_or(url.scheme());
    let path = url.path();
    if path == "/" {
        host.to_owned()
    } else {
        format!("{host}{path}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch::FetchError;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    /// The window is hww's until a page has a name, and the page's after that.
    #[test]
    fn the_window_takes_the_page_name() {
        assert_eq!(window_title(None), "hww");
        assert_eq!(window_title(Some("A headline")), "A headline \u{2014} hww");
        // A `<title>` of nothing but spaces is a page a reader would otherwise see named by
        // whitespace in their task list.
        assert_eq!(window_title(Some("   ")), "hww");
    }

    /// The one string hww hands to somebody else's chrome, so the page does not get to write a
    /// second line, a tab stop, or a right-to-left override into a strip of other windows.
    #[test]
    fn a_page_cannot_write_control_characters_into_the_window_title() {
        let hostile = "Quiet\nweb\ttitle\u{202e}reversed\u{2066}isolated";
        let got = window_title(Some(hostile));
        assert_eq!(got, "Quiet web titlereversedisolated \u{2014} hww");
        assert!(!got.chars().any(|c| c.is_control() || is_bidi_control(c)));
    }

    /// A title long enough to be a paragraph is cut, and what follows the cut — the name the
    /// window is identified by — survives it.
    #[test]
    fn a_long_title_is_cut_and_the_name_survives() {
        let long = "word ".repeat(60);
        let got = window_title(Some(&long));
        assert!(got.ends_with("\u{2026} \u{2014} hww"), "{got}");
        assert!(
            got.chars().count() <= WINDOW_TITLE_MAX + " \u{2014} hww".chars().count() + 1,
            "{got}"
        );
    }

    /// The shape `session` builds when a rewrite lands somewhere the rule did not aim at.
    fn dead_rule() -> Provenance {
        let mut p = Provenance::fixture("https://example.com/a");
        p.rewritten_to = Some(url("https://alt.example.net/a"));
        p.rule_appears_dead = true;
        p.final_url = url("https://www.example.org/a");
        p
    }

    /// Every way a body can fail to arrive whole. `Complete` is deliberately absent: it is the
    /// one variant that must produce nothing, and it is asserted separately.
    fn every_truncation() -> Vec<Truncation> {
        vec![
            Truncation::AtCap(5_000_000),
            Truncation::MaybeAtCap(5_000_000),
            Truncation::Timeout(Duration::from_secs(15)),
            Truncation::Incomplete(812_345),
        ]
    }

    /// A clean 200 for `about_page` to vary one field of at a time.
    ///
    /// Shared with `session`'s own tests via `Provenance::fixture` rather than restated: an
    /// eleven-field literal copied per module is one the fetcher can outgrow silently.
    fn page(status: u16) -> Provenance {
        let mut p = Provenance::fixture("https://example.com/a");
        p.status = status;
        p
    }

    /// Every failure that can be built without a live `reqwest::Error`.
    ///
    /// `FetchError::Transport` wraps one and cannot be constructed here, so the fallback arm
    /// is reached through `Io` instead.
    fn every_failure() -> Vec<Notice> {
        let u = url("https://example.com/a");
        [
            LoadError::Fetch(FetchError::LikelyBlocked),
            LoadError::Fetch(FetchError::Timeout(Duration::from_secs(15))),
            LoadError::Fetch(FetchError::SchemeDowngrade(url("http://example.net/x"))),
            LoadError::Fetch(FetchError::TooManyRedirects(vec![
                url("https://a.example/1"),
                url("https://b.example/2"),
            ])),
            LoadError::Fetch(FetchError::RedirectWithoutLocation(302)),
            LoadError::Fetch(FetchError::BadLocation("::not a url::".to_owned())),
            LoadError::Fetch(FetchError::Io(std::io::Error::other("boom"))),
            LoadError::NotWebPage {
                content_type: "application/pdf".to_owned(),
            },
            LoadError::SearchBlocked { engine: "Mojeek" },
            LoadError::SearchEmpty {
                engine: "Mojeek",
                terms: "lorem ipsum".to_owned(),
            },
            LoadError::FeedEmpty {
                title: "Lorem Weekly".to_owned(),
            },
            LoadError::FeedUnreadable {
                why: "unknown entity reference 'nbsp' at 3:24".to_owned(),
                truncated: false,
            },
            LoadError::FeedUnreadable {
                why: "expected element end tag at 812:1".to_owned(),
                truncated: true,
            },
        ]
        .iter()
        .map(|e| failure(e, &u))
        .collect()
    }

    /// Three severities, three words, and none of them a prefix of another.
    ///
    /// The word is the icon's accessible name, so loudness reaches a screen reader; two
    /// severities sharing a word, or one reading as an abbreviation of the next, would put that
    /// back where it was.
    #[test]
    fn each_severity_says_something_different() {
        let words = [Severity::Quiet, Severity::Caution, Severity::Failure].map(Severity::word);
        for (i, a) in words.iter().enumerate() {
            assert!(!a.is_empty());
            assert_eq!(*a, a.to_lowercase(), "{a:?} is not set like the rest");
            for b in &words[i + 1..] {
                assert_ne!(a, b);
                assert!(!a.starts_with(b) && !b.starts_with(a), "{a:?} and {b:?}");
            }
        }
    }

    /// Refusing the downgrade is correct, and a bypass would be a policy hole. That was a
    /// comment inside a `Ui` closure, which is to say it was not checked by anything.
    #[test]
    fn a_scheme_downgrade_is_never_retryable() {
        let notice = failure(
            &LoadError::Fetch(FetchError::SchemeDowngrade(url("http://example.net/"))),
            &url("https://example.net/"),
        );
        assert_eq!(notice.actions, &[Button::CopyUrl]);
    }

    /// A rewrite rule can be the reason a request is blocked or looping, so those two offer a
    /// way round it. A timeout cannot be, so it does not.
    #[test]
    fn only_failures_a_rewrite_could_cause_offer_retry_without_rewrite() {
        let u = url("https://example.com/");
        let offers = |e: LoadError| {
            failure(&e, &u)
                .actions
                .contains(&Button::RetryWithoutRewrite)
        };
        assert!(offers(LoadError::Fetch(FetchError::LikelyBlocked)));
        assert!(offers(LoadError::Fetch(FetchError::TooManyRedirects(
            vec![]
        ))));
        assert!(!offers(LoadError::Fetch(FetchError::Timeout(
            Duration::from_secs(15)
        ))));
    }

    /// The same parser failure from opposite causes. `feed.rs` measured one feed in 28 over the
    /// 5 MB cap: it arrives cut mid-element and cannot parse, and the publisher did nothing
    /// wrong. Telling that reader to wait for a generator fix is a confident wrong diagnosis,
    /// and it points them away from the reload that would actually work.
    #[test]
    fn a_feed_hww_truncated_does_not_blame_the_publisher() {
        let u = url("https://example.com/feed.xml");
        let cut = failure(
            &LoadError::FeedUnreadable {
                why: "expected element end tag at 812:1".to_owned(),
                truncated: true,
            },
            &u,
        );
        let theirs = failure(
            &LoadError::FeedUnreadable {
                why: "unknown entity reference 'nbsp' at 3:24".to_owned(),
                truncated: false,
            },
            &u,
        );
        assert_ne!(cut.heading, theirs.heading);
        assert_ne!(cut.code, theirs.code);
        let body = cut.body.to_lowercase();
        assert!(
            !body.contains("publisher") && !body.contains("generator"),
            "hww cut this one: {}",
            cut.body
        );
        assert!(body.contains("hww"), "and it says so: {}", cut.body);
        // The other half of the claim: the one hww did not cut still names whose it is, and
        // still says a reload will not help.
        assert!(theirs.body.contains("publisher"), "{}", theirs.body);
        // The parser's message reaches both, because it is what a bug report quotes.
        assert!(cut.detail.iter().any(|d| d.contains("812:1")));
        assert!(theirs.detail.iter().any(|d| d.contains("nbsp")));
    }

    #[test]
    fn every_failure_gets_its_own_words() {
        let notices = every_failure();
        let mut headings: Vec<&str> = notices.iter().filter_map(|n| n.heading).collect();
        assert_eq!(headings.len(), notices.len(), "a failure with no heading");
        headings.sort_unstable();
        let before = headings.len();
        headings.dedup();
        assert_eq!(before, headings.len(), "two failures share a heading");
        for n in &notices {
            assert!(!n.body.trim().is_empty());
            assert_eq!(n.severity, Severity::Failure);
            // The URL is always the first detail line, so "which page was that?" is answered
            // on every failure screen without going back to the strip.
            assert_eq!(
                n.detail.first().map(String::as_str),
                Some("https://example.com/a")
            );
            assert!(
                n.code.is_some(),
                "a failure with no code: {}",
                n.heading.unwrap()
            );
        }
        // And the codes are distinct too: the code is what a bug report quotes.
        let mut codes: Vec<&str> = notices.iter().filter_map(|n| n.code).collect();
        codes.sort_unstable();
        let before = codes.len();
        codes.dedup();
        assert_eq!(before, codes.len(), "two failures share a code");
    }

    #[test]
    fn a_redirect_loop_lists_its_hops_in_order() {
        let notice = failure(
            &LoadError::Fetch(FetchError::TooManyRedirects(vec![
                url("https://a.example/1"),
                url("https://b.example/2"),
            ])),
            &url("https://start.example/"),
        );
        assert!(notice.detail[1].contains("a.example/1"));
        assert!(notice.detail[2].contains("b.example/2"));
    }

    /// The whole point of the third argument: a page still on screen gets no line, because the
    /// strip and the progress rule are reporting instead.
    #[test]
    fn a_load_over_a_readable_page_draws_no_line() {
        let u = url("https://example.com/a");
        assert!(loading(&u, Duration::from_secs(3), true).is_none());
        let line = loading(&u, Duration::from_secs(3), false).expect("no page under it");
        assert!(line.contains("example.com/a"));
        assert!(line.contains("3.0s"), "the clock is the point: {line}");
    }

    /// Inside Material's range for a snackbar, and long enough to read the hosts `I` names.
    #[test]
    fn a_toast_lives_between_four_and_ten_seconds() {
        assert!(TOAST_FOR >= Duration::from_secs(4));
        assert!(TOAST_FOR <= Duration::from_secs(10));
    }

    /// The idle screen names the two keys that matter and nothing the chrome already shows.
    #[test]
    fn the_splash_names_its_keys() {
        let s = idle();
        assert!(!s.wordmark.is_empty());
        assert!(
            s.keys
                .iter()
                .any(|(k, _)| *k == crate::reader::menu::OPEN_LOCATION)
        );
        assert!(s.keys.iter().any(|(k, _)| *k == "?"));
        assert!(s.keys.iter().all(|(_, what)| !what.is_empty()));
    }

    /// The rewrite bar is hww owning up, so `hww` is its subject and the one action it offers
    /// is the way out of it. It is not a caution: nothing is wrong.
    #[test]
    fn a_rewrite_is_an_information_bar_naming_hww_as_the_actor() {
        assert!(rewrite(&page(200)).is_none(), "no rule, no bar");
        let mut p = page(200);
        p.rewritten_to = Some(url("https://text.example.com/a"));
        let n = rewrite(&p).expect("a bar");
        assert_eq!(n.severity, Severity::Quiet);
        assert!(n.body.starts_with("hww "), "{}", n.body);
        assert!(n.body.contains("text.example.com"));
        assert!(n.body.contains("example.com"));
        assert_eq!(n.actions, &[Button::RetryWithoutRewrite]);
    }

    /// Every sentence about a loaded page names who did the thing: the host for what the server
    /// did, `hww` for what the reader did. A passive ("the body was cut") says neither.
    fn names_hww_or_a_host(body: &str) {
        let first = body.split_whitespace().next().unwrap_or("");
        assert!(
            first == "hww" || first.contains('.'),
            "no actor as subject: {body}"
        );
    }

    #[test]
    fn every_remark_names_its_actor() {
        let mut p = dead_rule();
        p.status = 404;
        p.truncation = Truncation::AtCap(5_000_000);
        let notices = about_page(&p, 10, Arrival::Fetched);
        assert!(
            notices[0].body.starts_with("alt.example.net "),
            "{}",
            notices[0].body
        );
        assert!(notices[1].body.starts_with("hww "), "{}", notices[1].body);
        assert!(
            notices[2].body.starts_with("www.example.org "),
            "{}",
            notices[2].body
        );
        assert!(notices[3].body.starts_with("hww "), "{}", notices[3].body);
        for n in &notices {
            names_hww_or_a_host(&n.body);
            assert!(!n.body.contains("The server"), "{}", n.body);
        }
        for t in every_truncation() {
            let mut p = page(200);
            p.truncation = t;
            for n in about_page(&p, 9_000, Arrival::Fetched) {
                names_hww_or_a_host(&n.body);
            }
        }
        let mut rewritten = page(200);
        rewritten.rewritten_to = Some(url("https://text.example.com/a"));
        names_hww_or_a_host(&rewrite(&rewritten).expect("a bar").body);

        let mut d = crate::html::extract(
            "<html><body><article><p>A page in Latin script, long enough to be a paragraph on \
             its own merits and then some.</p></article></body></html>",
            &url::Url::parse("https://example.test/").unwrap(),
        );
        d.blocks
            .push(crate::ir::Block::Paragraph(vec![crate::ir::Inline::Text(
                "שלום עולם".to_owned(),
            )]));
        names_hww_or_a_host(&rtl(&d).expect("a caution").body);
    }

    /// Pinned against `TIMEOUT`, never the literal 15: the whole reason it is re-exported is so
    /// that moving the deadline moves the bar with it.
    #[test]
    fn the_progress_fraction_spans_the_fetch_deadline() {
        assert_eq!(elapsed_fraction(Duration::ZERO), 0.0);
        assert_eq!(elapsed_fraction(TIMEOUT), 1.0);
        let half = elapsed_fraction(TIMEOUT / 2);
        assert!((half - 0.5).abs() < 1e-6, "{half}");
    }

    /// `started` covers rewrite and extraction as well as the fetch, so it outruns the deadline
    /// on a slow page. A bar wider than its track is a panic in egui, not a cosmetic problem.
    #[test]
    fn the_progress_fraction_never_exceeds_its_track() {
        assert_eq!(elapsed_fraction(TIMEOUT * 4), 1.0);
    }

    /// The inline marks all carry it, and this is the newest of them.
    #[test]
    fn the_pending_mark_carries_the_bracket_convention() {
        assert!(PENDING.starts_with('['));
        assert!(PENDING.ends_with(']'));
        assert!(!PENDING.contains(char::is_whitespace));
    }

    /// The column is free on the pages that are fine, which is nearly all of them.
    #[test]
    fn a_healthy_page_says_nothing() {
        assert!(about_page(&page(200), 5_000, Arrival::Fetched).is_empty());
    }

    /// A built page has nothing to be cautioned about, and the thin-text caution is the one that
    /// would otherwise fire: a bookmarks list of two items is well under `THIN_TEXT`, and hww would tell
    /// the reader their own saved list may need JavaScript.
    #[test]
    fn a_built_page_draws_no_bars_least_of_all_the_thin_one() {
        let p = Provenance::built(url(crate::reader::archive::BOOKMARKS));
        assert!(about_page(&p, 0, Arrival::Built).is_empty());
        assert!(about_page(&p, THIN_TEXT - 1, Arrival::Built).is_empty());
        // The same provenance read as a fetch would say three things, which is the measure of
        // what the variant is suppressing.
        assert!(!about_page(&p, 0, Arrival::Fetched).is_empty());
        // And a built page produces no rewrite bar either, without any change there: it was
        // never rewritten.
        assert!(rewrite(&p).is_none());
    }

    /// An empty history says which of the two things it means: nothing read yet, or nothing
    /// being written down. A blank list that could mean either is indistinguishable from a bug,
    /// and this is the one page in hww whose emptiness is a setting away.
    #[test]
    fn an_empty_history_says_whether_it_is_off() {
        assert!(!HISTORY_TITLE.is_empty());
        let on = history_is_empty(true);
        let off = history_is_empty(false);
        assert_ne!(on, off);
        assert!(off.contains("not to remember"), "{off}");
        for s in [&on, &off] {
            assert!(s.contains("Settings"), "the switch is named: {s}");
        }
    }

    /// The bookmarks' own wording: a title for the masthead, and an empty state that names the
    /// key that fills it, spelled for the platform the reader is on.
    #[test]
    fn the_bookmarks_say_what_they_are_and_how_to_fill_them() {
        assert!(!BOOKMARKS_TITLE.is_empty());
        let empty = bookmarks_are_empty();
        assert!(
            empty.contains(crate::reader::menu::BOOKMARK_PAGE),
            "{empty}"
        );
        assert!(
            empty.contains("never bookmarks"),
            "the empty page states the doctrine, not just the key: {empty}"
        );
        // A toast that names a key has to name one the reader can press. This is the cheap half
        // of the gap `menu`'s agreement test leaves open: nothing can check `handle_keys` from
        // here, but a hardcoded letter left behind by a rebinding is catchable, and this one was
        // written as a bare `b` for as long as `b` opened the bookmarks.
        let toast = bookmarked("A page");
        assert!(
            toast.contains(crate::reader::menu::BOOKMARKS),
            "the key is named for this platform: {toast}"
        );
        // Every string this module can put on screen about the bookmarks names hww or the
        // reader, never a passive.
        for s in [
            bookmarked("A page"),
            ALREADY_BOOKMARKED.to_owned(),
            NOTHING_TO_BOOKMARK.to_owned(),
            FORGOTTEN.to_owned(),
            bookmarks_are_full(crate::reader::archive::MAX_ITEMS),
            BOOKMARKING_IS_OFF.to_owned(),
            bookmarks_forgotten(0),
            bookmarks_forgotten(1),
            bookmarks_forgotten(4),
            history_is_empty(true),
            history_is_empty(false),
            history_forgotten(0),
            history_forgotten(1),
            history_forgotten(4),
            read_later("A link"),
            ALREADY_READ_LATER.to_owned(),
            READ_LATER_REMOVED.to_owned(),
            CANNOT_READ_LATER.to_owned(),
            ADDRESS_TOO_LONG_TO_READ_LATER.to_owned(),
            reading_list_is_full(crate::reader::archive::MAX_NEXT),
            READING_LIST_IS_OFF.to_owned(),
            no_link_to_read_later(),
            reading_list_is_empty(true),
            reading_list_is_empty(false),
            reading_list_forgotten(0),
            reading_list_forgotten(1),
            reading_list_forgotten(4),
        ] {
            assert!(!s.is_empty());
            assert!(
                s.contains("hww") || s.contains("your"),
                "a remark with no actor: {s}"
            );
        }
    }

    /// The reading list's own wording. Two halves, like the history's, and one extra demand the
    /// other two empty states do not carry: it is filled from a *link* rather than from the page
    /// on screen, so the empty state has to name both ways of aiming at one or a reader who
    /// arrives here has been shown a list and not told how to add to it.
    #[test]
    fn an_empty_reading_list_says_how_to_fill_it_and_whether_it_is_off() {
        assert!(!READING_LIST_TITLE.is_empty());
        let on = reading_list_is_empty(true);
        let off = reading_list_is_empty(false);
        assert_ne!(on, off);
        assert!(
            on.contains(crate::reader::menu::READ_LATER),
            "the key is named for this platform: {on}"
        );
        assert!(
            on.contains("right-click"),
            "the mouse route is named too, or a mouse-only reader cannot fill it: {on}"
        );
        assert!(off.contains("not to keep"), "{off}");
        for s in [&on, &off] {
            assert!(s.contains("Settings") || s.contains("Tab"), "{s}");
        }
        // The key that fails names the key that would have worked, rather than only reporting
        // that nothing was focused.
        let none = no_link_to_read_later();
        assert!(none.contains(crate::reader::menu::BOOKMARK_PAGE), "{none}");
    }

    /// The reading list's own two controls. Labels rather than remarks, which is why they are not
    /// in the loop above that demands an actor: a button says what pressing it does, and the toast
    /// it produces says who did it.
    #[test]
    fn the_reading_lists_controls_are_labelled_for_the_act_they_perform() {
        for s in [READING_LIST_REMOVE, READING_LIST_FORGET_ALL] {
            assert!(!s.is_empty());
            // Set in the page's own family, and none of the embedded faces is a symbol font. ASCII
            // is the range all of them carry, and a two-word label has no reason to leave it.
            assert!(
                s.is_ascii(),
                "a control label may need a glyph the faces lack: {s}"
            );
            assert_eq!(
                s.to_lowercase(),
                s,
                "the panel's buttons are lower case and these are the same act: {s}"
            );
        }
        assert_ne!(READING_LIST_REMOVE, READING_LIST_FORGET_ALL);
        // The destructive one says so before it is pressed, and names the two tenants it leaves
        // where they are: the whole argument for three buttons is that none takes another with it.
        for part in ["cannot be undone", "kept", "history"] {
            assert!(
                READING_LIST_FORGET_ALL_HOVER.contains(part),
                "the hover drops {part:?}: {READING_LIST_FORGET_ALL_HOVER}"
            );
        }
    }

    /// The three tenants' wording stays distinct. One list's sentence shown about another is a
    /// wrong claim about where the reader's data went, and the strings are close enough to copy.
    #[test]
    fn no_tenant_borrows_another_tenants_words() {
        for (a, b) in [
            (BOOKMARKING_IS_OFF, READING_LIST_IS_OFF),
            (ALREADY_BOOKMARKED, ALREADY_READ_LATER),
            (ADDRESS_TOO_LONG, ADDRESS_TOO_LONG_TO_READ_LATER),
        ] {
            assert_ne!(a, b);
        }
        assert_ne!(BOOKMARKS_TITLE, READING_LIST_TITLE);
        assert_ne!(HISTORY_TITLE, READING_LIST_TITLE);
        assert_ne!(bookmarks_forgotten(3), reading_list_forgotten(3));
        assert_ne!(bookmarks_are_empty(), reading_list_is_empty(true));
    }

    /// A readable 404 is not a failure. Many paywalls answer 403 with the article intact, and
    /// replacing that with an error screen would throw away a page the reader can read.
    #[test]
    fn a_readable_404_is_a_caution_not_a_failure() {
        let notices = about_page(&page(404), 5_000, Arrival::Fetched);
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].severity, Severity::Caution);
        assert!(
            notices[0].heading.is_none(),
            "a caution is one line, not a screen"
        );
        assert!(notices[0].actions.is_empty());
    }

    /// The threshold is `ir::THIN_TEXT`, which `html::extract` reads too, so this pins the
    /// boundary behaviour rather than the number: whatever the extractor falls back at, that is
    /// where the warning starts.
    #[test]
    fn the_thin_extraction_threshold_matches_the_extractor() {
        assert_eq!(about_page(&page(200), THIN_TEXT, Arrival::Fetched).len(), 0);
        assert_eq!(
            about_page(&page(200), THIN_TEXT - 1, Arrival::Fetched).len(),
            1
        );
    }

    /// A feed draws no thin-text caution, however little text it carries.
    ///
    /// `ir::block_text_len` counts a `Block::Figure` as its caption alone, so a picture-only
    /// feed — a comic, a photo blog — comes out under the floor while having parsed perfectly,
    /// and the caution would blame JavaScript for a page hww read correctly. The IR must not
    /// change to fix it, so the pipeline says the page was a feed and this reads `prov.feed`.
    #[test]
    fn a_thin_feed_is_not_a_page_that_needs_javascript() {
        let mut p = page(200);
        p.feed = Some(crate::feed::Report {
            kind: crate::feed::Kind::Atom,
            entries: 5,
        });
        assert!(about_page(&p, 0, Arrival::Fetched).is_empty());
        assert!(about_page(&p, THIN_TEXT - 1, Arrival::Fetched).is_empty());
        // And the same page read as anything else still gets it, so nothing else was suppressed.
        p.feed = None;
        assert_eq!(about_page(&p, THIN_TEXT - 1, Arrival::Fetched).len(), 1);
        // Everything a feed *can* be wrong about is still said.
        p.feed = Some(crate::feed::Report {
            kind: crate::feed::Kind::Rss,
            entries: 1,
        });
        p.status = 500;
        assert_eq!(about_page(&p, 0, Arrival::Fetched).len(), 1);
    }

    /// Loudest first: the remark most likely to change how the page is read sits nearest the
    /// top, where it is read before the prose it is about.
    #[test]
    fn a_page_that_is_both_broken_and_empty_leads_with_the_status() {
        let notices = about_page(&page(500), 10, Arrival::Fetched);
        assert_eq!(notices.len(), 2);
        assert!(notices[0].body.contains("500"));
        assert!(notices[1].body.contains("JavaScript"));
    }

    /// The early warning that stands in for a retry, in the one place it cannot expire.
    ///
    /// It used to be a twelve-second `flash` in the status strip. A reader who looked away had
    /// no way to learn that the page in front of them came from a host the rule did not aim
    /// at, and no way to ask.
    #[test]
    fn a_dead_rule_is_a_bar_naming_both_hosts() {
        let notices = about_page(&dead_rule(), 9_000, Arrival::Fetched);
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].severity, Severity::Caution);
        assert!(
            notices[0].body.contains("alt.example.net"),
            "the host aimed at"
        );
        assert!(
            notices[0].body.contains("www.example.org"),
            "the host that answered"
        );
        assert!(
            notices[0].detail[0].contains("example.com/a"),
            "what was asked for"
        );
    }

    /// A truncated body ends mid-article with nothing on screen to say so, which is the exact
    /// definition of something the reader would otherwise mis-read.
    #[test]
    fn every_truncation_is_a_bar_and_a_whole_body_is_not() {
        let mut clean = page(200);
        clean.truncation = Truncation::Complete;
        assert!(about_page(&clean, 9_000, Arrival::Fetched).is_empty());

        for t in every_truncation() {
            let mut p = page(200);
            p.truncation = t;
            let notices = about_page(&p, 9_000, Arrival::Fetched);
            assert_eq!(notices.len(), 1, "{t:?} produced {} bars", notices.len());
            assert_eq!(notices[0].severity, Severity::Caution);
            assert!(!notices[0].body.is_empty());
        }
    }

    /// `len >= cap` cannot tell a body that was cut from one that ended on the boundary, so the
    /// two caps must not claim the same certainty. Losing this distinction is how a reader comes
    /// to distrust the warning on the pages where it is right.
    #[test]
    fn the_two_caps_hedge_differently() {
        let mut certain = page(200);
        certain.truncation = Truncation::AtCap(5_000_000);
        let mut hedged = page(200);
        hedged.truncation = Truncation::MaybeAtCap(5_000_000);

        assert!(
            !about_page(&certain, 9_000, Arrival::Fetched)[0]
                .body
                .contains("may")
        );
        assert!(
            about_page(&hedged, 9_000, Arrival::Fetched)[0]
                .body
                .contains("may")
        );
    }

    /// Loudest first, across all four. A dead rule outranks everything because it is the only
    /// one that says the article may be the wrong article.
    #[test]
    fn the_bars_are_ordered_loudest_first() {
        let mut p = dead_rule();
        p.status = 500;
        p.truncation = Truncation::Incomplete(812_345);
        let notices = about_page(&p, 10, Arrival::Fetched);
        assert_eq!(notices.len(), 4);
        assert!(notices[0].body.contains("appears dead"));
        assert!(notices[1].body.contains("incomplete"));
        assert!(notices[2].body.contains("500"));
        assert!(notices[3].body.contains("JavaScript"));
    }

    /// Moved out of `ui/app.rs`, where it had no test.
    #[test]
    fn a_view_with_no_host_is_named_as_written() {
        assert_eq!(
            host_and_path(&url(crate::reader::archive::BOOKMARKS)),
            "hww:bookmarks",
            "shortening an authority-less address spells hwwbookmarks"
        );
    }

    #[test]
    fn host_and_path_drops_a_bare_slash() {
        assert_eq!(host_and_path(&url("https://example.com/")), "example.com");
        assert_eq!(
            host_and_path(&url("https://example.com/a/b?q=1")),
            "example.com/a/b"
        );
    }

    #[test]
    fn a_page_with_hebrew_or_arabic_gets_the_rtl_caution_and_a_latin_page_does_not() {
        let mut d = crate::html::extract(
            "<html><body><article><p>A page in Latin script, long enough to be a paragraph on \
             its own merits and then some.</p></article></body></html>",
            &url::Url::parse("https://example.test/").unwrap(),
        );
        assert!(rtl(&d).is_none());
        d.blocks
            .push(crate::ir::Block::Paragraph(vec![crate::ir::Inline::Text(
                "שלום עולם".to_owned(),
            )]));
        let n = rtl(&d).expect("a caution");
        assert_eq!(n.severity, Severity::Caution);
    }

    /// The scan reaches everywhere the flatten it replaced did.
    ///
    /// [`carries_rtl`] used to build `ir::plain_text` per run and look at the characters in it.
    /// It walks the tree directly now, and a place the walk forgets is a page that lays out
    /// backwards with nothing said about it — so every variant that carries text is asserted,
    /// including the image alt that `plain_text` folded in and the nesting that hides one.
    #[test]
    fn the_rtl_scan_reaches_every_place_text_hides() {
        use crate::ir::{Block, Image, Inline};
        let hebrew = || Inline::Text("שלום".to_owned());
        let latin = || Inline::Text("plain".to_owned());
        let cases: Vec<Block> = vec![
            Block::Paragraph(vec![hebrew()]),
            Block::Heading {
                level: 2,
                inlines: vec![hebrew()],
            },
            Block::Paragraph(vec![Inline::Emph(vec![Inline::Strong(vec![hebrew()])])]),
            Block::Paragraph(vec![Inline::Link {
                href: "https://example.test/".to_owned(),
                inlines: vec![hebrew()],
            }]),
            Block::Paragraph(vec![Inline::Code("שלום".to_owned())]),
            Block::Paragraph(vec![Inline::Image(Image {
                src: "https://example.test/i.png".to_owned(),
                alt: Some("שלום".to_owned()),
            })]),
            Block::Code {
                lang: None,
                text: "שלום".to_owned(),
            },
            Block::List {
                ordered: false,
                items: vec![vec![Block::Paragraph(vec![hebrew()])]],
            },
            Block::Quote {
                blocks: vec![Block::Paragraph(vec![hebrew()])],
                cite: None,
            },
            Block::Figure {
                image: Image {
                    src: "https://example.test/i.png".to_owned(),
                    alt: None,
                },
                caption: Some(vec![hebrew()]),
            },
            Block::Table {
                headers: vec![vec![hebrew()]],
                rows: vec![],
            },
            Block::Table {
                headers: vec![vec![latin()]],
                rows: vec![vec![vec![hebrew()]]],
            },
            Block::Thread(vec![crate::ir::Comment {
                author: None,
                timestamp: None,
                depth: 0,
                id: None,
                blocks: vec![Block::Paragraph(vec![hebrew()])],
            }]),
            Block::Entries(vec![crate::ir::Entry {
                title: vec![hebrew()],
                href: None,
                published: None,
                summary: vec![],
                image: None,
                address: None,
                icon: None,
            }]),
            Block::Entries(vec![crate::ir::Entry {
                title: vec![latin()],
                href: None,
                published: None,
                summary: vec![Block::Paragraph(vec![hebrew()])],
                image: None,
                address: None,
                icon: None,
            }]),
        ];
        for block in cases {
            let d = crate::ir::Document {
                url: "https://example.test/".to_owned(),
                title: None,
                byline: None,
                published: None,
                site_name: None,
                favicon: None,
                lang: None,
                blocks: vec![block.clone()],
            };
            assert!(carries_rtl(&d), "missed the right-to-left run in {block:?}");
        }

        // And says no when there is none to find, in the same shapes.
        let d = crate::ir::Document {
            url: "https://example.test/".to_owned(),
            title: Some("A Latin title".to_owned()),
            byline: None,
            published: None,
            site_name: None,
            favicon: None,
            lang: None,
            blocks: vec![
                Block::Paragraph(vec![Inline::Emph(vec![latin()])]),
                Block::Rule,
                Block::Embed {
                    kind: crate::ir::EmbedKind::Video,
                    url: "https://example.test/v".to_owned(),
                },
                Block::Paragraph(vec![Inline::Break]),
            ],
        };
        assert!(!carries_rtl(&d));
    }
}
