use crate::handlers::pr_tracking::load_workqueue;
use crate::jobs::Job;
use async_trait::async_trait;

pub struct PullRequestAssignmentUpdate;

#[async_trait]
impl Job for PullRequestAssignmentUpdate {
    fn name(&self) -> &'static str {
        "pull_request_assignment_update"
    }

    async fn run(&self, ctx: &super::Context, _metadata: &serde_json::Value) -> anyhow::Result<()> {
        tracing::trace!("starting pull_request_assignment_update");
        for (repo, workqueue_arc) in ctx.workqueue_map.tracked_repositories() {
            match load_workqueue(&ctx.octocrab, repo).await {
                Ok(workqueue) => {
                    *workqueue_arc.write().await = workqueue;
                }
                Err(error) => {
                    tracing::error!(
                        "Cannot reload workqueue for {}: {error:?}",
                        repo.full_name()
                    );
                }
            }
        }
        tracing::trace!("finished pull_request_assignment_update");

        Ok(())
    }
}
