//! Utils for GitHub-Zulip markdown processing:
//!
//! - [`zulipify_github_links_and_mentions`]

use pulldown_cmark::{CowStr, Event, LinkType, Options, Parser, Tag, TagEnd};
use regex::{Captures, Regex};
use std::{borrow::Cow, ops::Range, sync::LazyLock};

/// Shared pattern for a GitHub issue or PR URL, optionally followed
/// by a PR subpage and/or an issue/review comment anchor.
const ISSUE_URL_PAT: &str = r"
    https?://(?:www\.)?github\.com/
    (?P<owner>[[:word:].-]+)/
    (?P<repo>[[:word:].-]+)/
    (?P<kind>issues|pull)/
    (?P<number>[0-9]+)
    (?P<subpage>/(?:files|changes))?
    /?
    (?:
        \#(?P<comment>issuecomment-[0-9]+|discussion_r[0-9]+|r[0-9]+)
    )?
";

/// GitHub issue, PR, or comment link, e.g.,
/// - https://github.com/rust-lang/rust/issues/12345
/// - https://github.com/rust-lang/rust/issues/12345#issuecomment-12
/// - https://github.com/rust-lang/rust/pull/12345#discussion_r123
/// - https://github.com/rust-lang/rust/pull/12345/changes#r123
static ISSUE_URL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"(?ix)^(?:{ISSUE_URL_PAT})$")).unwrap());

/// Matches "zulipifiable" tokens (URLs, references, and mentions).
static ZULIPIFIABLE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?ix)

        # GitHub issue, PR, or comment link.
        (?P<url>{ISSUE_URL_PAT})

        |

        # Global issue or PR reference, e.g. rust-lang/rust#123.
        (?P<global>
            (?P<global_prefix>
                ^|[^\pL\pN_./\\-]
            )
            (?P<global_ref>
                (?P<global_owner>[[:word:].-]+)/
                (?P<global_repo>[[:word:].-]+)
                \#
                (?P<global_number>[0-9]+)
                \b
            )
        )

        |

        # Local issue or PR reference, e.g. #123.
        (?P<local>
            (?P<local_prefix>
                ^|[^\pL\pN_/\#\\]
            )
            (?P<local_ref>
                \#[0-9]+\b
            )
        )

        |

        # GitHub login, optionally followed by /team, e.g. @ghost or @rust-lang/goals.
        (?P<mention>
            (?P<mention_prefix>
                ^|[^\pL\pN_@\\]
            )
            (?P<mention_ref>
                @
                (?P<mention_name>
                    [[:word:]]
                    (?:[[:word:]-]*[[:word:]])?
                    (?:
                        /
                        [[:word:]]
                        (?:[[:word:]-]*[[:word:]])?
                    )?
                )
                \b
            )
        )
        ",
    ))
    .unwrap()
});

struct IssueReference<'source> {
    owner: &'source str,
    repo: &'source str,
    number: &'source str,
}

/// Checks whether a label is a recognized shorthand for the given issue.
fn label_matches_shorthand(
    label: &str,
    issue: &IssueReference<'_>,
    is_rustlang: bool,
    is_llvm: bool,
) -> bool {
    let Some((prefix, label_number)) = label.rsplit_once('#') else {
        return false;
    };
    if label_number != issue.number {
        return false;
    }
    match prefix.split_once('/') {
        _ if prefix.is_empty() => true,
        Some((label_owner, label_repo)) => {
            label_owner.eq_ignore_ascii_case(issue.owner)
                && label_repo.eq_ignore_ascii_case(issue.repo)
        }
        None if is_rustlang => prefix.eq_ignore_ascii_case(issue.repo),
        None => is_llvm && prefix.eq_ignore_ascii_case("llvm"),
    }
}

enum IssueShorthand<'source> {
    Rust {
        number: &'source str,
    },
    Rustlang {
        repo: &'source str,
        number: &'source str,
    },
    Global {
        owner: &'source str,
        repo: &'source str,
        number: &'source str,
    },
}

fn issue_shorthand<'source>(
    issue: IssueReference<'source>,
    source: &str,
    label: Option<&str>,
    is_url: bool,
    is_comment: bool,
) -> Option<IssueShorthand<'source>> {
    let is_rustlang = issue.owner.eq_ignore_ascii_case("rust-lang");
    let is_llvm =
        issue.owner.eq_ignore_ascii_case("llvm") && issue.repo.eq_ignore_ascii_case("llvm-project");

    if !is_rustlang && !is_llvm && !is_url {
        return None;
    }

    if let Some(label) = label
        && let label = label.trim()
        && !label.eq_ignore_ascii_case(source)
        && (is_comment || !label_matches_shorthand(label, &issue, is_rustlang, is_llvm))
    {
        return None;
    }

    Some(if is_rustlang {
        if issue.repo.eq_ignore_ascii_case("rust") {
            IssueShorthand::Rust {
                number: issue.number,
            }
        } else {
            IssueShorthand::Rustlang {
                repo: issue.repo,
                number: issue.number,
            }
        }
    } else if is_llvm {
        IssueShorthand::Rustlang {
            repo: "llvm",
            number: issue.number,
        }
    } else {
        IssueShorthand::Global {
            owner: issue.owner,
            repo: issue.repo,
            number: issue.number,
        }
    })
}

enum Replacement<'source, 'mention> {
    Issue {
        shorthand: IssueShorthand<'source>,
        comment_url: Option<&'source str>,
    },
    Mention(&'mention str),
}

fn replacement<'source, 'mention>(
    source: &'source str,
    label: Option<&str>,
    captures: &Captures<'source>,
    repo: &'source str,
    resolve_mention: &mut impl FnMut(&str) -> Option<&'mention str>,
) -> Option<(Range<usize>, Replacement<'source, 'mention>)> {
    let url = if label.is_some() {
        captures.get(0)
    } else {
        captures.name("url")
    };

    let (token, issue, is_comment) = if let Some(url) = url {
        // Do not shorten URLs followed by a query, fragment, or path (.../pull/59/files#diff-abc).
        if label.is_none()
            && let Some(next) = source[url.end()..].chars().next()
            && (matches!(next, '/' | '?' | '#')
                || (url.as_str().ends_with('/')
                    && (next.is_ascii_alphanumeric() || matches!(next, '_' | '-' | '~' | '%'))))
        {
            return None;
        }

        // Skip /files and /changes for non-PRs.
        if captures.name("subpage").is_some() && !captures["kind"].eq_ignore_ascii_case("pull") {
            return None;
        }

        (
            url,
            IssueReference {
                owner: captures.name("owner").unwrap().as_str(),
                repo: captures.name("repo").unwrap().as_str(),
                number: captures.name("number").unwrap().as_str(),
            },
            captures.name("comment").is_some(),
        )
    } else {
        let (token, issue) = if let Some(global) = captures.name("global_ref") {
            (
                global,
                Some(IssueReference {
                    owner: captures.name("global_owner").unwrap().as_str(),
                    repo: captures.name("global_repo").unwrap().as_str(),
                    number: captures.name("global_number").unwrap().as_str(),
                }),
            )
        } else if let Some(local) = captures.name("local_ref") {
            (
                local,
                Some(IssueReference {
                    owner: "rust-lang",
                    repo,
                    number: &local.as_str()[1..],
                }),
            )
        } else {
            (captures.name("mention_ref").unwrap(), None)
        };

        // Do not normalize issues or mentions inside URLs, e.g.
        // https://example.com/rust-lang/rust#12 or https://example.com/@alice.
        if source[..token.start()]
            .rsplit(char::is_whitespace)
            .next()
            .unwrap()
            .contains("://")
        {
            return None;
        }

        let Some(issue) = issue else {
            match source[token.end()..].as_bytes() {
                // Skip trailing paths (we already catch @org/repo and @org),
                // but allow stuff like @alice/@bob (i.e. "@alice or @bob").
                [b'/', rest @ ..] if !rest.starts_with(b"@") => return None,
                // Skip trailing dash.
                [b'-', ..] => return None,
                _ => {}
            }

            let mention = resolve_mention(&captures["mention_name"])?;
            return Some((token.range(), Replacement::Mention(mention)));
        };

        (token, issue, false)
    };

    Some((
        token.range(),
        Replacement::Issue {
            shorthand: issue_shorthand(issue, source, label, url.is_some(), is_comment)?,
            comment_url: is_comment.then(|| token.as_str()),
        },
    ))
}

fn append_replacement(output: &mut String, replacement: Replacement<'_, '_>) {
    match replacement {
        Replacement::Issue {
            shorthand,
            comment_url,
        } => {
            if comment_url.is_some() {
                output.push('[');
            }
            let number = match shorthand {
                IssueShorthand::Rust { number } => number,
                IssueShorthand::Rustlang { repo, number } => {
                    output.push_str(repo);
                    number
                }
                IssueShorthand::Global {
                    owner,
                    repo,
                    number,
                } => {
                    output.push_str(owner);
                    output.push('/');
                    output.push_str(repo);
                    number
                }
            };

            output.push('#');
            output.push_str(number);
            if let Some(comment_url) = comment_url {
                output.push_str(" (comment)](");
                output.push_str(comment_url);
                output.push(')');
            }
        }
        Replacement::Mention(mention) => output.push_str(mention),
    }
}

fn is_escaped_at(markdown: &str, position: usize) -> bool {
    // Text events can separate an escape from its token,
    // so count backslashes in the original Markdown.
    let backslashes = markdown[..position]
        .bytes()
        .rev()
        .take_while(|&byte| byte == b'\\')
        .count();
    backslashes % 2 != 0
}

struct Link<'markdown> {
    start: usize,
    dest_url: CowStr<'markdown>,
    // None means the link is reference-style or the label contains non-plain text,
    // and such links aren't normalized.
    label: Option<String>,
}

/// Turns issue, PR, and comment references and URLs to Zulip shorthands,
/// and lets the caller resolve usernames/teams to custom strings (e.g., Zulip mentions).
///
/// Returns the transformed string, or the borrowed original if nothing was replaced.
///
/// `repo` must be a `rust-lang` repository (e.g., `goals`).
///
/// # Examples
///
/// + `rust-lang/rust#123` and URLs => `#123`
/// + `rust-lang/<repo>#123` and URLs => `<repo>#123`
/// + `llvm/llvm-project#123` and URLs => `llvm#123`
/// + `#123` => `#123` in `rust-lang/rust`, or `<repo>#123` in `rust-lang/<repo>`
/// + `<owner>/<repo>#123` and URLs => `<owner>/<repo>#123`
/// + bare URLs to comments => `[<issue shorthand> (comment)](<url>)`
/// + Markdown links with non-shorthand labels and reference-style links are unchanged
/// + trailing `/files` and `/changes` in PR URLs are removed
///
/// # Mentions
///
/// - `@username` => `resolve_mention(username)` (leaves it alone if the callback returns `None`)
/// - `@rust-lang/team` => `resolve_mention(rust-lang/team)` (same as above)
pub fn zulipify_github_links_and_mentions<'markdown, 'repo, 'mention>(
    markdown: &'markdown str,
    repo: &'repo str,
    mut resolve_mention: impl FnMut(&str) -> Option<&'mention str>,
) -> Cow<'markdown, str> {
    const OPTIONS: Options = Options::empty()
        .union(Options::ENABLE_TABLES)
        .union(Options::ENABLE_STRIKETHROUGH)
        .union(Options::ENABLE_TASKLISTS)
        .union(Options::ENABLE_GFM);

    let mut output = String::new();
    let mut cursor = 0;

    let mut links: Vec<Link<'markdown>> = Vec::new();
    let mut image_depth = 0;
    let mut code_block = false;

    for (event, mut span) in Parser::new_ext(markdown, OPTIONS).into_offset_iter() {
        let (source, label) = match event {
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                ..
            }) => {
                links.push(Link {
                    start: span.start,
                    dest_url,
                    label: matches!(link_type, LinkType::Inline | LinkType::Autolink)
                        .then(String::new),
                });
                continue;
            }
            Event::End(TagEnd::Link) => {
                let Some(Link {
                    start,
                    dest_url,
                    label: Some(label),
                }) = links.pop()
                else {
                    continue;
                };
                span.start = start;
                (dest_url, Some(label))
            }
            Event::Start(Tag::Image { .. }) => {
                if let Some(link) = links.last_mut() {
                    link.label = None;
                }
                image_depth += 1;
                continue;
            }
            Event::End(TagEnd::Image) => {
                image_depth -= 1;
                continue;
            }
            Event::Start(Tag::CodeBlock(_)) => {
                code_block = true;
                continue;
            }
            Event::End(TagEnd::CodeBlock) => {
                code_block = false;
                continue;
            }
            Event::Text(text) if let Some(link) = links.last_mut() => {
                if let Some(label) = &mut link.label {
                    label.push_str(&text);
                }
                continue;
            }
            Event::Text(_) if image_depth == 0 && !code_block => {
                (CowStr::Borrowed(&markdown[span.clone()]), None)
            }
            _ => {
                if let Some(link) = links.last_mut() {
                    link.label = None;
                }
                continue;
            }
        };

        let captures = if label.is_some() {
            ISSUE_URL_RE.captures_iter(&source)
        } else {
            ZULIPIFIABLE_RE.captures_iter(&source)
        };

        for captures in captures {
            let Some((token_span, replacement)) = replacement(
                &source,
                label.as_deref(),
                &captures,
                repo,
                &mut resolve_mention,
            ) else {
                continue;
            };

            let edit_span = if label.is_some() {
                span.clone()
            } else {
                span.start + token_span.start..span.start + token_span.end
            };

            if label.is_none() && is_escaped_at(markdown, edit_span.start) {
                continue;
            }

            debug_assert!(
                cursor <= edit_span.start,
                "replacement overlaps previous edit"
            );

            if cursor == 0 {
                output.reserve(markdown.len());
            }

            output.push_str(&markdown[cursor..edit_span.start]);
            append_replacement(&mut output, replacement);
            cursor = edit_span.end;
        }
    }

    if cursor == 0 {
        Cow::Borrowed(markdown)
    } else {
        output.push_str(&markdown[cursor..]);
        Cow::Owned(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::assert_matches;

    const REPO: &str = "goals";

    #[track_caller]
    fn assert_normalizes(markdown: &str, expected: &str) {
        assert_eq!(
            zulipify_github_links_and_mentions(markdown, REPO, |_| None),
            expected
        );
    }

    #[track_caller]
    fn assert_normalizes_in(markdown: &str, repo: &str, expected: &str) {
        assert_eq!(
            zulipify_github_links_and_mentions(markdown, repo, |_| None),
            expected
        );
    }

    #[track_caller]
    fn assert_normalizes_with(
        markdown: &str,
        resolve_mention: impl FnMut(&str) -> Option<&'static str>,
        expected: &str,
    ) {
        assert_eq!(
            zulipify_github_links_and_mentions(markdown, REPO, resolve_mention),
            expected
        );
    }

    #[track_caller]
    fn assert_unchanged(markdown: &str) {
        match zulipify_github_links_and_mentions(markdown, REPO, |_| None) {
            Cow::Borrowed(result) => assert_eq!(result, markdown),
            Cow::Owned(result) => panic!("expected borrowed Markdown, got {result:?}"),
        }
    }

    #[track_caller]
    fn assert_unchanged_with(
        markdown: &str,
        resolve_mention: impl FnMut(&str) -> Option<&'static str>,
    ) {
        match zulipify_github_links_and_mentions(markdown, REPO, resolve_mention) {
            Cow::Borrowed(result) => assert_eq!(result, markdown),
            Cow::Owned(result) => panic!("expected borrowed Markdown, got {result:?}"),
        }
    }

    #[track_caller]
    fn assert_normalizes_comment(url: &str, reference: &str) {
        assert_normalizes(url, &format!("[{reference} (comment)]({url})"));
    }

    #[test]
    fn unchanged_markdown_is_borrowed() {
        assert_matches!(
            zulipify_github_links_and_mentions("Nothing to normalize.", REPO, |_| None),
            Cow::Borrowed("Nothing to normalize.")
        );
        assert_matches!(
            zulipify_github_links_and_mentions("bevyengine/bevy#18121", REPO, |_| None),
            Cow::Borrowed("bevyengine/bevy#18121")
        );
    }

    fn resolve_mention(mention: &str) -> Option<&'static str> {
        match mention {
            "alice" => Some("@_**|1**"),
            "rust-lang/lang" => Some("@*T-lang*"),
            _ => None,
        }
    }

    #[test]
    fn mentions_are_replaced_only_when_resolved() {
        let normalized = zulipify_github_links_and_mentions(
            "@alice @rust-lang/lang @ghost `@alice #12`",
            REPO,
            resolve_mention,
        );

        assert_eq!(normalized, "@_**|1** @*T-lang* @ghost `@alice #12`");
    }

    #[test]
    fn identical_replacements_are_owned() {
        assert_matches!(
            zulipify_github_links_and_mentions("#12", "rust", |_| None),
            Cow::Owned(result) if result == "#12"
        );
        assert_matches!(
            zulipify_github_links_and_mentions("@alice", REPO, |_| Some("@alice")),
            Cow::Owned(result) if result == "@alice"
        );
    }

    #[test]
    fn empty_mention_replacements_are_owned() {
        assert_matches!(
            zulipify_github_links_and_mentions("@alice", REPO, |_| Some("")),
            Cow::Owned(result) if result.is_empty()
        );
        assert_normalizes_with("@alice #12", |_| Some(""), " goals#12");
    }

    #[test]
    fn interleaved_edits_remain_in_source_order() {
        assert_normalizes_with(
            "#1 [#2](https://github.com/rust-lang/rust/issues/2) @alice #3",
            resolve_mention,
            "goals#1 #2 @_**|1** goals#3",
        );
    }

    #[test]
    fn local_references_use_the_current_rustlang_repo() {
        assert_normalizes_in("#12", "goals", "goals#12");
        assert_normalizes_in("#12", "rust", "#12");
    }

    #[test]
    fn global_references_use_zulip_shorthands() {
        assert_normalizes("rust-lang/rust#34", "#34");
        assert_normalizes("rust-lang/cargo#35", "cargo#35");
        assert_normalizes("llvm/llvm-project#36", "llvm#36");
    }

    #[test]
    fn bare_urls_use_zulip_shorthands() {
        assert_normalizes("https://github.com/rust-lang/cargo/pull/56/", "cargo#56");
        assert_normalizes(
            "https://github.com/Rust-Lang/CaRgO/issues/00056#IssueComment-09",
            "[CaRgO#00056 (comment)](https://github.com/Rust-Lang/CaRgO/issues/00056#IssueComment-09)",
        );
        assert_normalizes(
            "https://github.com/BevyEngine/BeVy/issues/0001",
            "BevyEngine/BeVy#0001",
        );
    }

    #[test]
    fn surrounding_markdown_is_handled() {
        assert_normalizes("<https://github.com/rust-lang/rust/issues/12>", "#12");
        assert_normalizes(
            "(https://github.com/rust-lang/cargo/pull/13),",
            "(cargo#13),",
        );
        assert_normalizes("before  \n> #12", "before  \n> goals#12");
    }

    #[test]
    fn shorthand_labeled_links_become_shorthands() {
        assert_normalizes("[#12](https://github.com/rust-lang/rust/issues/12)", "#12");
        assert_normalizes(
            "[cargo#14](https://github.com/rust-lang/cargo/issues/14)",
            "cargo#14",
        );
        assert_normalizes(
            "[rust-lang/cargo#16](https://github.com/rust-lang/cargo/issues/16)",
            "cargo#16",
        );
        assert_normalizes(
            "[bevyengine/bevy#18121](https://github.com/bevyengine/bevy/issues/18121)",
            "bevyengine/bevy#18121",
        );
    }

    #[test]
    fn mismatched_shorthand_labeled_links_do_not_normalize() {
        assert_normalizes(
            "[RUST#12](https://github.com/Rust-Lang/RuSt/issues/12)",
            "#12",
        );
        assert_normalizes(
            "[LLVM#12](https://github.com/LLVM/LLVM-Project/issues/12)",
            "llvm#12",
        );
        assert_normalizes(
            "[llvm/llvm-project#12](https://github.com/LLVM/LLVM-Project/issues/12)",
            "llvm#12",
        );
        assert_normalizes(
            "[#12](https://github.com/BevyEngine/BeVy/issues/12)",
            "BevyEngine/BeVy#12",
        );
        assert_unchanged("[#13](https://github.com/rust-lang/rust/issues/12)");
        assert_unchanged("[cargo#12](https://github.com/rust-lang/rust/issues/12)");
        assert_unchanged("[bevy#12](https://github.com/bevyengine/bevy/issues/12)");
        assert_unchanged("[#012](https://github.com/rust-lang/rust/issues/12)");
    }

    #[test]
    fn reference_style_links_are_unchanged() {
        assert_unchanged("[#12][pr]\n\n[pr]: https://github.com/rust-lang/rust/issues/12");
        assert_unchanged("[#12][]\n\n[#12]: https://github.com/rust-lang/rust/issues/12");
        assert_unchanged("[#12]\n\n[#12]: https://github.com/rust-lang/rust/issues/12");

        let url = "https://github.com/rust-lang/rust/pull/12#discussion_r2";
        assert_unchanged(&format!("[{url}][pr]\n\n[pr]: {url}"));
        assert_unchanged_with(
            "[@alice #12][pr]\n\n[pr]: https://github.com/rust-lang/rust/issues/12",
            |_| panic!("mentions inside reference-style links are not resolved"),
        );
    }

    #[test]
    fn bare_comment_urls_become_labeled_links() {
        assert_normalizes_comment(
            "https://github.com/rust-lang/rust/pull/56#issuecomment-1",
            "#56",
        );
        assert_normalizes_comment(
            "https://github.com/rust-lang/rust/pull/56#discussion_r2",
            "#56",
        );
    }

    #[test]
    fn comment_links_with_url_labels_become_labeled_links() {
        let url = "https://github.com/rust-lang/rust/pull/56#discussion_r2";
        assert_normalizes(
            &format!("[{url}]({url})"),
            &format!("[#56 (comment)]({url})"),
        );
    }

    #[test]
    fn pr_subpages_without_comments_become_shorthands() {
        assert_normalizes("https://github.com/rust-lang/rust/pull/56/files", "#56");
        assert_normalizes("https://github.com/rust-lang/rust/pull/56/changes/", "#56");
    }

    #[test]
    fn changes_review_comments_become_labeled_links() {
        assert_normalizes_comment(
            "https://github.com/rust-lang/team/pull/2651/changes#r3740816350",
            "team#2651",
        );
    }

    #[test]
    fn unsupported_pr_subpage_urls_are_unchanged() {
        assert_unchanged("https://github.com/rust-lang/rust/issues/58/changes");
        assert_unchanged("https://github.com/rust-lang/rust/pull/59/files#diff-123");
    }

    #[test]
    fn links_with_authored_labels_are_unchanged() {
        assert_unchanged("[details](https://github.com/rust-lang/cargo/pull/34)");
        assert_unchanged("[review](https://github.com/rust-lang/rust/pull/56#discussion_r2)");
        assert_unchanged("[#56](https://github.com/rust-lang/rust/pull/56#issuecomment-2)");
        assert_unchanged("[files](https://github.com/rust-lang/rust/pull/57/files)");
        assert_unchanged("[`#12`](https://github.com/rust-lang/rust/issues/12)");
        assert_unchanged("[see #13](https://example.com)");
        assert_unchanged("[pr](https://github.com/bevyengine/bevy/pull/20377)");
        assert_unchanged_with("[see @alice and #12](https://example.com)", resolve_mention);
    }

    #[test]
    fn references_inside_urls_are_handled() {
        assert_unchanged_with("https://example.com/@alice", resolve_mention);
        assert_unchanged("https://example.com/?issue=#12");
        assert_unchanged("https://example.com/?issue=rust-lang/rust#12");
        assert_unchanged("https://example.com#12");

        assert_normalizes(
            "[site](https://example.com)#12",
            "[site](https://example.com)goals#12",
        );
        assert_normalizes_with(
            "https://example.com @alice",
            resolve_mention,
            "https://example.com @_**|1**",
        );
        assert_normalizes_with(
            "https://example.com/ @alice",
            resolve_mention,
            "https://example.com/ @_**|1**",
        );
    }
}
