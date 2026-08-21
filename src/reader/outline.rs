//! The document outline, and the fragment matcher that rides on it.
//!
//! Built from `Block::Heading` levels alone. Two things need normalizing before an outline is
//! usable: pages whose smallest heading is `h2` (so nothing is at level 1), and pages that
//! skip levels. Both are common enough that an un-normalized outline is mostly indentation
//! noise.

use crate::ir;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineEntry {
    /// Index into `doc.blocks`, so jumping is a scroll and never a second traversal.
    pub block: usize,
    /// 1-based, normalized and capped — see [`build`].
    pub level: u8,
    pub text: String,
    /// The heading text as an anchor slug, for `#fragment` matching.
    pub slug: String,
}

/// Levels past this are displayed at this depth. Three levels of indent is as much structure
/// as a single reading column can show without becoming a diagram.
pub const MAX_DISPLAY_LEVEL: u8 = 3;

pub fn build(blocks: &[ir::Block]) -> Vec<OutlineEntry> {
    let raw: Vec<(usize, u8, String)> = blocks
        .iter()
        .enumerate()
        .filter_map(|(i, b)| match b {
            ir::Block::Heading { level, inlines } => {
                let text = ir::plain_text(inlines)
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                (!text.is_empty()).then_some((i, *level, text))
            }
            _ => None,
        })
        .collect();
    let Some(min) = raw.iter().map(|(_, l, _)| *l).min() else {
        return Vec::new();
    };
    raw.into_iter()
        .map(|(block, level, text)| OutlineEntry {
            block,
            level: (level - min + 1).min(MAX_DISPLAY_LEVEL),
            slug: slugify(&text),
            text,
        })
        .collect()
}

/// Lowercase, non-alphanumerics to hyphens, runs collapsed, ends trimmed.
///
/// This is the shape nearly every CMS derives its anchor ids from, which is the entire reason
/// fragment matching works at all.
pub fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_sep = false;
    for c in s.chars() {
        if c.is_alphanumeric() {
            for lower in c.to_lowercase() {
                if pending_sep && !out.is_empty() {
                    out.push('-');
                }
                pending_sep = false;
                out.push(lower);
            }
        } else {
            pending_sep = true;
        }
    }
    out
}

/// Which outline entry a `#fragment` refers to, if any.
///
/// There are no block ids in the IR, so this cannot resolve exactly — and adding ids to
/// `ir::Block` to make a heuristic exact is contract creep in the direction the charter guards.
/// A miss is reported in the status strip rather than silently doing nothing, which is the
/// difference between a heuristic and a lie.
pub fn find_fragment<'a>(outline: &'a [OutlineEntry], fragment: &str) -> Option<&'a OutlineEntry> {
    let want = slugify(fragment);
    if want.is_empty() {
        return None;
    }
    outline
        .iter()
        .find(|e| e.slug == want)
        // Anchors are routinely the heading's slug with a numeric or hash suffix.
        .or_else(|| {
            outline
                .iter()
                .find(|e| want.starts_with(&e.slug) || e.slug.starts_with(&want))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heading(level: u8, text: &str) -> ir::Block {
        ir::Block::Heading {
            level,
            inlines: vec![ir::Inline::Text(text.to_owned())],
        }
    }

    #[test]
    fn levels_normalize_so_the_shallowest_heading_is_level_one() {
        let blocks = vec![
            heading(2, "First"),
            ir::Block::Rule,
            heading(3, "Second"),
            heading(2, "Third"),
        ];
        let o = build(&blocks);
        assert_eq!(
            o.iter().map(|e| (e.block, e.level)).collect::<Vec<_>>(),
            vec![(0, 1), (2, 2), (3, 1)]
        );
    }

    #[test]
    fn display_depth_is_capped() {
        let blocks = vec![heading(1, "a"), heading(6, "b")];
        assert_eq!(build(&blocks)[1].level, MAX_DISPLAY_LEVEL);
    }

    #[test]
    fn empty_headings_and_empty_documents_produce_no_outline() {
        assert!(build(&[]).is_empty());
        assert!(build(&[heading(1, "   ")]).is_empty());
    }

    #[test]
    fn slugs_match_the_shape_cms_anchors_take() {
        assert_eq!(slugify("What's *next*, then?"), "what-s-next-then");
        assert_eq!(slugify("  Spaced   Out  "), "spaced-out");
        assert_eq!(slugify("---"), "");
    }

    #[test]
    fn fragments_resolve_by_slug_and_by_suffixed_slug() {
        let o = build(&[heading(1, "Getting Started"), heading(2, "Notes")]);
        assert_eq!(find_fragment(&o, "getting-started").unwrap().block, 0);
        // The suffix shape CMSs add for uniqueness.
        assert_eq!(find_fragment(&o, "getting-started-2").unwrap().block, 0);
        assert!(find_fragment(&o, "nothing-like-this").is_none());
        assert!(find_fragment(&o, "").is_none());
    }
}
