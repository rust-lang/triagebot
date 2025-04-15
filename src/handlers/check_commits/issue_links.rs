use std::sync::LazyLock;

use regex::Regex;

use crate::{config::IssueLinksConfig, github::GithubCommit};

static LINKED_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"( |^)([a-zA-Z-_]+/[a-zA-Z-_]+)?(#[0-9]+)\b").unwrap());

pub(super) fn issue_links_in_commits(
    _conf: &IssueLinksConfig,
    commits: &[GithubCommit],
) -> Option<String> {
    let issue_links_commits = commits
        .into_iter()
        .filter(|c| LINKED_RE.is_match(&c.commit.message))
        .map(|c| format!("    - {}\n", c.sha))
        .collect::<String>();

    if issue_links_commits.is_empty() {
        None
    } else {
        Some(format!(
            r"There are issue links (such as `#123`) in the commit messages of the following commits.
  *Please remove them as they will spam the issue with references to the commit.*
{issue_links_commits}",
        ))
    }
}

#[test]
fn test_mentions_in_commits() {
    use super::dummy_commit_from_body;

    let config = IssueLinksConfig {};

    let mut commits = vec![dummy_commit_from_body(
        "d1992a392617dfb10518c3e56446b6c9efae38b0",
        "This is simple without issue links!",
    )];

    assert_eq!(issue_links_in_commits(&config, &commits), None);

    commits.push(dummy_commit_from_body(
        "d7daa17bc97df9377640b0d33cbd0bbeed703c3a",
        "This is a body with a issue link #123.",
    ));

    assert_eq!(
        issue_links_in_commits(&config, &commits),
        Some(
            r"There are issue links (such as `#123`) in the commit messages of the following commits.
  *Please remove them as they will spam the issue with references to the commit.*
    - d7daa17bc97df9377640b0d33cbd0bbeed703c3a
".to_string()
        )
    );

    commits.push(dummy_commit_from_body(
        "891f0916a07c215ae8173f782251422f1fea6acb",
        "This is a body with a issue link rust-lang/rust#123.",
    ));

    assert_eq!(
        issue_links_in_commits(&config, &commits),
        Some(
            r"There are issue links (such as `#123`) in the commit messages of the following commits.
  *Please remove them as they will spam the issue with references to the commit.*
    - d7daa17bc97df9377640b0d33cbd0bbeed703c3a
    - 891f0916a07c215ae8173f782251422f1fea6acb
".to_string()
        )
    );
}
