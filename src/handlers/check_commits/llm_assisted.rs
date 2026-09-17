use std::fmt::Write;
use std::sync::LazyLock;

use regex::Regex;

use crate::{github::GithubCommit, handlers::check_commits::MERGE_IGNORE_LIST};

static TRAILER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)(?P<key>[A-Za-z][A-Za-z0-9-]*):[ \t]*(?P<value>\S.*)$").unwrap()
});

/// Trailer keys indicating LLM assistance.
const LLM_TRAILER_KEYS: &[&str] = &["assisted-by", "claude-session"];

/// `Co-authored-by` values identifying an LLM agent.
/// Matched on the email, as "Claude" could possibly be a human name.
const LLM_CO_AUTHORS: &[&str] = &[
    "noreply@anthropic.com",
    "copilot@users.noreply.github.com",
    "cursoragent@cursor.com",
    "@aider.chat",
    "devin-ai-integration[bot]",
    "google-labs-jules[bot]",
];

fn llm_trailer(msg: &str) -> Option<&str> {
    TRAILER_RE.captures_iter(msg).find_map(|cap| {
        let key = cap.name("key").unwrap().as_str().to_lowercase();
        let value = cap.name("value").unwrap().as_str().to_lowercase();

        if (key == "co-authored-by" && LLM_CO_AUTHORS.iter().any(|a| value.contains(a)))
            || LLM_TRAILER_KEYS.contains(&&*key)
        {
            Some(cap.get(0).unwrap().as_str().trim_end())
        } else {
            None
        }
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LlmTrailer<'a> {
    sha: &'a str,
    trailer: &'a str,
}

pub(super) fn llm_trailers_in_commits(commits: &[GithubCommit]) -> Vec<LlmTrailer<'_>> {
    commits
        .iter()
        .filter(|c| {
            !MERGE_IGNORE_LIST
                .iter()
                .any(|i| c.commit.message.starts_with(i))
        })
        .filter_map(|c| {
            llm_trailer(&c.commit.message).map(|trailer| LlmTrailer {
                sha: &c.sha,
                trailer,
            })
        })
        .collect()
}

pub(super) fn warning(policy_url: &str, detected: &[LlmTrailer]) -> String {
    let mut message =
        "The following commits contain trailers indicating LLM assistance:\n".to_string();

    for LlmTrailer { sha, trailer } in detected {
        writeln!(message, "- {sha} (`{trailer}`)").unwrap();
    }
    writeln!(
        message,
        "Please make sure that this pull request complies with our [LLM policy]({policy_url})."
    )
    .unwrap();

    message
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::check_commits::dummy_commit_from_body;

    #[test]
    fn detects_llm_co_authored_by() {
        assert_eq!(
            llm_trailer("Fix a bug\n\nCo-Authored-By: Claude <noreply@anthropic.com>"),
            Some("Co-Authored-By: Claude <noreply@anthropic.com>")
        );
        assert_eq!(
            llm_trailer(
                "Fix a bug\n\nCo-authored-by: Copilot <175728472+Copilot@users.noreply.github.com>"
            ),
            Some("Co-authored-by: Copilot <175728472+Copilot@users.noreply.github.com>")
        );
    }

    #[test]
    fn detects_llm_assisted_by() {
        assert_eq!(
            llm_trailer("Fix a bug\n\nAssisted-by: Claude Code:claude-opus-5"),
            Some("Assisted-by: Claude Code:claude-opus-5")
        );
    }

    #[test]
    fn detects_llm_session() {
        assert_eq!(
            llm_trailer("Fix a bug\n\nClaude-Session: https://claude.ai/code/session_123  "),
            Some("Claude-Session: https://claude.ai/code/session_123")
        );
    }

    #[test]
    fn ignores_non_llm_trailers() {
        assert_eq!(
            llm_trailer("Fix a bug\n\nCo-authored-by: Claude Dupont <claude@dupont.com>"),
            None
        );
        assert_eq!(
            llm_trailer("Fix a bug\n\nSigned-off-by: Jane Doe <jane@example.com>"),
            None
        );
    }

    #[test]
    fn end_to_end() {
        let llm_commit = dummy_commit_from_body(
            "9cc6dce67c917fe5937e984f58f5003ccbb5c37e",
            "Add feature\n\nCo-Authored-By: Claude <noreply@anthropic.com>",
        );
        let non_llm_commit =
            dummy_commit_from_body("499bdd2d766f98420c66a80a02b7d3ceba4d06ba", "Fix typo");
        let merge_commit = dummy_commit_from_body(
            "febd545030008f13541064895ae36e19d929a043",
            "Merge pull request #1 from foo/bar\n\nCo-Authored-By: Claude <noreply@anthropic.com>",
        );

        let commits = [llm_commit, non_llm_commit, merge_commit];
        let detected = llm_trailers_in_commits(&commits);
        assert_eq!(
            detected.as_slice(),
            &[LlmTrailer {
                sha: "9cc6dce67c917fe5937e984f58f5003ccbb5c37e",
                trailer: "Co-Authored-By: Claude <noreply@anthropic.com>",
            }]
        );

        assert_eq!(
            warning("https://example.com/llm-policy", &detected),
            r#"The following commits contain trailers indicating LLM assistance:
- 9cc6dce67c917fe5937e984f58f5003ccbb5c37e (`Co-Authored-By: Claude <noreply@anthropic.com>`)
Please make sure that this pull request complies with our [LLM policy](https://example.com/llm-policy).
"#
        );
    }
}
