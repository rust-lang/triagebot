use crate::github::{retrieve_open_pull_requests, UserId};
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
        let gh = &ctx.github;

        tracing::trace!("starting pull_request_assignment_update");

        let rust_repo = gh.repository("rust-lang/rust").await?;
        let prs = retrieve_open_pull_requests(&rust_repo, &gh).await?;

        // Aggregate PRs by user
        let aggregated: HashMap<UserId, HashSet<PullRequestNumber>> =
            prs.into_iter().fold(HashMap::new(), |mut acc, (user, pr)| {
                let prs = acc.entry(user.id).or_default();
                prs.insert(pr as PullRequestNumber);
                acc
            });
        tracing::info!("PR assignments\n{aggregated:?}");
        *ctx.workqueue.write().await = ReviewerWorkqueue::new(aggregated);

        Ok(())
    }
}
