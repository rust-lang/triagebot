use crate::{
    config::TeamMemberWorkQueueConfig,
    github::{IssuesAction, IssuesEvent},
    handlers::Context,
    TeamMemberWorkQueue,
};
use anyhow::Context as _;
use tokio_postgres::Client as DbClient;
use tracing as log;

// This module updates the PR work queue of team members
// - When a PR has been assigned, adds the PR to the work queue of team members
// - When a PR is unassigned, removes the PR from the work queue of all team members

/// Get all assignees for a pull request
async fn get_pr_assignees(
    db: &DbClient,
    issue_num: i32,
) -> anyhow::Result<Vec<TeamMemberWorkQueue>> {
    let q = "
SELECT u.username, r.*, array_length(assigned_prs, 1) as num_assigned_prs
FROM review_prefs r
JOIN users u on u.user_id=r.user_id
WHERE $1 = ANY (assigned_prs)";
    let rows = db.query(q, &[&issue_num]).await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| Some(TeamMemberWorkQueue::from(row)))
        .collect())
}

/// Update a team member work queue
async fn update_team_member_workqueue(
    db: &DbClient,
    assignee: &TeamMemberWorkQueue,
) -> anyhow::Result<TeamMemberWorkQueue> {
    let q = "
UPDATE review_prefs r
SET assigned_prs = $2
FROM users u
WHERE r.user_id=$1 AND u.user_id=r.user_id
RETURNING u.username, r.*, array_length(assigned_prs, 1) as num_assigned_prs";
    let rec = db
        .query_one(q, &[&assignee.user_id, &assignee.assigned_prs])
        .await
        .context("Update DB error")?;
    Ok(rec.into())
}

/// Add a new user (if not existing)
async fn ensure_team_member(db: &DbClient, user_id: i64, username: &str) -> anyhow::Result<u64> {
    let q = "
INSERT INTO users (user_id, username) VALUES ($1, $2)
ON CONFLICT DO NOTHING";
    let rec = db
        .execute(q, &[&user_id, &username])
        .await
        .context("Insert user DB error")?;
    Ok(rec)
}

/// Create or increase by one a team member work queue
async fn upsert_team_member_workqueue(
    db: &DbClient,
    user_id: i64,
    pr: i32,
) -> anyhow::Result<u64, anyhow::Error> {
    let q = "
INSERT INTO review_prefs
(user_id, assigned_prs) VALUES ($1, $2)
ON CONFLICT (user_id)
DO UPDATE SET assigned_prs = array_append(review_prefs.assigned_prs, $3)
WHERE review_prefs.user_id=$1";
    let pr_v = vec![pr];
    db.execute(q, &[&user_id, &pr_v, &pr])
        .await
        .context("Upsert DB error")
}

pub(super) struct ReviewPrefsInput {}

pub(super) async fn parse_input(
    _ctx: &Context,
    event: &IssuesEvent,
    config: Option<&TeamMemberWorkQueueConfig>,
) -> Result<Option<ReviewPrefsInput>, String> {
    // IMPORTANT: this config check MUST exist. Else, the triagebot will emit an error that this
    // feature is not enabled
    if config.is_none() {
        return Ok(None);
    }

    // Act only if this is a PR or an assignment / unassignment
    if !event.issue.is_pr()
        || !matches!(
            event.action,
            IssuesAction::Assigned | IssuesAction::Unassigned
        )
    {
        return Ok(None);
    }
    Ok(Some(ReviewPrefsInput {}))
}

pub(super) async fn handle_input<'a>(
    ctx: &Context,
    _config: &TeamMemberWorkQueueConfig,
    event: &IssuesEvent,
    _inputs: ReviewPrefsInput,
) -> anyhow::Result<()> {
    let db_client = ctx.db.get().await;
    let iss_num = event.issue.number as i32;

    // Note: When changing assignees for a PR, we don't receive the assignee(s) removed, we receive
    // an event `Unassigned` and the remaining assignees

    // 1) Remove the PR from everyones' work queue
    let mut current_assignees = get_pr_assignees(&db_client, iss_num).await?;
    log::debug!("Removing assignment from user(s): {:?}", current_assignees);
    for assignee in &mut current_assignees {
        if let Some(index) = assignee
            .assigned_prs
            .iter()
            .position(|value| *value == iss_num)
        {
            assignee.assigned_prs.swap_remove(index);
        }
        update_team_member_workqueue(&db_client, &assignee).await?;
    }

    // 2) create or increase by one the team members work queue
    // create team members if they don't exist
    for u in event.issue.assignees.iter() {
        let user_id = u.id.expect("Github user was expected! Please investigate.");

        if let Err(err) = ensure_team_member(&db_client, user_id, &u.login).await {
            log::error!("Failed to create user in DB: this PR assignment won't be tracked.");
            return Err(err);
        }

        if let Err(err) = upsert_team_member_workqueue(&db_client, user_id, iss_num).await {
            log::error!("Failed to track PR for user: this PR assignment won't be tracked.");
            return Err(err);
        }
    }

    Ok(())
}
