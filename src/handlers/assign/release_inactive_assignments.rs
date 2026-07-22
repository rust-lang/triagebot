//! This module defines the logic for releasing inactive issue assignments

use std::collections::HashMap;

use anyhow::Context as _;
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

const INACTIVITY_REMINDER_DAYS: i64 = 49; // 7 weeks
const INACTIVITY_LIMIT_DAYS: i64 = 56; // 8 weeks
const INACTIVITY_GRACE_PERIOD_DAYS: i64 = INACTIVITY_LIMIT_DAYS - INACTIVITY_REMINDER_DAYS;

use crate::{
    db::issue_data::IssueData,
    github::{IssueNumber, IssueRepository, ReportedContentClassifiers, utils::Selection},
    handlers::Context,
    jobs::Job,
};

pub(crate) const RELEASE_INACTIVE_ASSIGNMENTS_JOB_NAME: &str = "release_inactive_assignments";

pub(crate) struct ReleaseInactiveAssignmentsJob;

#[async_trait]
impl Job for ReleaseInactiveAssignmentsJob {
    fn name(&self) -> &'static str {
        RELEASE_INACTIVE_ASSIGNMENTS_JOB_NAME
    }

    async fn run(&self, ctx: &Context, _metadata: &serde_json::Value) -> anyhow::Result<()> {
        tracing::info!("Starting job to release inactive assignments");

        for (owner, repo) in [("rust-lang", "rust-clippy")] {
            tracing::info!("Starting checking {owner}/{repo}");
            match update_assignments(ctx, owner, repo).await {
                Ok(()) => {
                    tracing::info!("Checking of {owner}/{repo} succesful");
                }
                Err(err) => {
                    tracing::error!("Failed to check {owner}/{repo}: {err}");
                }
            }
        }

        tracing::info!("Job finished to release inactive assignments");
        Ok(())
    }
}

async fn update_assignments(ctx: &Context, owner: &str, repo: &str) -> anyhow::Result<()> {
    let issues_assigned = ctx
        .github
        .issues_assigned(owner, repo)
        .await
        .context("unable to load the assigned issues")?;

    let prs_with_closing_issues_references = ctx
        .github
        .closing_issues_references(owner, repo)
        .await
        .context("unable to load the closing issues references")?;

    tracing::info!("Loaded {} assigned issues", issues_assigned.len());
    tracing::info!(
        "Loaded {} PRs (with their closing issues references)",
        prs_with_closing_issues_references.len()
    );

    let references: HashMap<IssueNumber, Vec<&_>> = prs_with_closing_issues_references
        .iter()
        .flat_map(|pr| {
            pr.closing_issues_references
                .iter()
                .map(move |i| (i.number, pr))
        })
        .fold(HashMap::new(), |mut map, (issue_num, pr)| {
            map.entry(issue_num).or_default().push(pr);
            map
        });

    let now = Utc::now();
    let owner_repo = format!("{owner}/{repo}");

    for issue_assigned in issues_assigned {
        let [assignee] = &issue_assigned.assignees[..] else {
            continue;
        };

        let last_update_time = references
            .get(&issue_assigned.number)
            .iter()
            .flat_map(|references| references.iter().map(|r| r.updated_at))
            .max()
            .unwrap_or(DateTime::<Utc>::MIN_UTC)
            .max(issue_assigned.updated_at);

        if last_update_time + Duration::days(INACTIVITY_GRACE_PERIOD_DAYS) > now {
            continue;
        }

        let mut db = ctx.db.get().await;

        {
            let issue_assignee = IssueData::<super::AssignData>::load_raw(
                &mut db,
                owner_repo.to_string(),
                issue_assigned.number as i32,
                super::ASSIGN_KEY,
            )
            .await
            .context("couldn't load issue data")?;

            if Some(assignee.login.as_str()) != issue_assignee.data.user.as_deref() {
                // Assigned user is different from `claim` command, skipping
                continue;
            }
        }

        let mut issue_reminder = IssueData::<IssueReminderDetails>::load_raw(
            &mut db,
            owner_repo.to_string(),
            issue_assigned.number as i32,
            ISSUE_REMINDER_DETAILS_KEY,
        )
        .await
        .context("couldn't load issue data")?;

        if issue_reminder.data.login_id == assignee.id {
            let issue = ctx
                .github
                .issue(
                    &IssueRepository {
                        organization: owner.to_string(),
                        repository: repo.to_string(),
                    },
                    issue_assigned.number,
                )
                .await
                .with_context(|| {
                    format!(
                        "failed to load issue {owner_repo}#{}",
                        issue_assigned.number
                    )
                })?;

            if last_update_time <= issue_reminder.data.posted_at {
                tracing::info!(
                    "[] inactivity warning was posted but assignee ({assignee:?}) is still inactive, removing the assignement, and posting warning"
                );
                issue
                    .remove_assignees(&ctx.github, Selection::One(&assignee.login))
                    .await?;
                issue
                    .post_comment(
                        &ctx.github,
                        &release_message(&assignee.login, INACTIVITY_LIMIT_DAYS),
                    )
                    .await
                    .context("couldn't post release message")?;

                issue_reminder.data = IssueReminderDetails::default();
                issue_reminder.save().await?;
            } else {
                tracing::info!(
                    "[] new activity detected, hidding previous warning comment and scheduling new inactivity check"
                );
                issue
                    .hide_comment(
                        &ctx.github,
                        &issue_reminder.data.node_id,
                        ReportedContentClassifiers::Resolved,
                    )
                    .await?;

                issue_reminder.data = IssueReminderDetails::default();
                issue_reminder.save().await?;
            }
        } else if last_update_time + Duration::days(INACTIVITY_REMINDER_DAYS) <= now {
            tracing::info!(
                "inactivity detected (last_update_time: {last_update_time}), posting warning comment and scheduling new inactivity check"
            );

            let issue = ctx
                .github
                .issue(
                    &IssueRepository {
                        organization: owner.to_string(),
                        repository: repo.to_string(),
                    },
                    issue_assigned.number,
                )
                .await
                .with_context(|| {
                    format!(
                        "failed to load issue {owner_repo}#{}",
                        issue_assigned.number
                    )
                })?;

            let comment = issue
                .post_comment(
                    &ctx.github,
                    &inactivity_message(
                        &assignee.login,
                        INACTIVITY_REMINDER_DAYS,
                        INACTIVITY_GRACE_PERIOD_DAYS,
                        &ctx.username,
                    ),
                )
                .await
                .context("failed to post inactivity comment")?;

            issue_reminder.data = IssueReminderDetails {
                posted_at: comment.created_at.context("no created_at for a comment")?,
                login_id: assignee.id,
                node_id: comment.node_id,
            };
            issue_reminder.save().await?;
        }
    }

    Ok(())
}

const ISSUE_REMINDER_DETAILS_KEY: &str = "ISSUE_REMINDER_DETAILS";

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
pub(super) struct IssueReminderDetails {
    posted_at: DateTime<Utc>,
    login_id: u64,
    node_id: String,
}

fn inactivity_message(assignee: &str, remind: i64, grace_period: i64, username: &str) -> String {
    format!("Hey @{assignee}, just checking in! It's been {remind} days since the last update on this issue or linked PRs.

No worries if you're busy! If you still want to tackle this, just drop a quick comment below. If you've run out of time or interest, you can release it by commenting `@{username} release-assignment` so another contributor can jump in.

If we don't hear from you in {grace_period} days, you will be automatically unassigned.")
}

fn release_message(assignee: &str, total: i64) -> String {
    format!(
        "Hey @{assignee}, just letting you know we've freed up this issue since we haven't seen any updates for {total} days. If you find the time later on, feel free to reclaim it and pick up where you left off."
    )
}
