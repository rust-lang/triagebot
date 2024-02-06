use crate::config::ConvertedToDraftConfig;
use crate::github::{IssuesAction, IssuesEvent, Label};
use crate::handlers::Context;

pub(crate) struct ConvertedToDraftInput {}

pub(crate) async fn parse_input(
    _ctx: &Context,
    event: &IssuesEvent,
    _config: Option<&ConvertedToDraftConfig>,
) -> Result<Option<ConvertedToDraftInput>, String> {
    // PR author requests a review from one of the assignees

    match &event.action {
        IssuesAction::ConvertedToDraft => Ok(Some(ConvertedToDraftInput {})),
        _ => Ok(None),
    }
}

pub(crate) async fn handle_input(
    ctx: &Context,
    config: &ConvertedToDraftConfig,
    event: &IssuesEvent,
    ConvertedToDraftInput {}: ConvertedToDraftInput,
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
