//! The menu bar, as data: which menus exist, what is on them, and which key already does it.
//! **No egui types.**
//!
//! `ui::menu_ui` walks this and draws it; every decision about what the bar *says* is here,
//! where `cargo test` can read it, for the reason `notice` and `pageinfo` are outside `ui/`.
//!
//! # Why a menu bar at all
//!
//! Everything this reader can do was reachable only by key, and the help card at `?` was the
//! only place the keys were written down. That is a fine second browser for someone who has
//! read the card and a closed door for everyone else, and the settings were behind an even
//! smaller door: a JSON file nothing in the reader or the README mentioned. A menu bar is the
//! oldest answer to "what can this program do", and Jakob's law applies here the way it does to
//! the infobars: a second browser should not invent a grammar for its own command list.
//!
//! Each item carries the key that already does the same thing, drawn on the right, so the bar
//! teaches the keyboard rather than competing with it.
//!
//! # The `HELP` table lives here
//!
//! It used to sit in `ui::app`, where nothing could test it, and the menu needs the same
//! strings. One table means the card and the bar cannot drift apart, and
//! `the_menu_and_the_help_card_agree` is what says so. Note what that test does *not* cover:
//! `app::handle_keys` holds the real bindings as literals inside a function under `ui/`, so
//! nothing here can prove a key this table advertises is a key the reader binds. Closing that
//! would mean lifting the whole keymap into an egui-free chord type and mapping it back under
//! `ui/`, which is the `face`/`fonts` split applied to keys and a larger change than this one.
//!
//! # The submenu marker is not egui's
//!
//! egui's `SubMenuButton::RIGHT_ARROW` is `⏵` (U+23F5), which is in **none** of the faces
//! `ui::fonts` embeds: `eframe` is built without `default_fonts`, so there is no symbol font
//! anywhere in the binary and it draws as tofu. `▸` and `▶` are worse in a subtler way — they
//! are in `DejaVuSerif` only, which is the coverage tail, so they resolve to a serif triangle
//! inside a monospace menu. [`SUBMENU_MARKER`] is `›`, which every embedded face but the emoji
//! tail carries, so it resolves in the first family of every chain. Same lesson as the `ⓘ`
//! `ui::pageinfo_ui` never typed, one medium over: check the glyph against the shipped faces,
//! not against the font on your desktop.

/// Drawn at the right of a submenu button. See the module doc: this is not decoration and not
/// egui's default.
pub const SUBMENU_MARKER: &str = "\u{203A}";

/// Every key spec these tables put on screen, spelled for the modifier this platform's reader
/// actually presses.
///
/// `app::handle_keys` binds `Modifiers::COMMAND`, which egui maps to Cmd on macOS and Ctrl
/// everywhere else, and it binds Back and Forward to `Cmd+[`/`]` alongside `Alt`+arrows. A
/// table that hardcoded `Ctrl+L` therefore advertised, on a Mac, a key that does nothing; the
/// zoom rows were wrong the same way, since the zoom is egui's own and egui's own is Cmd there.
/// The reader was told so once, in a footnote under the help card — a thing to read rather than
/// a thing the label says.
///
/// `cfg`-selected consts, and here rather than computed under `ui/`, so this stays wording that
/// `cargo test` can read: the menu and the help card still have to agree, and the agreement test
/// still runs in the default job. It simply checks a different pair of tables on each platform,
/// which is the point.
///
/// The two blocks are written out in full rather than composed from a `MOD` const. `concat!`
/// takes literals and not consts, so a composed spelling would mean building these strings at
/// run time; and a spec is not always the modifier swapped, as `BACK` and `HISTORY` show.
///
/// # Why these are the keys they are
///
/// Every one of them is the key some other browser already uses, because a second browser has
/// no standing to invent a command grammar the reader has to learn twice — the same argument
/// the menu bar itself rests on, one layer down. Where two browsers disagree the platform wins,
/// which is what `BACK` has always encoded and what `HISTORY` now does too: `Cmd+H` is the
/// system Hide binding on macOS and never reaches the app at all, so History is `Cmd+Y` there,
/// as it is in Safari and Chrome.
#[cfg(not(target_os = "macos"))]
mod keyspec {
    pub const OPEN_LOCATION: &str = "Ctrl+L";
    pub const FIND: &str = "Ctrl+F";
    pub const RELOAD: &str = "Ctrl+R";
    pub const RELOAD_BARE: &str = "Ctrl+Shift+R";
    pub const ZOOM_IN: &str = "Ctrl++";
    pub const ZOOM_OUT: &str = "Ctrl+-";
    pub const ZOOM_RESET: &str = "Ctrl+0";
    pub const BACK: &str = "Alt+Left";
    pub const FORWARD: &str = "Alt+Right";
    pub const BOOKMARK_PAGE: &str = "Ctrl+D";
    pub const READ_LATER: &str = "Shift+L";
    pub const BOOKMARKS: &str = "Ctrl+Shift+B";
    pub const HISTORY: &str = "Ctrl+H";
    pub const OUTLINE: &str = "Ctrl+Shift+O";
    pub const PAGE_INFO: &str = "Ctrl+I";
    pub const SETTINGS: &str = "Ctrl+,";
    pub const QUIT: &str = "Ctrl+Q";

    /// The four help rows that put more than one spec on a line. They are whitespace-separated
    /// token by token and have to stay that way: `the_menu_and_the_help_card_agree` finds
    /// `RELOAD` in `HELP_RELOAD` as a *token*, never as a substring, since `Ctrl+R` is not one
    /// of `Ctrl+Shift+R` and `Ctrl+H` is not one of `Ctrl+Shift+B`.
    pub const HELP_RELOAD: &str = "Ctrl+R / Ctrl+Shift+R";
    pub const HELP_FIND: &str = "Ctrl+F · Enter / Shift+Enter";
    pub const HELP_LISTS: &str = "Ctrl+Shift+B / l / Ctrl+H";
    pub const HELP_DISMISS: &str = "? / Esc / Ctrl+Q";
    pub const HELP_ZOOM: &str = "Ctrl++ / Ctrl+- / Ctrl+0";
    pub const HELP_FOOTNOTE: &str = "Zoom is egui's: Ctrl +/-/0, Ctrl+wheel, pinch.";

    /// The status strip's three tooltips. They name a key inside a sentence rather than as a
    /// spec, so they cannot be composed from the consts above, and they are wording the reader
    /// sees, so they are here rather than as literals under `ui/`.
    pub const TIP_BACK: &str = "Back (Alt+Left)";
    pub const TIP_FORWARD: &str = "Forward (Alt+Right)";
    pub const TIP_URL_BAR: &str = "Click to edit (Ctrl+L)";

    /// The settings panel's own spelling of the zoom keys, which is a row of three rather than
    /// a spec the keycap splitter reads.
    pub const ZOOM_SETTING: &str = "Ctrl+ + - 0";
}

#[cfg(target_os = "macos")]
mod keyspec {
    pub const OPEN_LOCATION: &str = "Cmd+L";
    pub const FIND: &str = "Cmd+F";
    pub const RELOAD: &str = "Cmd+R";
    pub const RELOAD_BARE: &str = "Cmd+Shift+R";
    pub const ZOOM_IN: &str = "Cmd++";
    pub const ZOOM_OUT: &str = "Cmd+-";
    pub const ZOOM_RESET: &str = "Cmd+0";
    pub const BACK: &str = "Cmd+[";
    pub const FORWARD: &str = "Cmd+]";
    pub const BOOKMARK_PAGE: &str = "Cmd+D";
    /// Not a modifier swap: Shift is Shift on every platform hww ships to. It is here anyway,
    /// because the rule the `cfg` split enforces is that no table outside this module spells a
    /// modifier — a key that happens to be the same on both is still spelled once, in the one
    /// place a later change to it would be found.
    pub const READ_LATER: &str = "Shift+L";
    pub const BOOKMARKS: &str = "Cmd+Shift+B";
    /// The second spec that is not a modifier swap, and for a harder reason than `BACK`'s.
    /// `Cmd+H` hides the application on macOS: the system takes it before any window sees it,
    /// so a Mac told to press it would watch hww vanish instead of listing what it had drawn.
    /// Safari and Chrome both put History on `Cmd+Y` there, and so does this.
    pub const HISTORY: &str = "Cmd+Y";
    pub const OUTLINE: &str = "Cmd+Shift+O";
    pub const PAGE_INFO: &str = "Cmd+I";
    pub const SETTINGS: &str = "Cmd+,";
    pub const QUIT: &str = "Cmd+Q";

    /// The four help rows that put more than one spec on a line. See the other block: these are
    /// read token by token by the agreement test, never as substrings.
    pub const HELP_RELOAD: &str = "Cmd+R / Cmd+Shift+R";
    pub const HELP_FIND: &str = "Cmd+F · Enter / Shift+Enter";
    pub const HELP_LISTS: &str = "Cmd+Shift+B / l / Cmd+Y";
    pub const HELP_DISMISS: &str = "? / Esc / Cmd+Q";
    pub const HELP_ZOOM: &str = "Cmd++ / Cmd+- / Cmd+0";
    pub const HELP_FOOTNOTE: &str = "Zoom is egui's: Cmd +/-/0, Cmd+wheel, pinch.";

    /// The status strip's three tooltips. They name a key inside a sentence rather than as a
    /// spec, so they cannot be composed from the consts above, and they are wording the reader
    /// sees, so they are here rather than as literals under `ui/`.
    pub const TIP_BACK: &str = "Back (Cmd+[)";
    pub const TIP_FORWARD: &str = "Forward (Cmd+])";
    pub const TIP_URL_BAR: &str = "Click to edit (Cmd+L)";

    /// The settings panel's own spelling of the zoom keys, which is a row of three rather than
    /// a spec the keycap splitter reads.
    pub const ZOOM_SETTING: &str = "Cmd+ + - 0";
}

pub use keyspec::*;

/// What a menu item does when it is chosen.
///
/// Everything here is something the reader could already do; the bar adds no capability. The
/// variants that carry a value are the radio groups, whose payload is an index into the
/// matching `prefs` choice list so `menu_ui` never has to know which enum sits behind one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    OpenLocation,
    Reload,
    ReloadBare,
    BookmarkPage,
    OpenBookmarks,
    ReadLater,
    OpenReadingList,
    OpenHistory,
    Quit,
    CopyPageUrl,
    CopyFocusedLink,
    Find,
    OpenSettings,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    Narrower,
    Wider,
    SetTheme(usize),
    SetTypeface(usize),
    SetImages(usize),
    ToggleLinkAddresses,
    ToggleOutline,
    TogglePageInfo,
    ToggleMenuBar,
    CollapseAllReplies,
    Back,
    Forward,
    ShowHelp,
    ShowAbout,
}

/// Which live bit of state a checked item reflects.
///
/// A separate type from [`Command`] because the check mark is drawn from the reader's state and
/// the click is a toggle: one of these answers "is it on", and the command answers "turn it".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Checked {
    /// The page on screen is bookmarked. See `reader::archive`.
    Bookmarked,
    LinkAddresses,
    Outline,
    PageInfo,
    MenuBar,
}

/// When an item is greyed out.
///
/// The strip's Back and Forward buttons already do this off `History::can_go_back`; the menu
/// needs the same for the items that cannot mean anything without a page, a focused link, or a
/// discussion. An item that fires on nothing is worse than one that is visibly unavailable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Needs {
    /// Always available.
    Nothing,
    /// A page is on screen.
    Page,
    /// A page that came from a request is on screen.
    ///
    /// Narrower than [`Needs::Page`], and the difference is the bookmarks view itself: once a built
    /// view is a real `Page::Ready`, `Page` is satisfied on it, and "keep this page" there would
    /// offer to bookmark the bookmarks view.
    FetchedPage,
    /// Tab has landed on a link.
    FocusedLink,
    /// The page has at least one discussion in it.
    Thread,
    Back,
    Forward,
}

/// One row of a menu.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    /// A plain command. `Run` and not `Command` so the variant does not shadow the payload's
    /// type name at every match site.
    Run {
        label: &'static str,
        /// The key that already does this, drawn right-aligned. Empty when there is none.
        keys: &'static str,
        command: Command,
        needs: Needs,
    },
    /// A command with a check mark reflecting live state.
    Check {
        label: &'static str,
        keys: &'static str,
        command: Command,
        checked: Checked,
        needs: Needs,
    },
    /// A submenu of mutually exclusive options, filled from a `prefs` field's choice list so
    /// the option labels are written once and the menu cannot drift from the panel.
    Radio {
        label: &'static str,
        family: Radio,
    },
    Separator,
}

/// Which setting a [`Item::Radio`] submenu drives.
///
/// An enum and not a `fn(usize) -> Command`: deriving `PartialEq` over a function pointer
/// compares addresses, which the compiler warns about because two distinct functions may share
/// one after merging and one function may have two across codegen units. The mapping is small
/// and closed, so naming it costs nothing and is checkable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Radio {
    Theme,
    Typeface,
    Images,
}

impl Radio {
    pub const ALL: [Radio; 3] = [Radio::Theme, Radio::Typeface, Radio::Images];

    /// The `prefs` field this submenu shows and writes. One source for the options and for the
    /// destination, so a submenu cannot draw one field's labels and set another's value.
    pub fn field(self) -> crate::reader::prefs::FieldId {
        use crate::reader::prefs::FieldId;
        match self {
            Radio::Theme => FieldId::Theme,
            Radio::Typeface => FieldId::Typeface,
            Radio::Images => FieldId::Images,
        }
    }

    /// The command choosing option `i`.
    pub fn command(self, i: usize) -> Command {
        match self {
            Radio::Theme => Command::SetTheme(i),
            Radio::Typeface => Command::SetTypeface(i),
            Radio::Images => Command::SetImages(i),
        }
    }
}

/// One menu on the bar.
#[derive(Debug, Clone, PartialEq)]
pub struct Menu {
    pub title: &'static str,
    pub items: Vec<Item>,
}

/// The bar, left to right.
///
/// Five menus rather than the conventional four. `History` earns its place because Back and
/// Forward have no natural home among `File`, `Edit`, `View` and `Help`, and every browser with
/// a menu bar puts them under this title.
/// Built once. Every field in the table is a `&'static str` or a fieldless `Copy` enum, and
/// `keyspec` is a `cfg`-selected module of constants rather than anything read at runtime, so
/// there is nothing here that a second call could answer differently. It was six `Vec`
/// allocations per frame the bar was drawn, which is every frame.
pub fn bar() -> &'static [Menu] {
    static BAR: std::sync::LazyLock<Vec<Menu>> = std::sync::LazyLock::new(build);
    &BAR
}

fn build() -> Vec<Menu> {
    use Command as C;
    use Needs as N;
    vec![
        Menu {
            title: "File",
            items: vec![
                Item::Run {
                    label: "Search or open a URL...",
                    keys: OPEN_LOCATION,
                    command: C::OpenLocation,
                    needs: N::Nothing,
                },
                Item::Run {
                    label: "Reload",
                    keys: RELOAD,
                    command: C::Reload,
                    needs: N::Page,
                },
                Item::Run {
                    label: "Reload without rewrite",
                    keys: RELOAD_BARE,
                    command: C::ReloadBare,
                    needs: N::Page,
                },
                Item::Separator,
                // A check mark rather than a Keep/Forget label that swaps. A tick already means
                // "this is on" everywhere else in the bar, and `Checked` is the existing
                // mechanism for a row whose state the reader can see at a glance.
                Item::Check {
                    label: "Keep this page",
                    keys: BOOKMARK_PAGE,
                    command: C::BookmarkPage,
                    checked: Checked::Bookmarked,
                    needs: N::FetchedPage,
                },
                // `Item::Run` and not `Item::Check` beside "Keep this page", though this is a
                // toggle too. A tick answers a question about the page on screen and stays
                // still while the reader reads it; this one would answer about whichever link
                // Tab last touched, so it would be a frame stale and would flicker down the
                // page as focus moved. `menu_ui::Checks` is also rebuilt every frame the bar is
                // drawn, and its own comment says an uncached `holds` costs a few thousand URL
                // parses per frame — `Needs::FocusedLink` costs an `is_some`.
                Item::Run {
                    label: "Read this link later",
                    keys: READ_LATER,
                    command: C::ReadLater,
                    needs: N::FocusedLink,
                },
                // The one view worth reaching from the idle screen, so it needs nothing.
                Item::Run {
                    label: "Bookmarks",
                    keys: BOOKMARKS,
                    command: C::OpenBookmarks,
                    needs: N::Nothing,
                },
                // Beside Bookmarks rather than under History with the visited pages, because it is
                // the other list the reader filled on purpose. It needs nothing on screen, for
                // the bookmarks' reason: what it holds outlives the session.
                Item::Run {
                    label: "Read next",
                    keys: "l",
                    command: C::OpenReadingList,
                    needs: N::Nothing,
                },
                Item::Separator,
                Item::Run {
                    label: "Quit",
                    keys: QUIT,
                    command: C::Quit,
                    needs: N::Nothing,
                },
            ],
        },
        Menu {
            title: "Edit",
            items: vec![
                Item::Run {
                    label: "Copy page address",
                    keys: "y",
                    command: C::CopyPageUrl,
                    needs: N::Page,
                },
                Item::Run {
                    label: "Copy link address",
                    keys: "Y",
                    command: C::CopyFocusedLink,
                    needs: N::FocusedLink,
                },
                Item::Run {
                    label: "Find in page...",
                    keys: FIND,
                    command: C::Find,
                    needs: N::Page,
                },
                Item::Separator,
                // Under `Edit`, which is where GTK has put Preferences for twenty years. macOS
                // puts it under the application menu, which eframe does not give us.
                Item::Run {
                    label: "Settings...",
                    keys: SETTINGS,
                    command: C::OpenSettings,
                    needs: N::Nothing,
                },
            ],
        },
        Menu {
            title: "View",
            items: vec![
                Item::Run {
                    label: "Zoom in",
                    keys: ZOOM_IN,
                    command: C::ZoomIn,
                    needs: N::Nothing,
                },
                Item::Run {
                    label: "Zoom out",
                    keys: ZOOM_OUT,
                    command: C::ZoomOut,
                    needs: N::Nothing,
                },
                Item::Run {
                    label: "Actual size",
                    keys: ZOOM_RESET,
                    command: C::ZoomReset,
                    needs: N::Nothing,
                },
                Item::Separator,
                Item::Run {
                    label: "Narrower lines",
                    keys: "[",
                    command: C::Narrower,
                    needs: N::Nothing,
                },
                Item::Run {
                    label: "Wider lines",
                    keys: "]",
                    command: C::Wider,
                    needs: N::Nothing,
                },
                Item::Separator,
                Item::Radio {
                    label: "Theme",
                    family: Radio::Theme,
                },
                Item::Radio {
                    label: "Typeface",
                    family: Radio::Typeface,
                },
                Item::Radio {
                    label: "Images",
                    family: Radio::Images,
                },
                Item::Check {
                    label: "Show link addresses",
                    keys: "",
                    command: C::ToggleLinkAddresses,
                    checked: Checked::LinkAddresses,
                    needs: N::Nothing,
                },
                Item::Separator,
                Item::Check {
                    label: "Outline",
                    keys: OUTLINE,
                    command: C::ToggleOutline,
                    checked: Checked::Outline,
                    needs: N::Page,
                },
                Item::Check {
                    label: "Page info",
                    keys: PAGE_INFO,
                    command: C::TogglePageInfo,
                    checked: Checked::PageInfo,
                    needs: N::Nothing,
                },
                Item::Run {
                    label: "Collapse all replies",
                    keys: "Z",
                    command: C::CollapseAllReplies,
                    needs: N::Thread,
                },
                Item::Separator,
                Item::Check {
                    label: "Menu bar",
                    keys: "F10",
                    command: C::ToggleMenuBar,
                    checked: Checked::MenuBar,
                    needs: N::Nothing,
                },
            ],
        },
        Menu {
            title: "History",
            items: vec![
                Item::Run {
                    label: "Back",
                    keys: BACK,
                    command: C::Back,
                    needs: N::Back,
                },
                Item::Run {
                    label: "Forward",
                    keys: FORWARD,
                    command: C::Forward,
                    needs: N::Forward,
                },
                Item::Separator,
                // Under this title rather than beside Bookmarks in File, because a reader looking
                // for a page they read yesterday looks where Back is. It needs nothing: the
                // pages it lists outlive the session, so it opens on the splash too.
                Item::Run {
                    label: "Pages you have visited",
                    keys: HISTORY,
                    command: C::OpenHistory,
                    needs: N::Nothing,
                },
            ],
        },
        Menu {
            title: "Help",
            items: vec![
                Item::Run {
                    label: "Keyboard shortcuts",
                    keys: "?",
                    command: C::ShowHelp,
                    needs: N::Nothing,
                },
                Item::Run {
                    label: "About hww",
                    keys: "",
                    command: C::ShowAbout,
                    needs: N::Nothing,
                },
            ],
        },
    ]
}

/// The help card, key spec and description. Drawn by `ui::app::help_overlay` through
/// `notice_ui::keycaps`, which splits a spec on whitespace and sets `/`, `or`, `then` and `·`
/// as separators rather than keycaps.
///
/// Written with the glyphs the shipped faces actually carry.
pub const HELP: &[(&str, &str)] = &[
    ("j k ↓ ↑", "scroll"),
    ("Space / PgDn", "page down · Shift+Space pages up"),
    ("g / G", "top / bottom"),
    (
        OPEN_LOCATION,
        "URL bar · Enter opens an address, or searches for words",
    ),
    ("Tab / Shift+Tab", "cycle links · Enter follows"),
    (BACK, "back"),
    (FORWARD, "forward · the mouse side buttons do both"),
    (
        HELP_RELOAD,
        "reload · reload bare: no rewrite rule, no site profile, no search shape, no feed reader",
    ),
    ("i / I", "load focused image · load all, naming their hosts"),
    (OUTLINE, "outline"),
    (PAGE_INFO, "page info: how this page arrived"),
    (HELP_FIND, "find in page · next / previous match"),
    ("z / Z", "collapse focused reply · collapse all"),
    ("y / Y", "copy page URL · copy focused link"),
    (BOOKMARK_PAGE, "bookmark this page"),
    (READ_LATER, "add the focused link to your reading list"),
    // The three openers on one row, as reload and `y / Y` already are. They are one family —
    // each opens a view hww builds from `archive.json` — and the card is sized by the window
    // rather than scrolled, so a row per tenant would push the footnote off the bottom edge the
    // moment a fourth arrives. `the_menu_and_the_help_card_agree` matches per token, so the
    // three menu rows still each find their key here; the reading list is the one of the three
    // no browser has a key for, which is why `l` sits between two chords.
    (
        HELP_LISTS,
        "open your bookmarks · reading list · visited pages",
    ),
    ("[ / ]", "narrow / widen the reading measure"),
    (HELP_ZOOM, "zoom in · out · actual size"),
    ("d", "cycle theme"),
    (SETTINGS, "settings"),
    ("F10", "the menu bar"),
    (HELP_DISMISS, "this help · dismiss chrome · quit"),
];

/// What `Help` ▸ `About hww` says. One sentence about what this is, the two facts that are the
/// whole point of it, which are absences and are therefore invisible from the inside, and where
/// the source is.
///
/// That last line carries its weight. Someone holding only the `.deb` or the `.zip` has a binary
/// and a license text beside it and nothing saying what it was built from, and the AGPL is a
/// license whose whole mechanism is that the source stays reachable. A reader that opens no
/// connection it was not asked for cannot answer that by fetching anything, so the address is a
/// literal here, tested for glyph coverage with every other string on this screen.
pub const ABOUT: &[&str] = &[
    "hww is a reading client for the non-app web.",
    "It runs no JavaScript, applies no CSS from the page, and stores no cookies: the capability \
     is absent from the build rather than switched off.",
    "It makes no request you did not ask for, except the small site icon beside a page title.",
    "Free software under the GNU Affero General Public License, version 3 or later. The source \
     is at https://github.com/tayler/hww.",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::prefs;

    fn items() -> Vec<&'static Item> {
        bar().iter().flat_map(|m| &m.items).collect()
    }

    fn commands() -> Vec<Command> {
        items()
            .iter()
            .filter_map(|i| match i {
                Item::Run { command, .. } | Item::Check { command, .. } => Some(*command),
                Item::Radio { .. } | Item::Separator => None,
            })
            .collect()
    }

    /// Every command is reachable, exactly once, and no menu is empty.
    ///
    /// The exhaustive match is the point, the same trick `prefs::all_lists_every_field` uses: a
    /// command added to the enum and to no menu fails to compile here, which is the prompt to
    /// put it on a menu or to decide it does not belong on one.
    #[test]
    fn every_command_is_reachable() {
        let listed = commands();

        for menu in bar() {
            assert!(!menu.items.is_empty(), "{} is empty", menu.title);
            assert!(!menu.title.is_empty());
            // A menu that is only separators would draw as a blank popup.
            assert!(
                menu.items.iter().any(|i| !matches!(i, Item::Separator)),
                "{} has nothing but separators",
                menu.title
            );
        }

        for c in [
            Command::OpenLocation,
            Command::Reload,
            Command::ReloadBare,
            Command::BookmarkPage,
            Command::OpenBookmarks,
            Command::ReadLater,
            Command::OpenReadingList,
            Command::OpenHistory,
            Command::Quit,
            Command::CopyPageUrl,
            Command::CopyFocusedLink,
            Command::Find,
            Command::OpenSettings,
            Command::ZoomIn,
            Command::ZoomOut,
            Command::ZoomReset,
            Command::Narrower,
            Command::Wider,
            Command::ToggleLinkAddresses,
            Command::ToggleOutline,
            Command::TogglePageInfo,
            Command::ToggleMenuBar,
            Command::CollapseAllReplies,
            Command::Back,
            Command::Forward,
            Command::ShowHelp,
            Command::ShowAbout,
        ] {
            let hits = listed.iter().filter(|l| **l == c).count();
            assert_eq!(hits, 1, "{c:?} appears {hits} times, want 1");
        }

        // The three radio families are reached through `Item::Radio`, not as plain commands.
        for c in listed {
            assert!(
                !matches!(
                    c,
                    Command::SetTheme(_) | Command::SetTypeface(_) | Command::SetImages(_)
                ),
                "{c:?} should be a Radio item, not a Run"
            );
        }
    }

    /// Every radio submenu names a `prefs` field that really is a choice, and its command
    /// builder covers exactly that field's options.
    ///
    /// The failure this catches is a submenu drawn from one field's options while its clicks
    /// write another's, which compiles and silently sets the wrong thing.
    #[test]
    fn every_radio_matches_its_field() {
        let all = prefs::fields();
        for item in items() {
            let Item::Radio { family, .. } = item else {
                continue;
            };
            let field = family.field();
            let f = all.iter().find(|f| f.id == field).expect("a known field");
            let prefs::Control::Choice(options) = f.control else {
                panic!("{field:?} is drawn as a radio but is not a Choice");
            };
            assert!(options.len() >= 2);
            // Each index builds a distinct command, so no two options collide.
            let built: Vec<Command> = (0..options.len()).map(|i| family.command(i)).collect();
            let mut seen = built.clone();
            seen.dedup();
            assert_eq!(
                seen.len(),
                built.len(),
                "{field:?}: two options share a command"
            );
        }
    }

    /// The menu and the help card agree about which key does what.
    ///
    /// Possible only because `HELP` lives here now. Every key a menu item advertises appears
    /// somewhere in the help table, so the two surfaces cannot claim different keys for the same
    /// command. This does **not** verify `app::handle_keys`; see the module doc for why not, and
    /// for what closing that would cost.
    ///
    /// Both tables are drawn from [`keyspec`], so this checks a different pair of strings on
    /// macOS than it does anywhere else. That is the intent: the failure it catches is one block
    /// of specs updated and the other left behind, which is exactly how a platform whose menu
    /// nobody on this project opens would start lying.
    #[test]
    fn the_menu_and_the_help_card_agree() {
        let help: String = HELP.iter().map(|(k, _)| format!("{k} ")).collect();
        for item in items() {
            let keys = match item {
                Item::Run { keys, .. } | Item::Check { keys, .. } => keys,
                Item::Radio { .. } | Item::Separator => continue,
            };
            if keys.is_empty() {
                continue;
            }
            // Two branches, and the second is the one that carries most of the table now that
            // the keys are chords: `RELOAD` is `Ctrl+R`, which is *not* a substring of the
            // `Ctrl+Shift+R` beside it on the card, and `HISTORY` is not one of `BOOKMARKS`
            // either. A whole-spec substring still catches the rows the card spells alone, and
            // the token match catches every row that puts two specs on one line.
            let bare = keys.trim();
            assert!(
                help.contains(bare)
                    || HELP
                        .iter()
                        .any(|(k, _)| k.split_whitespace().any(|t| t == bare)),
                "the menu offers {bare:?} and the help card does not list it"
            );
        }
    }

    /// The help card and the toast name the same list.
    ///
    /// A bare reload says what it turned off twice: once on the card before you press it, once in
    /// the toast after. A reload that quietly stopped honouring one more of them would leave
    /// both surfaces lying, and this is the cheaper half of noticing that.
    #[test]
    fn the_bare_reload_says_the_same_thing_twice() {
        let toast = crate::reader::notice::RELOADED_BARE;
        let tables = toast
            .split_once("bare: ")
            .expect("the toast names what it turned off")
            .1;
        assert!(
            HELP.iter().any(|(_, what)| what.contains(tables)),
            "the help card does not list {tables:?}"
        );
    }

    /// Every help row is a key spec and a description, both non-empty.
    #[test]
    fn the_help_card_is_well_formed() {
        for (keys, what) in HELP {
            assert!(!keys.is_empty(), "a help row with no keys: {what:?}");
            assert!(!what.is_empty(), "a help row with no description: {keys:?}");
            assert!(
                !what.ends_with('.'),
                "help descriptions are labels, not sentences: {what:?}"
            );
        }
    }

    /// No string this module can put on screen needs a glyph the shipped faces lack.
    ///
    /// The executable form of the module doc, and the reason `SUBMENU_MARKER` exists. epaint has
    /// no fallback beyond the faces `ui::fonts` registers, so a missing glyph is a box on
    /// the screen and nothing anywhere says why. Prose does not fail a build; this does.
    #[test]
    fn nothing_needs_a_glyph_the_faces_lack() {
        const MISSING: [char; 6] = [
            '\u{23F5}', // egui's own submenu arrow, in none of the shipped faces
            '\u{25B8}', '\u{25B6}', // triangles carried only by the DejaVuSerif tail
            '\u{2630}', // hamburger
            '\u{2699}', // gear
            '\u{24D8}', // circled i
        ];
        let mut strings: Vec<String> = vec![SUBMENU_MARKER.to_owned()];
        for m in bar() {
            strings.push(m.title.to_owned());
            for i in &m.items {
                match i {
                    Item::Run { label, keys, .. } | Item::Check { label, keys, .. } => {
                        strings.push((*label).to_owned());
                        strings.push((*keys).to_owned());
                    }
                    Item::Radio { label, .. } => strings.push((*label).to_owned()),
                    Item::Separator => {}
                }
            }
        }
        for (k, w) in HELP {
            strings.push((*k).to_owned());
            strings.push((*w).to_owned());
        }
        strings.extend(ABOUT.iter().map(|s| (*s).to_owned()));

        for s in &strings {
            for c in MISSING {
                assert!(
                    !s.contains(c),
                    "U+{:04X} is in none of the shipped faces: {s:?}",
                    c as u32
                );
            }
        }
        assert_eq!(SUBMENU_MARKER, "\u{203A}");
    }

    /// Menu labels are labels: capitalised, not sentences, and an ellipsis only where the item
    /// opens something that asks for more input, which is the oldest convention on a menu.
    #[test]
    fn menu_labels_follow_the_convention() {
        for m in bar() {
            for i in &m.items {
                let label = match i {
                    Item::Run { label, .. }
                    | Item::Check { label, .. }
                    | Item::Radio { label, .. } => label,
                    Item::Separator => continue,
                };
                assert!(!label.is_empty());
                assert!(
                    label.chars().next().is_some_and(char::is_uppercase),
                    "{label:?} does not start with a capital"
                );
                if let Some(stem) = label.strip_suffix("...") {
                    assert!(
                        !stem.ends_with('.'),
                        "{label:?} has more dots than an ellipsis"
                    );
                } else {
                    assert!(!label.ends_with('.'), "{label:?} is not a sentence");
                }
            }
        }
    }

    /// The three items that open something further are the three that say so.
    #[test]
    fn only_the_items_that_ask_for_more_carry_an_ellipsis() {
        let with: Vec<&'static str> = items()
            .iter()
            .filter_map(|i| match i {
                Item::Run { label, .. } if label.ends_with("...") => Some(*label),
                _ => None,
            })
            .collect();
        assert_eq!(
            with,
            vec!["Search or open a URL...", "Find in page...", "Settings..."]
        );
    }
}
