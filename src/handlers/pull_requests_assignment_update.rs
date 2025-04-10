use crate::github::PullRequestNumber;
use crate::github::{retrieve_pull_request_assignments, UserId};
use crate::handlers::pr_tracking::ReviewerWorkqueue;
use crate::jobs::Job;
use async_trait::async_trait;
use octocrab::Octocrab;
use std::collections::{HashMap, HashSet};

pub struct PullRequestAssignmentUpdate;

#[async_trait]
impl Job for PullRequestAssignmentUpdate {
    fn name(&self) -> &'static str {
        "pull_request_assignment_update"
    }

    async fn run(&self, ctx: &super::Context, _metadata: &serde_json::Value) -> anyhow::Result<()> {
        tracing::trace!("starting pull_request_assignment_update");
        let workqueue = load_workqueue(&ctx.octocrab).await?;
        *ctx.workqueue.write().await = workqueue;
        tracing::trace!("finished pull_request_assignment_update");

        Ok(())
    }
}

/// Loads the workqueue (mapping of open PRs assigned to users) from GitHub
pub async fn load_workqueue(client: &Octocrab) -> anyhow::Result<ReviewerWorkqueue> {
    tracing::debug!("Loading workqueue for rust-lang/rust");
    let prs = retrieve_pull_request_assignments("rust-lang", "rust", &client).await?;

    // Aggregate PRs by user
    let aggregated: HashMap<UserId, HashSet<PullRequestNumber>> =
        prs.into_iter().fold(HashMap::new(), |mut acc, (user, pr)| {
            let prs = acc.entry(user.id).or_default();
            prs.insert(pr);
            acc
        });
    tracing::debug!("PR assignments\n{aggregated:?}");
    Ok(ReviewerWorkqueue::new(aggregated))
}
