//! Per-site rules: the only place in the crate where behaviour is keyed on a hostname.
//!
//! Everything else in hww is structural: `html.rs` scores subtrees, `thread.rs` groups siblings
//! by class signature, `cards.rs` reads card lists, `bin/corpus.rs` classifies by URL shape.
//! None of them know a domain name and none of them should, and none of them import this
//! module. Two tables live here, both keyed the same way ([`host_matches`], a label boundary):
//!
//! - [`RULES`]: a host swap, applied to the entry URL before the request goes out. Phase 0
//!   measured ~7 genuine JS-only reading losses out of 87 document-shaped pages; two of them
//!   are pages of one JS-gated discussion site that serves a fully server-rendered alternate
//!   domain, same operator, same content. Reaching it is a host swap, and that is all a rule
//!   does. See `docs/phase0-findings.md`, Phase 2.
//! - [`PROFILES`]: a [`Profile`] applied to the page that arrived, keyed on the final URL.
//!   Phase 3 measured the top-ten US news sites after the generic fixes and found what only a
//!   selector can know: in-body chrome on otherwise good articles. The vocabulary is one
//!   field, `strip`, and the charter below is why it is data and not code.
//!
//! # Charter for the builtin tables
//!
//! A rewrite rule may ship compiled in only if all four hold:
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
//! A profile may ship compiled in only if all five hold:
//!
//! 1. **Non-network.** A profile changes nothing about what is fetched or from where. It is
//!    applied to bytes already in hand.
//! 2. **Public.** The host is a widely read public site, so naming it discloses nothing about
//!    whose browsing the measurement came from. A niche host does not belong in this table.
//! 3. **Cited.** A structural row in `docs/phase0-findings.md`, Phase 3, with the measured
//!    delta, and a fixture in the table itself that `every_profile_earns_its_keep` runs.
//! 4. **Offline.** Compiled in, as above.
//! 5. **Reported.** Every field's outcome is printed on stderr after the page, listed by
//!    `--why`, and shown in the page-info panel; a field that matched nothing is named. A
//!    profile never *replaces* a measured choice: `strip` removes subtrees before scoring and
//!    is refused when it would remove most of the page.
//!
//! The earlier per-site *extraction* hint (`force_thread`) was a boolean override of a measured
//! merge and destroyed 73% of a page; it was removed as mechanism without a caller. A profile
//! is the opposite shape: data, a floor, a report, and a caller for every field it has.

use crate::profile::Profile;
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

/// One host's profile, and the fixture that proves it earns its place.
pub struct SiteProfile {
    /// Same form and matching as [`Rule::host`].
    pub host: &'static str,
    /// Same as [`Rule::path_contains`]: a profile written for article pages does not fire,
    /// or report itself dead, on the host's front page.
    pub path_contains: Option<&'static str>,
    pub profile: Profile,
    /// Synthetic markup in the shape the profile was written for: lorem text, the selectors'
    /// own class names, nothing pasted from a page. `every_profile_earns_its_keep` (in
    /// `session`, which imports both this and the extractor) runs it with and without the
    /// profile and asserts that every field matched and the document changed.
    pub fixture: &'static str,
}

/// One profile per measured host. Hosts here are the sites that need extra
/// extraction rules. A host not listed here relies on default extraction rules.
/// Each fixture is the shape the selector was written for, in
/// lorem, and `every_profile_earns_its_keep` runs it.
///
/// **Keep the table sorted by `host`**, alphabetically, so a reader can find a host. Lookup is
/// first-match-wins, which only matters for two entries on the same host, and the soundness
/// test checks both the order and that no path-gated entry is shadowed.
pub const PROFILES: &[SiteProfile] = &[
    SiteProfile {
        host: "apnews.com",
        path_contains: Some("/article/"),
        profile: Profile {
            strip: crate::profile::sel(".CarouselOverlay-carousel"),
        },
        fixture: "<html><body><main><h1>A headline about a thing</h1>\
            <bsp-carousel class=\"CarouselOverlay-carousel\"><div class=\"CarouselOverlay-slides\">\
            <div class=\"CarouselOverlay-slide\"><p>A person speaks at a lectern, Friday, in a town. (Photo/Name)</p></div>\
            <div class=\"CarouselOverlay-slide\"><p>Workers hold signs ahead of a speech, Friday, in a town. (Photo/Name)</p></div>\
            </div></bsp-carousel>\
            <p>Body prose long enough to clear the floor, and then some more of it for good measure.</p>\
            <p>Body prose long enough to clear the floor, and then some more of it for good measure.</p>\
            <p>Body prose long enough to clear the floor, and then some more of it for good measure.</p>\
            </main></body></html>",
    },
    SiteProfile {
        host: "cnn.com",
        path_contains: None,
        profile: Profile {
            strip: crate::profile::sel("[class^=\"follow-topics-bar\"]"),
        },
        fixture: "<html><body><article><h1>A headline about a thing</h1>\
            <div class=\"follow-topics-bar\"><button>Crime</button>\
            <a class=\"follow-topics-bar_overlay__explore-more-link\" href=\"/follow\">See all topics</a></div>\
            <p>Body prose long enough to clear the floor, and then some more of it for good measure.</p>\
            <p>Body prose long enough to clear the floor, and then some more of it for good measure.</p>\
            </article></body></html>",
    },
    SiteProfile {
        host: "corriere.it",
        path_contains: None,
        profile: Profile {
            strip: crate::profile::sel("a[href*=\"/newsletter/\"]"),
        },
        fixture: "<html><body><article><h1>A headline about a thing</h1>\
            <p>Body prose long enough to clear the floor, and then some more of it for good measure.</p>\
            <p>Body prose long enough to clear the floor, and then some more of it for good measure.</p>\
            <p><b><a href=\"https://example.test/newsletter/?theme=1\">Iscriviti alla newsletter, una riga lunga abbastanza</a></b></p>\
            <p>22 agosto 2026 (modifica il 22 agosto 2026 | 17:25)</p>\
            </article></body></html>",
    },
    SiteProfile {
        host: "dailymaverick.co.za",
        path_contains: None,
        profile: Profile {
            strip: crate::profile::sel(".comments-container"),
        },
        fixture: "<html><body><article><h1>A headline about a thing</h1>\
            <p>Body prose long enough to clear the floor, and then some more of it for good measure.</p>\
            <p>Body prose long enough to clear the floor, and then some more of it for good measure.</p>\
            <div class=\"comments-container\"><h2>Comments</h2><p>Scroll down to load comments, a line long \
            enough to be a paragraph of its own.</p></div>\
            </article></body></html>",
    },
    SiteProfile {
        host: "elmundo.es",
        path_contains: None,
        profile: Profile {
            strip: crate::profile::sel(".comments-panel"),
        },
        fixture: "<html><body><article><h1>A headline about a thing</h1>\
            <p>Body prose long enough to clear the floor, and then some more of it for good measure.</p>\
            <p>Body prose long enough to clear the floor, and then some more of it for good measure.</p>\
            </article><div class=\"right-panel comments-panel\"><a href=\"#\" class=\"boton-volver\">\
            <span>Volver a la noticia</span>A headline about a thing, repeated at its full length for the \
            control, which on the page runs to a hundred characters and more</a></div>\
            </body></html>",
    },
    SiteProfile {
        host: "folha.uol.com.br",
        path_contains: None,
        profile: Profile {
            strip: crate::profile::sel(".c-modal-drop, .c-sign-in"),
        },
        fixture: "<html><body><article><h1>A headline about a thing</h1>\
            <div class=\"c-modal-drop\"><div class=\"c-modal-drop__text\"><p>Recurso exclusivo para assinantes, \
            uma linha longa o suficiente para passar.</p><p><a href=\"/assine\">assine</a> ou <a href=\"#\">faça login</a></p></div></div>\
            <p>Body prose long enough to clear the floor, and then some more of it for good measure.</p>\
            <p>Body prose long enough to clear the floor, and then some more of it for good measure.</p>\
            </article></body></html>",
    },
    SiteProfile {
        host: "foxnews.com",
        path_contains: None,
        profile: Profile {
            strip: crate::profile::sel(".beyondwords-wrapper, a[href*=\"onelink.me\"]"),
        },
        fixture: "<html><body><article><h1>A headline about a thing</h1>\
            <div class=\"beyondwords-wrapper\"><div class=\"info\"><span class=\"label-bg\">NEW</span>\
            You can now listen to articles!</div></div>\
            <p>Body prose long enough to clear the floor, and then some more of it for good measure.</p>\
            <p>Body prose long enough to clear the floor, and then some more of it for good measure.</p>\
            <p><strong><a href=\"https://example.onelink.me/x\">CLICK HERE TO DOWNLOAD THE APP</a></strong></p>\
            <p>Body prose long enough to clear the floor, and then some more of it for good measure.</p>\
            </article></body></html>",
    },
    SiteProfile {
        host: "independent.co.uk",
        path_contains: None,
        profile: Profile {
            strip: crate::profile::sel(".show-comments"),
        },
        fixture: "<html><body><article><h1>A headline about a thing</h1>\
            <p>Body prose long enough to clear the floor, and then some more of it for good measure.</p>\
            <p>Body prose long enough to clear the floor, and then some more of it for good measure.</p>\
            <div class=\"show-comments\"><h2>Join our commenting forum</h2><p>Join thought-provoking \
            conversations, follow other readers and see their replies, a line long enough to pass.</p></div>\
            </article></body></html>",
    },
    // A cookie-consent banner's copy rendered inline ("you have chosen to refuse cookies ...
    // refuse and subscribe") under every article. `banner` is a chrome hint, but the page
    // holds most of its text in ad slots, so the root is walked with hints off (see
    // `html::blocks_guarded`) and the banner comes through; the profile is what removes it.
    // The fixture has the same shape: more hinted text than prose. Phase 3, fourth sample.
    SiteProfile {
        host: "lefigaro.fr",
        path_contains: None,
        profile: Profile {
            strip: crate::profile::sel(".fig-consent-banner"),
        },
        fixture: "<html><body><main>\
            <h1>A headline about a thing</h1>\
            <p>Body prose long enough to clear the floor, and then some more of it for good measure.</p>\
            <div class=\"fig-ad-content\"><p>Ad slot copy, long enough to outweigh the prose of the page \
            several times over, repeated so the chrome hints would remove most of the root's text. \
            Ad slot copy, long enough to outweigh the prose of the page several times over, repeated \
            so the chrome hints would remove most of the root's text. Ad slot copy, long enough to \
            outweigh the prose of the page several times over.</p></div>\
            <div class=\"fig-consent-banner\"><div class=\"fig-consent-banner__content\">Et pourtant, la publicité \
            personnalisée est un moyen de soutenir le travail de notre rédaction, une ligne assez longue.</div></div>\
            </main></body></html>",
    },
    SiteProfile {
        host: "news24.com",
        path_contains: None,
        profile: Profile {
            strip: crate::profile::sel(".gifting, .traffic-widget"),
        },
        fixture: "<html><body><article><h1>A headline about a thing</h1>\
            <div class=\"gifting\"><div class=\"gifting__description\">You have <b>5 articles</b> to share \
            every month. Send this story to a friend, a line long enough to pass!</div></div>\
            <p>Body prose long enough to clear the floor, and then some more of it for good measure.</p>\
            <p>Body prose long enough to clear the floor, and then some more of it for good measure.</p>\
            <div class=\"traffic-widget\"><div class=\"traffic-widget__title\">Editorial feedback</div>\
            <p class=\"traffic-widget__blurb\">Contact the public editor with feedback for our journalists, a line long enough.</p></div>\
            </article></body></html>",
    },
    SiteProfile {
        host: "scmp.com",
        path_contains: None,
        profile: Profile {
            strip: crate::profile::sel("[data-qa^=\"TextToSpeechPlayer\"]"),
        },
        fixture: "<html><body><article><h1>A headline about a thing</h1>\
            <p>Body prose long enough to clear the floor, and then some more of it for good measure.</p>\
            <p>Body prose long enough to clear the floor, and then some more of it for good measure.</p>\
            <div data-qa=\"TextToSpeechPlayer-TextContainer\"><div data-qa=\"TextToSpeechPlayer-TitleText\">\
            Select Speed 0.8x 0.9x 1.0x 1.1x 1.2x 1.5x 1.75x, a line long enough to be a paragraph.</div></div>\
            </article></body></html>",
    },
    SiteProfile {
        host: "straitstimes.com",
        path_contains: None,
        profile: Profile {
            strip: crate::profile::sel("[data-testid=\"headline-stack-promo-liner-test-id\"]"),
        },
        fixture: "<html><body><article><h1>A headline about a thing</h1>\
            <div data-testid=\"headline-stack-promo-liner-test-id\"><p><a href=\"/newsletter-signup\">Sign up now:</a> \
            Get insights on the region's fast-moving developments, a line long enough to pass.</p></div>\
            <p>Body prose long enough to clear the floor, and then some more of it for good measure.</p>\
            <p>Body prose long enough to clear the floor, and then some more of it for good measure.</p>\
            </article></body></html>",
    },
];

/// The profile for the page that arrived at `url`, and the host it was matched on. First
/// match wins; http(s) only.
pub fn profile_for(url: &Url) -> Option<(&'static str, &'static Profile)> {
    profile_for_in(PROFILES, url)
}

fn profile_for_in<'t>(table: &'t [SiteProfile], url: &Url) -> Option<(&'static str, &'t Profile)> {
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?;
    table
        .iter()
        .find(|p| {
            host_matches(host, p.host)
                && p.path_contains
                    .is_none_or(|needle| url.path().contains(needle))
        })
        .map(|p| (p.host, &p.profile))
}

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
/// Both tables, for `--show-rewrites` and `--show-profiles`.
pub fn describe() -> String {
    format!("{}{}", describe_rewrites(), describe_profiles())
}

pub fn describe_rewrites() -> String {
    if RULES.is_empty() {
        return "no per-site rewrite rules\n".to_string();
    }
    RULES
        .iter()
        .map(|r| {
            format!(
                "rewrite {}{} -> {}\n",
                r.host,
                r.path_contains
                    .map(|p| format!(" (path contains {p:?})"))
                    .unwrap_or_default(),
                r.to_host,
            )
        })
        .collect()
}

pub fn describe_profiles() -> String {
    if PROFILES.is_empty() {
        return "no per-site profiles\n".to_string();
    }
    PROFILES
        .iter()
        .map(|p| {
            let fields: Vec<String> = p
                .profile
                .fields()
                .iter()
                .map(|(k, v)| format!("{k} {v:?}"))
                .collect();
            format!(
                "profile {}{}: {}\n",
                p.host,
                p.path_contains
                    .map(|q| format!(" (path contains {q:?})"))
                    .unwrap_or_default(),
                fields.join(", "),
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

    const PROFILE_FIXTURE: &[SiteProfile] = &[
        SiteProfile {
            host: "example.com",
            path_contains: Some("/story/"),
            profile: Profile {
                strip: crate::profile::sel(".banner"),
            },
            fixture: "",
        },
        SiteProfile {
            host: "example.com",
            path_contains: None,
            profile: Profile {
                strip: crate::profile::sel(".promo"),
            },
            fixture: "",
        },
    ];

    fn pf(u: &str) -> Option<&'static str> {
        profile_for_in(PROFILE_FIXTURE, &Url::parse(u).unwrap())
            .map(|(_, p)| p.strip.as_deref().unwrap())
    }

    #[test]
    fn profile_lookup_matches_at_a_label_boundary_and_honours_the_path_gate() {
        assert_eq!(pf("https://example.com/story/1"), Some(".banner"));
        assert_eq!(pf("https://www.example.com/story/1"), Some(".banner"));
        assert_eq!(pf("https://example.com/"), Some(".promo"));
        assert_eq!(pf("https://evil-example.com/story/1"), None);
        assert_eq!(pf("https://example.com.attacker.net/story/1"), None);
        assert_eq!(pf("gemini://example.com/story/1"), None);
    }

    /// `docs/sites-checked.md` is the ledger of hosts measured; a profile for a host nobody
    /// has recorded checking is a profile with no measurement behind it.
    #[test]
    fn every_profile_host_is_in_the_ledger() {
        let ledger = include_str!("../docs/sites-checked.md");
        for p in PROFILES {
            assert!(
                ledger.contains(&format!("| {} |", p.host)),
                "{}: no row in docs/sites-checked.md",
                p.host
            );
        }
    }

    #[test]
    fn every_selector_in_the_table_parses_and_every_profile_is_sound() {
        for (i, p) in PROFILES.iter().enumerate() {
            let probe = Url::parse(&format!("https://{}/", p.host));
            assert_eq!(
                probe
                    .as_ref()
                    .ok()
                    .and_then(|u| u.host_str().map(str::to_owned)),
                Some(p.host.to_owned()),
                "profile {i}: host {:?} is not in the form Url produces",
                p.host
            );
            if let Some(needle) = p.path_contains {
                assert!(
                    needle.starts_with('/'),
                    "profile {i}: path needle {needle:?}"
                );
            }
            assert!(!p.profile.is_none(), "profile {i}: no field set");
            assert!(!p.fixture.trim().is_empty(), "profile {i}: no fixture");
            for (field, s) in p.profile.fields() {
                assert!(
                    scraper::Selector::parse(s).is_ok(),
                    "profile {i}: {field} selector {s:?} does not parse"
                );
            }
            // Sorted by host, as the table's doc comment asks, so the table reads as a list
            // of hosts and a new entry has one place to go.
            if let Some(prev) = PROFILES[..i].last() {
                assert!(
                    prev.host <= p.host,
                    "profile {i} ({}) is out of order after {}: keep PROFILES sorted by host",
                    p.host,
                    prev.host
                );
            }
            // Reachable: no earlier unconditional profile on the same host.
            for q in &PROFILES[..i] {
                assert!(
                    !(host_matches(p.host, q.host) && q.path_contains.is_none()),
                    "profile {i} is shadowed by an earlier profile on {}",
                    q.host
                );
            }
        }
    }
}
