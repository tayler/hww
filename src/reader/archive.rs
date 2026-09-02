//! The archive: what hww is allowed to remember, and the first thing it remembers.
//!
//! # The doctrine
//!
//! **hww remembers three things, and none of them quietly.**
//!
//! The *bookmarks* is what the reader named: an item goes in because a keypress put it there,
//! about one page, once, in the same category as `search_engine`. The *reading list* is what the
//! reader named about a page they have **not** read: a link on the page in front of them, marked
//! to open later. The *history* is what hww watched: the address and title of every page it drew,
//! written without anybody asking for it. Those are different objects with different consent
//! stories — the watched one is what makes a browser profile worth stealing — so they are three
//! tenants of one file, each with its own switch, its own way to be forgotten, and its own
//! sentence in the settings panel and in README. None borrows another's rules.
//!
//! The bookmarks and the reading list are both *chosen*, and they still are not the same tenant.
//! What separates them is tense, and it decides everything else about them: the bookmarks holds
//! pages the reader has read and wants back, so its rows are titled by a document hww fetched and
//! its addresses are redundant; the reading list holds links the reader has not opened, so its
//! rows are titled by whatever words the link was wearing and the address is the only thing
//! telling two of them apart. That is why [`Archive::add_next`] is not [`Archive::bookmark`] with a
//! different discriminant, and why the one guard it adds is the scheme: `keep` is handed the URL
//! a fetch actually resolved, and `add_next` is handed page-controlled text.
//!
//! **History is on by default.** That reverses what this module said while the bookmarks was its
//! only tenant, and the cost of the default is paid here rather than left for the reader to
//! discover: a record kept without being asked for has to be visible before anybody thinks to
//! look for it, switchable off in one click, and forgettable in one more.
//! [`Settings::keep_history`] sits in a settings group of its own with this file's path printed
//! under it, beside the button that empties it; the pages it holds are a page the reader can
//! open, with the key every browser opens its history with, rather than a file they have to
//! find. A default that could not be seen or undone
//! would be the quiet lie this project's whole notice system exists to refuse.
//!
//! Seven consequences, each settled here rather than left to the call site:
//!
//! **Where.** Beside `settings.json`, through [`settings::archive_path`]. One directory, so
//! `HWW_CONFIG_DIR` still names everything hww writes and README's uninstall paragraphs still
//! name one path. See that function for why the `XDG_STATE_HOME` split was not taken.
//!
//! **Disclosure.** The settings panel prints this file's path under every group that writes to
//! it, the way it already prints the settings path at its foot, and page info carries a row
//! saying whether the page on screen is kept. A store the reader cannot see the shape of is a
//! store they cannot reason about. The history has no page-info row of its own on purpose: with
//! the switch on, every page on screen is in it, so a row that said `yes` on every page in the
//! program would disclose nothing and read as noise. What discloses the history is the switch,
//! the path beside it, and the list itself.
//!
//! **Off.** "Off" means stop writing *and* offer to forget, once per tenant.
//! [`Settings::keep_bookmarks`] gates [`Archive::bookmark`], [`Settings::keep_reading_list`] gates
//! [`Archive::add_next`], and [`Settings::keep_history`] gates [`Archive::visit`] — each answered
//! inside the door it guards, never at the call site — and each switch's group carries the button
//! that empties what that switch has already gathered ([`Archive::forget_all`],
//! [`Archive::forget_all_next`], [`Archive::forget_visits`]). A switch that silently retains what
//! it collected is the quiet lie this project's whole notice system exists to refuse, and it
//! would be a louder one on the tenant that was never asked for.
//!
//! **Deletion.** [`Archive::forget`] for one kept page, [`Archive::forget_next`] for one link on
//! the reading list, [`Archive::forget_all`] for the bookmarks, [`Archive::forget_all_next`] for the
//! reading list, [`Archive::forget_visits`] for the history, and the file itself is deleted with
//! the config directory by the uninstall commands in README. Each of the four that can take a
//! marked address away also prunes the icons kept for it (`ReaderApp::prune_icons`), by "no
//! marked row still names this" and never per tenant: one icon serves rows on both lists. Each of them leaves the other tenants
//! alone: forgetting the history must not empty a bookmarks list the reader chose page by page, and
//! forgetting the bookmarks must not look like a way to erase a trail.
//!
//! **Forward compatibility.** This file holds data the reader cannot retype, which is what makes
//! it worth more care than `settings.json` gets. Three mechanisms: [`VERSION`], so a later shape
//! can be recognised rather than guessed at; [`Item::extra`], so a field written by a later build
//! survives a write by this one; and [`Kind::Other`], so an item this build has no idea how to
//! show is carried through untouched instead of being displayed as something it is not. One
//! malformed item costs that item and not the file, and a file that will not parse at all yields
//! an empty archive *and a complaint*, never a refusal to start — [`load`] matches
//! `settings::load`'s contract exactly. It parts company with `settings` in one place, and
//! history's default is what put it there: the empty archive is **sealed**, so the first page
//! drawn cannot write it over a file whose contents hww merely failed to read. See
//! [`Archive::file_unreadable`]. `settings.json` can be retyped in a minute and this file
//! cannot, which is the whole reason it gets the extra care.
//!
//! **Caps.** The bookmarks refuses at [`MAX_ITEMS`] and the reading list refuses at [`MAX_NEXT`];
//! the history evicts at [`MAX_VISITS`]. The two answers are not an inconsistency, they are the
//! doctrine restated as arithmetic: every item in the bookmarks and on the reading list was chosen,
//! and dropping the oldest chosen thing to make room for a newer one is the silent loss these
//! rules exist to prevent, while nothing in the history was chosen and a watched record that grew
//! without bound would be a file nobody agreed to keep for ever. So the two chosen lists say they
//! are full and keep what is there; the history forgets its oldest page and says nothing, which is
//! what a reader expects of the thing that is already forgetting for them. Which side of that line
//! a tenant falls on is the only question a new one has to answer.
//!
//! **Derived bytes.** The rule below is about *tenants* — things the reader chose, that they
//! cannot retype, that need a switch and a way to be forgotten. It is not about everything hww
//! writes. Data that is derived, reconstructible, and byte-valued lives **beside** this file and
//! not in it: [`icons_path`] is the first and so far the only case, and `reader::iconcache` is
//! where the argument for its existence is. The concrete reason it may not come inside is
//! arithmetic rather than taste — the history rewrites this file on every navigation, so base64
//! icons in it would rewrite kilobytes per page drawn.
//!
//! The rule it *does* answer to is the one below it: it holds bytes only for addresses this file
//! already names, on the two lists the reader chose, so it is emptied by the same two buttons and
//! discloses its path under the same two groups. What it is not is a fourth tenant, and the test
//! of that is that nothing in it is lost by deleting it.
//!
//! **Future tenants.** Feed subscriptions, and whatever follows them, are new [`Kind`]s in this
//! file, not new files. That is why an item carries a kind at all. The reading list was the first
//! to arrive that way, and what it had to bring with it is the checklist: a [`Kind`], a cap on the
//! refusing or the evicting side, a switch in [`Settings`], a door that answers that switch inside
//! itself, a way to forget one item and a way to forget all of them, a `prefs` row, a line in
//! README's uninstall paragraphs, and — if it is a list the reader can open — an address in
//! `ui::app::builtin_view`.
//!
//! # What is stored, and what is not
//!
//! No page content. Three fields and a kind: the address, the title, the day, and — for a kept
//! page alone — the address of the site's mark, which draws as a favicon in a column left of each
//! row. All three lists are still names and addresses; the icon is a third address, and only a
//! bookmark has one.
//!
//! **Drawing a mark and storing one are two different questions.** The bookmarks and the reading
//! list both draw the column; only a bookmark stores anything for it. A bookmarked page was
//! fetched, so it declared an icon and [`Archive::bookmark`] writes that address down. A
//! reading-list row is a link off some other page, so hww has never fetched what it points at and has nothing to
//! record — [`well_known_icon`] guesses `/favicon.ico` at the row's own origin when the view is
//! built, and the guess stays out of the file. The history neither stores nor draws.
//!
//! This module used to refuse the icon outright, on the ground that twenty bookmarked sites would
//! put twenty third-party requests on one screen. That cost is real and is now paid
//! deliberately rather than avoided: opening either chosen list contacts the hosts of the rows on
//! screen, under [`ImagePolicy::allows_any_request`] like the masthead favicon, bounded to the
//! layout band so a five-hundred-row reading list is a screenful of requests and not five
//! hundred, and counted in page info, which is why `pageinfo::rows` reports third parties on a
//! page `Arrival::Built` says fetched no document. The reading list pays it twice over, because
//! a mark there is a guess that may 404, and README's promise about that list is amended to say
//! so: no *page* on it is fetched until the reader opens one, and that is still true.
//!
//! **The bookmarks sidebar is the third surface that draws marks**, and it asks the same two
//! questions of this module rather than any new one: [`Archive::bookmark_marks`] is what it
//! reads, so its rows are the same rows in the same order the view draws, and every address it
//! requests came out of [`mark_beside`] like the column's. What it adds is a surface that is up
//! while the reader is on some *other* page, and the two answers to that are elsewhere: the band
//! keeps it to a screenful of requests whatever the list's length, and `ui::images::clear_page`
//! keeps a settled mark across a navigation so the strip is not refetched on every click. It
//! stores nothing of its own and is not a fourth tenant; it is a second reader of the first
//! one's rows.
//!
//! **The mark's bytes are kept, and the address is why that is allowed.** `reader::iconcache`
//! holds them, and holds the argument; the short form is that the rule against writing what hww
//! fetched separates bytes revealing *what you read* from bytes revealing *which sites you
//! already wrote down*. Page content and article images are the first kind and stay off disk. A
//! site icon for an address this file already names is the second — the address is on disk
//! already, written by the same feature, one directory entry over — so keeping its bytes adds no
//! local disclosure at all, and it removes a repeated contact with a third party, which is a
//! privacy cost the other way. That is the whole trade and it is lopsided.
//!
//! Three things follow, and [`Archive::marked_icons`] is where the first is enforced: only an
//! address one of these two lists still names may be kept, never one the history holds; reading
//! from that store is allowed on any page under any policy while writing to it is not, because
//! the masthead's mark fires on every page and a write there would be a trail of everywhere the
//! reader went; and a bad file there is deleted without a word, which is the exact opposite of
//! [`load`]'s contract for this one and is the contrast that explains both.
//!
//! What the old argument got right is kept where it still holds: nothing is stored for a page hww
//! merely watched ([`Archive::visit`]), and the history draws no column ([`Mark`]). It is
//! thousands of rows nobody chose, and opening it is not an invitation to contact thousands of
//! hosts.
//!
//! [`ImagePolicy::allows_any_request`]: crate::reader::opts::ImagePolicy::allows_any_request
//!
//! The history holds **one entry per page, not one per visit**: opening a page again moves its
//! entry to the top and restamps its date rather than adding a second row. A visit-by-visit
//! trail would record how often and in what order somebody read things, which is a sharper claim
//! about a person than the list of pages they have seen, and nothing in hww has a use for it.
//! There is no visit count, no dwell time, and no per-visit clock in [`Item`] to put one in.
//!
//! # Two windows, one file
//!
//! Two hww windows share this file and the last write wins, losing the other's keeps. That is
//! already true of `settings.json` and the project accepted it there; locking a second file
//! would be the first lock in the program, and it would be for a race that needs two windows and
//! one second.
//!
//! # The stored title is untrusted text
//!
//! It comes from a page, and unlike everything else hww renders it now outlives the process.
//! `serde_json` escapes it on the way out, so the file itself is safe, and the GUI draws it as
//! `ir::Inline::Text` rather than as markup. Two things are still owed: it is capped at
//! [`MAX_TITLE`] characters on the way *in*, so a megabyte `<title>` cannot become a megabyte of
//! this file, and nothing may print it to a terminal without `render::sanitize_for_terminal`,
//! which this crate already requires of every terminal print.
//!
//! **So is the address**, which a page chose as surely as it chose the title: a link carrying a
//! kilobyte of query string is one click from being written down, and once written it is
//! re-serialized on every later save. [`MAX_URL`] is the answer, and it refuses where the title
//! cap truncates, because half an address is a link to somewhere else.

use crate::ir::{self, Block, Inline};
use crate::reader::settings::{self, Settings};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use url::Url;

/// The address of the bookmarks view.
///
/// The opaque-path form, with no `//`: there is no authority component, so no hostname enters
/// any file but `sites.rs`, and every `host_str()` in the reader already answers `None` through
/// an `unwrap_or`. Page content can never link here — `session::classify_link` refuses every
/// scheme it does not handle, and `hww` is not one of them — so the only way in is a command the
/// reader gave. `a_page_cannot_link_into_the_readers_own_views` is what pins that.
pub const BOOKMARKS: &str = "hww:bookmarks";

/// The address of the history view.
///
/// The same opaque-path form as [`BOOKMARKS`], and for the same three reasons: no hostname leaves
/// `sites.rs`, no `host_str()` in the reader gains a case, and page content cannot link here
/// because `session::classify_link` refuses the scheme. Two addresses now, which is why
/// `ui::app::builtin_view` matches each of them exactly rather than testing the scheme.
pub const HISTORY: &str = "hww:history";

/// The address of the reading list view.
///
/// The same opaque-path form as [`BOOKMARKS`] and [`HISTORY`], for the same three reasons. Three
/// addresses now, and `ui::app::builtin_view` still matches each of them exactly rather than
/// testing the scheme — a scheme test would divert anything a later build spelled `hww:` into a
/// view that does not exist, and the point of matching exactly is that an address hww does not
/// claim goes to the fetch that will say so.
pub const READING_LIST: &str = "hww:reading-list";

/// The schema this build writes.
///
/// Bumped when the *shape* changes in a way a reader of the old shape would misread. Additive
/// fields do not qualify: [`Item::extra`] carries those through in both directions.
pub const VERSION: u32 = 1;

/// How many items the bookmarks holds.
///
/// **A cap refuses; it does not evict.** `reader::pagecache` drops its oldest entry when it is
/// full and that is right for a cache, because nothing there was chosen. Every item here was
/// chosen, and silently dropping the oldest one is exactly the quiet loss the disclosure rules
/// exist to prevent. Reaching this says so in a toast and keeps what is already here.
///
/// Generous on purpose: a reader who saves a page a day reaches it in five years.
pub const MAX_ITEMS: usize = 2_000;

/// How many pages the history holds before it starts forgetting.
///
/// **This cap evicts; it does not refuse.** [`MAX_ITEMS`] does the opposite, and the module doc
/// gives the argument: nothing here was chosen, so the oldest page is the one with the least
/// claim on the file, and a reader who has to empty their history by hand before hww will record
/// another page has been handed a chore by the feature that was supposed to be automatic.
///
/// Distinct pages, not visits, so a page reopened every day costs one row for ever. At a hundred
/// new pages a day it is a month and a half of reading, and the file is a few hundred kilobytes
/// at that size.
pub const MAX_VISITS: usize = 5_000;

/// How many links the reading list holds.
///
/// **This cap refuses**, with [`MAX_ITEMS`] and against [`MAX_VISITS`], because every link here
/// was chosen: the reader pressed a key or picked a menu item about one link, and dropping the
/// oldest of those to make room is the silent loss the doctrine refuses. Its own number rather
/// than [`MAX_ITEMS`] shared, because tenants do not borrow each other's rules and these two are
/// not the same size of thing — a bookmarks list is a shelf and a reading list is a stack on the desk.
///
/// Smaller than the bookmarks' cap on purpose, and the smallness is the point: five hundred links
/// is already far past the size at which a list of things to read next is a list of things to read
/// next. Reaching it says so and keeps what is there, which is the honest answer to a reading list
/// that has become a second bookmarks.
pub const MAX_NEXT: usize = 500;

/// The longest stored title, in characters.
///
/// Characters and not bytes, so the cut cannot land inside one. Long enough for any headline
/// anybody writes; short enough that a page whose `<title>` is a novel costs a line rather than a
/// file. See the module doc for why a page's title is treated as untrusted here in particular.
pub const MAX_TITLE: usize = 160;

/// The longest stored address, in bytes.
///
/// Refused rather than shortened, which is the one place this parts company with [`MAX_TITLE`]:
/// a title cut in half still names the page, and an address cut in half opens something else or
/// nothing at all. Bytes and not characters for the same reason — nothing is cut, so there is no
/// boundary to land inside, and what the number is protecting is the size of the file.
///
/// An address is as much the page's text as its `<title>` is: a link carrying a megabyte of query
/// string is one click away, and unlike the title it is written back on every later save. Eight
/// kilobytes is past any address a site actually serves and short of anything that would matter.
pub const MAX_URL: usize = 8_192;

/// What sort of thing an item is.
///
/// Three known kinds, which are the doctrine's three tenants. A feed subscribed to would be a
/// fourth here rather than a second file, which is what this discriminant exists for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// A page the reader bookmarked.
    Bookmark,
    /// A link the reader marked to read later.
    ///
    /// Chosen, like [`Kind::Bookmark`], and stored by the same arithmetic — but about a page hww
    /// has never fetched, so its title is the words the link was wearing rather than a `<title>`,
    /// and its address is drawn under it because those words are often "Read more".
    ReadNext,
    /// A page hww drew, recorded because [`Settings::keep_history`] was on when it did.
    ///
    /// Never produced by a keypress about this page. It carries the same two facts a bookmark
    /// does — address and title — and one date, which is the last time hww drew it rather than
    /// the first; see [`Archive::visit`].
    Visited,
    /// A kind written by a later build.
    ///
    /// Carried through a load and a save untouched, and shown nowhere. An older binary that
    /// coerced this to [`Kind::Bookmark`] would list a page the reader merely *visited* among the
    /// pages they chose to save, which is the one confusion this whole module is arranged to
    /// prevent; one that dropped it would delete a later build's data on the next keypress.
    Other(String),
}

impl Kind {
    /// The wire spelling. Lowercase words, because this file is meant to be readable by the
    /// person whose data it is.
    pub fn as_str(&self) -> &str {
        match self {
            Kind::Bookmark => "bookmark",
            Kind::ReadNext => "next",
            Kind::Visited => "visited",
            Kind::Other(s) => s,
        }
    }
}

impl serde::Serialize for Kind {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> serde::Deserialize<'de> for Kind {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Ok(match s.as_str() {
            "bookmark" => Kind::Bookmark,
            "next" => Kind::ReadNext,
            "visited" => Kind::Visited,
            _ => Kind::Other(s),
        })
    }
}

/// One thing the reader named.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Item {
    /// The address as it was given, fragment and all, so the link opens what was kept.
    /// De-duplication compares a fragment-stripped form instead; see [`Archive::is_bookmarked`].
    pub url: String,
    /// The page's title, capped at [`MAX_TITLE`] characters. Empty when the page had none, in
    /// which case [`Item::label`] falls back to the address.
    #[serde(default)]
    pub title: String,
    pub kind: Kind,
    /// When it was kept, or for a [`Kind::Visited`] item when it was last drawn: seconds since
    /// the Unix epoch, UTC.
    ///
    /// A number and not a formatted date, because formatting is a reading decision and this is
    /// the record. Zero for an item whose file did not carry one, which reads as "unknown" and
    /// is what [`Item::on_day`] returns `None` for.
    #[serde(default)]
    pub added: u64,
    /// The address of the site's mark, so the bookmarks can draw a favicon column, or empty.
    ///
    /// Written only for [`Kind::Bookmark`]; see the module doc's "What is stored" for why the
    /// history does not get one. Absolute and `http(s)` or not stored at all
    /// ([`icon_to_store`]), because it is an address hww will later request without being asked
    /// again, and one that is neither of those things is a request nothing can vouch for.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub icon: String,
    /// Fields a later build wrote that this one has no name for.
    ///
    /// Flattened, so they sit beside the known fields in the file rather than in a nested bag,
    /// and preserved across a save. Without this, an older binary opening a newer bookmarks and
    /// keeping one page would strip every unknown field from every other item.
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Item {
    /// What the bookmarks calls this item: its title, or its address when it has none.
    ///
    /// A row with no text at all would be an unclickable blank line, which is worse than an
    /// address. `notice::host_and_path` is the same shortening the loading line uses, so an
    /// untitled page is named the same way wherever the reader meets it.
    pub fn label(&self) -> String {
        let title = self.title.trim();
        if !title.is_empty() {
            return title.to_owned();
        }
        match Url::parse(&self.url) {
            Ok(u) => crate::reader::notice::host_and_path(&u),
            Err(_) => self.url.clone(),
        }
    }

    /// The day [`Item::added`] names, as `YYYY-MM-DD`, or `None` for an item that carries no
    /// time. What that day *means* is the item's kind: the day a page was kept, or the last day
    /// hww drew a visited one.
    ///
    /// UTC, and said so in the settings group's note. A local date needs a timezone database,
    /// and hww carrying one to put a date on a list would be a large dependency for a small
    /// nicety; a date that is occasionally a day out at the edges of the world is a smaller cost
    /// than a wrong claim about what it is.
    pub fn on_day(&self) -> Option<String> {
        if self.added == 0 {
            return None;
        }
        let (y, m, d) = civil_from_days((self.added / 86_400) as i64);
        Some(format!("{y:04}-{m:02}-{d:02}"))
    }
}

/// Days since 1970-01-01 to a civil `(year, month, day)`, by Howard Hinnant's algorithm.
///
/// Written out rather than pulled in. The alternative is a date crate for one line of a list, and
/// this is a closed piece of arithmetic with no configuration, no locale, and no clock in it:
/// `every_epoch_day_maps_to_its_civil_date` checks it against dates known by hand, including the
/// two leap-year cases that catch a hand-rolled version.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// What [`Archive::bookmark`] or [`Archive::add_next`] did, so the reader can be told which it was.
///
/// An enum and not a `bool`, because "already there" and "no room" are different facts and a
/// reader who is told neither has pressed a key that appeared to do nothing.
///
/// One enum for both chosen tenants rather than two identical ones. They answer the same
/// questions because they are the same kind of act — a keypress about one address, which either
/// lands, or is switched off, or is already there, or does not fit, or names something this file
/// will not write down. [`Stored::Refused`] is the one arm only [`Archive::add_next`] can return,
/// and it is what stops the sharing from being a lie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stored {
    Added,
    /// Keeping is switched off in the settings. Nothing was written, and nothing already kept
    /// was touched.
    Off,
    /// This page is already in the bookmarks. Not an error, and not a failure to act: the item is
    /// there, which is what the reader wanted.
    Already,
    /// The bookmarks is at [`MAX_ITEMS`]. Nothing was written and nothing was dropped.
    Full,
    /// The address is longer than [`MAX_URL`]. Nothing was written, and it is refused rather
    /// than shortened because a shortened address is a link to somewhere else.
    TooLong,
    /// The address is not one this file writes down: not `http`/`https`, or carrying a
    /// credential. Only [`Archive::add_next`] answers this, and only because only it is handed an
    /// address a *page* chose; see the guard there.
    Refused,
}

/// What [`Archive::visit`] did, so the caller knows whether the file changed.
///
/// There is no "already there" answer, because reopening a page restamps its entry and moves it
/// to the top, which is a change to the file like any other. Nothing is ever said to the reader
/// about a visit — it happens on every page, and a status line about it would be the application
/// narrating its own bookkeeping — so this is read by the one caller to decide whether to write,
/// and by tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visited {
    /// The history changed: a new page, or an existing one moved to the top with a new date.
    Recorded,
    /// History is switched off. Nothing was written, and nothing already recorded was touched.
    Off,
    /// The address is not one hww writes down. See [`Archive::visit`].
    Refused,
}

/// Everything hww was told to remember.
///
/// Insertion order, oldest first. [`Archive::bookmarks`] reverses it, so "most recent first" is a
/// property of the order things were added rather than of the clock — a hand-edited timestamp,
/// or a machine whose clock went backwards, cannot shuffle the list.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Archive {
    version: u32,
    items: Vec<Item>,
    /// Whether this archive is standing in for a file that could not be parsed at all.
    ///
    /// Set by [`parse`], never serialized, and read by exactly one place: [`save`], which
    /// refuses. An empty archive over an unreadable file used to be harmless, because the file
    /// only changed when the reader pressed `Ctrl+D` and a reader who read the complaint could
    /// quit and repair it. History changed that — the first page drawn writes the file — so
    /// without this the complaint and the deletion arrive in the same second, and what is
    /// deleted is the one thing in hww the reader cannot retype.
    ///
    /// Cleared by [`Archive::forget_all`] and [`Archive::forget_visits`], which are the reader
    /// saying in as many words that they want this file replaced by what hww holds now. That is
    /// the way out that does not involve finding the file by hand, and both buttons already say
    /// the thing cannot be undone.
    #[serde(skip)]
    file_unreadable: bool,
}

impl Archive {
    pub fn new() -> Self {
        Self {
            version: VERSION,
            items: Vec::new(),
            file_unreadable: false,
        }
    }

    /// The bookmarks, most recent first.
    pub fn bookmarks(&self) -> impl Iterator<Item = &Item> {
        self.items.iter().rev().filter(|i| i.kind == Kind::Bookmark)
    }

    /// The links the reader marked to read later, most recent first.
    ///
    /// Reversed like [`Archive::bookmarks`], so the link marked a moment ago is at the top of the list
    /// rather than the bottom: a reading list is read from the end the reader was last thinking
    /// about.
    pub fn read_next(&self) -> impl Iterator<Item = &Item> {
        self.items.iter().rev().filter(|i| i.kind == Kind::ReadNext)
    }

    /// The pages hww has drawn, most recent first.
    ///
    /// Insertion order reversed, as [`Archive::bookmarks`] is, so "most recent" is where the entry was
    /// put rather than what its date says: [`Archive::visit`] moves a page it has seen before to
    /// the end of the vector, and a hand-edited or backwards clock cannot shuffle the list.
    pub fn visited(&self) -> impl Iterator<Item = &Item> {
        self.items.iter().rev().filter(|i| i.kind == Kind::Visited)
    }

    /// How many pages the bookmarks holds. Not `items.len()`, which counts the history and
    /// whatever a later build wrote as well.
    ///
    /// A walk of `items` and no allocation, which is also what makes it the right thing for the
    /// bookmarks sidebar to ask every frame: the strip's length has to be right or its
    /// scrollbar and every row under its band move as marks arrive.
    pub fn len(&self) -> usize {
        self.bookmarks().count()
    }

    /// How many links the reading list holds. Not `items.len()`, for the reason
    /// [`Archive::len`] is not.
    pub fn next_len(&self) -> usize {
        self.read_next().count()
    }

    /// How many pages the history holds.
    pub fn visits(&self) -> usize {
        self.visited().count()
    }

    pub fn is_empty(&self) -> bool {
        self.bookmarks().next().is_none()
    }

    /// Every icon address a marked row still names, bookmarks and reading list alike.
    ///
    /// **Never the history**, which is the same rule [`Mark`] states for drawing, restated for
    /// keeping: a cached mark for a page hww merely watched would be a record of somewhere the
    /// reader went, on disk, outliving *forget history* and with no switch over it. The two
    /// lists here are the two whose addresses are already in this file by the reader's own act.
    ///
    /// One iterator over both rather than one per tenant, because an icon can serve a row on
    /// each: `example.org/favicon.ico` is the mark of a bookmark and of a reading-list row
    /// pointing at the same site, and "which tenant does this file belong to" is a question with
    /// no answer. What `reader::iconcache` needs is "does any marked row still name this", and
    /// that is what this yields the set for.
    ///
    /// Through [`mark_beside`], so what comes out is what the view would actually draw: an
    /// address that fails [`icon_to_store`] on the way out yields the well-known guess or
    /// nothing at all, exactly as the column does.
    pub fn marked_icons(&self) -> impl Iterator<Item = String> + '_ {
        self.items
            .iter()
            .filter(|i| matches!(i.kind, Kind::Bookmark | Kind::ReadNext))
            .filter_map(|i| mark_beside(i, Mark::Shown))
    }

    /// The bookmarks as the sidebar draws them: address, label, and the site mark beside it.
    ///
    /// Most recent first, like [`Archive::bookmarks`] it is built on, because the strip and the
    /// `hww:bookmarks` view are two windows onto one list and a reader who sees them disagree
    /// about the order learns not to trust either.
    ///
    /// Its own method rather than the caller pairing [`Archive::bookmarks`] with a mark of its
    /// own devising, and [`mark_beside`] is why: that function is the door where a credential
    /// comes off an address and where a scheme `fetch` does not speak is refused, and it is
    /// private so that there is exactly one of it. A second caller deriving `/favicon.ico` for
    /// itself would be a second door, and the first hand-edited `archive.json` carrying
    /// `file:///etc/passwd` in an `icon` field would find it.
    ///
    /// The mark is an `Option` and the row is yielded either way: a bookmark whose address
    /// yields nothing still opens, and the strip draws an empty slot rather than dropping the
    /// row and renumbering everything under it.
    ///
    /// **A window and not the list**, because the caller draws a window: `skip` rows are stepped
    /// over before anything is built for them. Both of the things this yields cost — `label`
    /// allocates and may parse the address, and `mark_beside` parses one or two more — so the
    /// strip asking for all of them at repaint rate was a `Url::parse` per bookmark per frame
    /// against a list capped in the thousands, to draw the thirty rows in its band. Skipping
    /// before the map rather than after is the whole of the fix: [`Archive::bookmarks`] yields
    /// borrowed items and allocates nothing.
    ///
    /// [`Archive::len`] is what the caller pairs this with to keep the rows it skips as tall as
    /// the rows it draws.
    pub fn bookmark_marks(
        &self,
        skip: usize,
        take: usize,
    ) -> impl Iterator<Item = (&str, String, Option<String>)> + '_ {
        self.bookmarks()
            .skip(skip)
            .take(take)
            .map(|i| (i.url.as_str(), i.label(), mark_beside(i, Mark::Shown)))
    }

    /// Whether any marked row still names this icon address.
    ///
    /// [`Archive::marked_icons`] asked about one address, and its own method because the caller
    /// that needs it is not the one that prunes: a worker's write lands on the UI thread after
    /// the permission for it was granted, and the row that granted it may have been forgotten in
    /// between. That caller has an address and needs a yes or a no, not a set to build.
    pub fn marks_icon(&self, icon: &str) -> bool {
        self.marked_icons().any(|m| m == icon)
    }

    /// Whether this page is already kept, fragment ignored.
    ///
    /// `Url` equality includes the fragment, so a raw comparison files `article#comments` and
    /// `article` as two different keeps and the menu's check mark goes off when the reader
    /// follows an anchor within the page they just saved.
    pub fn is_bookmarked(&self, url: &Url) -> bool {
        let target = without_fragment(url);
        self.bookmarks().any(|i| same_page(&i.url, &target))
    }

    /// Whether this address is already on the reading list, fragment ignored.
    ///
    /// Its own function and not a parameter on [`Archive::is_bookmarked`], because the two answer for two
    /// tenants and a caller that passed the wrong kind would silently make one list answer for the
    /// other: `Ctrl+D` would report a page kept because a link to it was marked to read, and
    /// `Shift+L` would refuse a link because the page was in the bookmarks. The kind is the whole
    /// question here, so it is in the name rather than in an argument.
    pub fn holds_next(&self, url: &Url) -> bool {
        let target = without_fragment(url);
        self.read_next().any(|i| same_page(&i.url, &target))
    }

    /// Keep one page, if the reader's settings allow it. **The one door.**
    ///
    /// The off switch is answered here, in the door's own name, rather than at the call site,
    /// for the reason `ReaderApp::request_image` states about the image policy: a door that is
    /// guarded at some call sites and not others reads as guarded and is not. A second caller —
    /// a menu item, a context menu, a future "keep every tab" — cannot route around it.
    ///
    /// Immediately, and with no debounce; the caller writes the file. `settings` debounces at
    /// 1.5 s because it absorbs a slider being dragged, and a visit debounces at the same figure
    /// because the write it schedules would otherwise land on the frame the page appears. A keep
    /// is neither: it is a single deliberate act, and the reader who presses `Ctrl+D` and shuts
    /// the laptop should find it there. That is the same asymmetry [`MAX_ITEMS`] and
    /// [`MAX_VISITS`] draw, one property over — what was chosen is treated more carefully than
    /// what was merely watched.
    pub fn bookmark(
        &mut self,
        settings: &Settings,
        url: &Url,
        title: Option<&str>,
        icon: Option<&str>,
    ) -> Stored {
        if !settings.keep_bookmarks {
            return Stored::Off;
        }
        self.bookmark_at(url, title, icon, now())
    }

    /// [`Archive::bookmark`] with the clock supplied, so a test can name the day.
    fn bookmark_at(
        &mut self,
        url: &Url,
        title: Option<&str>,
        icon: Option<&str>,
        added: u64,
    ) -> Stored {
        if url.as_str().len() > MAX_URL {
            return Stored::TooLong;
        }
        if self.is_bookmarked(url) {
            return Stored::Already;
        }
        if self.len() >= MAX_ITEMS {
            return Stored::Full;
        }
        self.items.push(Item {
            url: url.to_string(),
            title: cap_title(title.unwrap_or_default()),
            kind: Kind::Bookmark,
            added,
            // A page with no mark, or one whose mark is not an address hww will request, is
            // kept anyway: the icon is a column in a list, and refusing the page over it would
            // lose the thing the reader actually asked to keep.
            icon: icon.and_then(icon_to_store).unwrap_or_default(),
            extra: serde_json::Map::new(),
        });
        Stored::Added
    }

    /// Mark one link to read later, if the reader's settings allow it. **The one door.**
    ///
    /// The switch is answered here, in the door's own name, for the reason [`Archive::bookmark`] gives
    /// one door over. It has two callers already — a key and a context-menu item — which is
    /// exactly the case that argument was written about.
    ///
    /// **The scheme guard is what this door has and [`Archive::bookmark`] does not**, and the
    /// difference is where the address came from. `keep` is handed `prov.final_url`: an
    /// `http`/`https` address that a fetch resolved, followed, and got a document back from. This
    /// is handed an `href` **a page wrote**, so `mailto:`, `tel:`, `javascript:`, and `data:` all
    /// reach it, as does a link carrying a credential. Refusing them here rather than at the
    /// caller is the same rule as [`Archive::visit`]'s three refusals: a door guarded at one
    /// caller is a door guarded nowhere, and this one has two callers on the day it lands.
    ///
    /// The reader is still told — `ReaderApp` runs the href through `session::classify_link` first
    /// and reports what it says, because "hww will not put a `javascript:` link on a reading list"
    /// is a better sentence than silence. This guard is what makes that reporting a courtesy
    /// rather than the only thing standing between a page and this file.
    ///
    /// No debounce, for the reason [`Archive::bookmark`] gives: it is a single deliberate act.
    pub fn add_next(&mut self, settings: &Settings, url: &Url, title: Option<&str>) -> Stored {
        if !settings.keep_reading_list {
            return Stored::Off;
        }
        if refuses(url) {
            return Stored::Refused;
        }
        self.add_next_at(url, title, now())
    }

    /// [`Archive::add_next`] with the clock supplied, so a test can name the day.
    fn add_next_at(&mut self, url: &Url, title: Option<&str>, added: u64) -> Stored {
        if url.as_str().len() > MAX_URL {
            return Stored::TooLong;
        }
        if self.holds_next(url) {
            return Stored::Already;
        }
        if self.next_len() >= MAX_NEXT {
            return Stored::Full;
        }
        self.items.push(Item {
            url: url.to_string(),
            title: cap_title(title.unwrap_or_default()),
            kind: Kind::ReadNext,
            added,
            // Empty, and not for [`Archive::visit`]'s reason. This door is handed an `href` off
            // a page hww has **never fetched**, so there is no declared address to write down.
            // The reading list still draws a column: [`well_known_icon`] guesses one from the
            // row's own host when it is drawn, which keeps the guess out of the file — nothing
            // here is a record of anything a site said.
            icon: String::new(),
            extra: serde_json::Map::new(),
        });
        Stored::Added
    }

    /// Take one link off the reading list, fragment ignored, and say whether there was one.
    ///
    /// Kind-filtered like [`Archive::forget`], and here the filter is doing visible work rather
    /// than guarding against a later build: a page can be in the bookmarks *and* on the reading
    /// list at the same address, and taking it off the list must not un-keep it.
    pub fn forget_next(&mut self, url: &Url) -> bool {
        let before = self.items.len();
        let target = without_fragment(url);
        self.items
            .retain(|i| !(i.kind == Kind::ReadNext && same_page(&i.url, &target)));
        self.items.len() != before
    }

    /// Drop one page, fragment ignored, and say whether there was one. Kinds this build does not
    /// know are left alone: forgetting a page must not delete a later build's record of
    /// something else about the same address.
    pub fn forget(&mut self, url: &Url) -> bool {
        let before = self.items.len();
        let target = without_fragment(url);
        self.items
            .retain(|i| !(i.kind == Kind::Bookmark && same_page(&i.url, &target)));
        self.items.len() != before
    }

    /// Record that hww drew this page, if the reader's settings allow it. **The one door.**
    ///
    /// The switch is answered here rather than at the call site, for the reason [`Archive::bookmark`]
    /// gives one door over: a guard at some callers and not others reads as a guard and is not.
    /// This one is the more important of the two, because its caller is not a keypress — every
    /// page that reaches the column comes through here, so this `if` is the whole difference
    /// between a browser that watches and one that does not.
    ///
    /// A page already in the history is **moved rather than repeated**: its entry is removed and
    /// pushed again with the new time, so the list stays one row per page and reads most-recent
    /// first. See the module doc for why a visit-by-visit trail is not what this collects.
    ///
    /// The title is the document's, and it wins over whatever was stored: a page that has been
    /// retitled since it was last read should be listed under the title it has now.
    /// **Three addresses are refused whatever the switch says**, and they are refused here
    /// rather than at the caller for the reason above. A URL carrying a username or a password would
    /// put a credential in a plain-text file, for ever, because the reader opened a link — the
    /// bookmarks can hold one, since a keypress put it there and `Ctrl+D` takes it out again, but
    /// nothing the reader did not ask for may write one down. And an address that is not `http`
    /// or `https` is not a page fetched from the web: `hww:history` must not list itself, and
    /// `ReaderApp::install` already declines to record a built view, but a door guarded at one
    /// caller is a door guarded nowhere. And an address past [`MAX_URL`] is refused the way
    /// [`Archive::bookmark`] refuses one, silently here because nothing asked for it: this is the
    /// door a page's own links come through, and one of them carrying a kilobyte of query string
    /// is a line this file would then rewrite on every later save.
    pub fn visit(&mut self, settings: &Settings, url: &Url, title: Option<&str>) -> Visited {
        if !settings.keep_history {
            return Visited::Off;
        }
        if refuses(url) || url.as_str().len() > MAX_URL {
            return Visited::Refused;
        }
        self.visit_at(url, title, now());
        Visited::Recorded
    }

    /// [`Archive::visit`] with the clock supplied, so a test can name the day.
    fn visit_at(&mut self, url: &Url, title: Option<&str>, at: u64) {
        // The kind is part of the match: a page that is both kept and visited is two items, and
        // recording a visit must not move, restamp, or retitle the entry the reader chose.
        let target = without_fragment(url);
        self.items
            .retain(|i| !(i.kind == Kind::Visited && same_page(&i.url, &target)));
        self.items.push(Item {
            url: url.to_string(),
            title: cap_title(title.unwrap_or_default()),
            kind: Kind::Visited,
            added: at,
            // Empty, always. The history is written without anybody asking for it, so it does
            // not also write down a second address per page, and the history view draws no
            // icon column; see the module doc.
            icon: String::new(),
            extra: serde_json::Map::new(),
        });
        self.evict_oldest_visits();
    }

    /// Drop visited pages from the front until the history is inside [`MAX_VISITS`].
    ///
    /// The one place in this module that removes something nobody asked it to remove, which is
    /// why it is a function with a name rather than three lines inside [`Archive::visit_at`].
    /// Only [`Kind::Visited`] items are candidates: a full history must never evict a page the
    /// reader kept, nor a record a later build wrote.
    fn evict_oldest_visits(&mut self) {
        let mut over = self.visits().saturating_sub(MAX_VISITS);
        if over == 0 {
            return;
        }
        self.items.retain(|i| {
            if over > 0 && i.kind == Kind::Visited {
                over -= 1;
                return false;
            }
            true
        });
    }

    /// Forget the history, and say how many pages that was.
    ///
    /// The other half of [`Settings::keep_history`], and it leaves the bookmarks and the reading
    /// list exactly where they are: the three tenants are forgotten by three buttons under three
    /// switches, because a reader clearing a trail is not asking to lose the pages they saved on
    /// purpose or the links they lined up to read.
    pub fn forget_visits(&mut self) -> usize {
        let before = self.visits();
        self.items.retain(|i| i.kind != Kind::Visited);
        self.file_unreadable = false;
        before
    }

    /// Forget the whole reading list, and say how many links that was.
    ///
    /// The other half of [`Settings::keep_reading_list`], and it leaves the bookmarks and the
    /// history exactly where they are, for the reason [`Archive::forget_visits`] leaves the
    /// bookmarks: three tenants, three buttons, and none of them may take another with it.
    pub fn forget_all_next(&mut self) -> usize {
        let before = self.next_len();
        self.items.retain(|i| i.kind != Kind::ReadNext);
        self.file_unreadable = false;
        before
    }

    /// Forget the whole bookmarks, and say how many pages that was.
    ///
    /// The other half of [`Settings::keep_bookmarks`]: turning keeping off stops the writing, and
    /// this is what makes it possible to also undo it. The history and the reading list survive,
    /// as do items of a kind this build cannot show, for the reason [`Archive::forget`] leaves
    /// them: this button says "forget my bookmarks", and neither the trail, nor the links waiting to
    /// be read, nor a record this build cannot describe is something it can claim that sentence
    /// covers.
    pub fn forget_all(&mut self) -> usize {
        let before = self.len();
        self.items.retain(|i| i.kind != Kind::Bookmark);
        self.file_unreadable = false;
        before
    }

    /// Whether [`save`] will refuse to write this archive over the file it came from.
    ///
    /// True only after a file that could not be parsed at all; see [`Archive::file_unreadable`]
    /// for why an empty archive is not allowed to become the file. A dropped *item* is not this:
    /// the rest of that file parsed, and the tolerance rule says one bad item costs that item.
    pub fn refuses_to_be_written(&self) -> bool {
        self.file_unreadable
    }
}

/// Whether hww should open on the bookmarks rather than on the splash.
///
/// Out here rather than as a method on `ReaderApp`, so the fast CI job reads the decision: it is
/// two fields and an `&&`, and both halves are the sort of thing that gets inverted once and
/// noticed a release later.
///
/// The empty case is the half worth stating. `notice::idle`'s argument for not being a notice is
/// that nothing has happened yet, and on a fresh install nothing has, so an empty bookmarks list falls
/// back to the splash whatever the setting says and **first run is unchanged**.
pub fn opens_on_launch(settings: &Settings, archive: &Archive) -> bool {
    settings.open_on == crate::reader::settings::OpenOn::Bookmarks && !archive.is_empty()
}

/// The bookmarks as a document, mirroring `search::document`.
///
/// One `ir::Block::Entries`, which `ui::blocks::entries_ui` already draws, so a bookmarks list view
/// needs no rendering code of its own. It carries a title, because `title::masthead` derives the
/// masthead from the document and a page with none would open under a blank heading.
///
/// **An empty bookmarks list is a document, not an error.** `search`'s empty result is a `LoadError` and
/// an error page, and that is right there: an engine that answered with nothing is a dead end.
/// An empty bookmarks list is a new reader, and an error page would be the application scolding them for
/// not having used it yet. `entries_ui` over an empty slice draws literally nothing, so the
/// one-line paragraph is what stops the bookmarks key from opening a blank screen.
pub fn bookmarks_document(archive: &Archive) -> ir::Document {
    view(
        BOOKMARKS,
        crate::reader::notice::BOOKMARKS_TITLE,
        archive.bookmarks(),
        crate::reader::notice::bookmarks_are_empty(),
        Address::Hidden,
        Mark::Shown,
    )
}

/// The reading list as a document, built the same way and drawn by the same code.
///
/// Addresses shown, with the history and against the bookmarks, and the reason is the one the
/// [`Address`] doc gives: these rows are titled by link text, and link text is "Read more" often
/// enough that a list of them is a column of identical rows. It is also the only way to see where
/// a row goes before following it, which matters most on the list whose rows the reader has never
/// opened.
pub fn reading_list_document(archive: &Archive, settings: &Settings) -> ir::Document {
    view(
        READING_LIST,
        crate::reader::notice::READING_LIST_TITLE,
        archive.read_next(),
        crate::reader::notice::reading_list_is_empty(settings.keep_reading_list),
        Address::Shown,
        Mark::Shown,
    )
}

/// The history as a document, built the same way and drawn by the same code.
///
/// The reason the history is a page at all rather than a file the reader is told the path of:
/// the doctrine's disclosure rule is that a store nobody can see the shape of is one nobody can
/// reason about, and a list of five thousand addresses is only inspectable if opening it is a
/// keypress. It also makes the eviction in [`Archive::evict_oldest_visits`] visible, since the
/// bottom of this page is what the next new page will push out.
pub fn history_document(archive: &Archive, settings: &Settings) -> ir::Document {
    view(
        HISTORY,
        crate::reader::notice::HISTORY_TITLE,
        archive.visited(),
        crate::reader::notice::history_is_empty(settings.keep_history),
        Address::Shown,
        Mark::Hidden,
    )
}

/// Whether a view writes each item's address under its title.
///
/// The history and the reading list do; the bookmarks does not. It is a difference in what the
/// lists are for, and the bookmarks is the odd one out rather than the rule. The bookmarks is short and
/// every row is a page the reader chose, opened, read, and can name; the history is thousands of
/// rows long, nobody chose any of them, and the titles pages give themselves repeat — half a dozen
/// "Home"s and three "(1) Inbox"es are one list of identical rows without the address to tell them
/// apart. The reading list is short like the bookmarks and anonymous like the history: its rows are
/// named by link text, which is "Read more" and "Continue reading" often enough to be no name at
/// all. So the question is not how long the list is, it is whether the row's own words identify
/// it, and only the bookmarks' do. It is also the only way to see where a row goes before following
/// it, which matters most where the reader has never been.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Address {
    Shown,
    Hidden,
}

/// Whether a view draws each row's site mark in a column of its own.
///
/// **The two lists the reader chose do; the history does not**, and the difference is a request
/// rather than a look. A mark is a third-party image fetched when the view opens, so a page of
/// rows is a page of requests. The bookmarks and the reading list are short, capped at
/// [`MAX_ITEMS`] and [`MAX_NEXT`], and every row on either went there one keypress at a time.
/// The history is thousands of rows nobody chose, and opening it must not contact thousands of
/// hosts. [`Archive::visit`] stores no icon either, so that is the second of two locks rather
/// than the only one — a hand-edited file cannot turn the history into a request storm by adding
/// a field.
///
/// The two marked lists get their marks from different places, which [`mark_beside`] settles:
/// the bookmarks has the address the page declared for itself, and the reading list has nothing at
/// all, because hww has never fetched the page a row points at. That is what
/// [`well_known_icon`] is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mark {
    Shown,
    Hidden,
}

/// One of hww's own list views: entries when there are any, one sentence when there are not.
///
/// Shared by both, because the two differ in four strings, one flag, and nothing else, and the
/// half that would have drifted is the empty case — the argument for it being a document rather
/// than an error page is the same argument twice.
fn view<'a>(
    url: &str,
    title: &str,
    items: impl Iterator<Item = &'a Item>,
    when_empty: String,
    address: Address,
    mark: Mark,
) -> ir::Document {
    let entries: Vec<ir::Entry> = items
        .map(|i| ir::Entry {
            title: vec![Inline::Text(i.label())],
            href: Some(i.url.clone()),
            summary: Vec::new(),
            published: i.on_day(),
            address: address_under(i, address),
            icon: mark_beside(i, mark),
            image: None,
        })
        .collect();
    let blocks = if entries.is_empty() {
        vec![Block::Paragraph(vec![Inline::Text(when_empty)])]
    } else {
        vec![Block::Entries(entries)]
    };
    ir::Document {
        url: url.to_owned(),
        title: Some(title.to_owned()),
        byline: None,
        published: None,
        site_name: None,
        favicon: None,
        lang: None,
        blocks,
    }
}

/// The address to draw under an entry's title, or `None`.
///
/// [`ir::Entry::address`] and not a `Block::Paragraph` in the summary: the summary is a dek, set
/// in the page's body type, and an address set as prose reads as the first line of the page it
/// points at. The field says the row carries an address and lets `ui::blocks::entries_ui` set it
/// the way it sets the date beside it.
///
/// Nothing is drawn under an untitled page, because [`Item::label`] has already made its address
/// the headline and a row that says the same thing twice reads as a bug.
fn address_under(item: &Item, address: Address) -> Option<String> {
    (address == Address::Shown && !item.title.trim().is_empty()).then(|| item.url.clone())
}

/// The mark to draw in the column left of an entry's headline, or `None`.
///
/// Re-checked through [`icon_to_store`] on the way *out* as well as on the way in, because the
/// file between the two is one a reader may edit and a later build may write: an item carrying
/// `file:///etc/passwd` or a `data:` URL in this field must not become a request because an
/// older save wrote it. The cost of asking twice is a `Url::parse` per visible row.
fn mark_beside(item: &Item, mark: Mark) -> Option<String> {
    if mark != Mark::Shown {
        return None;
    }
    icon_to_store(&item.icon).or_else(|| well_known_icon(&item.url))
}

/// The mark a row's own address implies: `/favicon.ico` at its origin, or `None`.
///
/// **A guess, and the same guess `html::favicon_of` makes** when a page's markup names no icon,
/// so for the bookmarks this reconstructs what would have been stored rather than inventing a new
/// claim. For the reading list it is the only thing available: those rows point at pages hww has
/// never fetched, so there is no declared address to have kept, and the choice is this or a
/// column that is empty on the one list whose rows the reader has not seen.
///
/// What it costs is a request that may 404, which is why it is a *guess* stated in the name.
/// What it does not cost is accuracy about anything else: a site that serves no icon there draws
/// no mark, exactly as a site that declared one hww could not fetch does.
///
/// `http(s)` only, through [`icon_to_store`], so one function decides what may be requested and
/// a `mailto:` row — which [`Archive::add_next`] refuses, but a hand-edited file does not —
/// yields nothing rather than a joined address with a scheme `fetch` does not speak. That is
/// also where the credentials come off: `Url::join` keeps the userinfo of the address it joins
/// against, so this would otherwise turn a bookmarked `https://user:pass@intranet/page` into an
/// automatic authenticated request.
fn well_known_icon(url: &str) -> Option<String> {
    let url = Url::parse(url).ok()?;
    icon_to_store(url.join("/favicon.ico").ok()?.as_str())
}

/// An icon address worth writing down, or `None`.
///
/// Absolute, `http(s)`, and inside [`MAX_URL`]. The scheme test is the one that matters: this is
/// the only address in the file hww fetches *without* the reader following anything, so the set
/// of things it can name is kept to the two schemes `fetch` speaks. A `data:` URL would also be
/// page-chosen bytes written to disk, which is the one thing this file does not hold.
fn icon_to_store(icon: &str) -> Option<String> {
    let icon = icon.trim();
    if icon.len() > MAX_URL {
        return None;
    }
    let mut url = Url::parse(icon).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    // **Credentials come off here, and this is the only door they can come off at.**
    //
    // `Archive::add_next` and `Archive::visit` refuse a credential-bearing address outright;
    // `Archive::bookmark` deliberately does not, because a keypress put that page there and
    // `Ctrl+D` takes it back out. A mark is the third case and is neither: `html::favicon_of`
    // builds it by joining against `prov.final_url`, so a bookmarked
    // `https://user:pass@intranet.example/page` yields a mark carrying that password — and
    // unlike the row's own address, which is only ever followed by a click, *this* one hww
    // requests by itself the moment the view is drawn, with reqwest deriving a Basic auth
    // header from the userinfo. An automatic authenticated request, for a page nobody opened.
    //
    // Stripped rather than refused, so an intranet bookmark still has a column: the host is
    // being asked for its icon either way and learns nothing from the request it did not
    // already know, while the password is the part that must not go. An unauthenticated 401
    // draws no mark, which is what a site that serves no icon looks like.
    if !url.username().is_empty() || url.password().is_some() {
        url.set_username("").ok()?;
        url.set_password(None).ok()?;
    }
    Some(url.to_string())
}

/// The address a site mark may be requested and kept at, or `None`.
///
/// [`icon_to_store`] under a name that says what the *reader* of this function wants, which is
/// not always storage: `reader::iconcache` keys its files by this, and `ui::app` normalises the
/// masthead's own `<link rel=icon>` through it before asking whether the bytes are on disk. One
/// door, so a cache key can never be built from an address that skipped the scheme test or kept
/// its credentials — which is the trap `icon_to_store`'s own comment describes.
pub fn icon_address(icon: &str) -> Option<String> {
    icon_to_store(icon)
}

/// The address the two unasked-for tenants refuse: a non-web scheme, or a credential.
///
/// One predicate, and still asked inside each door rather than at either caller — the module doc
/// is explicit that a door guarded at one caller is a door guarded nowhere, and this changes
/// where the rule is *written*, not where it is applied. [`Archive::visit`] adds a length of its
/// own on top, which is why that half is not in here.
///
/// Deliberately not shared with `sites::apply_in`, which spells the two halves as separate `if`s
/// because they are separate reasons (gemini and gopher pass through untouched; a credential is
/// an auth boundary), nor with [`icon_to_store`], which *strips* a credential rather than
/// refusing the address. Three doors, three answers; merging them would erase two of them.
fn refuses(url: &Url) -> bool {
    !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
}

/// Whether a stored address and a live one are the same page, fragment aside.
///
/// `target` is [`without_fragment`] of the address being asked about, computed once by the
/// caller. **This is a byte comparison, and that is a decision rather than an oversight.**
///
/// It used to `Url::parse` the stored string, strip its fragment, and compare the two
/// normalized forms — per item, and the items are all three tenants in one vector. A navigation
/// asks it of every visited row and then of every bookmark, which at the caps is seven thousand
/// URL parses and fourteen thousand allocations on the frame a page installs; `Ctrl+D` asked
/// three times over. Every string this build stores came out of `Url::to_string`
/// ([`Archive::bookmark`], [`Archive::add_next`], [`Archive::visit_at`]), and `url` percent-
/// encodes `#` everywhere but the fragment delimiter, so the text before the first `#` *is* the
/// normalized fragment-stripped form and comparing bytes gives the same answer.
///
/// What it gives a different answer for is an entry no version of hww wrote: a hand-edited
/// `archive.json`, which `settings::archive_path` documents as the way to carry a 0.3
/// `library.json` across the rename. `HTTP://Example.COM/a` matched under the parse and does not
/// match here, so such a row can be bookmarked twice or gain a second history line instead of
/// having its first moved. That is the whole cost, it is confined to a file the reader edited by
/// hand, and `same_page_is_the_parse_it_replaced` pins the agreement for every shape hww itself
/// writes.
fn same_page(stored: &str, target: &str) -> bool {
    stored.split_once('#').map_or(stored, |(head, _)| head) == target
}

fn without_fragment(url: &Url) -> String {
    let mut u = url.clone();
    u.set_fragment(None);
    u.into()
}

/// Whether two live addresses are the same page, fragment aside.
///
/// The other half of [`same_page`], for a caller holding two `Url`s rather than a stored string:
/// `ReaderApp::same_page` asks it of the address a link names and the address of the page on
/// screen, which is a different question from what this module stores but the same comparison,
/// and it was spelled out a second time there. No parse and no `String`, because neither side
/// needs one — `Url` compares its own parts.
pub fn same_page_urls(a: &Url, b: &Url) -> bool {
    let strip = |u: &Url| {
        let mut u = u.clone();
        u.set_fragment(None);
        u
    };
    strip(a) == strip(b)
}

/// Truncate to [`MAX_TITLE`] characters, with an ellipsis when anything was dropped.
///
/// Whitespace is collapsed first: a `<title>` split over three indented lines of markup arrives
/// carrying its newlines, and a stored title is a single line in a list.
fn cap_title(title: &str) -> String {
    let flat = crate::ir::normalize_ws(title);
    if flat.chars().count() <= MAX_TITLE {
        return flat;
    }
    let cut: String = flat.chars().take(MAX_TITLE - 1).collect();
    format!("{}…", cut.trim_end())
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

pub fn path() -> Option<PathBuf> {
    settings::archive_path()
}

/// Where the kept site icons live: a directory beside this file, never inside it.
///
/// **Derived, reconstructible, byte-valued data lives beside `archive.json` and not in it.** The
/// module doc's rule about tenants — a new one is a new [`Kind`] in the one file, never a second
/// file — is about things the reader chose and cannot retype. Icon bytes are neither: every one
/// of them is a fetch away from coming back, and none of them was chosen. The concrete reason is
/// arithmetic rather than taste: the history rewrites this file on every navigation, so
/// base64 icons inside it would rewrite kilobytes per page drawn.
///
/// `reader::iconcache` holds the doctrine for what may go in there and why it is allowed at all.
pub fn icons_path() -> Option<PathBuf> {
    settings::icons_dir()
}

/// Load, or fall back to an empty bookmarks list with a reason.
///
/// [`settings::load`]'s contract, matched deliberately: a parse failure returns something usable
/// *and* the complaint, rather than refusing to start. A reader that will not open because of a
/// stray comma in a list of bookmarks is a worse outcome than one that opens and says so in the
/// status strip.
///
/// Tolerance is per item, which is the half `settings` does not need. An item that will not parse
/// costs that item and is counted in the complaint; the rest of the bookmarks still opens. Unknown
/// fields ride along in [`Item::extra`] and unknown kinds in [`Kind::Other`], so neither is a
/// parse failure at all.
pub fn load() -> (Archive, Option<String>) {
    let Some(path) = path() else {
        return (Archive::new(), None);
    };
    let text = match std::fs::read_to_string(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Archive::new(), None),
        Err(e) => {
            return (
                Archive::new(),
                Some(format!("could not read {}: {e}", path.display())),
            );
        }
        Ok(t) => t,
    };
    parse(&text, &path.display().to_string())
}

/// The half of [`load`] that touches no filesystem, so the tolerance rules are testable.
fn parse(text: &str, where_from: &str) -> (Archive, Option<String>) {
    /// The file as read: every item still a `Value`, so one bad item can be dropped rather than
    /// failing the file. `version` and `items` both default, so a file missing either is a file
    /// with an empty bookmarks list rather than a complaint about a shape nobody typed.
    #[derive(serde::Deserialize)]
    struct Wire {
        #[serde(default)]
        version: u32,
        #[serde(default)]
        items: Vec<serde_json::Value>,
    }

    let wire: Wire = match serde_json::from_str(text) {
        Ok(w) => w,
        Err(e) => {
            // Empty *and* sealed. The empty half is `settings::load`'s contract and unchanged;
            // the sealed half is what history's default made necessary, because the next page
            // drawn would otherwise write this empty archive over whatever the file really
            // holds. See `Archive::file_unreadable`, and note that the complaint has to say so:
            // a store that has stopped recording without saying it is the quiet lie again.
            let archive = Archive {
                file_unreadable: true,
                ..Archive::new()
            };
            return (
                archive,
                Some(format!(
                    "{where_from} is not a valid archive ({e}); hww opened without it and will \
                     not write over it — repair or move that file, or empty it from Settings"
                )),
            );
        }
    };
    let mut items = Vec::with_capacity(wire.items.len());
    let mut dropped = 0usize;
    for value in wire.items {
        match serde_json::from_value::<Item>(value) {
            Ok(item) => items.push(item),
            Err(_) => dropped += 1,
        }
    }
    // The version is kept as read rather than stamped forward. A file this build cannot fully
    // understand should not come back claiming this build's shape.
    let version = if wire.version == 0 {
        VERSION
    } else {
        wire.version
    };
    let note = (dropped > 0).then(|| {
        format!(
            "{where_from}: {dropped} item(s) could not be read and were left out; the rest of \
             your bookmarks opened"
        )
    });
    (
        Archive {
            version,
            items,
            file_unreadable: false,
        },
        note,
    )
}

/// Write the bookmarks, atomically, through the same helper `settings::save` uses.
///
/// Two refusals, and both are errors rather than a quiet `Ok`. **There is nowhere to write**:
/// with no `HWW_CONFIG_DIR`, no `HOME` and no `XDG_CONFIG_HOME` there is no config directory at
/// all, and answering `Ok(())` there let `bookmark_page` tell the reader their page was kept and the
/// menu draw a tick for a file that does not exist — the one claim the doctrine says may never
/// be made without the write behind it. **The file could not be read**: see
/// [`Archive::refuses_to_be_written`].
pub fn save(archive: &Archive) -> std::io::Result<()> {
    let Some(path) = path() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "there is no configuration directory to write to",
        ));
    };
    if archive.refuses_to_be_written() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "{} could not be read when hww started, so it will not be written over; \
                 repair or move that file, or empty it from Settings",
                path.display()
            ),
        ));
    }
    let json = serde_json::to_string_pretty(archive)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    settings::write_atomically(&path, json.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    /// Settings with keeping on, which is the default.
    fn on() -> Settings {
        Settings::default()
    }

    /// The window is the rows it claims to be, because the strip positions everything by index.
    ///
    /// `skip`/`take` is the only logic this façade adds — the marks themselves are `mark_beside`,
    /// pinned by `a_stored_mark_is_checked_again_before_it_is_drawn` and the three tests beside
    /// it — and it is the half a screenshot cannot catch. The strip pads above and below with
    /// `first * step` and `(count - last) * step`, so a window off by one row draws every mark
    /// against the wrong bookmark and the reader clicks a square for a page they did not choose.
    #[test]
    fn the_sidebars_window_is_the_slice_of_the_list_it_names() {
        let mut a = Archive::new();
        for i in 0..5 {
            a.bookmark(
                &on(),
                &u(&format!("https://example.com/{i}")),
                Some(&format!("page {i}")),
                None,
            );
        }
        let all: Vec<&str> = a.bookmarks().map(|i| i.url.as_str()).collect();
        assert_eq!(all.len(), 5);

        let window: Vec<String> = a
            .bookmark_marks(1, 2)
            .map(|(url, _, _)| url.to_owned())
            .collect();
        assert_eq!(
            window,
            all[1..3].iter().map(|u| u.to_string()).collect::<Vec<_>>(),
            "the window must be the rows the strip skipped space for"
        );

        // A window running off the end yields what exists and does not panic; the strip asks
        // for one row past the band on purpose. See `measure::Band::rows`.
        assert_eq!(a.bookmark_marks(4, 9).count(), 1);
        assert_eq!(a.bookmark_marks(9, 9).count(), 0);
        // And the count the strip pads against is the same list this walks.
        assert_eq!(a.len(), all.len());
    }

    /// The label travels with the row, because it is the only name an assistive technology gets
    /// for a control that carries no text.
    #[test]
    fn a_sidebar_row_is_named_even_when_the_page_had_no_title() {
        let mut a = Archive::new();
        a.bookmark(&on(), &u("https://example.com/untitled"), None, None);
        let (_, label, _) = a.bookmark_marks(0, usize::MAX).next().expect("one row");
        assert!(
            !label.trim().is_empty(),
            "a row with no name is an unreachable control"
        );
    }

    #[test]
    fn a_kept_page_round_trips_through_the_file_format() {
        let mut a = Archive::new();
        assert_eq!(
            a.bookmark(&on(), &u("https://example.com/a"), Some("A page"), None),
            Stored::Added
        );
        let text = serde_json::to_string_pretty(&a).unwrap();
        let (back, note) = parse(&text, "the fixture");
        assert!(note.is_none());
        assert_eq!(back, a);
        let item = back.bookmarks().next().expect("the item");
        assert_eq!(item.url, "https://example.com/a");
        assert_eq!(item.title, "A page");
        assert_eq!(item.kind, Kind::Bookmark);
    }

    /// The byte comparison answers as the parse did, for every address hww writes.
    ///
    /// [`same_page`] stopped parsing the stored string, which is only sound because every string
    /// this build stores came from `Url::to_string`. So the test is the claim: for each shape,
    /// store the address the way the doors do and check the new comparison against the old one,
    /// both for the address itself and for the same address wearing a fragment.
    ///
    /// The one case they disagree on is deliberate and is asserted last: an entry hand-written
    /// into `archive.json`, which is the only way a non-normalized string can get in.
    #[test]
    fn same_page_is_the_parse_it_replaced() {
        /// What [`same_page`] used to be.
        fn by_parse(stored: &str, url: &Url) -> bool {
            match Url::parse(stored) {
                Ok(s) => without_fragment(&s) == without_fragment(url),
                Err(_) => stored == url.as_str(),
            }
        }
        let shapes = [
            "https://example.com/a",
            "https://EXAMPLE.com/A",
            "https://example.com:443/a",
            "http://example.com:80/a",
            "https://example.com",
            "https://example.com/",
            "https://example.com/a%20b/c",
            "https://example.com/a?",
            "https://example.com/a?q=1&r=2",
            "https://example.com/a#",
            "https://example.com/a#frag",
            "https://example.com/%D7%A9%D7%9C%D7%95%D7%9D",
            "https://xn--80ak6aa92e.com/a",
            "https://example.com/a/../b",
            "https://user@example.com/a",
        ];
        for shape in shapes {
            let url = Url::parse(shape).expect("a fixture URL parses");
            // Stored the way every door stores it.
            let stored = url.to_string();
            for probe in [shape.to_owned(), format!("{}#later", url.as_str())] {
                let Ok(probe) = Url::parse(&probe) else {
                    continue;
                };
                let target = without_fragment(&probe);
                assert_eq!(
                    same_page(&stored, &target),
                    by_parse(&stored, &probe),
                    "{stored} against {probe}"
                );
                assert!(same_page(&stored, &target), "{stored} is {probe}");
            }
            // And a different page is still a different page.
            let other = Url::parse("https://example.net/z").unwrap();
            assert!(!same_page(&stored, &without_fragment(&other)));
        }

        // The documented divergence, in the one place it can arise: a row somebody typed.
        let hand_edited = "HTTPS://Example.COM/a";
        let live = Url::parse("https://example.com/a").unwrap();
        assert!(by_parse(hand_edited, &live), "the parse matched it");
        assert!(
            !same_page(hand_edited, &without_fragment(&live)),
            "and the byte comparison does not; see `same_page`"
        );
    }

    /// The `settings::load` contract, one file over: never fatal, always a reason.
    #[test]
    fn a_corrupt_archive_is_never_fatal_and_says_so() {
        let (a, note) = parse("{ not json", "archive.json");
        assert!(a.is_empty());
        let note = note.expect("a complaint");
        assert!(note.contains("archive.json"), "it names the file: {note}");
        assert!(
            note.contains("not write over it"),
            "and says the file is left alone: {note}"
        );
    }

    /// The half of that contract history's default made necessary. An empty archive over a file
    /// hww could not read must not become the file: with history on, the first page drawn writes
    /// it, so a stray comma would otherwise cost the whole bookmarks seconds after start.
    #[test]
    fn an_unreadable_file_is_not_written_over() {
        let (a, _) = parse("{ not json", "archive.json");
        assert!(a.refuses_to_be_written());
        // A file that parsed does not carry the seal, whatever else it says.
        let (good, _) = parse(r#"{"version":1,"items":[]}"#, "archive.json");
        assert!(!good.refuses_to_be_written());
        // Nor does a file that parsed with one item dropped: the tolerance rule is that one bad
        // item costs that item, and sealing there would let a single stray field freeze the file.
        let (tolerated, note) = parse(
            r#"{"version":1,"items":[{"url":7},{"url":"https://e.com/","kind":"bookmark"}]}"#,
            "archive.json",
        );
        assert!(note.is_some(), "the dropped item is still reported");
        assert!(!tolerated.refuses_to_be_written());
    }

    /// The way out that does not involve finding the file by hand. Either forget button is the
    /// reader saying they want this file replaced by what hww holds now, and both already say
    /// the thing cannot be undone.
    #[test]
    fn forgetting_a_tenant_unseals_an_unreadable_file() {
        let (mut a, _) = parse("{ not json", "archive.json");
        assert_eq!(a.forget_all(), 0);
        assert!(!a.refuses_to_be_written());

        let (mut b, _) = parse("{ not json", "archive.json");
        assert_eq!(b.forget_visits(), 0);
        assert!(!b.refuses_to_be_written());

        // Forgetting *one* page is not that sentence: it is about a page, not about the file.
        let (mut c, _) = parse("{ not json", "archive.json");
        assert!(!c.forget(&u("https://example.com/a")));
        assert!(c.refuses_to_be_written());
    }

    /// An address is a page's text as much as its title is, and unlike the title it is rewritten
    /// on every later save. The title is cut and the address is refused: half an address is a
    /// link to somewhere else.
    #[test]
    fn an_address_past_the_cap_is_refused_by_both_doors() {
        let long = u(&format!("https://example.com/{}", "a".repeat(MAX_URL)));
        let mut a = Archive::new();
        assert_eq!(
            a.bookmark(&on(), &long, Some("A page"), None),
            Stored::TooLong
        );
        assert_eq!(a.len(), 0);
        assert_eq!(a.visit(&on(), &long, Some("A page")), Visited::Refused);
        assert_eq!(a.visits(), 0);

        // And an address that fits is untouched, byte for byte: nothing here shortens anything.
        let ordinary = u("https://example.com/a?q=1#x");
        assert_eq!(a.bookmark(&on(), &ordinary, None, None), Stored::Added);
        assert_eq!(
            a.bookmarks().next().expect("the item").url,
            ordinary.as_str()
        );
    }

    /// The half `settings` does not need: one unreadable item costs that item.
    #[test]
    fn one_unreadable_item_does_not_cost_the_rest_of_the_file() {
        let text = r#"{
            "version": 1,
            "items": [
                {"url": "https://example.com/a", "title": "A", "kind": "bookmark", "added": 1},
                {"kind": "bookmark"},
                {"url": "https://example.com/b", "title": "B", "kind": "bookmark", "added": 2}
            ]
        }"#;
        let (a, note) = parse(text, "archive.json");
        let labels: Vec<String> = a.bookmarks().map(|i| i.label()).collect();
        assert_eq!(labels, vec!["B", "A"], "most recent first");
        assert!(note.expect("a complaint").contains("1 item(s)"));
    }

    /// Forward compatibility, both halves. A field this build has no name for survives a save,
    /// and a kind it cannot show is carried rather than displayed or deleted.
    #[test]
    fn an_unknown_field_or_kind_does_not_cost_the_rest_of_the_file() {
        let text = r#"{
            "version": 7,
            "items": [
                {"url": "https://example.com/a", "title": "A", "kind": "bookmark", "added": 1,
                 "colour": "blue"},
                {"url": "https://example.com/f", "title": "F", "kind": "subscribed", "added": 2}
            ]
        }"#;
        let (mut a, note) = parse(text, "archive.json");
        assert!(note.is_none(), "neither of these is a parse failure");
        assert_eq!(
            a.len(),
            1,
            "a tenant this build has no name for is not a keep"
        );
        assert_eq!(a.visits(), 0, "nor is it a page hww drew");
        assert_eq!(a.bookmarks().next().unwrap().label(), "A");

        // A keep by this build must not strip either of them.
        a.bookmark(&on(), &u("https://example.com/b"), Some("B"), None);
        let written = serde_json::to_string(&a).unwrap();
        assert!(written.contains("\"colour\":\"blue\""), "{written}");
        assert!(written.contains("\"subscribed\""), "{written}");
        assert!(
            written.contains("\"version\":7"),
            "a later shape is not restamped: {written}"
        );
    }

    /// The off switch's other half. Forgetting everything empties what this build shows and
    /// leaves a later build's records where they are.
    #[test]
    fn forgetting_everything_leaves_a_later_builds_records_alone() {
        let text = r#"{"version": 7, "items": [
            {"url": "https://example.com/a", "kind": "bookmark", "added": 1},
            {"url": "https://example.com/f", "kind": "subscribed", "added": 2}
        ]}"#;
        let (mut a, _) = parse(text, "archive.json");
        assert_eq!(a.forget_all(), 1);
        assert!(a.is_empty());
        assert!(serde_json::to_string(&a).unwrap().contains("subscribed"));
    }

    /// `Url` equality includes the fragment, so without the strip the same article is two keeps
    /// and the menu's check mark goes off when the reader follows an anchor within it.
    #[test]
    fn keeping_a_page_twice_or_with_a_fragment_is_one_entry() {
        let mut a = Archive::new();
        assert_eq!(
            a.bookmark(&on(), &u("https://example.com/a"), Some("A"), None),
            Stored::Added
        );
        assert_eq!(
            a.bookmark(&on(), &u("https://example.com/a"), Some("A"), None),
            Stored::Already
        );
        assert_eq!(
            a.bookmark(&on(), &u("https://example.com/a#comments"), Some("A"), None),
            Stored::Already
        );
        assert_eq!(a.len(), 1);
        assert!(a.is_bookmarked(&u("https://example.com/a#anything")));
        // A different path is a different page.
        assert!(!a.is_bookmarked(&u("https://example.com/b")));
        // And a query is part of the address, not decoration on it.
        assert!(!a.is_bookmarked(&u("https://example.com/a?page=2")));
    }

    /// The link opens what was kept, fragment included; only the comparison strips it.
    #[test]
    fn the_stored_address_keeps_the_fragment_it_was_given() {
        let mut a = Archive::new();
        a.bookmark(&on(), &u("https://example.com/a#part-two"), Some("A"), None);
        assert_eq!(
            a.bookmarks().next().unwrap().url,
            "https://example.com/a#part-two"
        );
    }

    #[test]
    fn forgetting_removes_one_page_and_ignores_the_fragment() {
        let mut a = Archive::new();
        a.bookmark(&on(), &u("https://example.com/a"), Some("A"), None);
        a.bookmark(&on(), &u("https://example.com/b"), Some("B"), None);
        assert!(a.forget(&u("https://example.com/a#anywhere")));
        assert!(!a.forget(&u("https://example.com/a")));
        assert_eq!(a.len(), 1);
        assert!(a.is_bookmarked(&u("https://example.com/b")));
    }

    /// The title is untrusted text from a page and now outlives the process.
    #[test]
    fn an_over_long_title_is_capped_on_the_way_in() {
        let mut a = Archive::new();
        let long = "x".repeat(MAX_TITLE * 4);
        a.bookmark(&on(), &u("https://example.com/a"), Some(&long), None);
        let stored = &a.bookmarks().next().unwrap().title;
        assert_eq!(stored.chars().count(), MAX_TITLE);
        assert!(stored.ends_with('…'));
        // Multi-byte characters must not be cut mid-character, which is a panic and not a bug
        // report.
        let mut b = Archive::new();
        b.bookmark(
            &on(),
            &u("https://example.com/b"),
            Some(&"é".repeat(MAX_TITLE * 2)),
            None,
        );
        assert_eq!(
            b.bookmarks().next().unwrap().title.chars().count(),
            MAX_TITLE
        );
    }

    /// A title arrives as it was written in the markup, newlines and all.
    #[test]
    fn a_stored_title_is_one_line() {
        let mut a = Archive::new();
        a.bookmark(
            &on(),
            &u("https://example.com/a"),
            Some("  A page\n   split over lines  "),
            None,
        );
        assert_eq!(
            a.bookmarks().next().unwrap().title,
            "A page split over lines"
        );
    }

    /// The cap refuses rather than evicting: the oldest keep is still there afterwards.
    #[test]
    fn a_full_bookmarks_refuses_rather_than_dropping_the_oldest() {
        let mut a = Archive::new();
        for i in 0..MAX_ITEMS {
            assert_eq!(
                a.bookmark(
                    &on(),
                    &u(&format!("https://example.com/{i}")),
                    Some("x"),
                    None
                ),
                Stored::Added
            );
        }
        assert_eq!(
            a.bookmark(&on(), &u("https://example.com/one-more"), Some("x"), None),
            Stored::Full
        );
        assert_eq!(a.len(), MAX_ITEMS);
        assert!(
            a.is_bookmarked(&u("https://example.com/0")),
            "the oldest keep survived"
        );
        assert!(!a.is_bookmarked(&u("https://example.com/one-more")));
    }

    /// The view is one `Entries` block, which is the whole reason the bookmarks needs no
    /// rendering code of its own.
    #[test]
    fn the_bookmarks_view_is_one_entries_block() {
        let mut a = Archive::new();
        a.bookmark(&on(), &u("https://example.com/a"), Some("A"), None);
        a.bookmark(&on(), &u("https://example.com/b"), Some("B"), None);
        let doc = bookmarks_document(&a);
        assert_eq!(doc.url, BOOKMARKS);
        assert!(doc.title.is_some(), "the masthead is derived from this");
        assert_eq!(doc.blocks.len(), 1);
        let ir::Block::Entries(entries) = &doc.blocks[0] else {
            panic!("expected one Entries block, got {:?}", doc.blocks);
        };
        assert_eq!(entries.len(), 2);
        // Most recent first, each linking to what was kept, and no picture: see the module doc.
        assert_eq!(ir::plain_text(&entries[0].title), "B");
        assert_eq!(entries[0].href.as_deref(), Some("https://example.com/b"));
        assert!(entries.iter().all(|e| e.image.is_none()));
    }

    /// An empty bookmarks list is a new reader, not a failure, so the bookmarks key opens a page
    /// that says what to
    /// do rather than a blank screen or an error.
    #[test]
    fn an_empty_bookmarks_view_is_a_page_that_says_so() {
        let doc = bookmarks_document(&Archive::new());
        assert_eq!(doc.blocks.len(), 1);
        assert!(matches!(doc.blocks[0], ir::Block::Paragraph(_)));
        let words = ir::plain_text(match &doc.blocks[0] {
            ir::Block::Paragraph(i) => i,
            _ => unreachable!(),
        });
        assert!(
            words.contains(crate::reader::menu::BOOKMARK_PAGE),
            "the empty bookmarks list names the key that fills it: {words}"
        );
    }

    /// A page with no title is named by its address rather than drawn as a blank row.
    #[test]
    fn an_untitled_page_is_named_by_its_address() {
        let mut a = Archive::new();
        a.bookmark(&on(), &u("https://example.com/a/b"), None, None);
        assert_eq!(a.bookmarks().next().unwrap().label(), "example.com/a/b");
        let mut b = Archive::new();
        b.bookmark(&on(), &u("https://example.com/"), Some("   "), None);
        assert_eq!(b.bookmarks().next().unwrap().label(), "example.com");
    }

    /// First run is unchanged: an empty bookmarks list opens on the splash whatever the setting says.
    #[test]
    fn an_empty_bookmarks_never_takes_the_home_screen() {
        let want_bookmarks = Settings::default();
        let want_splash = Settings {
            open_on: crate::reader::settings::OpenOn::Nothing,
            ..Settings::default()
        };
        let empty = Archive::new();
        assert!(
            !opens_on_launch(&want_bookmarks, &empty),
            "a fresh install must still meet the splash"
        );
        assert!(!opens_on_launch(&want_splash, &empty));

        let mut full = Archive::new();
        full.bookmark(&on(), &u("https://example.com/a"), Some("A"), None);
        assert!(opens_on_launch(&want_bookmarks, &full));
        assert!(
            !opens_on_launch(&want_splash, &full),
            "the reader who asked for the splash back gets it"
        );
    }

    /// Settings with history off, which is the switch a reader has to go and find.
    fn watching_off() -> Settings {
        Settings {
            keep_history: false,
            ..Settings::default()
        }
    }

    /// The second tenant, through the file and back, filed under its own kind and counted by
    /// neither of the bookmarks' counters.
    #[test]
    fn a_visited_page_round_trips_and_is_not_a_kept_page() {
        let mut a = Archive::new();
        assert_eq!(
            a.visit(&on(), &u("https://example.com/a"), Some("A page")),
            Visited::Recorded
        );
        let (back, note) = parse(&serde_json::to_string_pretty(&a).unwrap(), "the fixture");
        assert!(note.is_none());
        assert_eq!(back, a);
        let item = back.visited().next().expect("the item");
        assert_eq!(item.kind, Kind::Visited);
        assert_eq!(item.url, "https://example.com/a");
        assert_eq!(item.title, "A page");
        // The doctrine's line, as arithmetic: a page hww watched is not a page the reader named.
        assert_eq!(back.visits(), 1);
        assert_eq!(back.len(), 0);
        assert!(back.is_empty());
        assert!(!back.is_bookmarked(&u("https://example.com/a")));
    }

    /// **The switch.** Its caller is not a keypress, so this is the whole difference between a
    /// browser that watches and one that does not.
    #[test]
    fn history_off_writes_nothing_and_keeps_what_is_there() {
        let mut a = Archive::new();
        a.visit(&on(), &u("https://example.com/first"), Some("First"));
        assert_eq!(
            a.visit(&watching_off(), &u("https://example.com/second"), None),
            Visited::Off
        );
        let urls: Vec<&str> = a.visited().map(|i| i.url.as_str()).collect();
        assert_eq!(
            urls,
            vec!["https://example.com/first"],
            "off stops the writing and leaves what is already there"
        );
        // And the other switch is not this one: keeping still works with watching off.
        assert_eq!(
            a.bookmark(
                &watching_off(),
                &u("https://example.com/k"),
                Some("K"),
                None
            ),
            Stored::Added
        );
    }

    /// The two addresses the door refuses whatever the switch says: one that would write a
    /// credential into a plain-text file because a link was followed, and one that is not a page
    /// from the web at all. `ReaderApp::install` also declines to record a built view, and that
    /// is the point — a door guarded at one caller is a door guarded nowhere.
    #[test]
    fn a_credential_or_an_internal_address_is_never_written_down() {
        let mut a = Archive::new();
        for refused in [
            "https://user:secret@example.com/a",
            "https://user@example.com/a",
            HISTORY,
            BOOKMARKS,
        ] {
            assert_eq!(
                a.visit(&on(), &u(refused), Some("x")),
                Visited::Refused,
                "{refused}"
            );
        }
        assert_eq!(a.visits(), 0);
        let text = serde_json::to_string(&a).unwrap();
        assert!(!text.contains("secret"), "{text}");
        // The bookmarks is the reader's own choice, one keypress at a time, and `Ctrl+D` takes it
        // back out; it is not this door and does not inherit this rule.
        assert_eq!(
            a.bookmark(
                &on(),
                &u("https://user:secret@example.com/a"),
                Some("x"),
                None
            ),
            Stored::Added
        );
    }

    /// The reading list is a third tenant, and the file proves it: one address can be in all
    /// three lists at once and each of them answers only for itself.
    ///
    /// This is the test the `Kind` discriminant exists for. `holds`, `holds_next`, `forget`,
    /// `forget_next`, `forget_all`, `forget_all_next`, and `forget_visits` are seven functions
    /// that all match on a kind, and any one of them written without the filter compiles and
    /// quietly makes one list answer for another.
    #[test]
    fn the_three_tenants_never_answer_for_one_another() {
        let mut a = Archive::new();
        let url = u("https://example.com/a");
        assert_eq!(a.bookmark(&on(), &url, Some("Stored"), None), Stored::Added);
        assert_eq!(a.add_next(&on(), &url, Some("Later")), Stored::Added);
        assert_eq!(a.visit(&on(), &url, Some("Seen")), Visited::Recorded);
        assert_eq!(a.len(), 1);
        assert_eq!(a.next_len(), 1);
        assert_eq!(a.visits(), 1);
        assert!(a.is_bookmarked(&url) && a.holds_next(&url));

        // Taking it off the reading list leaves the other two exactly where they are, which is
        // the case a shared `forget` would silently get wrong.
        assert!(a.forget_next(&url));
        assert!(!a.holds_next(&url));
        assert!(
            a.is_bookmarked(&url),
            "un-keeping is not what was asked for"
        );
        assert_eq!(a.visits(), 1);

        assert_eq!(a.add_next(&on(), &url, Some("Later")), Stored::Added);
        assert_eq!(a.forget_all(), 1);
        assert_eq!(
            a.next_len(),
            1,
            "the bookmarks button took the reading list"
        );
        assert_eq!(a.visits(), 1, "the bookmarks button took the history");
        assert_eq!(a.forget_visits(), 1);
        assert_eq!(a.next_len(), 1, "the history button took the reading list");
        assert_eq!(a.forget_all_next(), 1);
        assert!(a.items.is_empty());
    }

    /// The reading list refuses at its cap and drops nothing, with the bookmarks and against the
    /// history. Everything on it was chosen, and the doctrine's whole arithmetic is that a chosen
    /// thing is never evicted to make room for a newer one.
    #[test]
    fn a_full_reading_list_refuses_and_keeps_what_it_has() {
        let mut a = Archive::new();
        for i in 0..MAX_NEXT {
            assert_eq!(
                a.add_next_at(&u(&format!("https://example.com/{i}")), Some("x"), 1),
                Stored::Added
            );
        }
        let before = a.read_next().next().map(|i| i.url.clone());
        assert_eq!(
            a.add_next_at(&u("https://example.com/one-more"), Some("x"), 1),
            Stored::Full
        );
        assert_eq!(a.next_len(), MAX_NEXT, "nothing was dropped to make room");
        assert_eq!(a.read_next().next().map(|i| i.url.clone()), before);
        // Its own cap, not the bookmarks': the two tenants do not borrow each other's rules, and
        // the bookmarks still has all its room.
        assert_ne!(MAX_NEXT, MAX_ITEMS);
        assert_eq!(
            a.bookmark(&on(), &u("https://example.com/k"), Some("K"), None),
            Stored::Added
        );
    }

    /// **The guard that makes this door not `keep`'s twin.** `keep` is handed the URL a fetch
    /// resolved; this one is handed an `href` a *page* wrote, so every scheme a page can put in an
    /// anchor arrives here. `ReaderApp` reports these in `classify_link`'s words before it ever
    /// calls in, and that is exactly why the refusal has to live here too: a door guarded at one
    /// caller is a door guarded nowhere, and this door has two callers on the day it lands.
    #[test]
    fn a_page_cannot_put_a_non_web_link_on_the_reading_list() {
        let mut a = Archive::new();
        for refused in [
            "javascript:alert(1)",
            "data:text/html,<b>x</b>",
            "mailto:someone@example.com",
            "tel:+15550100",
            "file:///etc/passwd",
            "https://user:secret@example.com/a",
            "https://user@example.com/a",
            READING_LIST,
            BOOKMARKS,
            HISTORY,
        ] {
            assert_eq!(
                a.add_next(&on(), &u(refused), Some("x")),
                Stored::Refused,
                "{refused}"
            );
        }
        assert_eq!(a.next_len(), 0);
        let text = serde_json::to_string(&a).unwrap();
        assert!(!text.contains("secret"), "{text}");
        // And the refusal is not the switch answering: it holds with the switch on, which is the
        // only state in which the rest of the door runs at all.
        assert!(on().keep_reading_list);
    }

    /// The switch is answered inside the door, and off means stop writing rather than lose what
    /// is there — [`Settings::keep_bookmarks`]'s meaning of the word, one tenant over.
    #[test]
    fn the_reading_list_switch_is_answered_in_the_door() {
        let mut a = Archive::new();
        assert_eq!(
            a.add_next(&on(), &u("https://example.com/a"), Some("A")),
            Stored::Added
        );
        let off = Settings {
            keep_reading_list: false,
            ..Settings::default()
        };
        assert_eq!(
            a.add_next(&off, &u("https://example.com/b"), Some("B")),
            Stored::Off
        );
        assert_eq!(
            a.next_len(),
            1,
            "off must not discard what is already there"
        );
        // Off for this tenant is not off for the others.
        assert_eq!(
            a.bookmark(&off, &u("https://example.com/c"), Some("C"), None),
            Stored::Added
        );
        assert_eq!(
            a.visit(&off, &u("https://example.com/d"), Some("D")),
            Visited::Recorded
        );
    }

    /// An address longer than [`MAX_URL`] is refused rather than shortened, and the reading list
    /// is the door a page's own links come through — so this is the one most likely to meet one.
    #[test]
    fn an_over_long_link_is_refused_rather_than_shortened() {
        let mut a = Archive::new();
        let long = format!("https://example.com/{}", "q".repeat(MAX_URL));
        assert_eq!(a.add_next(&on(), &u(&long), Some("x")), Stored::TooLong);
        assert_eq!(a.next_len(), 0);
    }

    /// A link already listed is said so rather than duplicated, fragment ignored — the same
    /// answer `keep` gives, over the same `same_page` comparison.
    #[test]
    fn a_link_already_on_the_reading_list_is_not_repeated() {
        let mut a = Archive::new();
        assert_eq!(
            a.add_next(&on(), &u("https://example.com/a"), Some("A")),
            Stored::Added
        );
        assert_eq!(
            a.add_next(&on(), &u("https://example.com/a#part-two"), Some("A")),
            Stored::Already
        );
        assert_eq!(a.next_len(), 1);
    }

    /// The wire spelling, and that an unknown kind still survives a build that has three.
    #[test]
    fn the_reading_lists_wire_spelling_round_trips_beside_an_unknown_kind() {
        let mut a = Archive::new();
        a.add_next_at(&u("https://example.com/a"), Some("A"), 86_400);
        let text = serde_json::to_string(&a).unwrap();
        assert!(text.contains(r#""kind":"next""#), "{text}");
        let (back, note) = parse(&text, "the fixture");
        assert!(note.is_none());
        assert_eq!(back, a);
        assert_eq!(
            back.read_next().next().expect("the item").kind,
            Kind::ReadNext
        );

        // A kind a later build wrote is carried through untouched and shown by none of the three.
        let mixed = r#"{"version":1,"items":[
            {"url":"https://example.com/f","title":"F","kind":"feed","added":1}]}"#;
        let (m, note) = parse(mixed, "the fixture");
        assert!(note.is_none(), "one unknown kind is not a broken file");
        assert_eq!(m.next_len(), 0);
        assert_eq!(m.len(), 0);
        assert_eq!(m.visits(), 0);
        assert!(serde_json::to_string(&m).unwrap().contains("feed"));
    }

    /// The reading list shows each row's address and the bookmarks does not, which is
    /// `Address::Shown`'s whole argument: these rows are named by link text, and link text
    /// repeats.
    #[test]
    fn the_reading_list_draws_the_address_under_each_row() {
        let mut a = Archive::new();
        a.add_next_at(&u("https://example.com/a"), Some("Read more"), 86_400);
        a.bookmark_at(&u("https://example.com/a"), Some("Read more"), None, 86_400);
        let listed = reading_list_document(&a, &on());
        let Block::Entries(rows) = &listed.blocks[0] else {
            panic!("the reading list draws entries");
        };
        assert_eq!(rows[0].address.as_deref(), Some("https://example.com/a"));
        let shelved = bookmarks_document(&a);
        let Block::Entries(kept) = &shelved.blocks[0] else {
            panic!("the bookmarks draws entries");
        };
        assert_eq!(kept[0].address, None, "the bookmarks names no addresses");
    }

    /// An empty reading list is a document with a sentence in it, not an error page and not a
    /// blank screen — `document`'s argument, third tenant.
    #[test]
    fn an_empty_reading_list_is_a_document_and_not_a_blank_screen() {
        let a = Archive::new();
        let doc = reading_list_document(&a, &on());
        assert_eq!(doc.url, READING_LIST);
        assert_eq!(
            doc.title.as_deref(),
            Some(crate::reader::notice::READING_LIST_TITLE)
        );
        let [Block::Paragraph(line)] = doc.blocks.as_slice() else {
            panic!("an empty reading list is one paragraph");
        };
        assert!(!ir::plain_text(line).trim().is_empty());
    }

    /// One row per page, not one per visit: the entry moves to the top and is restamped and
    /// retitled, which is what the module doc promises instead of a visit-by-visit trail.
    #[test]
    fn visiting_a_page_again_moves_one_row_rather_than_adding_another() {
        let mut a = Archive::new();
        a.visit_at(&u("https://example.com/a"), Some("A"), 86_400);
        a.visit_at(&u("https://example.com/b"), Some("B"), 172_800);
        a.visit_at(
            &u("https://example.com/a#part-two"),
            Some("A, retitled"),
            259_200,
        );

        let rows: Vec<(String, Option<String>)> =
            a.visited().map(|i| (i.label(), i.on_day())).collect();
        assert_eq!(
            rows,
            vec![
                ("A, retitled".to_owned(), Some("1970-01-04".to_owned())),
                ("B".to_owned(), Some("1970-01-03".to_owned())),
            ],
            "one row per page, most recent first, under the title it has now"
        );
        // The fragment is ignored on the way in and kept on the way out, as a keep does it.
        assert_eq!(
            a.visited().next().unwrap().url,
            "https://example.com/a#part-two"
        );
    }

    /// A page can be both kept and visited, and neither record may touch the other: a visit must
    /// not restamp, retitle or move the entry the reader chose, and a keep must not swallow the
    /// row hww wrote.
    #[test]
    fn the_two_tenants_do_not_overwrite_each_other() {
        let mut a = Archive::new();
        a.bookmark_at(&u("https://example.com/a"), Some("As kept"), None, 86_400);
        a.visit_at(&u("https://example.com/a"), Some("As visited"), 172_800);
        assert_eq!(a.len(), 1);
        assert_eq!(a.visits(), 1);
        let kept = a.bookmarks().next().unwrap();
        assert_eq!(kept.title, "As kept");
        assert_eq!(kept.added, 86_400);
        assert_eq!(a.visited().next().unwrap().title, "As visited");

        // And each button empties its own tenant only.
        let mut b = a.clone();
        assert_eq!(b.forget_visits(), 1);
        assert_eq!(b.len(), 1, "forgetting a trail is not losing the bookmarks");
        assert_eq!(a.forget_all(), 1);
        assert_eq!(a.visits(), 1, "and the reverse");
    }

    /// `Ctrl+D` twice removes a page from the bookmarks. It must not also erase the fact that hww
    /// drew it, which is a different tenant's record with a different switch over it.
    #[test]
    fn forgetting_a_kept_page_leaves_the_history_row_alone() {
        let mut a = Archive::new();
        a.visit(&on(), &u("https://example.com/a"), Some("A"));
        a.bookmark(&on(), &u("https://example.com/a"), Some("A"), None);
        assert!(a.forget(&u("https://example.com/a")));
        assert_eq!(a.visits(), 1);
    }

    /// The opposite of [`a_full_bookmarks_refuses_rather_than_dropping_the_oldest`], and the module
    /// doc says why: nothing here was chosen, so the oldest page goes and the newest is written.
    /// What it may never evict is a page the reader kept.
    #[test]
    fn a_full_history_drops_its_oldest_page_and_never_a_kept_one() {
        let mut a = Archive::new();
        a.bookmark(&on(), &u("https://example.com/kept"), Some("Stored"), None);
        for i in 0..MAX_VISITS {
            a.visit(&on(), &u(&format!("https://example.com/{i}")), Some("x"));
        }
        assert_eq!(a.visits(), MAX_VISITS);
        a.visit(&on(), &u("https://example.com/newest"), Some("Newest"));
        assert_eq!(a.visits(), MAX_VISITS, "the cap holds");

        let urls: Vec<&str> = a.visited().map(|i| i.url.as_str()).collect();
        assert_eq!(urls[0], "https://example.com/newest");
        assert!(
            !urls.contains(&"https://example.com/0"),
            "the oldest page was the one that went"
        );
        assert!(urls.contains(&"https://example.com/1"));
        assert_eq!(a.len(), 1, "the bookmarks is not eviction's business");
        assert!(a.is_bookmarked(&u("https://example.com/kept")));
    }

    /// The history view, drawn by the same code as the bookmarks and never listing itself.
    #[test]
    fn the_history_is_one_entries_block_at_its_own_address() {
        let mut a = Archive::new();
        a.visit(&on(), &u("https://example.com/a"), Some("A"));
        a.visit(&on(), &u("https://example.com/b"), Some("B"));
        let doc = history_document(&a, &on());
        assert_eq!(doc.url, HISTORY);
        assert_ne!(doc.url, BOOKMARKS);
        assert!(doc.title.is_some(), "the masthead is derived from this");
        let ir::Block::Entries(entries) = &doc.blocks[0] else {
            panic!("expected one Entries block, got {:?}", doc.blocks);
        };
        assert_eq!(entries.len(), 2);
        assert_eq!(ir::plain_text(&entries[0].title), "B");
        assert_eq!(entries[0].href.as_deref(), Some("https://example.com/b"));
        assert!(entries.iter().all(|e| e.image.is_none()));
    }

    /// The address under each row, as text. It is the history's alone: two pages that call
    /// themselves "Home" are one row twice without it, and it says where a row goes before the
    /// reader follows it.
    #[test]
    fn a_history_row_carries_its_address_as_text_and_not_a_second_link() {
        let mut a = Archive::new();
        a.visit(&on(), &u("https://example.com/a?page=2"), Some("A"));
        a.visit(&on(), &u("https://example.com/untitled"), None);
        let doc = history_document(&a, &on());
        let ir::Block::Entries(entries) = &doc.blocks[0] else {
            panic!("expected one Entries block, got {:?}", doc.blocks);
        };

        // The untitled page is named by its address already, and must not say it twice.
        assert_eq!(ir::plain_text(&entries[0].title), "example.com/untitled");
        assert!(entries[0].address.is_none());

        assert_eq!(
            entries[1].address.as_deref(),
            Some("https://example.com/a?page=2")
        );
        assert_eq!(
            entries[1].href.as_deref(),
            Some("https://example.com/a?page=2"),
            "the headline is the link; the address is text under it"
        );
        // Not a dek: the summary is the page's own words and stays empty here.
        assert!(entries[1].summary.is_empty());

        // The bookmarks is a short list of pages the reader chose and can name, and does not.
        let mut b = Archive::new();
        b.bookmark(&on(), &u("https://example.com/a"), Some("A"), None);
        let ir::Block::Entries(kept) = &bookmarks_document(&b).blocks[0] else {
            panic!("expected one Entries block");
        };
        assert!(kept[0].address.is_none());
    }

    /// The mark is stored for a page the reader chose, drawn in the bookmarks, and absent from the
    /// history — three halves of the argument in [`Mark`], each of which can be broken without
    /// breaking the other two. The reading list's half is
    /// [`a_reading_list_row_draws_a_mark_it_never_stored`].
    #[test]
    fn the_bookmarks_stores_its_marks_and_the_history_draws_none() {
        let mut a = Archive::new();
        a.bookmark(
            &on(),
            &u("https://example.com/a"),
            Some("A"),
            Some("https://example.com/favicon.png"),
        );
        assert_eq!(
            a.bookmarks().next().unwrap().icon,
            "https://example.com/favicon.png"
        );
        let ir::Block::Entries(kept) = &bookmarks_document(&a).blocks[0] else {
            panic!("expected one Entries block");
        };
        assert_eq!(
            kept[0].icon.as_deref(),
            Some("https://example.com/favicon.png")
        );

        // Watching a page writes no mark down, whatever the page declared.
        a.visit(&on(), &u("https://example.com/a"), Some("A"));
        assert_eq!(a.visited().next().unwrap().icon, "");

        // And the history view draws none even for an item that has one, which is what a
        // hand-edited file, or an item written by a later build, would produce. This is the
        // lock that is not `Archive::visit` declining to write the field: the two are separate
        // on purpose, because only one of them survives a file hww did not write.
        let mut b = Archive::new();
        b.visit_at(&u("https://example.com/a"), Some("A"), 86_400);
        b.items[0].icon = "https://example.com/favicon.png".to_owned();
        let ir::Block::Entries(seen) = &history_document(&b, &on()).blocks[0] else {
            panic!("expected one Entries block");
        };
        assert!(
            seen[0].icon.is_none(),
            "the history draws no column, stored icon or not"
        );
    }

    /// The mark is an address hww will later request without being asked again, so the set of
    /// things it may name is the two schemes `fetch` speaks. Anything else costs the mark and
    /// never the page: the reader pressed a key to keep this, and losing it over a favicon
    /// would be the silent loss this module exists to prevent.
    #[test]
    fn a_mark_that_is_not_a_web_address_is_dropped_and_the_page_is_kept() {
        let refused = [
            "data:image/png;base64,AAAA",
            "file:///home/reader/icon.png",
            "/favicon.ico",
            "",
            "   ",
        ];
        for (i, icon) in refused.iter().enumerate() {
            let mut a = Archive::new();
            let url = u(&format!("https://example.com/{i}"));
            assert_eq!(
                a.bookmark(&on(), &url, Some("A"), Some(icon)),
                Stored::Added
            );
            let item = a.bookmarks().next().expect("the page is kept anyway");
            assert_eq!(item.icon, "", "{icon} should not have been written down");
        }

        // And one past the address cap, which is refused where the title cap truncates for the
        // reason the module doc gives: half an address is a link to somewhere else.
        let mut a = Archive::new();
        let long = format!("https://example.com/{}", "a".repeat(MAX_URL));
        a.bookmark(
            &on(),
            &u("https://example.com/a"),
            Some("A"),
            Some(long.as_str()),
        );
        assert_eq!(a.bookmarks().next().unwrap().icon, "");
    }

    /// A mark written by an older build, or by hand, is re-checked on the way out: the file
    /// between the two saves is one a reader may edit, and this is a URL hww fetches.
    ///
    /// What replaces it is the guess, not nothing: `mark_beside` falls through to
    /// [`well_known_icon`], so the row keeps its place in the column and the address hww
    /// requests is one it derived rather than one the file handed it.
    #[test]
    fn a_stored_mark_is_checked_again_before_it_is_drawn() {
        let mut a = Archive::new();
        a.bookmark_at(&u("https://example.com/a"), Some("A"), None, 86_400);
        a.items[0].icon = "javascript:alert(1)".to_owned();
        let ir::Block::Entries(kept) = &bookmarks_document(&a).blocks[0] else {
            panic!("expected one Entries block");
        };
        assert_eq!(
            kept[0].icon.as_deref(),
            Some("https://example.com/favicon.ico")
        );

        // And a row whose own address is not one `fetch` speaks draws no mark at all, which is
        // the case the gutter is allocated for whether or not anything fills it.
        let mut b = Archive::new();
        b.bookmark_at(&u("https://example.com/a"), Some("A"), None, 86_400);
        b.items[0].url = "mailto:someone@example.com".to_owned();
        let ir::Block::Entries(odd) = &bookmarks_document(&b).blocks[0] else {
            panic!("expected one Entries block");
        };
        assert!(odd[0].icon.is_none());
    }

    /// A mark never carries a credential, whichever of the two doors it came through.
    ///
    /// What `reader::iconcache` may keep, and — sharply — what it may not.
    ///
    /// The history is the whole test. It draws no column and stores no icon, so a mark cached
    /// for a page hww merely watched would be a record of somewhere the reader went, on disk,
    /// outliving *forget history* and with no switch over it. `Cache::open` deletes everything
    /// this iterator does not name, so an address missing from here is an address that cannot
    /// be kept whatever writes it.
    #[test]
    fn only_a_marked_row_names_an_icon_worth_keeping() {
        let mut a = Archive::new();
        a.bookmark(
            &on(),
            &u("https://kept.example/page"),
            Some("Kept"),
            Some("https://kept.example/mark.png"),
        );
        a.add_next_at(&u("https://next.example/link"), Some("Read more"), 86_400);
        a.visit(&on(), &u("https://watched.example/page"), Some("Watched"));

        let icons: Vec<String> = a.marked_icons().collect();
        assert!(icons.contains(&"https://kept.example/mark.png".to_owned()));
        // The reading list's guess, which is the only mark it has: see `well_known_icon`.
        assert!(icons.contains(&"https://next.example/favicon.ico".to_owned()));
        assert_eq!(icons.len(), 2);
        assert!(
            !icons.iter().any(|i| i.contains("watched.example")),
            "a visited page names no icon: {icons:?}"
        );

        // And the guess is what the column would actually draw, so what is kept and what is
        // fetched are the same set rather than two that agree today.
        let ir::Block::Entries(rows) = &reading_list_document(&a, &on()).blocks[0] else {
            panic!("expected one Entries block");
        };
        assert_eq!(
            rows[0].icon.as_deref(),
            Some("https://next.example/favicon.ico")
        );

        // The same question asked about one address, which is what a worker's write has to be
        // checked against: the permission was granted when the job was submitted and the row
        // may have been forgotten since.
        assert!(a.marks_icon("https://kept.example/mark.png"));
        assert!(a.marks_icon("https://next.example/favicon.ico"));
        assert!(!a.marks_icon("https://watched.example/favicon.ico"));
        a.forget(&u("https://kept.example/page"));
        assert!(!a.marks_icon("https://kept.example/mark.png"));
    }

    /// The row's own address may hold one — `Archive::bookmark` allows it, because a keypress
    /// put that page there — and following it is a click. The mark is not: hww requests it by
    /// itself when the view is drawn, so a userinfo left on it would be an automatic
    /// authenticated request for a page nobody opened, with the password read straight out of
    /// `archive.json`.
    #[test]
    fn a_mark_never_carries_the_password_its_row_might() {
        let mut a = Archive::new();
        assert_eq!(
            a.bookmark(
                &on(),
                &u("https://user:secret@intranet.example/page"),
                Some("Intranet"),
                Some("https://user:secret@intranet.example/favicon.ico"),
            ),
            Stored::Added
        );
        // The mark is written without it. The row's *own* address keeps what the reader typed —
        // that is `Archive::bookmark`'s documented difference from the other two doors, and
        // `credentials_survive_a_keep_and_are_refused_everywhere_else` is where it is pinned.
        let item = a.bookmarks().next().expect("the page is bookmarked");
        assert_eq!(item.icon, "https://intranet.example/favicon.ico");
        assert!(item.url.contains("secret"), "the address is left as given");

        let ir::Block::Entries(rows) = &bookmarks_document(&a).blocks[0] else {
            panic!("expected one Entries block");
        };
        assert_eq!(
            rows[0].icon.as_deref(),
            Some("https://intranet.example/favicon.ico")
        );

        // And the guessed one, which `Url::join` would otherwise build from the row's own
        // credentialed address. This is the reading list's only path to a mark.
        let mut b = Archive::new();
        b.add_next_at(
            &u("https://user:secret@intranet.example/page"),
            Some("Read more"),
            86_400,
        );
        let ir::Block::Entries(next) = &reading_list_document(&b, &on()).blocks[0] else {
            panic!("expected one Entries block");
        };
        assert_eq!(
            next[0].icon.as_deref(),
            Some("https://intranet.example/favicon.ico"),
            "the guess must not reproduce the credential"
        );
    }

    /// The reading list draws the column with nothing stored for it.
    ///
    /// The three halves of [`Mark`]'s argument, one list at a time: a row hww has never fetched
    /// still gets a mark, the file it came from records no icon for it, and the address drawn is
    /// the well-known one at that row's own origin rather than anything a site said.
    #[test]
    fn a_reading_list_row_draws_a_mark_it_never_stored() {
        let mut a = Archive::new();
        assert_eq!(
            a.add_next(
                &on(),
                &u("https://example.net/deep/piece?ref=x"),
                Some("Read more")
            ),
            Stored::Added
        );
        assert_eq!(
            a.read_next().next().expect("the row").icon,
            "",
            "a guess must not be written to the file"
        );
        let ir::Block::Entries(rows) = &reading_list_document(&a, &on()).blocks[0] else {
            panic!("expected one Entries block");
        };
        assert_eq!(
            rows[0].icon.as_deref(),
            Some("https://example.net/favicon.ico"),
            "the origin's well-known path, never the row's own path"
        );
    }

    /// An empty history is a page, like an empty bookmarks list, and it says which of the two things it
    /// means: nothing read yet, or nothing being written down.
    #[test]
    fn an_empty_history_is_a_page_that_says_why_it_is_empty() {
        let words = |s: &Settings| {
            let doc = history_document(&Archive::new(), s);
            assert_eq!(doc.blocks.len(), 1);
            match &doc.blocks[0] {
                ir::Block::Paragraph(i) => ir::plain_text(i),
                other => panic!("expected a paragraph, got {other:?}"),
            }
        };
        assert_ne!(words(&on()), words(&watching_off()));
        assert!(words(&watching_off()).contains("not to remember"));
    }

    /// The date arithmetic, against dates known by hand: an epoch day, a leap day, the century
    /// rule, and the four-hundred-year exception to it.
    #[test]
    fn every_epoch_day_maps_to_its_civil_date() {
        for (day, want) in [
            (0i64, (1970, 1, 1)),
            (59, (1970, 3, 1)),
            (365, (1971, 1, 1)),
            (730, (1972, 1, 1)),
            (789, (1972, 2, 29)),
            (11_016, (2000, 2, 29)),
            (20_147, (2025, 2, 28)),
            (-1, (1969, 12, 31)),
        ] {
            assert_eq!(civil_from_days(day), (want.0, want.1, want.2), "day {day}");
        }
    }

    #[test]
    fn an_item_with_no_time_carries_no_date() {
        let mut a = Archive::new();
        a.bookmark_at(&u("https://example.com/a"), Some("A"), None, 1_756_339_200);
        assert_eq!(
            a.bookmarks().next().unwrap().on_day().as_deref(),
            Some("2025-08-28")
        );
        let (b, _) = parse(
            r#"{"items":[{"url":"https://example.com/a","kind":"bookmark"}]}"#,
            "archive.json",
        );
        assert!(b.bookmarks().next().unwrap().on_day().is_none());
    }

    /// The address of the bookmarks view carries no authority component, so no hostname enters any
    /// file but `sites.rs`, and every `host_str()` in the reader answers `None` without panicking.
    #[test]
    fn the_bookmarks_address_has_no_host() {
        let url = Url::parse(BOOKMARKS).expect("hww:bookmarks parses");
        assert_eq!(url.scheme(), "hww");
        assert!(url.host_str().is_none());
        assert!(
            url.cannot_be_a_base(),
            "the opaque-path form, not an authority"
        );
    }

    /// The off switch, at the door rather than at the call site. Nothing is written, and what was
    /// already kept is left exactly where it is: turning keeping off is not a way to lose a
    /// bookmarks, which is what the panel's "forget everything" is for.
    #[test]
    fn the_off_switch_prevents_the_write_and_keeps_what_is_there() {
        let off = Settings {
            keep_bookmarks: false,
            ..Settings::default()
        };
        let mut a = Archive::new();
        a.bookmark(&on(), &u("https://example.com/a"), Some("A"), None);
        assert_eq!(
            a.bookmark(&off, &u("https://example.com/b"), Some("B"), None),
            Stored::Off
        );
        assert_eq!(a.len(), 1);
        assert!(
            a.is_bookmarked(&u("https://example.com/a")),
            "the switch is not a delete"
        );
        // And the answer does not depend on whether the page was already there.
        assert_eq!(
            a.bookmark(&off, &u("https://example.com/a"), Some("A"), None),
            Stored::Off
        );
    }

    /// Page content can never reach the reader's own views. `classify_link` refuses every scheme
    /// it does not handle, and `hww` is not one of them, so only a command opens the bookmarks.
    #[test]
    fn a_page_cannot_link_into_the_readers_own_views() {
        use crate::session::{Target, classify_link};
        for href in [BOOKMARKS, "hww:anything", "hww://bookmarks/"] {
            let Target::Refuse { reason, .. } = classify_link(href) else {
                panic!("{href} was not refused");
            };
            assert!(reason.contains("hww"), "{reason}");
        }
    }
}
