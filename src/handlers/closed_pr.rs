use crate::config::ClosedPrConfig;
use crate::github::{IssuesAction, IssuesEvent, Label};
use crate::handlers::Context;

pub(crate) struct ClosedPrInput {}

pub(crate) async fn parse_input(
    _ctx: &Context,
    event: &IssuesEvent,
    _config: Option<&ClosedPrConfig>,
) -> Result<Option<ClosedPrInput>, String> {
    // PR author requests a review from one of the assignees

    match &event.action {
        IssuesAction::Closed if event.issue.is_pr() => Ok(Some(ClosedPrInput {})),
        _ => Ok(None),
    }
}

pub(crate) async fn handle_input(
    ctx: &Context,
    config: &ClosedPrConfig,
    event: &IssuesEvent,
    ClosedPrInput {}: ClosedPrInput,
) -> anyhow::Result<()> {
    event
        .issue
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
        event.issue.remove_label(&ctx.github, label).await?;
    }

    Ok(())
}
