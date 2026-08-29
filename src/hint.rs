//! Class-name hints, matched at token boundaries.
//!
//! Both extractors read intent off class and id attributes: `thread.rs` looks for an author
//! and a timestamp on a post, `html.rs` looks for chrome to skip. The first cut matched by
//! substring, and substring matching is how every article grew a "discussion" of image
//! captions (`age` inside `image`) and how one wire service's front page lost every story
//! (`promo` inside `PagePromo`). Measured on the top-ten US news sites, 2026-08-22; see
//! `docs/findings.md`, Phase 3.
//!
//! A hint matches a token, or a run of tokens for a hyphenated hint, where tokens are split on
//! anything that is not alphanumeric and on a lowercase-to-uppercase step, so `user-name`,
//! `userName`, and `user_name` are the same three letters apart and `image` is not `age`.
//!
//! # Nothing here allocates a token
//!
//! This is the hottest code in the extractor: [`matches`] is asked about every element the block
//! walker sees, about every inline child, about three ancestors of every scored node, about every
//! ancestor in `html::in_chrome`, and about every descendant of every member of every sibling
//! group. It used to build a `Vec<String>` of the element's tokens on each of those calls, plus a
//! `Vec<&str>` *per hint* — and `NOISE_HINT` holds forty-five, all of which are tried when the
//! answer is no, which is the usual answer. A token is now a borrowed slice of the caller's own
//! attribute string, compared against a hint segment character by character with the lowercasing
//! applied at the comparison rather than into a buffer. One `Vec` of slices per call remains.

/// Does any hint in `hints` occur in `attrs` as whole tokens?
///
/// `attrs` is whatever the caller concatenates: classes, id, tag name. Hints are lowercase
/// and may carry hyphens, each hyphen being one token boundary.
pub fn matches(attrs: &str, hints: &[&str]) -> bool {
    matches_in(&[attrs], hints)
}

/// [`matches`] over several attribute strings, without joining them first.
///
/// The callers hold two or three separate `&str`s — a class, an id, a tag name — and were
/// building a `format!` of them per element to ask this question. A separator between parts is
/// what the join was for, and a token cannot span one; everything else about the answer is the
/// same, a hyphenated hint still able to match across the seam as it could in the joined string.
pub fn matches_in(parts: &[&str], hints: &[&str]) -> bool {
    let mut toks: Vec<&str> = Vec::new();
    for p in parts {
        push_tokens(p, &mut toks);
    }
    hints.iter().any(|h| {
        let len = h.split('-').count();
        toks.windows(len)
            .any(|w| w.iter().zip(h.split('-')).all(|(t, seg)| eq_folded(t, seg)))
    })
}

/// Does any whitespace-separated class in `classes` consist of exactly one hint's tokens?
///
/// [`matches`] finds a hint *in* a class (`main-navigation` is chrome because `navigation`
/// is); this finds a hint that *is* a class. The body hints want that and the noise hints do
/// not: `message-header` contains `message` and is the byline over the body, not the body,
/// and `post-md` contains `md` and is a post. `comment-body`, `commentBody`, and
/// `comment_body` are the same class here as in [`matches`].
pub fn names(classes: &str, hints: &[&str]) -> bool {
    let mut toks: Vec<&str> = Vec::new();
    classes.split_whitespace().any(|c| {
        toks.clear();
        push_tokens(c, &mut toks);
        hints.iter().any(|h| {
            h.split('-').count() == toks.len()
                && h.split('-').zip(&toks).all(|(seg, t)| eq_folded(t, seg))
        })
    })
}

/// Is `tok`, lowercased, the hint segment `seg`?
///
/// The fold is applied here rather than into a buffer, and it is `char::to_lowercase` — the full
/// Unicode one, which can yield more than one character for a single input (`İ`) and so cannot
/// be done by comparing byte lengths first. Hint segments are ASCII, so the common answer is
/// decided on the first character.
fn eq_folded(tok: &str, seg: &str) -> bool {
    tok.chars().flat_map(char::to_lowercase).eq(seg.chars())
}

/// Split on non-alphanumerics and on camelCase steps. `PagePromo-title` is
/// `["page", "promo", "title"]`; `hnuser` is `["hnuser"]`.
///
/// The slices are borrowed and *not* folded; [`eq_folded`] does that at the comparison.
fn push_tokens<'a>(s: &'a str, out: &mut Vec<&'a str>) {
    let mut start: Option<usize> = None;
    let mut prev_lower = false;
    for (i, c) in s.char_indices() {
        if c.is_alphanumeric() {
            if c.is_uppercase()
                && prev_lower
                && let Some(from) = start
            {
                out.push(&s[from..i]);
                start = None;
            }
            start.get_or_insert(i);
            prev_lower = c.is_lowercase();
        } else {
            if let Some(from) = start {
                out.push(&s[from..i]);
            }
            start = None;
            prev_lower = false;
        }
    }
    if let Some(from) = start {
        out.push(&s[from..]);
    }
}

/// The tokens of `s`, lowercased. The tests' view of [`push_tokens`], and nothing else uses it:
/// the matchers compare without building a token.
#[cfg(test)]
fn tokens(s: &str) -> Vec<std::borrow::Cow<'_, str>> {
    let mut raw: Vec<&str> = Vec::new();
    push_tokens(s, &mut raw);
    raw.into_iter()
        .map(|t| {
            if t.chars().all(|c| !c.is_uppercase()) {
                std::borrow::Cow::Borrowed(t)
            } else {
                std::borrow::Cow::Owned(t.chars().flat_map(char::to_lowercase).collect())
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hint_is_a_whole_token_and_not_a_substring() {
        assert!(!matches("image_large", &["age"]));
        assert!(!matches("page-promo-title", &["age"]));
        assert!(!matches("runtime", &["time"]));
        assert!(!matches("last-update", &["date"]));
        assert!(matches("comment-age", &["age"]));
        assert!(matches("time", &["time"]));
        assert!(matches("post-date meta", &["date"]));
    }

    #[test]
    fn camel_case_and_separators_are_all_boundaries() {
        assert!(matches("PagePromo-byline-container", &["byline"]));
        assert!(matches("PagePromo", &["promo"]));
        assert!(matches("commentAuthor", &["author"]));
        assert!(matches("user_name", &["user-name"]));
        assert!(matches("userName", &["user-name"]));
        assert!(matches("user-name", &["user-name"]));
        assert!(!matches("username", &["user-name"]));
        assert!(matches("username", &["username"]));
    }

    #[test]
    fn hyphenated_hints_need_the_whole_run() {
        assert!(!matches("comment body", &["comment-body-text"]));
        assert!(matches("c comment-body-text x", &["comment-body-text"]));
    }

    #[test]
    fn a_name_is_the_whole_class_and_not_a_token_in_it() {
        assert!(names("message", &["message"]));
        assert!(names("bbWrapper message-body", &["message-body"]));
        assert!(names("commentBody", &["comment-body"]));
        assert!(names("comment_body", &["comment-body"]));
        assert!(!names("message-header", &["message"]));
        assert!(!names("post-md", &["md"]));
        assert!(!names("entry-content-meta", &["entry-content"]));
        assert!(!names("comment-body-text", &["comment-body"]));
        assert!(
            matches("message-header", &["message"]),
            "the contrast with `matches`"
        );
    }

    #[test]
    fn tokens_split_where_a_reader_would() {
        assert_eq!(tokens("PagePromo-title"), ["page", "promo", "title"]);
        assert_eq!(tokens("hnuser"), ["hnuser"]);
        assert_eq!(tokens("XMLHttpRequest"), ["xmlhttp", "request"]);
        assert_eq!(tokens("  a  "), ["a"]);
    }

    /// Separate parts answer as the joined string did.
    ///
    /// The callers stopped building a `format!` of their class, id, and tag name, so the
    /// equivalence has to be pinned rather than assumed — including the case that made the join
    /// visible in the first place, a hyphenated hint whose run crosses from one part into the
    /// next.
    #[test]
    fn parts_answer_as_the_joined_string_did() {
        let cases: &[(&[&str], &[&str])] = &[
            (&["post-date", "meta", "div"], &["date"]),
            (&["", "", "nav"], &["nav"]),
            (&["image_large", "", "figure"], &["age"]),
            (&["comment", "body", "div"], &["comment-body"]),
            (&["PagePromo", "lead", "section"], &["promo", "share"]),
            (&["", "", ""], &["nav"]),
        ];
        for (parts, hints) in cases {
            let joined = parts.join(" ");
            assert_eq!(
                matches_in(parts, hints),
                matches(&joined, hints),
                "{parts:?} against {hints:?}"
            );
        }
    }

    /// A token that is already lowercase is not copied to be compared.
    ///
    /// The fold happens at the comparison, so this is really a test that the split borrows: a
    /// token is a slice of the string the caller passed in.
    #[test]
    fn a_token_is_a_slice_of_the_input() {
        let s = String::from("comment-body");
        let mut toks: Vec<&str> = Vec::new();
        push_tokens(&s, &mut toks);
        assert_eq!(toks, ["comment", "body"]);
        for t in &toks {
            let inside = s.as_ptr() as usize;
            let at = t.as_ptr() as usize;
            assert!(
                at >= inside && at + t.len() <= inside + s.len(),
                "token {t:?} points outside the input"
            );
        }
    }
}
