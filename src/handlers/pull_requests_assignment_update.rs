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
        for (repo_name, workqueue_arc) in ctx.workqueue_map.tracked_repositories() {
            let (owner, repo) = repo_name
                .split_once('/')
                .expect("repo name should be in owner/repo format");
            match load_workqueue(&ctx.octocrab, owner, repo).await {
                Ok(workqueue) => {
                    *workqueue_arc.write().await = workqueue;
                }
                Err(error) => {
                    tracing::error!("Cannot reload workqueue for {repo_name}: {error:?}");
                }
            }
        }
        tracing::trace!("finished pull_request_assignment_update");

        Ok(())
    }
}
