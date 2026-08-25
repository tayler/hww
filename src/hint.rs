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

/// Does any hint in `hints` occur in `attrs` as whole tokens?
///
/// `attrs` is whatever the caller concatenates: classes, id, tag name. Hints are lowercase
/// and may carry hyphens, each hyphen being one token boundary.
pub fn matches(attrs: &str, hints: &[&str]) -> bool {
    let toks = tokens(attrs);
    hints.iter().any(|h| {
        let parts: Vec<&str> = h.split('-').collect();
        toks.windows(parts.len()).any(|w| w == parts)
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
    classes.split_whitespace().any(|c| {
        let toks = tokens(c);
        hints
            .iter()
            .any(|h| h.split('-').eq(toks.iter().map(String::as_str)))
    })
}

/// Split on non-alphanumerics and on camelCase steps, lowercased. `PagePromo-title` is
/// `["page", "promo", "title"]`; `hnuser` is `["hnuser"]`.
fn tokens(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut prev_lower = false;
    for c in s.chars() {
        if c.is_alphanumeric() {
            if c.is_uppercase() && prev_lower && !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            cur.extend(c.to_lowercase());
            prev_lower = c.is_lowercase();
        } else {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            prev_lower = false;
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
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
}
