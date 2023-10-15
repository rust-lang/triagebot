use crate::config::ReviewRequestedConfig;
use crate::github::{Event, IssuesAction, IssuesEvent, Label};
use crate::handlers::Context;

pub(crate) async fn handle(
    ctx: &Context,
    event: &Event,
    config: &ReviewRequestedConfig,
) -> anyhow::Result<()> {
    // PR author requests a review from one of the assignees
    if let Event::Issue(IssuesEvent {
        action,
        issue,
        sender,
        ..
    }) = event
    {
        if let IssuesAction::ReviewRequested { requested_reviewer } = action {
            if *sender == issue.user && issue.assignees.contains(requested_reviewer) {
                issue
                    .add_labels(
                        &ctx.github,
                        config
                            .add_labels
                            .iter()
                            .cloned()
                            .map(|name| Label { name })
                            .collect(),
                    )
                    .await?;

                for label in &config.remove_labels {
                    issue.remove_label(&ctx.github, label).await?;
                }
            }
        }
    }

    Ok(())
}
