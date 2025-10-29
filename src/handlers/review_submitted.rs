use anyhow::Context as _;

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

        // Let's switch the review labels if the user who issued the changes requested:
        //  - is one of the assignees
        //  - or has write/admin permission on the repository
        if event.issue.assignees.contains(&event.comment.user) || {
            let perm = event
                .issue
                .repository()
                .collaborator_permission(&ctx.github, &event.comment.user.login)
                .await
                .context("failed to get the user repository permission")?;

            perm.permission == "write" || perm.permission == "admin"
        } {
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
