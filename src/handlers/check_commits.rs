use anyhow::bail;

use super::Context;
use crate::{
    config::Config,
    db::issue_data::IssueData,
    github::{Event, IssuesAction, IssuesEvent, ReportedContentClassifiers},
};

#[cfg(test)]
use crate::github::GithubCommit;

mod modified_submodule;
mod no_mentions;
mod non_default_branch;

/// Key for the state in the database
const CHECK_COMMITS_WARNINGS_KEY: &str = "check-commits-warnings";

/// State stored in the database
#[derive(Debug, Default, serde::Deserialize, serde::Serialize)]
struct CheckCommitsWarningsState {
    /// List of the last warnings in the most recent comment.
    last_warnings: Vec<String>,
    /// ID of the most recent warning comment.
    last_warned_comment: Option<String>,
}

pub(super) async fn handle(ctx: &Context, event: &Event, config: &Config) -> anyhow::Result<()> {
    let Event::Issue(event) = event else {
        return Ok(());
    };

    if !matches!(
        event.action,
        IssuesAction::Opened | IssuesAction::Synchronize
    ) || !event.issue.is_pr()
    {
        return Ok(());
    }

    let Some(diff) = event.issue.diff(&ctx.github).await? else {
        bail!(
            "expected issue {} to be a PR, but the diff could not be determined",
            event.issue.number
        )
    };
    let commits = event.issue.commits(&ctx.github).await?;

    let mut warnings = Vec::new();

    // Compute the warnings
    if let Some(assign_config) = &config.assign {
        // For legacy reasons the non-default-branch and modifies-submodule warnings
        // are behind the `[assign]` config.

        if let Some(exceptions) = assign_config
            .warn_non_default_branch
            .enabled_and_exceptions()
        {
            warnings.extend(non_default_branch::non_default_branch(exceptions, event));
        }
        warnings.extend(modified_submodule::modifies_submodule(diff));
    }

    if let Some(no_mentions) = &config.no_mentions {
        warnings.extend(no_mentions::mentions_in_commits(no_mentions, &commits));
    }

    handle_warnings(ctx, event, warnings).await
}

// Add, hide or hide&add a comment with the warnings.
async fn handle_warnings(
    ctx: &Context,
    event: &IssuesEvent,
    warnings: Vec<String>,
) -> anyhow::Result<()> {
    // Get the state of the warnings for this PR in the database.
    let mut db = ctx.db.get().await;
    let mut state: IssueData<'_, CheckCommitsWarningsState> =
        IssueData::load(&mut db, &event.issue, CHECK_COMMITS_WARNINGS_KEY).await?;

    // We only post a new comment when we haven't posted one with the same warnings before.
    if !warnings.is_empty() && state.data.last_warnings != warnings {
        // New set of warnings, let's post them.

        // Hide a previous warnings comment if there was one before printing the new ones.
        if let Some(last_warned_comment_id) = state.data.last_warned_comment {
            event
                .issue
                .hide_comment(
                    &ctx.github,
                    &last_warned_comment_id,
                    ReportedContentClassifiers::Resolved,
                )
                .await?;
        }

        let warning = warning_from_warnings(&warnings);
        let comment = event.issue.post_comment(&ctx.github, &warning).await?;

        // Save new state in the database
        state.data.last_warnings = warnings;
        state.data.last_warned_comment = Some(comment.node_id);
        state.save().await?;
    } else if warnings.is_empty() {
        // No warnings to be shown, let's resolve a previous warnings comment, if there was one.
        if let Some(last_warned_comment_id) = state.data.last_warned_comment {
            event
                .issue
                .hide_comment(
                    &ctx.github,
                    &last_warned_comment_id,
                    ReportedContentClassifiers::Resolved,
                )
                .await?;

            state.data.last_warnings = Vec::new();
            state.data.last_warned_comment = None;
            state.save().await?;
        }
    }

    Ok(())
}

// Format the warnings for user consumption on Github
fn warning_from_warnings(warnings: &[String]) -> String {
    let warnings: Vec<_> = warnings
        .iter()
        .map(|warning| format!("* {warning}"))
        .collect();
    format!(":warning: **Warning** :warning:\n\n{}", warnings.join("\n"))
}

#[cfg(test)]
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
