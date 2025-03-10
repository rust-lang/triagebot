use crate::github::{retrieve_open_pull_requests, GithubClient, UserId};
use crate::handlers::pr_tracking::{PullRequestNumber, ReviewerWorkqueue};
use crate::jobs::Job;
use async_trait::async_trait;
use std::collections::{HashMap, HashSet};

pub struct PullRequestAssignmentUpdate;

#[async_trait]
impl Job for PullRequestAssignmentUpdate {
    fn name(&self) -> &'static str {
        "pull_request_assignment_update"
    }

    async fn run(&self, ctx: &super::Context, _metadata: &serde_json::Value) -> anyhow::Result<()> {
        tracing::trace!("starting pull_request_assignment_update");
        let workqueue = load_workqueue(&ctx.github).await?;
        *ctx.workqueue.write().await = workqueue;
        tracing::trace!("finished pull_request_assignment_update");

        Ok(())
    }
}

/// Loads the workqueue (mapping of open PRs assigned to users) from GitHub
pub async fn load_workqueue(gh: &GithubClient) -> anyhow::Result<ReviewerWorkqueue> {
    let prs = retrieve_open_pull_requests("rust-lang", "rust", &gh).await?;

    // Aggregate PRs by user
    let aggregated: HashMap<UserId, HashSet<PullRequestNumber>> =
        prs.into_iter().fold(HashMap::new(), |mut acc, (user, pr)| {
            let prs = acc.entry(user.id).or_default();
            prs.insert(pr as PullRequestNumber);
            acc
        });
    tracing::debug!("PR assignments\n{aggregated:?}");
    Ok(ReviewerWorkqueue::new(aggregated))
}
