//! Per-site rules: the only place in the crate where behaviour is keyed on a hostname.
//!
//! Everything else in hww is structural: `html.rs` scores subtrees, `thread.rs` groups siblings
//! by class signature, `bin/corpus.rs` classifies by URL shape. None of them know a domain name
//! and none of them should, and none of them import this module.
//!
//! Phase 0 measured ~7 genuine JS-only reading losses out of 87 document-shaped pages. Two of
//! them are pages of one JS-gated discussion site that serves a fully server-rendered alternate
//! domain with its own no-JS UI, the same operator, and the same content. Reaching it is a
//! host swap, and that is all this module does. See `docs/phase0-findings.md`.
//!
//! # Charter for the builtin table
//!
//! A rule may ship compiled in only if all four hold:
//!
//! 1. **Same operator.** The target host is run by the same entity as the source. A third-party
//!    frontend proxy relocates the reader onto an unrelated party, precisely the "no
//!    third-party requests" line this client exists to hold. Never builtin.
//! 2. **Cited.** The rule names a measured failure in `docs/phase0-findings.md`. No speculative
//!    entries, no curated directory of the web.
//! 3. **Offline.** The table is compiled in. Never fetched, never auto-updated: a remotely
//!    updated rule list is a channel that reports what you read.
//! 4. **Reported.** Every applied rule prints on stderr *before* the request goes out, so a
//!    fetch that fails or hangs cannot swallow it. hww never silently contacts a host the user
//!    did not name.
//!
//! Point 2 is why this layer rewrites URLs and does nothing else. Per-site *extraction* hints
//! were built and removed: the one hint that looked obviously right measured worse than no hint
//! at all, and nothing else in the corpus asked for one. `docs/phase0-findings.md`, Phase 2.

use url::Url;

struct Rule {
    /// Matched exactly, or as a suffix at a label boundary. Must already be in the normalized
    /// form `Url` produces: lowercase, punycode. Enforced by `builtin_table_is_sound`.
    host: &'static str,
    /// Literal substring of the path, slashes included. The only path predicate there will
    /// ever be, deliberately not a pattern language.
    path_contains: Option<&'static str>,
    /// Fetch this host instead. Same operator only (see the charter).
    to_host: &'static str,
}

/// One rule per measured loss.
const RULES: &[Rule] = &[
    // The JS-gated discussion site of "The real JS-only losses" and "Where it still loses" in
    // docs/phase0-findings.md, and its own server-rendered alternate. Measured on one comment
    // page: 0 chars before, 8,331 after; see "Phase 2: per-site rules". Sections, not line
    // numbers: that document grows.
    //
    // Gated to the site's subreddit paths, the shape that was measured. Without the gate the
    // rule also claims the auth, moderation, and API subdomains, whose paths the alternate
    // does not serve: a login page or a 404 in place of the URL the user asked for, explained
    // only by one line on stderr.
    Rule {
        host: "reddit.com",
        path_contains: Some("/r/"),
        to_host: "old.reddit.com",
    },
];

/// Rewrite `url` if a rule matches.
///
/// First match wins, and a rule is applied exactly once, never chained, so termination is
/// structural rather than bounded by a counter. Everything a rule does not name is preserved:
/// scheme, port, path, query, and fragment. Since `to_host` replaces the whole host, applying
/// the result again is a no-op.
pub fn apply(url: &Url) -> Url {
    apply_in(RULES, url)
}

fn apply_in(rules: &[Rule], url: &Url) -> Url {
    // Only the web schemes are rewritten. gemini and gopher are on the roadmap and must pass
    // through untouched, as must file: and mailto:.
    if !matches!(url.scheme(), "http" | "https") {
        return url.clone();
    }
    // Credentials are an auth boundary: moving them to another host would leak them.
    if !url.username().is_empty() || url.password().is_some() {
        return url.clone();
    }
    let Some(host) = url.host_str() else {
        return url.clone();
    };

    for rule in rules {
        if !host_matches(host, rule.host) {
            continue;
        }
        if let Some(needle) = rule.path_contains
            && !url.path().contains(needle)
        {
            continue;
        }
        let mut out = url.clone();
        // An unparseable target host is a table bug, caught by the guard test. At runtime the
        // safe thing is to leave the user's URL alone rather than guess.
        if out.set_host(Some(rule.to_host)).is_err() {
            return url.clone();
        }
        return out;
    }
    url.clone()
}

/// Match at a label boundary.
///
/// This is the whole safety property of the layer: `evil-example.com` and
/// `example.com.attacker.net` must not match the rule `example.com`.
fn host_matches(host: &str, rule: &str) -> bool {
    host == rule
        || (host.len() > rule.len()
            && host.ends_with(rule)
            && host.as_bytes()[host.len() - rule.len() - 1] == b'.')
}

/// The effective table, for `--show-rewrites`.
///
/// With no config file, this is the only way to answer "why did my request go to a host I did
/// not type" without reading the source.
pub fn describe() -> String {
    if RULES.is_empty() {
        return "no per-site rules\n".to_string();
    }
    RULES
        .iter()
        .map(|r| {
            format!(
                "{}{} -> {}\n",
                r.host,
                r.path_contains
                    .map(|p| format!(" (path contains {p:?})"))
                    .unwrap_or_default(),
                r.to_host,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The mechanism tests run against this rather than the shipped table, so they keep
    /// testing the mechanism no matter what does or does not ship.
    const FIXTURE: &[Rule] = &[
        Rule {
            host: "example.com",
            path_contains: Some("/comments/"),
            to_host: "threads.example.com",
        },
        Rule {
            host: "example.com",
            path_contains: None,
            to_host: "old.example.com",
        },
    ];

    /// A table whose only rule is path-gated, so a non-matching path means no rule at all.
    const GATED: &[Rule] = &[Rule {
        host: "example.com",
        path_contains: Some("/comments/"),
        to_host: "threads.example.com",
    }];

    fn go(u: &str) -> String {
        apply_in(FIXTURE, &Url::parse(u).unwrap()).to_string()
    }

    #[test]
    fn exact_host_is_rewritten_and_everything_else_preserved() {
        assert_eq!(
            go("https://example.com:8443/a/b?q=1&r=2#frag"),
            "https://old.example.com:8443/a/b?q=1&r=2#frag"
        );
    }

    #[test]
    fn subdomain_matches_at_a_label_boundary() {
        for h in ["www.example.com", "np.example.com", "a.b.example.com"] {
            assert_eq!(
                go(&format!("https://{h}/x")),
                "https://old.example.com/x",
                "{h} was not rewritten"
            );
        }
    }

    #[test]
    fn lookalike_host_does_not_match() {
        // The label boundary is the safety property. A prefix-extended host is a different
        // site and must never be silently redirected.
        assert_eq!(
            go("https://evil-example.com/x"),
            "https://evil-example.com/x"
        );
    }

    #[test]
    fn rule_host_as_a_prefix_does_not_match() {
        assert_eq!(
            go("https://example.com.attacker.net/x"),
            "https://example.com.attacker.net/x"
        );
    }

    #[test]
    fn host_case_is_normalized_by_url_before_matching() {
        assert_eq!(go("HTTPS://EXAMPLE.COM/x"), "https://old.example.com/x");
    }

    #[test]
    fn apply_is_idempotent() {
        for input in [
            "https://example.com/comments/1",
            "https://www.example.com/x",
            "https://unrelated.test/x",
        ] {
            let once = apply_in(FIXTURE, &Url::parse(input).unwrap());
            assert_eq!(
                apply_in(FIXTURE, &once),
                once,
                "rewriting {input} twice moved it again"
            );
        }
    }

    #[test]
    fn unmatched_host_is_untouched() {
        assert_eq!(
            go("https://unrelated.test/a?b=c"),
            "https://unrelated.test/a?b=c"
        );
    }

    #[test]
    fn non_web_schemes_are_never_rewritten() {
        // gemini and gopher are on the roadmap; file: and mailto: are never ours to redirect.
        for u in [
            "gemini://example.com/x",
            "gopher://example.com/x",
            "file:///tmp/x",
            "mailto:someone@example.com",
        ] {
            let parsed = Url::parse(u).unwrap();
            assert_eq!(apply_in(FIXTURE, &parsed), parsed, "{u} was rewritten");
        }
    }

    #[test]
    fn urls_carrying_credentials_are_never_rewritten() {
        // Credentials are an auth boundary. Carrying them to another host would leak them.
        for u in [
            "https://user:pass@example.com/x",
            "https://user@example.com/x",
        ] {
            let parsed = Url::parse(u).unwrap();
            assert_eq!(apply_in(FIXTURE, &parsed), parsed, "{u} was rewritten");
        }
    }

    #[test]
    fn path_predicate_selects_the_rule() {
        assert_eq!(
            go("https://example.com/r/x/comments/abc/title/"),
            "https://threads.example.com/r/x/comments/abc/title/"
        );
        assert_eq!(
            go("https://example.com/r/x/"),
            "https://old.example.com/r/x/"
        );
    }

    #[test]
    fn a_path_gated_rule_does_not_fire_on_other_paths() {
        let u = Url::parse("https://example.com/r/x/").unwrap();
        assert_eq!(apply_in(GATED, &u), u);
    }

    #[test]
    fn first_match_wins() {
        // Both rules match the host; the earlier, narrower one must not be shadowed.
        assert!(go("https://example.com/comments/1").starts_with("https://threads."));
    }

    #[test]
    fn builtin_table_is_sound() {
        // The guard that keeps the shipped table from rotting silently.
        for (i, r) in RULES.iter().enumerate() {
            for h in [r.host, r.to_host] {
                let parsed = Url::parse(&format!("https://{h}/")).unwrap();
                assert_eq!(
                    parsed.host_str(),
                    Some(h),
                    "rule {i}: host {h:?} is not in the form Url produces, so it can never match"
                );
            }

            // `path_contains` is a substring of the path, so it must start at a slash. A
            // needle like "comments/" would build the probe `https://example.comcomments/`,
            // which parses fine, matches no rule, and makes every assertion below trivially
            // true: the guard would stop guarding this rule without failing.
            if let Some(p) = r.path_contains {
                assert!(
                    p.starts_with('/'),
                    "rule {i}: path_contains {p:?} must start with '/'"
                );
            }

            // A rule swaps hosts; it must never be able to produce a downgrade.
            let probe = Url::parse(&format!(
                "https://{}{}",
                r.host,
                r.path_contains.unwrap_or("/")
            ))
            .unwrap();
            let out = apply_in(RULES, &probe);
            assert_ne!(out, probe, "rule {i}: its own probe URL matched no rule");
            assert_eq!(out.scheme(), "https", "rule {i}: produced a non-https URL");
            assert_eq!(
                apply_in(RULES, &out),
                out,
                "rule {i}: apply is not idempotent"
            );

            // Reachability. Two rules on one host is the intended pattern when the earlier one
            // carries a path predicate; only an earlier *unconditional* rule shadows this one.
            assert!(
                !RULES[..i]
                    .iter()
                    .any(|e| host_matches(r.host, e.host) && e.path_contains.is_none()),
                "rule {i} is unreachable: an earlier rule matches {} unconditionally",
                r.host
            );
        }
    }
}
