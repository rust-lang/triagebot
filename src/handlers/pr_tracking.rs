//! This module updates the PR workqueue of the Rust project contributors
//! Runs after a PR has been assigned or unassigned
//!
//! Purpose:
//!
//! - Adds the PR to the workqueue of one team member (after the PR has been assigned)
//! - Removes the PR from the workqueue of one team member (after the PR has been unassigned or closed)

use crate::{
    config::ReviewPrefsConfig,
    db::notifications::record_username,
    github::{IssuesAction, IssuesEvent},
    handlers::Context,
    ReviewPrefs,
};
use anyhow::Context as _;
use tokio_postgres::Client as DbClient;

use super::assign::{FindReviewerError, REVIEWER_HAS_NO_CAPACITY, SELF_ASSIGN_HAS_NO_CAPACITY};

pub(super) struct ReviewPrefsInput {}

pub(super) async fn parse_input(
    _ctx: &Context,
    event: &IssuesEvent,
    config: Option<&ReviewPrefsConfig>,
) -> Result<Option<ReviewPrefsInput>, String> {
    // NOTE: this config check MUST exist. Else, the triagebot will emit an error
    // about this feature not being enabled
    if config.is_none() {
        return Ok(None);
    };

    // Execute this handler only if this is a PR ...
    if !event.issue.is_pr() {
        return Ok(None);
    }

    // ... and if the action is an assignment or unassignment with an assignee
    match event.action {
        IssuesAction::Assigned { .. } | IssuesAction::Unassigned { .. } => {
            Ok(Some(ReviewPrefsInput {}))
        }
        _ => Ok(None),
    }
}

pub(super) async fn handle_input<'a>(
    ctx: &Context,
    _config: &ReviewPrefsConfig,
    event: &IssuesEvent,
    _inputs: ReviewPrefsInput,
) -> anyhow::Result<()> {
    let db_client = ctx.db.get().await;

    // extract the assignee or ignore this handler and return
    let IssuesEvent {
        action: IssuesAction::Assigned { assignee } | IssuesAction::Unassigned { assignee },
        ..
    } = event
    else {
        return Ok(());
    };

    // ensure the team member object of this action exists in the `users` table
    record_username(&db_client, assignee.id.unwrap(), &assignee.login)
        .await
        .context("failed to record username")?;

    if matches!(event.action, IssuesAction::Unassigned { .. }) {
        delete_pr_from_workqueue(&db_client, assignee.id.unwrap(), event.issue.number)
            .await
            .context("Failed to remove PR from work queue")?;
    }

    // This handler is reached also when assigning a PR using the Github UI
    // (i.e. from the "Assignees" dropdown menu).
    // We need to also check assignee availability here.
    if matches!(event.action, IssuesAction::Assigned { .. }) {
        let work_queue = has_user_capacity(&db_client, &assignee.login)
            .await
            .context("Failed to retrieve user work queue");

        // if user has no capacity, revert the PR assignment (GitHub has already assigned it)
        // and post a comment suggesting what to do
        if let Err(_) = work_queue {
            event
                .issue
                .remove_assignees(&ctx.github, crate::github::Selection::One(&assignee.login))
                .await?;

            let msg = if assignee.login.to_lowercase() == event.issue.user.login.to_lowercase() {
                SELF_ASSIGN_HAS_NO_CAPACITY.replace("{username}", &assignee.login)
            } else {
                REVIEWER_HAS_NO_CAPACITY.replace("{username}", &assignee.login)
            };
            event.issue.post_comment(&ctx.github, &msg).await?;
        }

        upsert_pr_into_workqueue(&db_client, assignee.id.unwrap(), event.issue.number)
            .await
            .context("Failed to add PR to work queue")?;
    }

    Ok(())
}

pub async fn has_user_capacity(
    db: &crate::db::PooledClient,
    assignee: &str,
) -> anyhow::Result<ReviewPrefs, FindReviewerError> {
    let q = "
SELECT username, r.*
FROM review_prefs r
JOIN users ON users.user_id = r.user_id
WHERE username = $1
AND CARDINALITY(r.assigned_prs) < max_assigned_prs;";
    let rec = db.query_one(q, &[&assignee]).await;
    if let Err(_) = rec {
        return Err(FindReviewerError::ReviewerHasNoCapacity {
            username: assignee.to_string(),
        });
    }
    Ok(rec.unwrap().into())
}

/// Add a PR to the workqueue of a team member.
/// Ensures no accidental PR duplicates.
async fn upsert_pr_into_workqueue(
    db: &DbClient,
    user_id: u64,
    pr: u64,
) -> anyhow::Result<u64, anyhow::Error> {
    let q = "
INSERT INTO review_prefs
(user_id, assigned_prs) VALUES ($1, $2)
ON CONFLICT (user_id)
DO UPDATE SET assigned_prs = uniq(sort(array_append(review_prefs.assigned_prs, $3)));";
    db.execute(q, &[&(user_id as i64), &vec![pr as i32], &(pr as i32)])
        .await
        .context("Upsert DB error")
}

/// Delete a PR from the workqueue of a team member
async fn delete_pr_from_workqueue(
    db: &DbClient,
    user_id: u64,
    pr: u64,
) -> anyhow::Result<u64, anyhow::Error> {
    let q = "
UPDATE review_prefs r
SET assigned_prs = array_remove(r.assigned_prs, $2)
WHERE r.user_id = $1;";
    db.execute(q, &[&(user_id as i64), &(pr as i32)])
        .await
        .context("Update DB error")
}
