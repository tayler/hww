//! The embedded faces. The only file in the crate that contains font bytes.
//!
//! `eframe` is built **without** `default_fonts`, so nothing here is inherited: every face the
//! reader draws with is named below, and the four faces epaint would otherwise embed
//! (Ubuntu-Light, Hack, NotoEmoji, emoji-icon-font) are absent from the binary. Same posture as
//! `reqwest`'s `default-features = false` one directory over: a capability that is not compiled
//! in cannot be reached by accident, and a face nobody chose cannot appear on a page.
//!
//! **Three roles, four styles each, one tail.** IBM Plex covers sans, serif, and mono from one
//! licence and one design. `DejaVuSerif` sits behind all three and is the only place coverage
//! for a script Plex does not carry comes from: polytonic Greek, Armenian, Georgian, `∀`, `∈`.
//! `font_data` is keyed by name and `families` are lists of names, so the tail's 380K appear
//! once no matter how many chains reference it.
//!
//! **The tail is `DejaVuSerif` rather than `DejaVuSans`, and the reason is bidi.** epaint shapes
//! with `harfrust`, so joining and marks work, but it has no bidi (`epaint::text::font`,
//! `TODO(emilk): heed bidi characters`), which means a Hebrew or Arabic run renders in logical
//! order: backwards. `DejaVuSans` has those glyphs and would turn a visible failure into a
//! confident, plausible, wrong one, which is the substituted-hotlink-image argument in AGENTS.md
//! wearing different clothes. The serif cut carries everything above and no RTL script at all;
//! the seven codepoints it maps in Hebrew Presentation Forms are the Latin `ﬀ ﬁ ﬂ` ligatures.
//! Do not swap it for the sans cut, and do not add an RTL face until epaint reorders runs.
//!
//! **Bold is a face and italic is a face.** Plex Sans is variable, so its bold is the same bytes
//! at `wght` 700 rather than a second file; Plex Serif and Plex Mono are static, so each weight
//! is its own file. `wdth` is pinned at 100 explicitly rather than left to the face's default,
//! so a future change to how skrifa resolves an unset axis cannot quietly narrow the page.

use crate::reader::face::Face;
use eframe::egui::{self, FontData, FontFamily, FontTweak, epaint::text::VariationCoords};
use std::sync::Arc;

const PLEX_SANS: &[u8] = include_bytes!("../../../fonts/IBMPlexSans-Variable.ttf");
const PLEX_SANS_ITALIC: &[u8] = include_bytes!("../../../fonts/IBMPlexSans-Italic-Variable.ttf");
const PLEX_SERIF: &[u8] = include_bytes!("../../../fonts/IBMPlexSerif-Regular.ttf");
const PLEX_SERIF_BOLD: &[u8] = include_bytes!("../../../fonts/IBMPlexSerif-Bold.ttf");
const PLEX_SERIF_ITALIC: &[u8] = include_bytes!("../../../fonts/IBMPlexSerif-Italic.ttf");
const PLEX_SERIF_BOLD_ITALIC: &[u8] = include_bytes!("../../../fonts/IBMPlexSerif-BoldItalic.ttf");
const PLEX_MONO: &[u8] = include_bytes!("../../../fonts/IBMPlexMono-Regular.ttf");
const PLEX_MONO_BOLD: &[u8] = include_bytes!("../../../fonts/IBMPlexMono-Bold.ttf");
const TAIL: &[u8] = include_bytes!("../../../fonts/DejaVuSerif.ttf");
const EMOJI: &[u8] = include_bytes!("../../../fonts/NotoEmoji-Regular.ttf");

/// Plex Sans runs `wght` 100–700, so 700 is its bold end, not an arbitrary number.
const REGULAR: f32 = 400.0;
const BOLD: f32 = 700.0;

/// The name each family is registered under, paired with its bytes and weight.
///
/// One list, walked twice: once to register the faces and once to build the chains. Two lists
/// would be two places for a name to be misspelled, and a misspelled name is silent.
const FACES: [(&str, &[u8], f32); 10] = [
    ("sans", PLEX_SANS, REGULAR),
    ("sans-bold", PLEX_SANS, BOLD),
    ("sans-italic", PLEX_SANS_ITALIC, REGULAR),
    ("sans-bold-italic", PLEX_SANS_ITALIC, BOLD),
    ("serif", PLEX_SERIF, REGULAR),
    ("serif-bold", PLEX_SERIF_BOLD, BOLD),
    ("serif-italic", PLEX_SERIF_ITALIC, REGULAR),
    ("serif-bold-italic", PLEX_SERIF_BOLD_ITALIC, BOLD),
    ("mono", PLEX_MONO, REGULAR),
    ("mono-bold", PLEX_MONO_BOLD, BOLD),
];

/// The family to draw a face in.
pub fn family(face: Face) -> FontFamily {
    FontFamily::Name(face.family_name().into())
}

/// The family a registered name refers to. Private: outside this module a face is asked for by
/// [`Face`], never by spelling a name.
fn family_named(name: &str) -> FontFamily {
    FontFamily::Name(name.into())
}

/// Install the embedded faces. Once, at startup: this rebuilds the glyph atlas, so it does not
/// belong on the per-frame path `theme::apply` sits on.
pub fn install(ctx: &egui::Context) {
    let mut defs = egui::FontDefinitions::empty();

    for (name, bytes, weight) in FACES {
        defs.font_data
            .insert(name.to_owned(), Arc::new(weighted(bytes, weight)));
    }
    defs.font_data
        .insert("tail".to_owned(), Arc::new(weighted(TAIL, REGULAR)));
    defs.font_data
        .insert("emoji".to_owned(), Arc::new(weighted(EMOJI, REGULAR)));

    // Every chain ends the same way. A chain that forgets the tail renders tofu for exactly the
    // scripts the tail exists to cover, and only on a page nobody tests with.
    let chain = |first: &str| vec![first.to_owned(), "tail".to_owned(), "emoji".to_owned()];
    for (name, _, _) in FACES {
        defs.families.insert(family_named(name), chain(name));
    }
    // egui's own widgets ask for these two by name, so they are aliases onto the plain sans and
    // mono rather than faces of their own. Without them a tooltip would draw in nothing.
    defs.families
        .insert(FontFamily::Proportional, chain("sans"));
    defs.families.insert(FontFamily::Monospace, chain("mono"));

    ctx.set_fonts(defs);
}

fn weighted(bytes: &'static [u8], weight: f32) -> FontData {
    FontData::from_static(bytes).tweak(FontTweak {
        coords: VariationCoords::new([(b"wght", weight), (b"wdth", 100.0)]),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::face::FAMILY_NAMES;

    /// The two halves of the split: `face` decides names without egui, `fonts` registers them.
    /// Nothing else connects them, and a name registered under one spelling and asked for under
    /// another resolves to a fallback rather than to an error.
    ///
    /// Admitted under `ui/` on the argument the directory doc makes, not on precedent: `FACES`
    /// is a const table and this test touches no `egui::Context`.
    #[test]
    fn every_name_the_reader_can_ask_for_is_registered() {
        for name in FAMILY_NAMES {
            assert!(
                FACES.iter().any(|(n, _, _)| *n == name),
                "{name} is asked for but never registered"
            );
        }
        for (name, _, _) in FACES {
            assert!(
                FAMILY_NAMES.contains(&name),
                "{name} is registered but nothing asks for it"
            );
        }
    }

    /// A face that is bytes-identical to another is registered twice on purpose (the variable
    /// sans is its own bold), but two *names* sharing one weight would mean a style silently
    /// rendering at the wrong one.
    #[test]
    fn the_bold_faces_are_the_ones_at_bold_weight() {
        for (name, _, weight) in FACES {
            let want_bold = name.contains("bold");
            assert_eq!(
                weight == BOLD,
                want_bold,
                "{name} is registered at {weight}"
            );
        }
    }
}
