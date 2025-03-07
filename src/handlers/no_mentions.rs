//! Purpose: When opening a PR, or pushing new changes, check for github mentions
//! in commits and notify the user of our no-mentions in commits policy.

use crate::{
    config::NoMentionsConfig,
    db::issue_data::IssueData,
    github::{IssuesAction, IssuesEvent, ReportedContentClassifiers},
    handlers::Context,
};
use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt::Write;
use tracing as log;

const NO_MENTIONS_KEY: &str = "no_mentions";

pub(super) struct NoMentionsInput {
    /// Hashes of commits that have mentions in the pull request.
    mentions_commits: HashSet<String>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct NoMentionsState {
    /// Hashes of mention commits that have already been mentioned by triagebot in a comment.
    mentioned_mention_commits: HashSet<String>,
    /// List of all the no_mention comments as GitHub GraphQL NodeId.
    #[serde(default)]
    no_mention_comments: Vec<String>,
}

pub(super) async fn parse_input(
    ctx: &Context,
    event: &IssuesEvent,
    config: Option<&NoMentionsConfig>,
) -> Result<Option<NoMentionsInput>, String> {
    if !matches!(
        event.action,
        IssuesAction::Opened | IssuesAction::Synchronize | IssuesAction::ReadyForReview
    ) {
        return Ok(None);
    }

    // Require a `[no_mentions]` configuration block to enable no-mentions notifications.
    let Some(_config) = config else {
        return Ok(None);
    };

    let mut mentions_commits = HashSet::new();
    let commits = event
        .issue
        .commits(&ctx.github)
        .await
        .map_err(|e| {
            log::error!("failed to fetch commits: {:?}", e);
        })
        .unwrap_or_default();

    for commit in commits {
        if !parser::get_mentions(&commit.commit.message).is_empty() {
            mentions_commits.insert(commit.sha.clone());
        }
    }

    // Run the handler even if we have no mentions,
    // so we can take an action if some were removed.
    Ok(Some(NoMentionsInput { mentions_commits }))
}

fn get_default_message() -> String {
    format!(
        "
There are mentions (`@mention`) in your commits. We have a no mention policy
so these commits will need to be removed for this pull request to be merged.

"
    )
}

pub(super) async fn handle_input(
    ctx: &Context,
    _config: &NoMentionsConfig,
    event: &IssuesEvent,
    input: NoMentionsInput,
) -> anyhow::Result<()> {
    let mut client = ctx.db.get().await;
    let mut state: IssueData<'_, NoMentionsState> =
        IssueData::load(&mut client, &event.issue, NO_MENTIONS_KEY).await?;

    // No commits with mentions.
    if input.mentions_commits.is_empty() {
        if state.data.mentioned_mention_commits.is_empty() {
            // No commits with mentions from before, so do nothing.
            return Ok(());
        }

        // Minimize prior no mention comments.
        for node_id in state.data.no_mention_comments.iter() {
            event
                .issue
                .hide_comment(
                    &ctx.github,
                    node_id.as_str(),
                    ReportedContentClassifiers::Resolved,
                )
                .await
                .context("failed to hide previous no-mention comment")?;
        }

        // Clear from state.
        state.data.mentioned_mention_commits.clear();
        state.data.no_mention_comments.clear();
        state.save().await?;
        return Ok(());
    }

    let first_time = state.data.mentioned_mention_commits.is_empty();

    let mut message = get_default_message();

    let since_last_posted = if first_time {
        ""
    } else {
        " (since this message was last posted)"
    };
    writeln!(
        message,
        "The following commits have mentions is them{since_last_posted}:"
    )
    .unwrap();

    let mut should_send = false;
    for commit in &input.mentions_commits {
        if state.data.mentioned_mention_commits.contains(commit) {
            continue;
        }

        should_send = true;
        state
            .data
            .mentioned_mention_commits
            .insert((*commit).clone());
        writeln!(message, "- {commit}").unwrap();
    }

    if should_send {
        // Post comment
        let comment = event
            .issue
            .post_comment(&ctx.github, &message)
            .await
            .context("failed to post no_mentions comment")?;

        state.data.no_mention_comments.push(comment.node_id);
        state.save().await?;
    }
    Ok(())
}
