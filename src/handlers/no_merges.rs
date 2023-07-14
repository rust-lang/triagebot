//! Purpose: When opening a PR, or pushing new changes, check for merge commits
//! and notify the user of our no-merge policy.

use crate::{
    config::NoMergesConfig,
    db::issue_data::IssueData,
    github::{IssuesAction, IssuesEvent, Label},
    handlers::Context,
};
use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt::Write;
use tracing as log;

const NO_MERGES_KEY: &str = "no_merges";

pub(super) struct NoMergesInput {
    /// Hashes of merge commits in the pull request.
    merge_commits: HashSet<String>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct NoMergesState {
    /// Hashes of merge commits that have already been mentioned by triagebot in a comment.
    mentioned_merge_commits: HashSet<String>,
}

pub(super) async fn parse_input(
    ctx: &Context,
    event: &IssuesEvent,
    config: Option<&NoMergesConfig>,
) -> Result<Option<NoMergesInput>, String> {
    if !matches!(
        event.action,
        IssuesAction::Opened | IssuesAction::Synchronize | IssuesAction::ReadyForReview
    ) {
        return Ok(None);
    }

    // Require a `[no_merges]` configuration block to enable no-merges notifications.
    let Some(config) = config else {
        return Ok(None);
    };

    // Don't ping on rollups or draft PRs.
    if event.issue.title.starts_with("Rollup of") || event.issue.draft {
        return Ok(None);
    }

    // Don't trigger if the PR has any of the excluded labels.
    for label in event.issue.labels() {
        if config.exclude_labels.contains(&label.name) {
            return Ok(None);
        }
    }

    let mut merge_commits = HashSet::new();
    let commits = event
        .issue
        .commits(&ctx.github)
        .await
        .map_err(|e| {
            log::error!("failed to fetch commits: {:?}", e);
        })
        .unwrap_or_default();
    for commit in commits {
        if commit.parents.len() > 1 {
            merge_commits.insert(commit.sha.clone());
        }
    }

    let input = NoMergesInput { merge_commits };
    Ok(if input.merge_commits.is_empty() {
        None
    } else {
        Some(input)
    })
}

const DEFAULT_MESSAGE: &str = "
There are merge commits (commits with multiple parents) in your changes. We have a \
[no merge policy](https://rustc-dev-guide.rust-lang.org/git.html#no-merge-policy) \
so these commits will need to be removed for this pull request to be merged.

You can start a rebase with the following commands:
```shell-session
$ # rebase
$ git rebase -i master
$ # delete any merge commits in the editor that appears
$ git push --force-with-lease
```

";

pub(super) async fn handle_input(
    ctx: &Context,
    config: &NoMergesConfig,
    event: &IssuesEvent,
    input: NoMergesInput,
) -> anyhow::Result<()> {
    let mut client = ctx.db.get().await;
    let mut state: IssueData<'_, NoMergesState> =
        IssueData::load(&mut client, &event.issue, NO_MERGES_KEY).await?;

    let mut message = config
        .message
        .as_deref()
        .unwrap_or(DEFAULT_MESSAGE)
        .to_string();

    let since_last_posted = if state.data.mentioned_merge_commits.is_empty() {
        ""
    } else {
        " (since this message was last posted)"
    };
    writeln!(
        message,
        "The following commits are merge commits{since_last_posted}:"
    )
    .unwrap();

    let mut should_send = false;
    for commit in &input.merge_commits {
        if state.data.mentioned_merge_commits.contains(commit) {
            continue;
        }

        should_send = true;
        state.data.mentioned_merge_commits.insert((*commit).clone());
        writeln!(message, "- {commit}").unwrap();
    }

    if should_send {
        // Set labels
        let labels = config
            .labels
            .iter()
            .cloned()
            .map(|name| Label { name })
            .collect();
        event
            .issue
            .add_labels(&ctx.github, labels)
            .await
            .context("failed to set no_merges labels")?;

        // Post comment
        event
            .issue
            .post_comment(&ctx.github, &message)
            .await
            .context("failed to post no_merges comment")?;
        state.save().await?;
    }
    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn message() {
        let mut message = DEFAULT_MESSAGE.to_string();

        writeln!(message, "The following commits are merge commits:").unwrap();

        for n in 1..5 {
            writeln!(message, "- commit{n}").unwrap();
        }

        assert_eq!(
            message,
            "
There are merge commits (commits with multiple parents) in your changes. We have a [no merge policy](https://rustc-dev-guide.rust-lang.org/git.html#no-merge-policy) so these commits will need to be removed for this pull request to be merged.

You can start a rebase with the following commands:
```shell-session
$ # rebase
$ git rebase -i master
$ # delete any merge commits in the editor that appears
$ git push --force-with-lease
```

The following commits are merge commits:
- commit1
- commit2
- commit3
- commit4
"
        );
    }
}
