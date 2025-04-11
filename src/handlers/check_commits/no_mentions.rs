//! Purpose: When opening a PR, or pushing new changes, check for github mentions
//! in commits and notify the user of our no-mentions in commits policy.

use crate::{config::NoMentionsConfig, github::GithubCommit};

pub(super) fn mentions_in_commits(
    _conf: &NoMentionsConfig,
    commits: &[GithubCommit],
) -> Option<String> {
    let mentions_commits = commits
        .into_iter()
        .filter(|c| !parser::get_mentions(&c.commit.message).is_empty())
        .map(|c| format!("    - {}\n", c.sha))
        .collect::<String>();

    if mentions_commits.is_empty() {
        None
    } else {
        Some(format!(
            r"There are username mentions (such as `@user`) in the commit messages of the following commits.
  *Please remove the mentions to avoid spamming these users.*
{mentions_commits}",
        ))
    }
}

#[test]
fn test_mentions_in_commits() {
    fn dummy_commit_from_body(sha: &str, body: &str) -> GithubCommit {
        use chrono::{DateTime, FixedOffset};

        GithubCommit {
            sha: sha.to_string(),
            commit: crate::github::GithubCommitCommitField {
                author: crate::github::GitUser {
                    date: DateTime::<FixedOffset>::MIN_UTC.into(),
                },
                message: body.to_string(),
                tree: crate::github::GitCommitTree {
                    sha: "60ff73dfdd81aa1e6737eb3dacdfd4a141f6e14d".to_string(),
                },
            },
            parents: vec![],
        }
    }

    let mut commits = vec![dummy_commit_from_body(
        "d1992a392617dfb10518c3e56446b6c9efae38b0",
        "This is simple without mentions!",
    )];

    assert_eq!(mentions_in_commits(&NoMentionsConfig {}, &commits), None);

    commits.push(dummy_commit_from_body(
        "d7daa17bc97df9377640b0d33cbd0bbeed703c3a",
        "This is a body with a @mention!",
    ));

    assert_eq!(
        mentions_in_commits(&NoMentionsConfig {}, &commits),
        Some(
            r"There are username mentions (such as `@user`) in the commit messages of the following commits.
  *Please remove the mentions to avoid spamming these users.*
    - d7daa17bc97df9377640b0d33cbd0bbeed703c3a
".to_string()
        )
    );
}
