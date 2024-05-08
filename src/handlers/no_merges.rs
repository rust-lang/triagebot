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
    /// Labels that the bot added as part of the no-merges check.
    #[serde(default)]
    added_labels: Vec<String>,
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

    // Don't trigger if the PR has any of the excluded title segments.
    if config
        .exclude_titles
        .iter()
        .any(|s| event.issue.title.contains(s))
    {
        return Ok(None);
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

    // Run the handler even if we have no merge commits,
    // so we can take an action if some were removed.
    Ok(Some(NoMergesInput { merge_commits }))
}

fn get_default_message(repository_name: &str, default_branch: &str) -> String {
    format!(
        "
There are merge commits (commits with multiple parents) in your changes. We have a \
[no merge policy](https://rustc-dev-guide.rust-lang.org/git.html#no-merge-policy) \
so these commits will need to be removed for this pull request to be merged.

You can start a rebase with the following commands:
```shell-session
$ # rebase
$ git pull --rebase https://github.com/{repository_name}.git {default_branch}
$ git push --force-with-lease
```

"
    )
}

pub(super) async fn handle_input(
    ctx: &Context,
    config: &NoMergesConfig,
    event: &IssuesEvent,
    input: NoMergesInput,
) -> anyhow::Result<()> {
    let mut client = ctx.db.get().await;
    let mut state: IssueData<'_, NoMergesState> =
        IssueData::load(&mut client, &event.issue, NO_MERGES_KEY).await?;

    // No merge commits.
    if input.merge_commits.is_empty() {
        if state.data.mentioned_merge_commits.is_empty() {
            // No merge commits from before, so do nothing.
            return Ok(());
        }

        // Merge commits were removed, so remove the labels we added.
        for name in state.data.added_labels.iter() {
            event
                .issue
                .remove_label(&ctx.github, name)
                .await
                .context("failed to remove label")?;
        }

        // FIXME: Minimize prior no_merges comments.

        // Clear from state.
        state.data.mentioned_merge_commits.clear();
        state.data.added_labels.clear();
        state.save().await?;
        return Ok(());
    }

    let first_time = state.data.mentioned_merge_commits.is_empty();

    let mut message = config
        .message
        .as_deref()
        .unwrap_or(&get_default_message(
            &event.repository.full_name,
            &event.repository.default_branch,
        ))
        .to_string();

    let since_last_posted = if first_time {
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
        if !first_time {
            // Check if the labels are still set.
            // Otherwise, they were probably removed manually.
            let any_removed = state.data.added_labels.iter().any(|label| {
                // No label on the issue matches.
                event.issue.labels().iter().all(|l| &l.name != label)
            });

            if any_removed {
                // Assume it was a false positive, so don't
                // re-add the labels or send a message this time.
                state.save().await?;
                return Ok(());
            }
        }

        let existing_labels = event.issue.labels();

        let mut labels = Vec::new();
        for name in config.labels.iter() {
            // Only add labels not already on the issue.
            if existing_labels.iter().all(|l| &l.name != name) {
                state.data.added_labels.push(name.clone());
                labels.push(Label { name: name.clone() });
            }
        }

        // Set labels
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
        let mut message = get_default_message("foo/bar", "baz").to_string();

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
$ git pull --rebase https://github.com/foo/bar.git baz
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
