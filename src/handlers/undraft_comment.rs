use anyhow::Context as _;

use crate::config::UndraftCommentConfig;
use crate::github::{IssuesAction, IssuesEvent, ReportedContentClassifiers};
use crate::handlers::Context;

pub(crate) struct UndraftCommentInput;

pub(crate) async fn parse_input(
    _ctx: &Context,
    event: &IssuesEvent,
    config: Option<&UndraftCommentConfig>,
) -> Result<Option<UndraftCommentInput>, String> {
    if config.is_none() {
        return Ok(None);
    }

    if event.action != IssuesAction::ReadyForReview {
        return Ok(None);
    }

    Ok(Some(UndraftCommentInput {}))
}

pub(crate) async fn handle_input(
    ctx: &Context,
    _config: &UndraftCommentConfig,
    event: &IssuesEvent,
    _input: UndraftCommentInput,
) -> anyhow::Result<()> {
    let comment = event
        .issue
        .post_comment(
            &ctx.github,
            "This pull request has been undrafted and is now ready for review.",
        )
        .await
        .context("couldn't post the undraft comment")?;

    event
        .issue
        .hide_comment(
            &ctx.github,
            &comment.node_id,
            ReportedContentClassifiers::Resolved,
        )
        .await
        .context("couldn't hide the undraft comment")?;

    Ok(())
}
