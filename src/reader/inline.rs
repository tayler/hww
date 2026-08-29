//! The inline tree, flattened once.
//!
//! `&[ir::Inline]` is a tree; every renderer needs it as a sequence of styled segments. This
//! is that sequence, and it is deliberately the *only* styled flatten in the crate:
//! `render::inline_text` is built on top of it, and so is the GUI. Two walkers disagreeing
//! about where an emphasis boundary falls (the text renderer and the reader drawing different
//! spans from the same document) is exactly the bug nobody finds.
//!
//! No egui types appear here, so the default CI job compiles and tests it.
//!
//! # Two deliberate normalizations
//!
//! Flattening cannot preserve everything a tree can express, and both losses are improvements:
//!
//! 1. **Adjacent equal-styled segments merge.** `<em>a</em><em>b</em>` becomes one run, not
//!    two. This is required (the GUI emits one widget per run, and one per DOM element would
//!    be both slower and worse at wrapping), and it renders as `_ab_` where the old recursive
//!    walker wrote `_a__b_`. Same words, fewer sigils.
//! 2. **Emphasis nesting order is normalized.** `<b><i>x</i></b>` and `<i><b>x</b></i>` both
//!    become `{emph, strong}` and both render `_*x*_`. Order carries no meaning in the IR and
//!    none in any renderer, so keeping it would mean keeping a distinction nothing consumes.
//!
//! Everything else is byte-identical, and `render`'s tests hold a copy of the old recursive
//! walker to prove it.

use crate::ir;

/// Which marks apply to a segment. A set, not a stack (see the module doc).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    pub emph: bool,
    pub strong: bool,
    pub code: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Run {
    Text {
        text: String,
        style: Style,
    },
    /// A link's own runs. Nested links are flattened onto the outer href; the inner one is
    /// unreachable in any renderer, and hww will not guess which was meant.
    Link {
        href: String,
        runs: Vec<Run>,
    },
    Image(ir::Image),
    Break,
}

impl Run {
    /// The words this run contributes, with no sigils and no link decoration. Used by
    /// find-in-page and by the outline, both of which search what the reader actually shows.
    pub fn plain(&self) -> String {
        match self {
            Run::Text { text, .. } => text.clone(),
            Run::Link { runs, .. } => plain_of(runs),
            Run::Image(img) => img.alt.clone().unwrap_or_default(),
            Run::Break => "\n".to_owned(),
        }
    }
}

/// The plain text of a whole run list.
pub fn plain_of(runs: &[Run]) -> String {
    runs.iter().map(Run::plain).collect()
}

pub fn flatten(inlines: &[ir::Inline]) -> Vec<Run> {
    let mut out = Vec::new();
    walk(inlines, Style::default(), false, &mut out);
    out
}

/// [`flatten`], with the whole run wrapped in one link.
///
/// The alternative is building an `ir::Inline::Link` to hold a *clone* of the inlines and
/// flattening that, which is what every row of the bookmarks, the reading list, the history, and
/// every story card on a front page was doing on every frame: a deep copy of the title's inline
/// tree, per row, to say where it points. The output is the run list that clone produced, down
/// to the empty-link rule — a link with nothing to click is not a link, so an empty run list
/// stays empty rather than becoming a `Run::Link` around nothing.
pub fn flatten_linked(inlines: &[ir::Inline], href: Option<&str>) -> Vec<Run> {
    let Some(href) = href else {
        return flatten(inlines);
    };
    let mut inner = Vec::new();
    walk(inlines, Style::default(), true, &mut inner);
    if inner.is_empty() {
        return inner;
    }
    vec![Run::Link {
        href: href.to_owned(),
        runs: inner,
    }]
}

fn walk(inlines: &[ir::Inline], style: Style, in_link: bool, out: &mut Vec<Run>) {
    for i in inlines {
        match i {
            ir::Inline::Text(t) => push_text(out, t, style),
            ir::Inline::Code(t) => push_text(
                out,
                t,
                Style {
                    code: true,
                    ..style
                },
            ),
            ir::Inline::Emph(v) => walk(
                v,
                Style {
                    emph: true,
                    ..style
                },
                in_link,
                out,
            ),
            ir::Inline::Strong(v) => walk(
                v,
                Style {
                    strong: true,
                    ..style
                },
                in_link,
                out,
            ),
            ir::Inline::Link { href, inlines } => {
                if in_link {
                    // Already inside a link: the inner href is unreachable, so its content
                    // joins the outer one rather than nesting or disappearing.
                    walk(inlines, style, true, out);
                } else {
                    let mut inner = Vec::new();
                    walk(inlines, style, true, &mut inner);
                    // A link with nothing to click is not a link.
                    if !inner.is_empty() {
                        out.push(Run::Link {
                            href: href.clone(),
                            runs: inner,
                        });
                    }
                }
            }
            ir::Inline::Image(img) => out.push(Run::Image(img.clone())),
            ir::Inline::Break => out.push(Run::Break),
        }
    }
}

/// Append text, merging into the previous run when the style matches.
///
/// Boundary whitespace survives merging untouched, and it has to: the reader lays runs out
/// with `item_spacing.x = 0.0`, so word spacing comes from the text itself. `a <em>b</em> c`
/// must not become `ab c`.
fn push_text(out: &mut Vec<Run>, text: &str, style: Style) {
    if text.is_empty() {
        return;
    }
    if let Some(Run::Text {
        text: prev,
        style: prev_style,
    }) = out.last_mut()
        && *prev_style == style
    {
        prev.push_str(text);
        return;
    }
    out.push(Run::Text {
        text: text.to_owned(),
        style,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> ir::Inline {
        ir::Inline::Text(s.to_owned())
    }

    /// [`flatten_linked`] is the clone-and-wrap it replaced.
    ///
    /// The old shape built an `ir::Inline::Link` holding a copy of the title's inlines and
    /// flattened that. This asserts the two agree on the cases where they could differ: a run
    /// with no href at all, one that already contains a link (the inner href is unreachable and
    /// joins the outer), emphasis that has to survive the wrap, an image with no text beside it,
    /// and the empty run that must not become a link around nothing.
    #[test]
    fn a_linked_flatten_is_the_wrap_it_replaced() {
        let img = ir::Inline::Image(ir::Image {
            src: "https://example.test/i.png".to_owned(),
            alt: Some("alt".to_owned()),
        });
        let cases: Vec<Vec<ir::Inline>> = vec![
            vec![t("a headline")],
            vec![t("a "), ir::Inline::Emph(vec![t("headline")])],
            vec![ir::Inline::Link {
                href: "https://example.test/inner".to_owned(),
                inlines: vec![t("already linked")],
            }],
            vec![img.clone()],
            vec![],
            vec![t("")],
        ];
        for inlines in cases {
            for href in [None, Some("https://example.test/row")] {
                let want = match href {
                    Some(h) => flatten(&[ir::Inline::Link {
                        href: h.to_owned(),
                        inlines: inlines.clone(),
                    }]),
                    None => flatten(&inlines),
                };
                assert_eq!(
                    flatten_linked(&inlines, href),
                    want,
                    "{inlines:?} under {href:?}"
                );
            }
        }
    }

    fn text_runs(runs: &[Run]) -> Vec<(&str, Style)> {
        runs.iter()
            .filter_map(|r| match r {
                Run::Text { text, style } => Some((text.as_str(), *style)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn boundary_whitespace_survives() {
        // The reader draws runs with zero horizontal item spacing, so losing this space
        // silently welds two words together.
        let runs = flatten(&[t("a "), ir::Inline::Emph(vec![t("b")]), t(" c")]);
        assert_eq!(plain_of(&runs), "a b c");
    }

    #[test]
    fn adjacent_equal_styles_merge_and_different_ones_do_not() {
        let runs = flatten(&[t("a"), t("b"), ir::Inline::Emph(vec![t("c")])]);
        assert_eq!(
            text_runs(&runs),
            vec![
                ("ab", Style::default()),
                (
                    "c",
                    Style {
                        emph: true,
                        ..Style::default()
                    }
                )
            ]
        );
    }

    #[test]
    fn nesting_order_is_normalized_to_a_set() {
        let a = flatten(&[ir::Inline::Emph(vec![ir::Inline::Strong(vec![t("x")])])]);
        let b = flatten(&[ir::Inline::Strong(vec![ir::Inline::Emph(vec![t("x")])])]);
        assert_eq!(a, b);
        assert_eq!(
            text_runs(&a),
            vec![(
                "x",
                Style {
                    emph: true,
                    strong: true,
                    code: false
                }
            )]
        );
    }

    #[test]
    fn code_inside_strong_keeps_both_marks() {
        let runs = flatten(&[ir::Inline::Strong(vec![ir::Inline::Code("f()".into())])]);
        assert_eq!(
            text_runs(&runs),
            vec![(
                "f()",
                Style {
                    emph: false,
                    strong: true,
                    code: true
                }
            )]
        );
    }

    #[test]
    fn nested_links_flatten_onto_the_outer_href() {
        let runs = flatten(&[ir::Inline::Link {
            href: "https://outer/".into(),
            inlines: vec![
                t("a"),
                ir::Inline::Link {
                    href: "https://inner/".into(),
                    inlines: vec![t("b")],
                },
            ],
        }]);
        match &runs[..] {
            [Run::Link { href, runs }] => {
                assert_eq!(href, "https://outer/");
                // Merged, because both segments are unstyled text of one link.
                assert_eq!(plain_of(runs), "ab");
                assert!(!runs.iter().any(|r| matches!(r, Run::Link { .. })));
            }
            other => panic!("expected one link, got {other:?}"),
        }
    }

    #[test]
    fn a_link_with_no_text_is_dropped() {
        let runs = flatten(&[ir::Inline::Link {
            href: "https://example.com/".into(),
            inlines: vec![t("")],
        }]);
        assert!(runs.is_empty());
    }

    #[test]
    fn breaks_and_images_survive_as_their_own_runs() {
        let runs = flatten(&[
            t("a"),
            ir::Inline::Break,
            ir::Inline::Image(ir::Image {
                src: "x.png".into(),
                alt: Some("a cat".into()),
            }),
        ]);
        assert!(matches!(runs[1], Run::Break));
        assert_eq!(runs[2].plain(), "a cat");
    }

    /// Style carries into a link's contents; a bold link is bold.
    #[test]
    fn style_outside_a_link_applies_inside_it() {
        let runs = flatten(&[ir::Inline::Strong(vec![ir::Inline::Link {
            href: "https://example.com/".into(),
            inlines: vec![t("x")],
        }])]);
        match &runs[..] {
            [Run::Link { runs, .. }] => assert!(matches!(
                runs[0],
                Run::Text {
                    style: Style { strong: true, .. },
                    ..
                }
            )),
            other => panic!("expected a link, got {other:?}"),
        }
    }
}
