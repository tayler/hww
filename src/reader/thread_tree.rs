//! `Block::Thread` is a flat `Vec<Comment>` carrying a `depth`. Collapsible replies need the
//! tree back.
//!
//! Reconstruction is one stack pass, and the pop-while loop handles both malformed shapes for
//! free: a depth jump greater than one attaches to the nearest available ancestor, and a first
//! comment at `depth > 0` becomes a root. Extraction is a heuristic over real markup, so
//! "malformed" here is the normal case, not the exceptional one.

use crate::ir;

/// How a collapsed subtree is remembered across a re-render.
///
/// `Comment.id` comes straight from the DOM `id` attribute and is `None` whenever the site
/// sets none. The fallback is a **sibling path**, not an index (which shifts if extraction
/// changes by one comment) and not a content hash (`"+1"` appears eleven times on any forum
/// page).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CommentKey {
    Id(String),
    Path(Box<str>),
}

#[derive(Debug, Clone)]
pub struct TreeNode {
    /// Index into the original `Vec<Comment>`.
    pub comment: usize,
    pub children: Vec<usize>,
    /// Text length of this comment and everything under it. Drives auto-collapse and the
    /// "N replies" summary a collapsed node shows.
    pub subtree_len: usize,
    /// Comments in this subtree, including this one.
    pub subtree_count: usize,
    /// Normalized nesting depth: the tree's own, which is not always the IR's.
    pub depth: u16,
    pub key: CommentKey,
}

#[derive(Debug, Clone, Default)]
pub struct ThreadTree {
    /// Indexed by node id, which is also the comment index; `build` keeps them equal so
    /// callers never need a second mapping.
    pub nodes: Vec<TreeNode>,
    pub roots: Vec<usize>,
}

/// Auto-collapse below this depth, so a huge thread opens navigable.
pub const AUTO_COLLAPSE_DEPTH: u16 = 4;
/// Auto-collapse a subtree longer than this many characters, whatever its depth.
pub const AUTO_COLLAPSE_LEN: usize = 4000;

impl ThreadTree {
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Whether a node starts collapsed. A thread that opens as forty screens of nesting is a
    /// thread nobody reads.
    pub fn starts_collapsed(&self, node: usize) -> bool {
        let n = &self.nodes[node];
        !n.children.is_empty()
            && (n.depth >= AUTO_COLLAPSE_DEPTH || n.subtree_len > AUTO_COLLAPSE_LEN)
    }
}

pub fn build(comments: &[ir::Comment]) -> ThreadTree {
    let mut tree = ThreadTree {
        nodes: Vec::with_capacity(comments.len()),
        roots: Vec::new(),
    };
    // (ir depth, node index) of the ancestors of the comment being placed.
    let mut stack: Vec<(u16, usize)> = Vec::new();

    for (i, c) in comments.iter().enumerate() {
        while stack.last().is_some_and(|(d, _)| *d >= c.depth) {
            stack.pop();
        }
        let parent = stack.last().map(|(_, n)| *n);
        let depth = parent.map_or(0, |p| tree.nodes[p].depth + 1);
        let key = match &c.id {
            Some(id) if !id.trim().is_empty() => CommentKey::Id(id.clone()),
            _ => CommentKey::Path(path_for(&tree, parent).into_boxed_str()),
        };
        tree.nodes.push(TreeNode {
            comment: i,
            children: Vec::new(),
            subtree_len: ir::blocks_text_len(&c.blocks),
            subtree_count: 1,
            depth,
            key,
        });
        match parent {
            Some(p) => tree.nodes[p].children.push(i),
            None => tree.roots.push(i),
        }
        stack.push((c.depth, i));
    }

    // Reverse pass: every child precedes its parent in no particular order, but a child's
    // index is always greater than its parent's, so walking backwards accumulates correctly.
    for i in (0..tree.nodes.len()).rev() {
        let (len, count) = tree.nodes[i]
            .children
            .iter()
            .map(|&c| (tree.nodes[c].subtree_len, tree.nodes[c].subtree_count))
            .fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1));
        tree.nodes[i].subtree_len += len;
        tree.nodes[i].subtree_count += count;
    }
    tree
}

/// The sibling path of the node about to be pushed, as `"0.3.1"`.
fn path_for(tree: &ThreadTree, parent: Option<usize>) -> String {
    let siblings = match parent {
        Some(p) => tree.nodes[p].children.len(),
        None => tree.roots.len(),
    };
    match parent {
        Some(p) => {
            let head = match &tree.nodes[p].key {
                CommentKey::Path(s) => s.to_string(),
                // A parent with a DOM id still anchors its children's paths, so the path stays
                // stable even in a thread that only ids some of its comments.
                CommentKey::Id(id) => format!("#{id}"),
            };
            format!("{head}.{siblings}")
        }
        None => siblings.to_string(),
    }
}

/// Depth-first order, **iteratively**. A pathological thread is untrusted input and recursion
/// here would blow the stack on a page that is merely hostile, not even malicious.
pub fn dfs(tree: &ThreadTree, mut visit: impl FnMut(usize, u16)) {
    let mut stack: Vec<usize> = tree.roots.iter().rev().copied().collect();
    while let Some(n) = stack.pop() {
        visit(n, tree.nodes[n].depth);
        stack.extend(tree.nodes[n].children.iter().rev().copied());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comments(depths: &[u16]) -> Vec<ir::Comment> {
        depths
            .iter()
            .enumerate()
            .map(|(i, d)| ir::Comment {
                author: None,
                timestamp: None,
                depth: *d,
                id: None,
                blocks: vec![ir::Block::Paragraph(vec![ir::Inline::Text(format!(
                    "c{i}"
                ))])],
            })
            .collect()
    }

    fn shape(tree: &ThreadTree) -> Vec<(usize, u16)> {
        let mut out = Vec::new();
        dfs(tree, |n, d| out.push((tree.nodes[n].comment, d)));
        out
    }

    #[test]
    fn an_empty_thread_builds_an_empty_tree() {
        let tree = build(&[]);
        assert!(tree.is_empty() && tree.roots.is_empty());
    }

    #[test]
    fn a_single_comment_is_a_root() {
        let tree = build(&comments(&[0]));
        assert_eq!(tree.roots, vec![0]);
        assert_eq!(tree.nodes[0].subtree_count, 1);
    }

    #[test]
    fn a_flat_thread_is_all_roots() {
        let tree = build(&comments(&[0, 0, 0]));
        assert_eq!(tree.roots, vec![0, 1, 2]);
        assert!(tree.nodes.iter().all(|n| n.children.is_empty()));
    }

    #[test]
    fn nesting_attaches_replies_to_the_comment_above() {
        let tree = build(&comments(&[0, 1, 2, 1, 0]));
        assert_eq!(tree.roots, vec![0, 4]);
        assert_eq!(tree.nodes[0].children, vec![1, 3]);
        assert_eq!(tree.nodes[1].children, vec![2]);
        assert_eq!(shape(&tree), vec![(0, 0), (1, 1), (2, 2), (3, 1), (4, 0)]);
        assert_eq!(tree.nodes[0].subtree_count, 4);
    }

    /// Extraction skips levels routinely; a jump attaches to the nearest available ancestor
    /// rather than inventing intermediate comments or dropping the reply.
    #[test]
    fn a_depth_jump_attaches_to_the_nearest_ancestor() {
        let tree = build(&comments(&[0, 3]));
        assert_eq!(tree.nodes[0].children, vec![1]);
        assert_eq!(tree.nodes[1].depth, 1);
    }

    #[test]
    fn a_thread_that_starts_deep_still_has_a_root() {
        let tree = build(&comments(&[2, 3, 2]));
        assert_eq!(tree.roots, vec![0, 2]);
        assert_eq!(tree.nodes[0].depth, 0);
    }

    #[test]
    fn a_deep_reply_followed_by_a_shallow_one_unwinds_the_stack() {
        let tree = build(&comments(&[0, 1, 2, 3, 4, 1]));
        assert_eq!(tree.nodes[0].children, vec![1, 5]);
        assert_eq!(tree.roots, vec![0]);
    }

    #[test]
    fn subtree_length_sums_the_whole_branch() {
        let tree = build(&comments(&[0, 1, 1]));
        // "c0" + "c1" + "c2" = 6 characters.
        assert_eq!(tree.nodes[0].subtree_len, 6);
        assert_eq!(tree.nodes[1].subtree_len, 2);
    }

    #[test]
    fn keys_prefer_the_dom_id_and_fall_back_to_a_sibling_path() {
        let mut cs = comments(&[0, 1, 1, 0]);
        cs[0].id = Some("t1_abc".into());
        let tree = build(&cs);
        assert_eq!(tree.nodes[0].key, CommentKey::Id("t1_abc".into()));
        // Children of an id'd parent anchor on it, so the path is stable in a partly-id'd
        // thread.
        assert_eq!(tree.nodes[1].key, CommentKey::Path("#t1_abc.0".into()));
        assert_eq!(tree.nodes[2].key, CommentKey::Path("#t1_abc.1".into()));
        assert_eq!(tree.nodes[3].key, CommentKey::Path("1".into()));
    }

    /// A blank `id` attribute is not an id.
    #[test]
    fn a_blank_dom_id_falls_back_to_the_path() {
        let mut cs = comments(&[0]);
        cs[0].id = Some("  ".into());
        assert_eq!(build(&cs).nodes[0].key, CommentKey::Path("0".into()));
    }

    #[test]
    fn deep_or_long_subtrees_start_collapsed_and_leaves_never_do() {
        let tree = build(&comments(&[0, 1, 2, 3, 4, 5]));
        assert!(!tree.starts_collapsed(0), "a shallow parent opens");
        assert!(tree.starts_collapsed(4), "depth 4 auto-collapses");
        assert!(!tree.starts_collapsed(5), "a leaf has nothing to collapse");
    }

    /// A thread nested far past anything a human wrote still builds and traverses completely.
    /// `dfs` is iterative for this input class: a hostile page is untrusted input, and a
    /// recursive walk over it is a crash rather than a bad render.
    #[test]
    fn a_pathologically_deep_thread_builds_and_traverses_completely() {
        const DEEP: u16 = 3000;
        let depths: Vec<u16> = (0..DEEP).collect();
        let tree = build(&comments(&depths));
        assert_eq!(tree.roots, vec![0]);
        assert_eq!(tree.nodes[0].subtree_count, usize::from(DEEP));
        let mut seen = 0usize;
        dfs(&tree, |_, _| seen += 1);
        assert_eq!(seen, usize::from(DEEP));
    }
}
