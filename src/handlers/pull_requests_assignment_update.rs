use std::collections::HashMap;

use crate::db::notifications::record_username;
use crate::github::retrieve_pull_requests;
use crate::jobs::Job;
use crate::ReviewPrefs;
use anyhow::Context as _;
use async_trait::async_trait;
use tokio_postgres::Client as DbClient;

pub struct PullRequestAssignmentUpdate;

#[async_trait]
impl Job for PullRequestAssignmentUpdate {
    fn name(&self) -> &'static str {
        "pull_request_assignment_update"
    }

    async fn run(&self, ctx: &super::Context, _metadata: &serde_json::Value) -> anyhow::Result<()> {
        let db = ctx.db.get().await;
        let gh = &ctx.github;

        tracing::trace!("starting pull_request_assignment_update");

        let rust_repo = gh.repository("rust-lang/rust").await?;
        let prs = retrieve_pull_requests(&rust_repo, &gh).await?;

        // delete all PR assignments before populating
        init_table(&db).await?;

        // aggregate by user first
        let aggregated = prs.into_iter().fold(HashMap::new(), |mut acc, (user, pr)| {
            let (_, prs) = acc.entry(user.id).or_insert_with(|| (user, Vec::new()));
            prs.push(pr);
            acc
        });

        // populate the table
        for (_user_id, (assignee, prs)) in &aggregated {
            let assignee_id = assignee.id;
            let _ = record_username(&db, assignee_id, &assignee.login).await;
            create_team_member_workqueue(&db, assignee_id, &prs).await?;
        }

        Ok(())
    }
}

/// Truncate the review prefs table
async fn init_table(db: &DbClient) -> anyhow::Result<u64> {
    let res = db
        .execute("UPDATE review_prefs SET assigned_prs='{}';", &[])
        .await?;
    Ok(res)
}

/// Create a team member work queue
async fn create_team_member_workqueue(
    db: &DbClient,
    user_id: u64,
    prs: &Vec<i32>,
) -> anyhow::Result<u64, anyhow::Error> {
    let q = "
INSERT INTO review_prefs (user_id, assigned_prs) VALUES ($1, $2)
ON CONFLICT (user_id)
DO UPDATE SET assigned_prs = $2
WHERE review_prefs.user_id=$1";
    db.execute(q, &[&(user_id as i64), prs])
        .await
        .context("Insert DB error")
}

/// Get pull request assignments for a team member
pub async fn get_review_prefs(db: &DbClient, user_id: u64) -> anyhow::Result<ReviewPrefs> {
    let q = "
SELECT username,r.*
FROM review_prefs r
JOIN users on r.user_id=users.user_id
WHERE r.user_id = $1;";
    let row = db
        .query_one(q, &[&(user_id as i64)])
        .await
        .context("Error retrieving review preferences")
        .unwrap();
    Ok(row.into())
}
