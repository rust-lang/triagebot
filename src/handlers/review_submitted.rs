use crate::github::{Issue, IssueCommentAction, IssueCommentEvent, Label, PullRequestReviewState};
use crate::{config::ReviewSubmittedConfig, github::Event, handlers::Context};

pub(crate) async fn handle(
    ctx: &Context,
    event: &Event,
    config: &ReviewSubmittedConfig,
) -> anyhow::Result<()> {
    if let Event::IssueComment(
        event
        @
        IssueCommentEvent {
            action: IssueCommentAction::Created,
            issue: Issue {
                pull_request: Some(_),
                ..
            },
            ..
        },
    ) = event
    {
        if event.comment.pr_review_state != PullRequestReviewState::ChangesRequested {
            return Ok(());
        }

        if event.issue.assignees.contains(&event.comment.user) {
            let labels = event
                .issue
                .labels()
                .iter()
                // Remove review related labels
                .filter(|label| !config.review_labels.contains(&label.name))
                .cloned()
                // Add waiting on author label
                .chain(std::iter::once(Label {
                    name: config.reviewed_label.clone(),
                }));
            event
                .issue
                .set_labels(&ctx.github, labels.collect())
                .await?;
        }
    }

    Ok(())
}
