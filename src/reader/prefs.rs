//! Every setting, as a labelled control with a sentence about it. **No egui types.**
//!
//! This is [`pageinfo`](crate::reader::pageinfo)'s shape applied to a different question.
//! `pageinfo` answers "how did this page arrive"; this answers "what can I change", and both
//! keep their structure and their wording out of `ui/` so `cargo test` can read them. The panel
//! that draws this is `ui::prefs_ui`, and it decides nothing: it walks [`fields`] in order,
//! draws the control each [`Field`] names, and writes back through [`set`].
//!
//! # Why the field list is data
//!
//! Thirteen settings existed for a long time and three of them were reachable. The rest needed a
//! text editor and the knowledge that there was a file to edit, which nothing in the reader or
//! the README mentioned. A hand-written panel would have fixed that once; a *list* fixes it
//! permanently, because [`every_setting_is_exposed`](self#tests) walks the serialized form of
//! [`Settings`] and fails when a field has no entry here. The recurrence is the thing worth
//! preventing: the panel drifting behind the struct is how it got to three of thirteen.
//!
//! # Why `get`/`set` are matches and not closures
//!
//! A `Field` carrying `fn(&mut Settings, Value)` would be tidy and would let a new setting be
//! added with no compiler complaint anywhere. Two exhaustive `match`es on [`FieldId`] instead
//! mean a new variant is a compile error at both ends, and the compiler names the two places to
//! fix. `opts::Theme::REAL` earns its keep the same way, and its doc comment makes the same
//! argument: a palette added to one exhaustive match and to nothing else escapes every test in
//! the crate silently.
//!
//! # Wording
//!
//! Labels and notes are written for someone who has just opened hww, not for someone who has
//! read `opts.rs`. That is a real constraint and it changed four of these: `measure_chars` is
//! "Line width" and says the count is approximate, because `theme::measure_px`
//! multiplies by an *average* advance; `base_size_pt` says it scales the reader's own labels,
//! because `theme::chrome_font` is derived from it; `family` explains that a sans page gets
//! serif headings, because `theme::heading_role` does that and it is otherwise inexplicable;
//! and `images` no longer claims that "never" means nothing is fetched, because the favicon was
//! never covered by it. `no_label_leaks_an_internal_name` is what keeps the field names out.

use crate::reader::opts::{FontChoice, ImagePolicy, ReadOpts, Theme};
use crate::reader::settings::Settings;

/// Which heading a field sits under, in the order the panel draws them.
///
/// Reading first, then the page, then the two that are not about reading at all. `Privacy` is
/// last because it is the only group whose setting has a cost that is not visible on screen,
/// and burying it above the fold next to a font size would say the wrong thing about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    Text,
    Theme,
    Page,
    Replies,
    Window,
    Privacy,
}

impl Group {
    /// Every group, in panel order.
    pub const ALL: [Group; 6] = [
        Group::Text,
        Group::Theme,
        Group::Page,
        Group::Replies,
        Group::Window,
        Group::Privacy,
    ];

    /// The heading, as drawn: a letter-spaced chrome label.
    pub fn title(self) -> &'static str {
        match self {
            Group::Text => "Text",
            Group::Theme => "Theme",
            Group::Page => "Page",
            Group::Replies => "Replies",
            Group::Window => "Window",
            Group::Privacy => "Privacy",
        }
    }

    /// A sentence under the heading, for the two groups where the controls alone mislead.
    ///
    /// `Theme` needs one because `d` cycles four of the six and the panel is the only way to
    /// reach the other two, which is not discoverable from six radio buttons. `Privacy` needs
    /// one because a reader arriving at a single checkbox deserves to know why it is on.
    pub fn note(self) -> Option<&'static str> {
        match self {
            Group::Theme => Some(
                "The d key cycles the first four and never lands on the high-contrast pair, \
                 which the panel is the only way to reach. Pressing d while one of them is on \
                 leaves it, so choose here and leave d alone.",
            ),
            Group::Privacy => Some(
                "hww has no cookie store, sends no Referer for pages, and makes no request you \
                 did not ask for. This is the one disclosure it does make.",
            ),
            Group::Text | Group::Page | Group::Replies | Group::Window => None,
        }
    }
}

/// One setting. The variants are the panel's index into [`Settings`], and both [`get`] and
/// [`set`] match on them exhaustively so a new one cannot be added silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FieldId {
    LineWidth,
    TextSize,
    LineSpacing,
    ParagraphSpacing,
    Typeface,
    Theme,
    Images,
    ShowLinkAddresses,
    Zoom,
    ScrollStep,
    ReplyIndent,
    StopIndentingAfter,
    ShowMenuBar,
    SendImageReferer,
}

impl FieldId {
    /// Every field. Walked by the tests, and by nothing else: the panel walks [`fields`],
    /// which carries the order and the grouping.
    pub const ALL: [FieldId; 14] = [
        FieldId::LineWidth,
        FieldId::TextSize,
        FieldId::LineSpacing,
        FieldId::ParagraphSpacing,
        FieldId::Typeface,
        FieldId::Theme,
        FieldId::Images,
        FieldId::ShowLinkAddresses,
        FieldId::Zoom,
        FieldId::ScrollStep,
        FieldId::ReplyIndent,
        FieldId::StopIndentingAfter,
        FieldId::ShowMenuBar,
        FieldId::SendImageReferer,
    ];

    /// The path this field occupies in the serialized [`Settings`], as `settings.json` spells
    /// it: `("read", "measure_chars")` for a nested field, `(None, "zoom_factor")` for a
    /// top-level one.
    ///
    /// This exists for `every_setting_is_exposed`, which is the test the whole module is for.
    /// Naming the JSON key rather than deriving one means the mapping is checkable against the
    /// serialized struct without a macro, and a renamed field breaks the test rather than
    /// leaving it to cover nothing at all.
    pub fn json_path(self) -> (Option<&'static str>, &'static str) {
        match self {
            FieldId::LineWidth => (Some("read"), "measure_chars"),
            FieldId::TextSize => (Some("read"), "base_size_pt"),
            FieldId::LineSpacing => (Some("read"), "line_height"),
            FieldId::ParagraphSpacing => (Some("read"), "paragraph_spacing"),
            FieldId::Typeface => (Some("read"), "family"),
            FieldId::Theme => (Some("read"), "theme"),
            FieldId::Images => (Some("read"), "images"),
            FieldId::ShowLinkAddresses => (Some("read"), "show_link_urls"),
            FieldId::ReplyIndent => (Some("read"), "indent_per_depth"),
            FieldId::StopIndentingAfter => (Some("read"), "max_thread_indent"),
            FieldId::Zoom => (None, "zoom_factor"),
            FieldId::ScrollStep => (None, "scroll_lines"),
            FieldId::ShowMenuBar => (None, "show_menu_bar"),
            FieldId::SendImageReferer => (None, "send_image_referer"),
        }
    }
}

/// One option of a [`Control::Choice`], with the sentence that distinguishes it from its
/// neighbours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choice {
    pub label: &'static str,
    /// Set where the label alone cannot carry the difference. `Light` and `Dark` need nothing;
    /// `Never` versus `No image requests` is the whole reason this field exists.
    pub note: Option<&'static str>,
}

const fn choice(label: &'static str) -> Choice {
    Choice { label, note: None }
}

const fn choice_note(label: &'static str, note: &'static str) -> Choice {
    Choice {
        label,
        note: Some(note),
    }
}

/// What kind of control a field gets. Three kinds, and the vocabulary grows only when a field
/// asks for a fourth — the rule `profile::Profile` follows for the same reason.
#[derive(Debug, Clone, PartialEq)]
pub enum Control {
    /// A numeric range. `unit` is drawn after the value, so a reader sees `70 characters`
    /// rather than `70`.
    Slider {
        min: f32,
        max: f32,
        /// What one press of the panel's `-` / `+` moves. Those buttons are the keyboard route:
        /// `app::handle_keys` consumes the arrow keys for scrolling before any widget sees
        /// them, so a focused slider cannot be nudged with them.
        step: f32,
        /// `false` for the one field that counts whole levels.
        fractional: bool,
        unit: &'static str,
    },
    /// A radio group.
    Choice(&'static [Choice]),
    Toggle,
}

/// A setting as the panel draws it.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub id: FieldId,
    pub group: Group,
    pub label: &'static str,
    /// The sentence under the control. `None` only where the label says everything,
    /// which `every_field_explains_itself` holds to a short list.
    pub note: Option<&'static str>,
    /// The keys that already do this, for the hint beside the label. Empty for the settings
    /// that never had one.
    pub keys: &'static str,
    pub control: Control,
}

/// A value read from or written to [`Settings`], as the control kinds see it.
///
/// Deliberately not `serde_json::Value`: the panel needs to know a choice by *index* so a radio
/// group can be drawn and clicked without knowing which enum sits behind it, and JSON would
/// make that a string comparison against a variant name.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    Num(f32),
    /// Index into the field's [`Control::Choice`] slice.
    Index(usize),
    Bool(bool),
}

impl Value {
    pub fn num(self) -> Option<f32> {
        match self {
            Value::Num(n) => Some(n),
            _ => None,
        }
    }

    pub fn index(self) -> Option<usize> {
        match self {
            Value::Index(i) => Some(i),
            _ => None,
        }
    }

    pub fn boolean(self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(b),
            _ => None,
        }
    }
}

const TYPEFACES: &[Choice] = &[
    choice_note("Sans", "IBM Plex Sans."),
    choice_note("Serif", "IBM Plex Serif."),
    choice_note(
        "Mono",
        "IBM Plex Mono. No italic monospace is shipped, so emphasis in this one is slanted by \
         the renderer rather than drawn.",
    ),
];

/// The order here is the order [`Theme::REAL`] does not have: `System` first, because it is the
/// default and because it is the only entry that is not a set of colours, then the cycle `d`
/// walks, then the pair `d` deliberately leaves out.
const THEMES: &[Choice] = &[
    choice_note(
        "Follow the desktop",
        "Uses Light or Dark to match your desktop setting.",
    ),
    choice("Light"),
    choice_note(
        "Sepia",
        "Warm paper, at a deliberately gentler contrast than Light rather than a tint of it.",
    ),
    choice("Dark"),
    choice_note(
        "High contrast (light)",
        "Holds every colour to a 7:1 contrast ratio instead of 4.5:1.",
    ),
    choice_note("High contrast (dark)", "The same, dark."),
];

/// The three image policies, described by what each one *keeps*. A reader that silently drops a
/// picture is lying about the page, so every option here still marks where one was; what
/// changes is whether it offers to fetch it, and whether anything is fetched at all.
const IMAGES: &[Choice] = &[
    choice_note(
        "Ask first",
        "Shows a strip naming the site each picture would come from. Nothing is fetched until \
         you click it, or press i for the focused one or I for all of them.",
    ),
    choice_note(
        "Article images off",
        "Marks where each picture was but offers no way to load it. The small site icon beside \
         the page title is still fetched.",
    ),
    choice_note(
        "No image requests",
        "Nothing is fetched at all, the site icon included.",
    ),
];

/// Every setting, in the order the panel draws it, grouped.
///
/// Grouped in reading order, so `prefs_ui` draws a heading on every change of group and never
/// sorts — the same contract `pageinfo::rows` has, and `fields_are_grouped_in_reading_order`
/// is what holds it.
pub fn fields() -> Vec<Field> {
    use Control::*;
    use FieldId as F;
    use Group as G;
    vec![
        Field {
            id: F::LineWidth,
            group: G::Text,
            label: "Line width",
            note: Some(
                "Roughly how many characters fit on a line before it wraps. Books settle near \
                 66.",
            ),
            keys: "[ ]",
            control: Slider {
                min: ReadOpts::MEASURE_MIN,
                max: ReadOpts::MEASURE_MAX,
                step: 1.0,
                fractional: false,
                unit: "characters",
            },
        },
        Field {
            id: F::TextSize,
            group: G::Text,
            label: "Text size",
            note: Some(
                "The size the page's body text is set at. Headings, captions and the reader's \
                 own labels are all sized from it.",
            ),
            keys: "",
            control: Slider {
                min: ReadOpts::SIZE_MIN,
                max: ReadOpts::SIZE_MAX,
                step: 0.5,
                fractional: true,
                unit: "pt",
            },
        },
        Field {
            id: F::LineSpacing,
            group: G::Text,
            label: "Line spacing",
            note: Some(
                "The distance from one line down to the next, as a multiple of the text size.",
            ),
            keys: "",
            control: Slider {
                min: ReadOpts::LINE_HEIGHT_MIN,
                max: ReadOpts::LINE_HEIGHT_MAX,
                step: 0.05,
                fractional: true,
                unit: "x the text size",
            },
        },
        Field {
            id: F::ParagraphSpacing,
            group: G::Text,
            label: "Paragraph spacing",
            note: Some(
                "The gap between paragraphs, and between lists, quotations and other blocks, as \
                 a multiple of the text size.",
            ),
            keys: "",
            control: Slider {
                min: ReadOpts::PARAGRAPH_SPACING_MIN,
                max: ReadOpts::PARAGRAPH_SPACING_MAX,
                step: 0.05,
                fractional: true,
                unit: "x the text size",
            },
        },
        Field {
            id: F::Typeface,
            group: G::Text,
            label: "Typeface",
            note: Some(
                "Over a sans page the title and the headings are set in the serif, which is \
                 the oldest pairing in typography. A serif or mono page keeps one typeface \
                 throughout.",
            ),
            keys: "",
            control: Choice(TYPEFACES),
        },
        Field {
            id: F::Theme,
            group: G::Theme,
            label: "Theme",
            note: None,
            keys: "d",
            control: Choice(THEMES),
        },
        Field {
            id: F::Images,
            group: G::Page,
            label: "Images",
            note: None,
            keys: "i I",
            control: Choice(IMAGES),
        },
        Field {
            id: F::ShowLinkAddresses,
            group: G::Page,
            label: "Show link addresses",
            note: Some(
                "Prints each link's address after the link text, smaller and dimmer, like \
                 the quiet web <https://example.com/>.",
            ),
            keys: "",
            control: Toggle,
        },
        Field {
            id: F::Zoom,
            group: G::Page,
            label: "Zoom",
            note: Some(
                "Magnifies the whole window, borders and spacing included, the way a browser's \
                 zoom does. Text size changes the type; this scales everything.",
            ),
            keys: "Ctrl+ + - 0",
            control: Slider {
                min: Settings::ZOOM_MIN,
                max: Settings::ZOOM_MAX,
                step: 0.1,
                fractional: true,
                unit: "x",
            },
        },
        Field {
            id: F::ScrollStep,
            group: G::Page,
            label: "Scroll step",
            note: Some(
                "How far one wheel notch, or one press of j or k, moves the page. Measured in \
                 lines rather than pixels, so a step stays the same size when the text size \
                 changes.",
            ),
            keys: "j k",
            control: Slider {
                min: Settings::SCROLL_LINES_MIN,
                max: Settings::SCROLL_LINES_MAX,
                step: 0.5,
                fractional: true,
                unit: "lines",
            },
        },
        Field {
            id: F::ReplyIndent,
            group: G::Replies,
            label: "Reply indent",
            note: Some("How far each reply is indented from the one it answers."),
            keys: "",
            control: Slider {
                min: ReadOpts::INDENT_MIN,
                max: ReadOpts::INDENT_MAX,
                step: 1.0,
                fractional: false,
                unit: "pt",
            },
        },
        Field {
            id: F::StopIndentingAfter,
            group: G::Replies,
            label: "Stop indenting after",
            note: Some(
                "Replies deeper than this stay at the same indent, so a long chain is not \
                 squeezed into a sliver.",
            ),
            keys: "",
            control: Slider {
                min: f32::from(ReadOpts::MAX_INDENT_MIN),
                max: f32::from(ReadOpts::MAX_INDENT_MAX),
                step: 1.0,
                fractional: false,
                unit: "levels",
            },
        },
        Field {
            id: F::ShowMenuBar,
            group: G::Window,
            label: "Show the menu bar",
            note: Some("Press F10 to bring it back after hiding it."),
            keys: "F10",
            control: Toggle,
        },
        Field {
            id: F::SendImageReferer,
            group: G::Privacy,
            label: "Send the site name when loading images",
            note: Some(
                "Article pictures usually sit on a different server from the article. When hww \
                 asks for one, this tells that server which site the picture was on: the domain \
                 only, never the address of the page you are reading. Many servers refuse, or \
                 substitute a different picture without saying so, and a substituted \
                 picture cannot be told from the real one. Requests for pages themselves never \
                 send it.",
            ),
            keys: "",
            control: Toggle,
        },
    ]
}

/// Read one field. Exhaustive on purpose: see the module doc.
pub fn get(s: &Settings, id: FieldId) -> Value {
    match id {
        FieldId::LineWidth => Value::Num(s.read.measure_chars),
        FieldId::TextSize => Value::Num(s.read.base_size_pt),
        FieldId::LineSpacing => Value::Num(s.read.line_height),
        FieldId::ParagraphSpacing => Value::Num(s.read.paragraph_spacing),
        FieldId::Typeface => Value::Index(match s.read.family {
            FontChoice::Sans => 0,
            FontChoice::Serif => 1,
            FontChoice::Mono => 2,
        }),
        FieldId::Theme => Value::Index(match s.read.theme {
            Theme::System => 0,
            Theme::Light => 1,
            Theme::Sepia => 2,
            Theme::Dark => 3,
            Theme::ContrastLight => 4,
            Theme::ContrastDark => 5,
        }),
        FieldId::Images => Value::Index(match s.read.images {
            ImagePolicy::Placeholder => 0,
            ImagePolicy::Never => 1,
            ImagePolicy::NoRequests => 2,
        }),
        FieldId::ShowLinkAddresses => Value::Bool(s.read.show_link_urls),
        FieldId::Zoom => Value::Num(s.zoom_factor),
        FieldId::ScrollStep => Value::Num(s.scroll_lines),
        FieldId::ReplyIndent => Value::Num(s.read.indent_per_depth),
        FieldId::StopIndentingAfter => Value::Num(f32::from(s.read.max_thread_indent)),
        FieldId::ShowMenuBar => Value::Bool(s.show_menu_bar),
        FieldId::SendImageReferer => Value::Bool(s.send_image_referer),
    }
}

/// Write one field. A `Value` of the wrong shape for the field is dropped rather than panicking:
/// the panel is the only caller and it reads the control kind from the same table, so a mismatch
/// is a bug in `prefs_ui` and not something a reader can provoke.
///
/// Out-of-range numbers are clamped here rather than trusted, because [`Control::Slider`]'s
/// bounds are advice to a widget and this is the door to the struct.
pub fn set(s: &mut Settings, id: FieldId, v: Value) {
    let clamped = |field: &Field, n: f32| -> f32 {
        match field.control {
            Control::Slider { min, max, .. } => n.clamp(min, max),
            _ => n,
        }
    };
    let all = fields();
    let Some(field) = all.iter().find(|f| f.id == id) else {
        return;
    };
    match id {
        FieldId::LineWidth => {
            if let Some(n) = v.num() {
                s.read.measure_chars = clamped(field, n).round();
            }
        }
        FieldId::TextSize => {
            if let Some(n) = v.num() {
                s.read.base_size_pt = clamped(field, n);
            }
        }
        FieldId::LineSpacing => {
            if let Some(n) = v.num() {
                s.read.line_height = clamped(field, n);
            }
        }
        FieldId::ParagraphSpacing => {
            if let Some(n) = v.num() {
                s.read.paragraph_spacing = clamped(field, n);
            }
        }
        FieldId::Typeface => {
            s.read.family = match v.index() {
                Some(0) => FontChoice::Sans,
                Some(1) => FontChoice::Serif,
                Some(2) => FontChoice::Mono,
                _ => s.read.family,
            }
        }
        FieldId::Theme => {
            s.read.theme = match v.index() {
                Some(0) => Theme::System,
                Some(1) => Theme::Light,
                Some(2) => Theme::Sepia,
                Some(3) => Theme::Dark,
                Some(4) => Theme::ContrastLight,
                Some(5) => Theme::ContrastDark,
                _ => s.read.theme,
            }
        }
        FieldId::Images => {
            s.read.images = match v.index() {
                Some(0) => ImagePolicy::Placeholder,
                Some(1) => ImagePolicy::Never,
                Some(2) => ImagePolicy::NoRequests,
                _ => s.read.images,
            }
        }
        FieldId::ShowLinkAddresses => {
            if let Some(b) = v.boolean() {
                s.read.show_link_urls = b;
            }
        }
        FieldId::Zoom => {
            if let Some(n) = v.num() {
                s.zoom_factor = clamped(field, n);
            }
        }
        FieldId::ScrollStep => {
            if let Some(n) = v.num() {
                s.scroll_lines = clamped(field, n);
            }
        }
        FieldId::ReplyIndent => {
            if let Some(n) = v.num() {
                s.read.indent_per_depth = clamped(field, n).round();
            }
        }
        FieldId::StopIndentingAfter => {
            if let Some(n) = v.num() {
                s.read.max_thread_indent = clamped(field, n).round() as u16;
            }
        }
        FieldId::ShowMenuBar => {
            if let Some(b) = v.boolean() {
                s.show_menu_bar = b;
            }
        }
        FieldId::SendImageReferer => {
            if let Some(b) = v.boolean() {
                s.send_image_referer = b;
            }
        }
    }
}

/// Put one group back to its defaults, leaving every other group alone.
pub fn reset_group(s: &mut Settings, group: Group) {
    let d = Settings::default();
    for f in fields().iter().filter(|f| f.group == group) {
        set(s, f.id, get(&d, f.id));
    }
}

/// Put everything back. Not `*s = Settings::default()`: going through [`set`] means a field
/// that is somehow missing from [`fields`] is left alone rather than silently reset, and
/// `every_setting_is_exposed` is what says there is no such field.
pub fn reset_all(s: &mut Settings) {
    for group in Group::ALL {
        reset_group(s, group);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The test the module exists for.
    ///
    /// Walk the *serialized* form of `Settings`, because that is the only description of the
    /// struct that a test can enumerate without a macro, and assert every leaf is claimed by
    /// exactly one field. A setting added to `Settings` and not to `fields()` fails here, and
    /// nothing else in the crate would notice: the panel would simply not show it, which is the
    /// state this whole change exists to get out of.
    #[test]
    fn every_setting_is_exposed() {
        let json = serde_json::to_value(Settings::default()).expect("Settings serializes");
        let obj = json.as_object().expect("a JSON object");

        let mut leaves: Vec<(Option<&str>, String)> = Vec::new();
        for (key, value) in obj {
            match value.as_object() {
                Some(nested) => {
                    for inner in nested.keys() {
                        leaves.push((Some(key.as_str()), inner.clone()));
                    }
                }
                None => leaves.push((None, key.clone())),
            }
        }
        assert!(
            leaves.len() >= 14,
            "expected at least fourteen settings, walked {}",
            leaves.len()
        );

        let claimed: Vec<(Option<&str>, &str)> =
            FieldId::ALL.iter().map(|f| f.json_path()).collect();

        for (parent, leaf) in &leaves {
            let hits = claimed
                .iter()
                .filter(|(p, l)| p == parent && l == &leaf.as_str())
                .count();
            assert_eq!(
                hits, 1,
                "{parent:?}.{leaf} is claimed by {hits} fields, want exactly 1"
            );
        }
        // And nothing claims a path that is not in the file.
        for (parent, leaf) in &claimed {
            assert!(
                leaves
                    .iter()
                    .any(|(p, l)| p == parent && l.as_str() == *leaf),
                "{parent:?}.{leaf} is claimed but is not a setting"
            );
        }
        assert_eq!(claimed.len(), leaves.len());
    }

    /// `ALL` lists every variant, enforced by an exhaustive match rather than by a length.
    ///
    /// Same shape as `opts::real_lists_every_palette`, and for the same reason: a variant added
    /// to `FieldId` fails to compile *here*, which is the prompt to add it to `ALL`. Asserting
    /// `ALL.len()` cannot catch it, because the length is still whatever it was.
    #[test]
    fn all_lists_every_field() {
        for id in FieldId::ALL {
            let listed = match id {
                FieldId::LineWidth
                | FieldId::TextSize
                | FieldId::LineSpacing
                | FieldId::ParagraphSpacing
                | FieldId::Typeface
                | FieldId::Theme
                | FieldId::Images
                | FieldId::ShowLinkAddresses
                | FieldId::Zoom
                | FieldId::ScrollStep
                | FieldId::ReplyIndent
                | FieldId::StopIndentingAfter
                | FieldId::ShowMenuBar
                | FieldId::SendImageReferer => true,
            };
            assert!(listed);
        }
        let mut sorted = FieldId::ALL;
        sorted.sort();
        let mut deduped = sorted.to_vec();
        deduped.dedup();
        assert_eq!(deduped.len(), FieldId::ALL.len(), "a duplicate in ALL");
        assert_eq!(fields().len(), FieldId::ALL.len());
    }

    /// Every field appears in `fields()` exactly once, and every control's shape matches what
    /// `get` returns for it. A `Choice` field whose `get` hands back a `Num` would draw an empty
    /// radio group and swallow every click.
    #[test]
    fn every_field_is_listed_once_with_a_matching_control() {
        let all = fields();
        for id in FieldId::ALL {
            let found: Vec<&Field> = all.iter().filter(|f| f.id == id).collect();
            assert_eq!(found.len(), 1, "{id:?} appears {} times", found.len());
            let field = found[0];
            let value = get(&Settings::default(), id);
            match (&field.control, value) {
                (Control::Slider { .. }, Value::Num(_)) => {}
                (Control::Choice(_), Value::Index(_)) => {}
                (Control::Toggle, Value::Bool(_)) => {}
                (c, v) => panic!("{id:?}: control {c:?} does not match value {v:?}"),
            }
            if let (Control::Choice(options), Value::Index(i)) = (&field.control, value) {
                assert!(
                    i < options.len(),
                    "{id:?}: default index {i} is outside {} options",
                    options.len()
                );
                assert!(options.len() >= 2, "{id:?}: a radio group of one");
            }
        }
    }

    /// Set a non-default value on every field, read it back, and check nothing else moved.
    ///
    /// This is what catches a copy-paste between the two big matches, which is the likeliest
    /// bug in the module: `FieldId::LineSpacing => Value::Num(s.read.line_height)` written twice
    /// with one field name wrong compiles perfectly and is invisible until a slider moves the
    /// wrong text.
    #[test]
    fn every_field_round_trips_and_touches_nothing_else() {
        for id in FieldId::ALL {
            let mut s = Settings::default();
            let before = get(&s, id);
            let wanted = match before {
                Value::Bool(b) => Value::Bool(!b),
                Value::Index(i) => Value::Index(if i == 0 { 1 } else { 0 }),
                Value::Num(_) => {
                    let f = fields();
                    let field = f.iter().find(|f| f.id == id).unwrap();
                    let Control::Slider { min, max, .. } = field.control else {
                        unreachable!("a Num value on a non-slider is caught by another test")
                    };
                    // A value inside the band and away from the default, whichever end has room.
                    let n = before.num().unwrap();
                    Value::Num(if (max - n) > (n - min) { max } else { min })
                }
            };
            assert_ne!(wanted, before, "{id:?}: chose the value it already had");

            set(&mut s, id, wanted);
            assert_eq!(get(&s, id), wanted, "{id:?} did not round-trip");

            // Every *other* field is untouched.
            let d = Settings::default();
            for other in FieldId::ALL.iter().filter(|o| **o != id) {
                assert_eq!(
                    get(&s, *other),
                    get(&d, *other),
                    "setting {id:?} also moved {other:?}"
                );
            }
        }
    }

    /// A number outside the band is clamped, not stored. The slider's bounds are advice to a
    /// widget; this is the door to the struct, and `settings.json` is hand-edited.
    #[test]
    fn a_number_outside_its_band_is_clamped() {
        let mut s = Settings::default();
        set(&mut s, FieldId::LineWidth, Value::Num(10_000.0));
        assert_eq!(s.read.measure_chars, ReadOpts::MEASURE_MAX);
        set(&mut s, FieldId::LineWidth, Value::Num(-5.0));
        assert_eq!(s.read.measure_chars, ReadOpts::MEASURE_MIN);
        set(&mut s, FieldId::StopIndentingAfter, Value::Num(9_000.0));
        assert_eq!(s.read.max_thread_indent, ReadOpts::MAX_INDENT_MAX);
    }

    /// A `Value` of the wrong shape leaves the setting alone rather than panicking.
    #[test]
    fn a_mismatched_value_is_dropped() {
        let mut s = Settings::default();
        set(&mut s, FieldId::LineWidth, Value::Bool(true));
        set(&mut s, FieldId::ShowMenuBar, Value::Num(3.0));
        set(&mut s, FieldId::Theme, Value::Bool(false));
        set(&mut s, FieldId::Theme, Value::Index(99));
        assert_eq!(s, Settings::default());
    }

    /// A group resets to defaults and leaves every other group standing.
    #[test]
    fn resetting_one_group_leaves_the_others_alone() {
        for group in Group::ALL {
            let mut s = Settings::default();
            // Move every field in the file away from its default.
            for id in FieldId::ALL {
                let v = match get(&s, id) {
                    Value::Bool(b) => Value::Bool(!b),
                    Value::Index(i) => Value::Index(if i == 0 { 1 } else { 0 }),
                    Value::Num(n) => Value::Num(n + 1.0),
                };
                set(&mut s, id, v);
            }
            let moved = s.clone();
            reset_group(&mut s, group);

            let d = Settings::default();
            for f in fields() {
                if f.group == group {
                    assert_eq!(get(&s, f.id), get(&d, f.id), "{:?} was not reset", f.id);
                } else {
                    assert_eq!(get(&s, f.id), get(&moved, f.id), "{:?} was reset", f.id);
                }
            }
        }
    }

    /// `reset_all` is `Settings::default()`, reached the long way. If these ever disagree, some
    /// field is not in `fields()` and `every_setting_is_exposed` has a hole.
    #[test]
    fn reset_all_is_the_default() {
        let mut s = Settings::default();
        s.read.measure_chars = 91.0;
        s.read.theme = Theme::ContrastDark;
        s.read.images = ImagePolicy::NoRequests;
        s.zoom_factor = 2.5;
        s.show_menu_bar = false;
        s.send_image_referer = false;
        reset_all(&mut s);
        assert_eq!(s, Settings::default());
    }

    /// Rows come out grouped and in reading order, so `prefs_ui` draws a heading on every change
    /// of group in one pass and never sorts. The same contract `pageinfo::rows` has.
    #[test]
    fn fields_are_grouped_in_reading_order() {
        let mut last = 0usize;
        for f in fields() {
            let at = Group::ALL.iter().position(|g| *g == f.group).unwrap();
            assert!(at >= last, "{} is out of group order", f.label);
            last = at;
        }
        for g in Group::ALL {
            assert!(fields().iter().any(|f| f.group == g), "{g:?} has no fields");
            assert!(!g.title().is_empty());
        }
    }

    /// Prose hygiene. No test can check that a sentence is *true* — each note cites the code it
    /// was read from in the module doc for that — but a label with no sentence behind it, or a
    /// fragment where a sentence belongs, is mechanical.
    #[test]
    fn every_field_explains_itself() {
        // The two whose label is the whole explanation, and whose options carry the detail
        // instead. Everything else owes a sentence, and the list is kept short so that adding
        // to it is a decision rather than a shortcut.
        const SELF_EVIDENT: [FieldId; 2] = [FieldId::Theme, FieldId::Images];
        for f in fields() {
            assert!(!f.label.is_empty(), "{:?} has no label", f.id);
            assert!(
                f.label.chars().next().is_some_and(char::is_uppercase),
                "{:?}: label does not start with a capital",
                f.id
            );
            assert!(
                !f.label.ends_with('.'),
                "{:?}: a label is not a sentence",
                f.id
            );
            match f.note {
                Some(note) => {
                    assert!(
                        note.ends_with('.'),
                        "{:?}: note is not a whole sentence: {note:?}",
                        f.id
                    );
                    assert!(
                        note.chars().next().is_some_and(char::is_uppercase),
                        "{:?}: note does not start with a capital",
                        f.id
                    );
                }
                None => assert!(
                    SELF_EVIDENT.contains(&f.id),
                    "{:?} has no note and is not on the self-evident list",
                    f.id
                ),
            }
            // A group with no note is fine; one with an empty string is a mistake.
            if let Control::Choice(options) = f.control {
                for o in options {
                    assert!(!o.label.is_empty(), "{:?}: an unlabelled option", f.id);
                    if let Some(n) = o.note {
                        assert!(
                            n.ends_with('.'),
                            "{:?}: option note {n:?} is a fragment",
                            f.id
                        );
                    }
                }
            }
        }
        for g in Group::ALL {
            if let Some(n) = g.note() {
                assert!(n.ends_with('.'), "{g:?}: group note is a fragment");
            }
        }
    }

    /// No user-facing string leaks this codebase's own vocabulary.
    ///
    /// Every one of these fields is *named* after the thing it sets, and those names are the
    /// crate's words rather than a reader's: someone opening hww for the first time has not read
    /// `opts.rs` and does not know what a "measure" is, what "chrome" means, or that `ReadOpts`
    /// exists. In the spirit of `notice::every_remark_names_its_actor`, which polices that
    /// module's prose the same way.
    #[test]
    fn no_label_leaks_an_internal_name() {
        const BANNED: [&str; 12] = [
            "chrome",
            "ReadOpts",
            "measure_chars",
            "base_size_pt",
            "paragraph_spacing",
            "indent_per_depth",
            "max_thread_indent",
            "scroll_lines",
            "show_link_urls",
            "zoom_factor",
            "send_image_referer",
            "ImagePolicy",
        ];
        let mut strings: Vec<String> = Vec::new();
        for f in fields() {
            strings.push(f.label.to_owned());
            if let Some(n) = f.note {
                strings.push(n.to_owned());
            }
            if let Control::Choice(options) = f.control {
                for o in options {
                    strings.push(o.label.to_owned());
                    if let Some(n) = o.note {
                        strings.push(n.to_owned());
                    }
                }
            }
        }
        for g in Group::ALL {
            strings.push(g.title().to_owned());
            if let Some(n) = g.note() {
                strings.push(n.to_owned());
            }
        }
        for s in &strings {
            let lower = s.to_lowercase();
            for banned in BANNED {
                assert!(
                    !lower.contains(&banned.to_lowercase()),
                    "{banned:?} leaks into a reader-facing string: {s:?}"
                );
            }
        }
    }

    /// No reader-facing string needs a glyph the shipped faces lack.
    ///
    /// The same rule the menu's marker follows, applied here because a note is prose and prose
    /// does not fail a build. `ui::fonts` embeds twelve faces and none of them is a symbol font:
    /// the arrows are covered, and the enclosed and technical blocks are not.
    #[test]
    fn no_reader_facing_string_needs_a_missing_glyph() {
        const MISSING: [char; 6] = [
            '\u{23F5}', // egui's own submenu arrow
            '\u{25B8}', '\u{25B6}', // triangles only DejaVuSerif carries
            '\u{2630}', // hamburger
            '\u{2699}', // gear
            '\u{24D8}', // circled i
        ];
        for f in fields() {
            let mut all = vec![f.label.to_owned()];
            all.extend(f.note.map(str::to_owned));
            if let Control::Choice(options) = f.control {
                for o in options {
                    all.push(o.label.to_owned());
                    all.extend(o.note.map(str::to_owned));
                }
            }
            for s in all {
                for c in MISSING {
                    assert!(
                        !s.contains(c),
                        "{:?}: U+{:04X} is in none of the shipped faces: {s:?}",
                        f.id,
                        c as u32
                    );
                }
            }
        }
    }
}
