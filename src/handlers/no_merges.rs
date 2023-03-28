//! Purpose: When opening a PR, or pushing new changes, check for merge commits
//! and notify the user of our no-merge policy.

use crate::{
    config::NoMergesConfig,
    db::issue_data::IssueData,
    github::{IssuesAction, IssuesEvent},
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

    // Require an empty configuration block to enable no-merges notifications.
    if config.is_none() {
        return Ok(None);
    }

    // Don't ping on rollups or draft PRs.
    if event.issue.title.starts_with("Rollup of") || event.issue.draft {
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

    let input = NoMergesInput { merge_commits };
    Ok(if input.merge_commits.is_empty() {
        None
    } else {
        Some(input)
    })
}

pub(super) async fn handle_input(
    ctx: &Context,
    _config: &NoMergesConfig,
    event: &IssuesEvent,
    input: NoMergesInput,
) -> anyhow::Result<()> {
    let mut connection = ctx.db.connection().await;
    let repo = event.issue.repository().to_string();
    let issue_number = event.issue.number as i32;
    let mut state: IssueData<NoMergesState> =
        IssueData::load(&mut *connection, repo, issue_number, NO_MERGES_KEY).await?;

    let since_last_posted = if state.data.mentioned_merge_commits.is_empty() {
        ""
    } else {
        " (since this message was last posted)"
    };

    let mut should_send = false;
    let mut message = format!(
        "
        There are merge commits (commits with multiple parents) in your changes. We have a
        [no merge policy](https://rustc-dev-guide.rust-lang.org/git.html#no-merge-policy) so
        these commits will need to be removed for this pull request to be merged.

        You can start a rebase with the following commands:

        ```shell-session
        $ # rebase
        $ git rebase -i master
        $ # delete any merge commits in the editor that appears
        $ git push --force-with-lease
        ```

        The following commits are merge commits{since_last_posted}:

    "
    );
    for commit in &input.merge_commits {
        if state.data.mentioned_merge_commits.contains(commit) {
            continue;
        }

        should_send = true;
        state.data.mentioned_merge_commits.insert((*commit).clone());
        write!(message, "- {commit}").unwrap();
    }

    if should_send {
        event
            .issue
            .post_comment(&ctx.github, &message)
            .await
            .context("failed to post no_merges comment")?;
        state.save().await?;
    }
    Ok(())
}
