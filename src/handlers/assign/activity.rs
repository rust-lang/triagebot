use std::fmt::Display;

use anyhow::Context as _;
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    github::{GitHubUser, ReportedContentClassifiers, utils::Selection},
    jobs::Job,
};

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub(super) struct AssignCheckActivityInput {
    pub(super) repo: String,
    pub(super) issue: u64,
    pub(super) assignee: u64,
    pub(super) warning_details: Option<WarningDetails>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub(super) struct WarningDetails {
    posted_at: DateTime<Utc>,
    node_id: String,
}

const ASSIGN_CHECK_ACTIVITY_JOB_NAME: &str = "assign_check_activity";

pub(crate) struct AssignCheckActivityJob;

#[async_trait]
impl Job for AssignCheckActivityJob {
    fn name(&self) -> &'static str {
        ASSIGN_CHECK_ACTIVITY_JOB_NAME
    }

    async fn run(&self, ctx: &super::Context, metadata: &serde_json::Value) -> anyhow::Result<()> {
        let input: AssignCheckActivityInput = serde_json::from_value(metadata.clone())
            .context("unable to deserialize the metadata in major change acceptance job")?;

        let now = Utc::now();
        tracing::info!("processing: {input:?}");

        match check_activity(ctx, &input, now).await {
            Ok(()) => {
                tracing::info!(
                    "{}: checked activity ({:?}) succesfully",
                    self.name(),
                    &input,
                );
            }
            Err(err) if err.downcast_ref::<ActivityCheckingLogicError>().is_some() => {
                tracing::error!(
                    "{}: check activity ({:?}) has a logical error (no retry): {err}",
                    self.name(),
                    &input,
                );
                // exit job succesfully, so it's not retried
            }
            Err(err) => {
                tracing::error!(
                    "{}: check activity ({:?}) is in error: {err}",
                    self.name(),
                    &input,
                );
                return Err(err); // so it is retried
            }
        }

        Ok(())
    }
}

#[derive(Debug)]
enum ActivityCheckingLogicError {
    IssueNotReady {
        draft: bool,
        open: bool,
    },
    AssigneeChanged {
        expected: u64,
        found: Vec<GitHubUser>,
    },
    MissingAssignCheckActivityConfig,
}

impl std::error::Error for ActivityCheckingLogicError {}

impl Display for ActivityCheckingLogicError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActivityCheckingLogicError::IssueNotReady { draft, open } => {
                write!(f, "issue is not ready (draft: {draft}; open: {open})")
            }
            ActivityCheckingLogicError::AssigneeChanged { expected, found } => {
                write!(
                    f,
                    "issue assignee changed (expected: {expected}; found: {found:?})"
                )
            }
            ActivityCheckingLogicError::MissingAssignCheckActivityConfig => {
                write!(f, "missing `[assign.check_activity]` config")
            }
        }
    }
}

async fn check_activity(
    ctx: &super::Context,
    input: &AssignCheckActivityInput,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    let repo = ctx
        .github
        .repository(&input.repo)
        .await
        .context("failed retrieving the repository informations")?;

    let config = crate::config::get(&ctx.github, &repo)
        .await
        .context("failed to get triagebot configuration")?;

    let config = config
        .assign
        .as_ref()
        .ok_or(ActivityCheckingLogicError::MissingAssignCheckActivityConfig)?
        .check_activity
        .as_ref()
        .ok_or(ActivityCheckingLogicError::MissingAssignCheckActivityConfig)?;

    let issue = repo
        .get_issue(&ctx.github, input.issue)
        .await
        .context("unable to get the associated issue")?;

    if !issue.is_open() || issue.draft {
        anyhow::bail!(ActivityCheckingLogicError::IssueNotReady {
            draft: issue.draft,
            open: issue.is_open()
        });
    }

    let assignee = issue.assignees.iter().find(|u| u.id == input.assignee);
    let Some(assignee) = assignee else {
        anyhow::bail!(ActivityCheckingLogicError::AssigneeChanged {
            expected: input.assignee,
            found: issue.assignees.clone()
        });
    };

    let cross_references = issue
        .cross_references(&ctx.github)
        .await
        .context("failed to fetch the issue cross-references")?;

    let max_closing_cross_reference = cross_references
        .iter()
        .filter(|c| c.will_close_target)
        .map(|c| c.source.updated_at)
        .max();

    let last_update_time = if let Some(max_closing_cross_reference) = max_closing_cross_reference {
        issue.updated_at.max(max_closing_cross_reference)
    } else {
        issue.updated_at
    };
    tracing::debug!(?last_update_time, ?max_closing_cross_reference, ?input);

    if let Some(warning) = &input.warning_details {
        if last_update_time <= warning.posted_at {
            tracing::info!(
                "inactivity warning was posted but assignee ({assignee:?}) is still inactive, removing the assignement, and posting warning"
            );
            issue
                .remove_assignees(&ctx.github, Selection::One(&assignee.login))
                .await?;
            issue
                .post_comment(
                    &ctx.github,
                    &release_message(&assignee.login, config.inactivity_limit.get()),
                )
                .await
                .context("couldn't post release message")?;
        } else {
            tracing::info!(
                "new activity detected, hidding previous warning comment and scheduling new inactivity check"
            );
            issue
                .hide_comment(
                    &ctx.github,
                    &warning.node_id,
                    ReportedContentClassifiers::Resolved,
                )
                .await?;
            schedule_activity_job(
                ctx,
                AssignCheckActivityInput {
                    warning_details: None,
                    ..input.clone()
                },
                last_update_time + Duration::days(config.inactivity_reminder.get().into()),
            )
            .await?;
        }
    } else {
        if last_update_time + Duration::days(config.inactivity_reminder.get().into()) <= now {
            tracing::info!(
                "inactivity detected (last_update_time: {last_update_time}), posting warning comment and scheduling new inactivity check"
            );

            let grace_period = Duration::days(
                config
                    .inactivity_limit
                    .get()
                    .saturating_sub(config.inactivity_reminder.get())
                    .into(),
            );

            let comment = issue
                .post_comment(
                    &ctx.github,
                    &inactivity_message(
                        &assignee.login,
                        config.inactivity_reminder.get(),
                        grace_period.num_days(),
                        &ctx.username,
                    ),
                )
                .await
                .context("failed to post inactivity comment")?;

            schedule_activity_job(
                ctx,
                AssignCheckActivityInput {
                    warning_details: Some(WarningDetails {
                        posted_at: comment.created_at.unwrap(),
                        node_id: comment.node_id,
                    }),
                    ..input.clone()
                },
                comment.created_at.unwrap() + grace_period,
            )
            .await?;
        } else {
            // Issue not inactive, check again in X days from last update
            schedule_activity_job(
                ctx,
                AssignCheckActivityInput {
                    warning_details: None,
                    ..input.clone()
                },
                last_update_time + Duration::days(config.inactivity_reminder.get().into()),
            )
            .await?;
        }
    }

    Ok(())
}

pub(super) async fn schedule_activity_job(
    ctx: &super::Context,
    input: AssignCheckActivityInput,
    execute_at: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<()> {
    let payload =
        serde_json::to_value(input).context("unable to serialize the check activity metadata")?;

    crate::db::schedule_job(
        &*ctx.db.get().await,
        ASSIGN_CHECK_ACTIVITY_JOB_NAME,
        payload,
        execute_at,
    )
    .await
    .context("failed to schedule the check activity job")?;

    Ok(())
}

fn inactivity_message(assignee: &str, remind: u8, grace_period: i64, username: &str) -> String {
    format!("Hey @{assignee}, just checking in! It's been {remind} days since the last update on this issue or linked PRs.

No worries if you're busy! If you still want to tackle this, just drop a quick comment below. If you've run out of time or interest, you can release it by commenting `@{username} release-assignment` so another contributor can jump in.

If we don't hear from you in {grace_period} days, you will be automatically unassigned.")
}

fn release_message(assignee: &str, total: u8) -> String {
    format!(
        "Hey @{assignee}, just letting you know we've freed up this issue since we haven't seen any updates for {total} days. If you find the time later on, feel free to reclaim it and pick up where you left off."
    )
}

#[test]
fn check_activity_input_serialize() {
    let original = AssignCheckActivityInput {
        repo: "rust-lang/rust".to_string(),
        issue: 1245,
        assignee: 123,
        warning_details: Some(WarningDetails {
            posted_at: Utc::now(),
            node_id: "azerty".to_string(),
        }),
    };

    let value = serde_json::to_value(original.clone()).unwrap();

    let deserialized = serde_json::from_value(value).unwrap();

    assert_eq!(original, deserialized);
}
