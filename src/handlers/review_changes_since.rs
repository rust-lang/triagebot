use anyhow::Context as _;

use crate::{
    config::ReviewChangesSinceConfig,
    github::{Comment, Event, Issue, IssueCommentAction, IssueCommentEvent},
    handlers::Context,
};

/// Checks if this event is a PR review creation and adds in the body a link our `gh-changes-since`
/// endpoint to view changes since this review.
pub(crate) async fn handle(
    ctx: &Context,
    host: &str,
    event: &Event,
    _config: &ReviewChangesSinceConfig,
) -> anyhow::Result<()> {
    if let Event::IssueComment(
        event @ IssueCommentEvent {
            action: IssueCommentAction::Created,
            issue: Issue {
                pull_request: Some(_),
                ..
            },
            comment:
                Comment {
                    pr_review_state: Some(_),
                    ..
                },
            ..
        },
    ) = event
    {
        // Add link our gh-changes-since endpoint to view changes since this review

        let issue_repo = event.issue.repository();
        let pr_num = event.issue.number;

        let base = &event.issue.base.as_ref().context("no base")?.sha;
        let head = &event.issue.head.as_ref().context("no head")?.sha;

        let link = format!("https://{host}/gh-changes-since/{issue_repo}/{pr_num}/{base}..{head}");
        let new_body = format!(
            "{}\n\n*[View changes since this review]({link})*",
            event.comment.body
        );

        event
            .issue
            .edit_review(&ctx.github, event.comment.id, &new_body)
            .await
            .context("failed to update the review body")?;
    }

    Ok(())
}
