//! A site profile: what a host's pages need beyond the generic extractor, as data.
//!
//! The vocabulary is closed and today it is one word. Phase 3 measured the top-ten US news
//! sites after three rounds of generic fixes and the front-page work, and what remained that
//! only a selector could know was in-body chrome on otherwise good articles: a "listen to
//! this article" banner, a "see all topics" link, a lead gallery of captions. Every one of
//! them is a `strip`. Nothing asked for a root, a metadata source, or a thread shape, and the
//! previous per-site layer was removed precisely for being mechanism without a caller
//! (`AGENTS.md`, Closed decisions). So `Profile` has the field that has callers and grows when
//! a page asks.
//!
//! Three rules, all borrowed from the rewrite table's charter. A profile is **data**: CSS
//! selectors, no code, no hostnames (the host is `sites.rs`'s business, and this module knows
//! none). A profile is a **candidate with a floor**, never an override: `strip` removes
//! subtrees before anything is scored, and if that removes most of the page the outcome says
//! so. And every field **reports**: matched how many, matched nothing, or rejected, so a
//! profile that has gone stale is visible in `--why`, on stderr, and in the page-info panel
//! rather than silently doing nothing.

use std::borrow::Cow;

/// A selector in the builtin table is a `'static` literal; one read from a file later would
/// be owned. Same type either way.
pub type Sel = Cow<'static, str>;

pub const fn sel(s: &'static str) -> Option<Sel> {
    Some(Cow::Borrowed(s))
}

/// What a host's pages need. Every field optional; every set field reported.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Profile {
    /// Subtrees removed before metadata, scoring, walking, and thread detection. In-body
    /// chrome: banners, "see all", share rows, a lead gallery with no images loaded.
    pub strip: Option<Sel>,
}

impl Profile {
    pub const NONE: Profile = Profile { strip: None };

    pub fn is_none(&self) -> bool {
        self.strip.is_none()
    }

    /// `(field, selector)` for every field that is set. The one list the parse test, the
    /// report, and `explain` all walk, so none of them can forget a field.
    pub fn fields(&self) -> Vec<(&'static str, &str)> {
        let mut out = Vec::new();
        if let Some(s) = &self.strip {
            out.push(("strip", s.as_ref()));
        }
        out
    }
}

/// What applying a profile did, one line per set field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileReport {
    /// The host the profile was chosen for, as `sites.rs` matched it. Filled by `session`.
    pub host: String,
    pub fields: Vec<FieldReport>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldReport {
    pub field: &'static str,
    pub outcome: Outcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The selector matched `n` elements and the field was applied.
    Matched(usize),
    /// The selector matched nothing. The stale-profile signal.
    Dead,
    /// The selector matched, but applying it was refused: it would have removed most of the
    /// page, or it does not parse.
    Rejected(&'static str),
}

impl ProfileReport {
    /// Fields that did not do what the profile said they would.
    pub fn dead_fields(&self) -> impl Iterator<Item = &FieldReport> {
        self.fields
            .iter()
            .filter(|f| !matches!(f.outcome, Outcome::Matched(_)))
    }
}

impl std::fmt::Display for Outcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Outcome::Matched(n) => write!(f, "matched {n}"),
            Outcome::Dead => write!(f, "matched nothing"),
            Outcome::Rejected(why) => write!(f, "rejected: {why}"),
        }
    }
}

impl std::fmt::Display for ProfileReport {
    /// `[profile example.com: strip matched 2]`. The bracket convention of the rewrite notice
    /// and the inline `[image]` / `[loading]` marks; ASCII only.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[profile {}:", self.host)?;
        for (i, fr) in self.fields.iter().enumerate() {
            write!(
                f,
                "{}{} {}",
                if i == 0 { " " } else { " | " },
                fr.field,
                fr.outcome
            )?;
        }
        write!(f, "]")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_has_no_fields_and_a_set_field_is_listed_once() {
        assert!(Profile::NONE.fields().is_empty());
        assert!(Profile::NONE.is_none());
        let p = Profile {
            strip: sel(".banner, .share"),
        };
        assert_eq!(p.fields(), vec![("strip", ".banner, .share")]);
    }

    #[test]
    fn report_display_names_every_outcome() {
        let r = ProfileReport {
            host: "example.com".into(),
            fields: vec![
                FieldReport {
                    field: "strip",
                    outcome: Outcome::Matched(2),
                },
                FieldReport {
                    field: "strip",
                    outcome: Outcome::Dead,
                },
                FieldReport {
                    field: "strip",
                    outcome: Outcome::Rejected("does not parse"),
                },
            ],
        };
        assert_eq!(
            r.to_string(),
            "[profile example.com: strip matched 2 | strip matched nothing | strip rejected: does not parse]"
        );
        assert_eq!(r.dead_fields().count(), 2);
        assert!(r.to_string().is_ascii());
    }
}
