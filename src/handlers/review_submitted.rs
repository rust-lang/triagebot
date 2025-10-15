use crate::github::{Issue, IssueCommentAction, IssueCommentEvent, Label, PullRequestReviewState};
use crate::{config::ReviewSubmittedConfig, github::Event, handlers::Context};

pub(crate) async fn handle(
    ctx: &Context,
    event: &Event,
    config: &ReviewSubmittedConfig,
) -> anyhow::Result<()> {
    if let Event::IssueComment(
        event @ IssueCommentEvent {
            action: IssueCommentAction::Created,
            issue: Issue {
                pull_request: Some(_),
                ..
            },
            ..
        },
    ) = event
    {
        if event.comment.pr_review_state != Some(PullRequestReviewState::ChangesRequested) {
            return Ok(());
        }

        if event.issue.assignees.contains(&event.comment.user) {
            // Remove review labels
            event
                .issue
                .remove_labels(
                    &ctx.github,
                    config
                        .review_labels
                        .iter()
                        .map(|label| Label {
                            name: label.clone(),
                        })
                        .collect(),
                )
                .await?;

            // Add waiting on author
            event
                .issue
                .add_labels(
                    &ctx.github,
                    vec![Label {
                        name: config.reviewed_label.clone(),
                    }],
                )
                .await?;
        }
    }

    Ok(())
}
