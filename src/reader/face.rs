//! Which face an inline style is set in. No egui types, so the fast CI job tests it.
//!
//! The mapping is a decision about the *shipped set of faces*, not about egui, which is why it
//! lives here and not in `ui/`: whether `Strong` is a weight or a colour, and whether `Emph` is
//! a real italic or a skew, are answered by what the binary embeds. `ui::fonts` embeds those
//! faces and registers one family per name this returns; `ui::fonts::family` is the only place
//! a name becomes an `egui::FontFamily`.
//!
//! Three roles, four styles each, and one face missing on purpose: no italic monospace is
//! shipped, because an italic mono face costs 285K to serve emphasis inside a code span. That
//! single hole is what [`Face::synthesises_italics`] reports, and epaint's skew fills it.

use crate::reader::opts::FontChoice;

/// A role crossed with an inline style: the face to draw one run in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Face {
    pub role: FontChoice,
    pub strong: bool,
    pub emph: bool,
}

impl Face {
    pub fn new(role: FontChoice, strong: bool, emph: bool) -> Self {
        Self { role, strong, emph }
    }

    /// The registered family name. Every value this returns is a name `ui::fonts::install`
    /// inserts; `family_names_are_all_registered` over there is what keeps the two in step.
    ///
    /// Mono answers with the upright names when asked for an italic: the face does not exist,
    /// and an unbound family name is not a fallback in epaint but a panic on the first frame
    /// that draws it.
    pub fn family_name(self) -> &'static str {
        match self.role {
            FontChoice::Sans => match (self.strong, self.emph) {
                (false, false) => "sans",
                (true, false) => "sans-bold",
                (false, true) => "sans-italic",
                (true, true) => "sans-bold-italic",
            },
            FontChoice::Serif => match (self.strong, self.emph) {
                (false, false) => "serif",
                (true, false) => "serif-bold",
                (false, true) => "serif-italic",
                (true, true) => "serif-bold-italic",
            },
            FontChoice::Mono => {
                if self.strong {
                    "mono-bold"
                } else {
                    "mono"
                }
            }
        }
    }

    /// Every role crossed with every style: the twelve faces the reader can ask for.
    pub fn all() -> impl Iterator<Item = Face> {
        [FontChoice::Sans, FontChoice::Serif, FontChoice::Mono]
            .into_iter()
            .flat_map(|role| {
                [(false, false), (true, false), (false, true), (true, true)]
                    .into_iter()
                    .map(move |(strong, emph)| Face::new(role, strong, emph))
            })
    }

    /// Whether epaint must skew this face to stand in for an italic one.
    ///
    /// True for monospace and nothing else. Asking for the skew *and* a real italic face would
    /// slant an already-slanted design twice, which is the failure this exists to prevent.
    pub fn synthesises_italics(self) -> bool {
        self.emph && matches!(self.role, FontChoice::Mono)
    }
}

/// Every family name the reader can ask for, which is what `ui::fonts` must register.
pub const FAMILY_NAMES: [&str; 10] = [
    "sans",
    "sans-bold",
    "sans-italic",
    "sans-bold-italic",
    "serif",
    "serif-bold",
    "serif-italic",
    "serif-bold-italic",
    "mono",
    "mono-bold",
];

#[cfg(test)]
mod tests {
    use super::*;

    /// A name that is never registered does not fail in this job: it is a panic in epaint's
    /// `Fonts::font` on the first frame that draws the style, which only a running reader
    /// reaches. Both directions, so a name in the list that no style resolves to is caught too.
    #[test]
    fn every_style_resolves_to_a_declared_family() {
        let mut seen = Vec::new();
        for face in Face::all() {
            let name = face.family_name();
            assert!(FAMILY_NAMES.contains(&name), "{face:?} -> {name}");
            seen.push(name);
        }
        for name in FAMILY_NAMES {
            assert!(
                seen.contains(&name),
                "{name} is declared but no style resolves to it"
            );
        }
    }

    /// The skew stands in for a face that is not shipped, so it must be asked for in exactly the
    /// case where none exists.
    #[test]
    fn only_monospace_synthesises_its_italic() {
        for role in [FontChoice::Sans, FontChoice::Serif] {
            assert!(!Face::new(role, false, true).synthesises_italics());
            assert!(!Face::new(role, true, true).synthesises_italics());
        }
        assert!(Face::new(FontChoice::Mono, false, true).synthesises_italics());
        assert!(!Face::new(FontChoice::Mono, false, false).synthesises_italics());
    }

    /// Bold and italic are separate axes and a run can carry both. Losing the combination is
    /// the kind of thing a four-arm match invites.
    #[test]
    fn bold_italic_is_its_own_face_where_one_is_shipped() {
        assert_eq!(
            Face::new(FontChoice::Sans, true, true).family_name(),
            "sans-bold-italic"
        );
        assert_eq!(
            Face::new(FontChoice::Serif, true, true).family_name(),
            "serif-bold-italic"
        );
        // Mono keeps its weight and takes the skew rather than resolving to a face that is not
        // there.
        assert_eq!(
            Face::new(FontChoice::Mono, true, true).family_name(),
            "mono-bold"
        );
    }
}
