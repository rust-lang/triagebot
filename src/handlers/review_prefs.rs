use crate::{
    config::ReviewPrefsConfig,
    github::{IssuesAction, IssuesEvent, Selection},
    handlers::Context,
    ReviewCapacityUser,
};
use anyhow::Context as _;
use tokio_postgres::Client as DbClient;
use tracing as log;

// This module updates the PR work queue of reviewers
// - Adds the PR to the work queue of the user (when the PR has been assigned)
// - Removes the PR from the work queue of the user (when the PR is unassigned or closed)
// - Rollbacks the PR assignment in case the specific user ("r? user") is inactive/not available

pub async fn set_prefs(
    db: &DbClient,
    prefs: ReviewCapacityUser,
) -> anyhow::Result<ReviewCapacityUser> {
    let q = "
UPDATE review_capacity r
SET max_assigned_prs = $2, pto_date_start = $3, pto_date_end = $4, active = $5, allow_ping_after_days = $6, publish_prefs = $7
FROM users u
WHERE r.user_id=$1 AND u.user_id=r.user_id
RETURNING u.username, r.*";
    log::debug!("pref {:?}", prefs);
    let rec = db
        .query_one(
            q,
            &[
                &prefs.user_id,
                &prefs.max_assigned_prs,
                &prefs.pto_date_start,
                &prefs.pto_date_end,
                &prefs.active,
                &prefs.allow_ping_after_days,
                &prefs.publish_prefs,
            ],
        )
        .await
        .context("Update DB error")?;
    Ok(rec.into())
}

/// Return a user
pub async fn get_user(db_client: &DbClient, checksum: &str) -> anyhow::Result<ReviewCapacityUser> {
    let q = "
SELECT username,r.*
FROM review_capacity r
JOIN users on r.user_id=users.user_id
WHERE r.checksum=$1";
    let rec = db_client
        .query_one(q, &[&checksum])
        .await
        .context("SQL error")?;
    Ok(rec.into())
}

/// Get all review capacity preferences
/// - me: sort the current user at the top of the list
/// - is_admin: if `true` pull also profiles marked as not public
pub async fn get_prefs(
    db: &DbClient,
    users: &mut Vec<String>,
    me: &str,
    is_admin: bool,
) -> Vec<ReviewCapacityUser> {
    let q = format!(
        "
SELECT username,r.*
FROM review_capacity r
JOIN users on r.user_id=users.user_id
WHERE username = any($1)
ORDER BY case when username='{}' then 1 else 2 end, username;",
        me
    );

    let rows = db.query(&q, &[users]).await.unwrap();
    rows.into_iter()
        .filter_map(|row| {
            let rec = ReviewCapacityUser::from(row);
            // FIXME: Hmm is this "username == me" showing all records al the time?
            if is_admin || rec.username == me || rec.publish_prefs == true {
                Some(rec)
            } else {
                None
            }
        })
        .collect()
}

pub async fn get_review_candidates_by_username(
    db: &DbClient,
    usernames: Vec<String>,
) -> anyhow::Result<Vec<ReviewCapacityUser>> {
    let q = format!(
        "
SELECT u.username, r.*
FROM review_capacity r
JOIN users u on u.user_id=r.user_id
WHERE username = ANY('{{ {} }}')",
        usernames.join(",")
    );

    log::debug!("get review prefs for {:?}", usernames);
    let rows = db.query(&q, &[]).await.unwrap();
    Ok(rows
        .into_iter()
        .filter_map(|row| Some(ReviewCapacityUser::from(row)))
        .collect())
}

pub async fn get_review_candidate_by_capacity(
    db: &DbClient,
    usernames: Vec<String>,
) -> anyhow::Result<ReviewCapacityUser> {
    let q = format!(
        "
SELECT username, r.*, sum(r.max_assigned_prs - r.num_assigned_prs) as avail_slots
FROM review_capacity r
JOIN users on users.user_id=r.user_id
WHERE active = true
AND current_date NOT BETWEEN pto_date_start AND pto_date_end
AND username = ANY('{{ {} }}')
AND num_assigned_prs < max_assigned_prs
GROUP BY username, r.id
ORDER BY avail_slots DESC
LIMIT 1",
        usernames.join(",")
    );
    let rec = db.query_one(&q, &[]).await.context("Select DB error")?;
    Ok(rec.into())
}

async fn get_review_pr_assignees(
    db: &DbClient,
    issue_num: u64,
) -> anyhow::Result<Vec<ReviewCapacityUser>> {
    let q = format!(
        "
SELECT u.username, r.*
FROM review_capacity r
JOIN users u on u.user_id=r.user_id
WHERE {} = ANY (assigned_prs);",
        issue_num,
    );

    let rows = db.query(&q, &[]).await.unwrap();
    Ok(rows
        .into_iter()
        .filter_map(|row| Some(ReviewCapacityUser::from(row)))
        .collect())
}

pub(super) struct ReviewPrefsInput {}

pub(super) async fn parse_input(
    _ctx: &Context,
    event: &IssuesEvent,
    config: Option<&ReviewPrefsConfig>,
) -> Result<Option<ReviewPrefsInput>, String> {
    log::debug!("[review_prefs] parse_input");
    let _config = match config {
        Some(config) => config,
        None => return Ok(None),
    };

    log::debug!(
        "[review_prefs] now matching the action for event {:?}",
        event
    );
    match event.action {
        IssuesAction::Assigned => {
            log::debug!("[review_prefs] IssuesAction::Assigned: Will add to work queue");
            Ok(Some(ReviewPrefsInput {}))
        }
        IssuesAction::Unassigned | IssuesAction::Closed => {
            log::debug!("[review_prefs] IssuesAction::Unassigned | IssuesAction::Closed: Will remove from work queue");
            Ok(Some(ReviewPrefsInput {}))
        }
        _ => {
            log::debug!("[review_prefs] Other action on PR {:?}", event.action);
            Ok(None)
        }
    }
}

async fn update_assigned_prs(
    db: &DbClient,
    user_id: i64,
    assigned_prs: &Vec<i32>,
) -> anyhow::Result<ReviewCapacityUser> {
    let q = "
UPDATE review_capacity r
SET assigned_prs = $2, num_assigned_prs = $3
FROM users u
WHERE r.user_id=$1 AND u.user_id=r.user_id
RETURNING u.username, r.*";
    let num_assigned_prs = assigned_prs.len() as i32;
    let rec = db
        .query_one(q, &[&user_id, assigned_prs, &num_assigned_prs])
        .await
        .context("Update DB error")?;
    Ok(rec.into())
}

pub(super) async fn handle_input<'a>(
    ctx: &Context,
    _config: &ReviewPrefsConfig,
    event: &IssuesEvent,
    _inputs: ReviewPrefsInput,
) -> anyhow::Result<()> {
    log::debug!("[review_prefs] handle_input");
    let db_client = ctx.db.get().await;

    // Note:
    // When assigning or unassigning a PR, we don't receive the assignee(s) removed from the PR
    // so we need to run two queries:

    // 1) unassign this PR from everyone
    let current_assignees = get_review_pr_assignees(&db_client, event.issue.number)
        .await
        .unwrap();
    for mut rec in current_assignees {
        if let Some(index) = rec
            .assigned_prs
            .iter()
            .position(|value| *value == event.issue.number as i32)
        {
            rec.assigned_prs.swap_remove(index);
        }
        update_assigned_prs(&db_client, rec.user_id, &rec.assigned_prs)
            .await
            .unwrap();
    }

    // If the action is to unassign/close a PR, nothing else to do
    if event.action == IssuesAction::Closed || event.action == IssuesAction::Unassigned {
        return Ok(());
    }

    // 2) assign the PR to the requested team members
    let usernames = event
        .issue
        .assignees
        .iter()
        .map(|x| x.login.clone())
        .collect::<Vec<String>>();
    let requested_assignees = get_review_candidates_by_username(&db_client, usernames)
        .await
        .unwrap();
    // iterate the list of requested assignees and try to assign the issue to each of them
    // in case of failure, emit a comment (for each failure)
    for mut assignee_prefs in requested_assignees {
        log::debug!(
            "Trying to assign to {}, with prefs {:?}",
            &assignee_prefs.username,
            &assignee_prefs
        );

        // If Github just assigned a PR to an inactive/unavailable user
        // publish a comment notifying the error and rollback the PR assignment
        if event.action == IssuesAction::Assigned
            && (!assignee_prefs.active || !assignee_prefs.is_available())
        {
            log::debug!(
                "PR assigned to {}, which is not available: will revert this.",
                &assignee_prefs.username
            );
            event
                .issue
                .post_comment(
                    &ctx.github,
                    &format!(
                        "Could not assign PR to user {}, please reroll for the team",
                        &assignee_prefs.username
                    ),
                )
                .await
                .context("Failed posting a comment on Github")?;
            event
                .issue
                .remove_assignees(&ctx.github, Selection::One(&assignee_prefs.username))
                .await
                .context("Failed unassigning the PR")?;
        }

        let iss_num = event.issue.number as i32;
        if !assignee_prefs.assigned_prs.contains(&iss_num) {
            assignee_prefs.assigned_prs.push(iss_num)
        }

        update_assigned_prs(
            &db_client,
            assignee_prefs.user_id,
            &assignee_prefs.assigned_prs,
        )
        .await
        .unwrap();
    }

    Ok(())
}
