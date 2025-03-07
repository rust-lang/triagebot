//! Purpose: When opening a PR, or pushing new changes, check for github mentions
//! in commits and notify the user of our no-mentions in commits policy.

use std::fmt::Write;

use crate::{config::NoMentionsConfig, github::GithubCommit};

pub(super) fn mentions_in_commits(
    _conf: &NoMentionsConfig,
    commits: &[GithubCommit],
) -> Option<String> {
    let mut mentions_commits = Vec::new();

    for commit in commits {
        if !parser::get_mentions(&commit.commit.message).is_empty() {
            mentions_commits.push(&*commit.sha);
        }
    }

    if mentions_commits.is_empty() {
        None
    } else {
        Some(mentions_in_commits_warn(mentions_commits))
    }
}

fn mentions_in_commits_warn(commits: Vec<&str>) -> String {
    let mut warning = format!("There are username mentions (such as `@user`) in the commit messages of the following commits.\n  *Please remove the mentions to avoid spamming these users.*\n");

    for commit in commits {
        let _ = writeln!(warning, "    - {commit}");
    }

    warning
}

#[test]
fn test_warning_printing() {
    let commits_to_warn = vec![
        "4d6ze57403udfrzefrfe6574",
        "f54efz57405u46z6ef465z4f6ze57",
        "404u57403uzf5fe5f4f5e57405u4zf",
    ];

    let msg = mentions_in_commits_warn(commits_to_warn);

    assert_eq!(
        msg,
        r#"There are username mentions (such as `@user`) in the commit messages of the following commits.
  *Please remove the mentions to avoid spamming these users.*
    - 4d6ze57403udfrzefrfe6574
    - f54efz57405u46z6ef465z4f6ze57
    - 404u57403uzf5fe5f4f5e57405u4zf
"#
    );
}
