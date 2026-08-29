//! Reading style: the GUI's `render::TextOpts`.
//!
//! **This is the only legal home for width, colour, font, and spacing.** The IR carries no
//! styling and never will; if you find yourself wanting a `color`, `width`, `align`, or
//! `font` field in `src/ir.rs`, it belongs here instead. `render::TextOpts` is the same idea
//! for the text renderer, and the two are deliberately separate: a reading column measured in
//! characters and a terminal wrapped at 78 columns are different presentation decisions about
//! the same document.
//!
//! No egui types appear in this file, so the default CI job, which never compiles egui,
//! still type-checks and tests it.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FontChoice {
    /// IBM Plex Sans. Variable, so its bold is the same file at `wght` 700.
    Sans,
    /// IBM Plex Serif.
    ///
    /// This variant used to be absent on the grounds that a setting with no face behind it is
    /// worse than no setting, the same reason there is still no `justify` field. The face
    /// arrived, so the variant did: see `reader::ui::fonts`.
    Serif,
    /// IBM Plex Mono. A whole page in it is a real preference, not a joke.
    ///
    /// The one role with no italic face shipped, so emphasis here is epaint's skew. See
    /// [`crate::reader::face::Face::synthesises_italics`].
    Mono,
    /// Atkinson Hyperlegible Next, drawn for low vision: the letterforms that ordinarily
    /// collapse into each other are pulled apart, so `I l 1`, `O 0`, and `b d p q` stay
    /// distinguishable at a glance.
    ///
    /// A reading preference like the other three and not an accessibility mode: nothing else
    /// changes with it, and the contrast floors the themes already hold are what carry that
    /// side. Its Latin coverage is narrower than Plex's, so more of a non-Latin page falls
    /// through to the tail here than under the other roles; see `reader::ui::fonts`.
    Hyperlegible,
}

impl FontChoice {
    /// Every role. `face::Face::all` crosses this with the inline styles, and
    /// `every_role_is_listed` is what makes a forgotten variant a failure rather than a face
    /// nothing ever registers.
    pub const ALL: [FontChoice; 4] = [
        FontChoice::Sans,
        FontChoice::Serif,
        FontChoice::Mono,
        FontChoice::Hyperlegible,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    Light,
    Dark,
    /// Warm paper. Not a tint of `Light`: the whole point is a lower-contrast ground.
    Sepia,
    /// Follow the desktop.
    System,
    /// Reached from `settings.json`, not from `d`. See [`Theme::next`].
    ContrastLight,
    ContrastDark,
}

impl Theme {
    /// Every theme with colours of its own.
    ///
    /// `System` is a redirect, not a palette. The contrast tests walk this rather than a
    /// hardcoded list, because a palette added without an entry here is a palette nothing
    /// checks, and the way that fails is a theme whose own warnings are unreadable.
    /// `real_lists_every_palette` is what makes a missing entry a compile error rather than a
    /// silence.
    pub const REAL: [Theme; 5] = [
        Theme::Light,
        Theme::Sepia,
        Theme::Dark,
        Theme::ContrastLight,
        Theme::ContrastDark,
    ];

    /// The contrast this theme's palette holds itself to: AA, or AAA for the contrast pair.
    ///
    /// Not a `Palette` field. It is not a colour, and a palette carries nothing else;
    /// `dark_mode` is already the one exception and a second would make it a pattern.
    pub fn floor(self) -> f32 {
        match self {
            Theme::ContrastLight | Theme::ContrastDark => 7.0,
            _ => 4.5,
        }
    }

    /// `d` cycles; `System` is deliberately in the cycle so it can be got back to without
    /// opening a settings file.
    ///
    /// The contrast palettes are **not** in the cycle. Someone who wants high contrast wants it
    /// permanently, and someone pressing `d` for reading comfort should not pass through two
    /// high-contrast screens to get back. They still need an arm, because this match is
    /// exhaustive and because a self-loop is a key that silently does nothing; both drop into
    /// the cycle at `System`.
    pub fn next(self) -> Self {
        match self {
            Theme::System => Theme::Light,
            Theme::Light => Theme::Sepia,
            Theme::Sepia => Theme::Dark,
            Theme::Dark => Theme::System,
            Theme::ContrastLight | Theme::ContrastDark => Theme::System,
        }
    }
}

/// A `theme` this binary does not know falls back to `System` instead of failing the whole file.
///
/// `ReadOpts` is `#[serde(default)]`, which covers a *missing* field and not an unparseable one.
/// Without this, an older binary reading a `settings.json` containing `"ContrastDark"` loses
/// every other setting in the file, and then writes the defaults back over it on the first `d`,
/// `[`, or `]`. That is a data-loss path with a keystroke as its trigger, for the price of one
/// type.
fn theme_or_system<'de, D>(d: D) -> Result<Theme, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Wire {
        Known(Theme),
        Unknown(serde::de::IgnoredAny),
    }
    Ok(match Wire::deserialize(d)? {
        Wire::Known(t) => t,
        Wire::Unknown(_) => Theme::System,
    })
}

/// A `family` this binary does not know falls back to the default, for exactly the reason
/// [`theme_or_system`] exists.
///
/// `FontChoice` gained [`FontChoice::Hyperlegible`] after shipping, which is what turns this
/// from a hypothetical into a live path: a binary predating that variant meets `"Hyperlegible"`
/// in a file written by this one, and without this it loses every other setting in the file on
/// the next write.
fn family_or_default<'de, D>(d: D) -> Result<FontChoice, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Wire {
        Known(FontChoice),
        Unknown(serde::de::IgnoredAny),
    }
    Ok(match Wire::deserialize(d)? {
        Wire::Known(f) => f,
        Wire::Unknown(_) => ReadOpts::default().family,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImagePolicy {
    /// Show a one-line strip naming the host. Loads only on an explicit click or `i`.
    Placeholder,
    /// Load the pictures near the reading position without being asked, a screenful ahead.
    ///
    /// The one policy that contacts an image host with no click behind it, which is why it is
    /// opt-in and why [`ImagePolicy::Placeholder`] remains the default. It is bounded by the
    /// layout band rather than by the document: a gallery of two hundred pictures requests the
    /// handful near the window and the rest only as they are scrolled towards, so the request
    /// queue never grows past what is about to be looked at. Everything downstream is
    /// unchanged — the same choke point, the same counters in the page-info panel — so what
    /// this setting moves is who presses the button, not what the reader admits to fetching.
    ///
    /// It says nothing as it goes: a reader who chose "load automatically" asked not to be
    /// interrupted per host, and the page-info panel still names every host contacted. The
    /// remark belongs to the policies where a control names a host and the reader clicks it.
    Auto,
    /// Never offer to load an article image. The strip still says an image was there;
    /// silently dropping content is how a reader lies about a page.
    ///
    /// This does **not** stop the page favicon, which `ui::app::ensure_favicon` fetches as
    /// chrome identity. That was true before this variant had a neighbour and it is kept true
    /// deliberately: renaming or re-scoping `Never` would change what existing `settings.json`
    /// files mean, and the failure mode is silent. A reader who wants no image request at all
    /// wants [`ImagePolicy::NoRequests`].
    Never,
    /// No image request at all, the favicon included.
    ///
    /// The distinction `Never` cannot make: one third-party request per page still leaves, and
    /// it carries the origin `Referer` when that is on, so "no article images" and "nothing
    /// contacted" are two different choices and now say so.
    NoRequests,
}

impl ImagePolicy {
    /// Every policy, most permissive first, which is also the order the panel lists them in.
    /// For the tests that must not be allowed to miss one.
    pub const ALL: [ImagePolicy; 4] = [
        ImagePolicy::Auto,
        ImagePolicy::Placeholder,
        ImagePolicy::Never,
        ImagePolicy::NoRequests,
    ];

    /// Whether a placeholder may offer to load the picture it marks.
    ///
    /// The call sites used to test `== ImagePolicy::Never`, which is why this exists: an
    /// equality check against one variant compiles unchanged when a third arrives and silently
    /// treats it as [`ImagePolicy::Placeholder`], which is the most permissive answer. A
    /// `matches!` here puts the decision in one tested file instead of four untested ones.
    ///
    /// [`ImagePolicy::Auto`] answers yes: a picture it has not reached yet, or one whose fetch
    /// failed, is still clickable, and `i` and `I` still work over the whole page.
    pub fn offers_loading(self) -> bool {
        matches!(self, ImagePolicy::Placeholder | ImagePolicy::Auto)
    }

    /// Whether hww may fetch an article image with no click behind it.
    ///
    /// A separate question from [`Self::offers_loading`] rather than a comparison against
    /// `Auto` at the call site, for the reason the paragraph above gives: a later policy that
    /// offers loading without automating it must not inherit this answer by being unequal to
    /// something.
    pub fn loads_automatically(self) -> bool {
        matches!(self, ImagePolicy::Auto)
    }

    /// Whether hww may request any image at all, the page favicon included.
    pub fn allows_any_request(self) -> bool {
        !matches!(self, ImagePolicy::NoRequests)
    }
}

/// An `images` value this binary does not know falls back to the default, for exactly the
/// reason [`theme_or_system`] exists.
///
/// `ReadOpts` is `#[serde(default)]`, which covers a *missing* field and not an unparseable
/// one. Without this, a binary predating [`ImagePolicy::NoRequests`] reading a file that names
/// it loses every other setting in the file, and then writes the defaults back over it on the
/// first `d`, `[`, or `]`. Adding a variant to a serialized enum is what makes that a live path
/// rather than a hypothetical one.
fn images_or_default<'de, D>(d: D) -> Result<ImagePolicy, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Wire {
        Known(ImagePolicy),
        Unknown(serde::de::IgnoredAny),
    }
    Ok(match Wire::deserialize(d)? {
        Wire::Known(p) => p,
        Wire::Unknown(_) => ReadOpts::default().images,
    })
}

/// Presentation settings for the reading column.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReadOpts {
    /// Reading measure in characters, the analogue of `render::TextOpts::width`. Typography's
    /// answer is 66; 66–78 is the usable band.
    pub measure_chars: f32,
    /// A typographic baseline, set once. **Not** a hotkey: live resizing is egui's
    /// `zoom_factor`, which already owns `Ctrl`+`+`/`-`/`0`, `Ctrl`+wheel, and pinch.
    pub base_size_pt: f32,
    /// Multiple of the font size.
    pub line_height: f32,
    /// Multiple of the font size, between blocks.
    pub paragraph_spacing: f32,
    #[serde(deserialize_with = "family_or_default")]
    pub family: FontChoice,
    #[serde(deserialize_with = "theme_or_system")]
    pub theme: Theme,
    /// Show a link's target inline, the way `render::TextOpts::show_links` does.
    pub show_link_urls: bool,
    #[serde(deserialize_with = "images_or_default")]
    pub images: ImagePolicy,
    /// How much of a feed entry's summary is drawn, in **bytes** of visible text; 0 draws all
    /// of it. A floor rather than a ceiling — see `reader::budget`, which owns the cut and is
    /// what the image walk reads too.
    ///
    /// Bytes and not characters, because `ir::inlines_text_len` sums `s.len()`: written as
    /// characters this would cut a CJK or Cyrillic feed at roughly a third of its visible
    /// length, and text length in this crate has one currency.
    pub entry_summary_bytes: u16,
    /// Points of indent per reply level in a thread.
    pub indent_per_depth: f32,
    /// Levels of indent before nesting stops accumulating. Without a cap a 40-deep thread
    /// renders two pixels wide.
    pub max_thread_indent: u16,
}

impl Default for ReadOpts {
    fn default() -> Self {
        Self {
            measure_chars: 70.0,
            base_size_pt: 17.0,
            line_height: 1.35,
            paragraph_spacing: 0.85,
            family: FontChoice::Sans,
            theme: Theme::System,
            show_link_urls: false,
            images: ImagePolicy::Placeholder,
            entry_summary_bytes: Self::SUMMARY_DEFAULT,
            indent_per_depth: 18.0,
            max_thread_indent: 8,
        }
    }
}

impl ReadOpts {
    pub const MEASURE_MIN: f32 = 45.0;
    pub const MEASURE_MAX: f32 = 110.0;

    // The bounds below exist because a slider needs two ends and the panel is not the place to
    // decide them: a literal in `ui/` is a number no test can reach, and every one of these is
    // a typographic judgement rather than an implementation detail. They live beside the fields
    // they bound, next to the defaults they have to contain.

    /// Body size in points. Ten is the floor at which `chrome_font`'s own 11 pt minimum takes
    /// over, so below it the reader's labels stop tracking the page; twenty-eight is already
    /// large-print, and `zoom_factor` is the right tool past it.
    pub const SIZE_MIN: f32 = 10.0;
    pub const SIZE_MAX: f32 = 28.0;

    /// Multiples of the font size. Under 1.0 the lines collide; past 2.2 the paragraph stops
    /// reading as one block.
    pub const LINE_HEIGHT_MIN: f32 = 1.0;
    pub const LINE_HEIGHT_MAX: f32 = 2.2;

    /// Zero is a real choice here, not a broken one: it sets paragraphs solid, the way a
    /// printed novel does, and the indent is not this reader's to add.
    pub const PARAGRAPH_SPACING_MIN: f32 = 0.0;
    pub const PARAGRAPH_SPACING_MAX: f32 = 2.5;

    /// Points per reply level. Zero flattens a discussion to a list, which is legible and is
    /// somebody's preference; past 48 a third-level reply has lost half the window.
    pub const INDENT_MIN: f32 = 0.0;
    pub const INDENT_MAX: f32 = 48.0;

    /// Levels before the indent stops accumulating. Zero means never indent at all, which is
    /// the same page `INDENT_MIN` gives and is reachable from either end on purpose.
    pub const MAX_INDENT_MIN: u16 = 0;
    pub const MAX_INDENT_MAX: u16 = 24;

    /// Bytes of an entry's summary. Zero is the whole entry, and it is the low end rather than
    /// a switch beside the slider because it is the same quantity asked for without a limit —
    /// the reader who wants every word and the reader who wants two paragraphs are turning one
    /// knob. Four thousand is already several screens per row, past which a list of entries has
    /// stopped being a list.
    pub const SUMMARY_MIN: f32 = 0.0;
    pub const SUMMARY_MAX: f32 = 4_000.0;

    /// Measured 2026-08-28 across six feeds: 1,000 bytes leaves a headlines-only feed's entries
    /// whole and turns a full-text feed into a lead-in. Without a budget one of those feeds was
    /// a single 271,965-character block, and one of its items 127,328 on its own.
    pub const SUMMARY_DEFAULT: u16 = 1_000;

    /// `[` and `]`. hww's own reading knob, the one a browser does not have.
    pub fn widen(&mut self) {
        self.measure_chars = (self.measure_chars + 4.0).min(Self::MEASURE_MAX);
    }

    pub fn narrow(&mut self) {
        self.measure_chars = (self.measure_chars - 4.0).max(Self::MEASURE_MIN);
    }

    /// Indent for a reply at `depth`, in points.
    pub fn indent_for(&self, depth: u16) -> f32 {
        f32::from(depth.min(self.max_thread_indent)) * self.indent_per_depth
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measure_stays_inside_the_readable_band() {
        let mut o = ReadOpts::default();
        for _ in 0..50 {
            o.widen();
        }
        assert_eq!(o.measure_chars, ReadOpts::MEASURE_MAX);
        for _ in 0..50 {
            o.narrow();
        }
        assert_eq!(o.measure_chars, ReadOpts::MEASURE_MIN);
    }

    #[test]
    fn thread_indent_is_capped_so_deep_replies_stay_readable() {
        let o = ReadOpts::default();
        assert_eq!(o.indent_for(3), 54.0);
        // A 40-deep thread must not render two pixels wide.
        assert_eq!(o.indent_for(40), o.indent_for(o.max_thread_indent));
    }

    #[test]
    fn theme_cycle_returns_to_system() {
        let mut t = Theme::System;
        for _ in 0..4 {
            t = t.next();
        }
        assert_eq!(t, Theme::System);
    }

    /// `d` from a contrast palette lands back in the four-cycle rather than doing nothing.
    ///
    /// The failure this catches is a self-loop: an arm added only to satisfy the exhaustive
    /// match, which compiles, ships, and makes the key look broken.
    #[test]
    fn a_contrast_palette_is_not_a_dead_end() {
        let cycle = [Theme::System, Theme::Light, Theme::Sepia, Theme::Dark];
        for t in [Theme::ContrastLight, Theme::ContrastDark] {
            assert!(cycle.contains(&t.next()), "{t:?} does not rejoin the cycle");
        }
    }

    /// [`Theme::REAL`] lists every theme that is not the redirect.
    ///
    /// The `match` is exhaustive on purpose and is the whole point of the test: a variant added
    /// to `Theme` fails to compile *here*, which is the prompt to classify it and put it in
    /// `REAL`. Without that, a palette can be added to `theme::palette`'s own exhaustive match
    /// and to nothing else, and it escapes every contrast test in the crate silently. Asserting
    /// `REAL.len()` cannot catch it: the length is still whatever it was.
    #[test]
    fn real_lists_every_palette() {
        for t in [
            Theme::System,
            Theme::Light,
            Theme::Sepia,
            Theme::Dark,
            Theme::ContrastLight,
            Theme::ContrastDark,
        ] {
            let is_redirect = match t {
                Theme::System => true,
                Theme::Light
                | Theme::Sepia
                | Theme::Dark
                | Theme::ContrastLight
                | Theme::ContrastDark => false,
            };
            assert_eq!(Theme::REAL.contains(&t), !is_redirect, "{t:?}");
        }
    }

    /// Every real theme names a floor, and the contrast pair asks for more than AA.
    #[test]
    fn the_contrast_pair_holds_itself_higher() {
        for t in Theme::REAL {
            assert!(t.floor() >= 4.5);
        }
        assert!(Theme::ContrastLight.floor() > Theme::Light.floor());
        assert!(Theme::ContrastDark.floor() > Theme::Dark.floor());
    }

    /// A theme this binary has never heard of costs the theme, not the file.
    #[test]
    fn an_unknown_theme_does_not_take_the_rest_of_the_settings_with_it() {
        let o: ReadOpts =
            serde_json::from_str(r#"{"theme": "Aubergine", "measure_chars": 80.0}"#).unwrap();
        assert_eq!(o.theme, Theme::System);
        assert_eq!(o.measure_chars, 80.0);
    }

    /// A `family` this binary has never heard of costs the typeface, not the file. The mirror
    /// of the two tests above, live from the moment `FontChoice` grew a fourth variant.
    #[test]
    fn an_unknown_family_does_not_take_the_rest_of_the_settings_with_it() {
        let o: ReadOpts =
            serde_json::from_str(r#"{"family": "Blackletter", "measure_chars": 80.0}"#).unwrap();
        assert_eq!(o.family, ReadOpts::default().family);
        assert_eq!(o.measure_chars, 80.0);
    }

    /// And every role this binary does know still round-trips.
    #[test]
    fn every_family_round_trips_through_json() {
        for f in FontChoice::ALL {
            let o = ReadOpts {
                family: f,
                ..Default::default()
            };
            let back: ReadOpts = serde_json::from_str(&serde_json::to_string(&o).unwrap()).unwrap();
            assert_eq!(back.family, f);
        }
    }

    /// `ALL` lists every variant, once each.
    ///
    /// The literal list is the test and the exhaustive `match` is what keeps it honest: a
    /// variant added to `FontChoice` fails to compile *here*, which is the prompt to write it
    /// into both. Walking `ALL` instead cannot catch the failure this exists for — a role
    /// missing from `ALL` is never visited, and the match arm that would have found it is
    /// discharged by adding an arm rather than by adding the entry. Same shape as
    /// `real_lists_every_palette` below.
    #[test]
    fn every_role_is_listed() {
        for f in [
            FontChoice::Sans,
            FontChoice::Serif,
            FontChoice::Mono,
            FontChoice::Hyperlegible,
        ] {
            match f {
                FontChoice::Sans
                | FontChoice::Serif
                | FontChoice::Mono
                | FontChoice::Hyperlegible => {}
            }
            // Counted, not `contains`: `ALL` is walked to register faces and to build the
            // settings row, so a variant listed twice is a duplicate face and a duplicate row.
            // `dedup` cannot say so, because it only sees neighbours.
            assert_eq!(
                FontChoice::ALL.iter().filter(|&&x| x == f).count(),
                1,
                "{f:?} is missing from ALL, or listed twice"
            );
        }
    }

    /// And a theme it does know still round-trips, including the two new ones.
    #[test]
    fn every_real_theme_round_trips_through_json() {
        for t in Theme::REAL {
            let o = ReadOpts {
                theme: t,
                ..Default::default()
            };
            let back: ReadOpts = serde_json::from_str(&serde_json::to_string(&o).unwrap()).unwrap();
            assert_eq!(back.theme, t);
        }
    }

    /// All three questions answered for every policy, so a fifth option cannot inherit an
    /// answer.
    ///
    /// The table is written out rather than derived: deriving it from the same `matches!` the
    /// implementation uses would assert only that the function equals itself. What is pinned is
    /// the *intent* — which policy loads article images, which one does so unasked, and which
    /// one lets any request out at all — and `ALL` is what makes a forgotten variant a failure
    /// here.
    #[test]
    fn every_policy_answers_all_three_questions() {
        let expected = [
            (ImagePolicy::Auto, true, true, true),
            (ImagePolicy::Placeholder, true, false, true),
            (ImagePolicy::Never, false, false, true),
            (ImagePolicy::NoRequests, false, false, false),
        ];
        assert_eq!(expected.len(), ImagePolicy::ALL.len());
        for (policy, offers, automatic, requests) in expected {
            assert!(
                ImagePolicy::ALL.contains(&policy),
                "{policy:?} is not in ALL"
            );
            assert_eq!(policy.offers_loading(), offers, "{policy:?} offers_loading");
            assert_eq!(
                policy.loads_automatically(),
                automatic,
                "{policy:?} loads_automatically"
            );
            assert_eq!(
                policy.allows_any_request(),
                requests,
                "{policy:?} allows_any_request"
            );
        }
    }

    /// The one policy that fetches unasked is the one policy that says so. Everything else
    /// waits for a click, the default included.
    #[test]
    fn only_auto_loads_without_being_asked() {
        let automatic: Vec<_> = ImagePolicy::ALL
            .into_iter()
            .filter(|p| p.loads_automatically())
            .collect();
        assert_eq!(automatic, vec![ImagePolicy::Auto]);
        assert!(!ReadOpts::default().images.loads_automatically());
    }

    /// The favicon is the whole reason `NoRequests` exists: it is the one image request that
    /// `Never` does not stop, so the two policies must not agree about it.
    #[test]
    fn never_still_allows_the_favicon_and_no_requests_does_not() {
        assert!(ImagePolicy::Never.allows_any_request());
        assert!(!ImagePolicy::NoRequests.allows_any_request());
        // And neither offers to load an article image, which is what they do share.
        assert!(!ImagePolicy::Never.offers_loading());
        assert!(!ImagePolicy::NoRequests.offers_loading());
        // `Auto` is on the far side of both: it fetches the favicon and the article images.
        assert!(ImagePolicy::Auto.allows_any_request());
        assert!(ImagePolicy::Auto.offers_loading());
    }

    /// An `images` value this binary has never heard of costs the policy, not the file.
    ///
    /// The mirror of `an_unknown_theme_does_not_take_the_rest_of_the_settings_with_it`, and it
    /// became load-bearing the moment `ImagePolicy` grew a third variant: an older binary will
    /// meet `"NoRequests"`, and now `"Auto"`, in a file written by this one — and the fallback
    /// direction is the safe one, since a binary that cannot parse `"Auto"` cannot honour it
    /// either and drops to asking first.
    #[test]
    fn an_unknown_image_policy_does_not_take_the_rest_of_the_settings_with_it() {
        let o: ReadOpts =
            serde_json::from_str(r#"{"images": "Telepathy", "measure_chars": 80.0}"#).unwrap();
        assert_eq!(o.images, ReadOpts::default().images);
        assert_eq!(o.measure_chars, 80.0);
    }

    /// And every policy this binary does know still round-trips.
    #[test]
    fn every_image_policy_round_trips_through_json() {
        for p in ImagePolicy::ALL {
            let o = ReadOpts {
                images: p,
                ..Default::default()
            };
            let back: ReadOpts = serde_json::from_str(&serde_json::to_string(&o).unwrap()).unwrap();
            assert_eq!(back.images, p);
        }
    }

    /// Every bound contains the default it bounds. A slider whose range excludes the value it
    /// opens on snaps the setting the moment the panel is drawn, which reads as the panel
    /// changing a setting nobody touched.
    #[test]
    fn every_range_contains_its_default() {
        let d = ReadOpts::default();
        let bands = [
            (
                "measure_chars",
                d.measure_chars,
                ReadOpts::MEASURE_MIN,
                ReadOpts::MEASURE_MAX,
            ),
            (
                "base_size_pt",
                d.base_size_pt,
                ReadOpts::SIZE_MIN,
                ReadOpts::SIZE_MAX,
            ),
            (
                "line_height",
                d.line_height,
                ReadOpts::LINE_HEIGHT_MIN,
                ReadOpts::LINE_HEIGHT_MAX,
            ),
            (
                "paragraph_spacing",
                d.paragraph_spacing,
                ReadOpts::PARAGRAPH_SPACING_MIN,
                ReadOpts::PARAGRAPH_SPACING_MAX,
            ),
            (
                "indent_per_depth",
                d.indent_per_depth,
                ReadOpts::INDENT_MIN,
                ReadOpts::INDENT_MAX,
            ),
        ];
        for (name, value, min, max) in bands {
            assert!(min < max, "{name}: empty band");
            assert!(
                (min..=max).contains(&value),
                "{name}: default {value} is outside {min}..={max}"
            );
        }
        assert!(
            (ReadOpts::MAX_INDENT_MIN..=ReadOpts::MAX_INDENT_MAX).contains(&d.max_thread_indent)
        );
    }

    /// Settings round-trip through the on-disk JSON, and an older file missing a field still
    /// loads: the reason for `#[serde(default)]`.
    #[test]
    fn partial_settings_json_fills_in_defaults() {
        let o: ReadOpts = serde_json::from_str(r#"{"measure_chars": 80.0}"#).unwrap();
        assert_eq!(o.measure_chars, 80.0);
        assert_eq!(o.base_size_pt, ReadOpts::default().base_size_pt);
    }
}
