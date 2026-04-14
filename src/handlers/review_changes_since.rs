use std::sync::{Arc, LazyLock};

use anyhow::Context as _;

use crate::{
    cache,
    config::ReviewChangesSinceConfig,
    github::{Comment, Event, Issue, IssueCommentAction, IssueCommentEvent},
    handlers::Context,
};

static REVIEW_BODY_CACHE: LazyLock<
    tokio::sync::Mutex<cache::LeastRecentlyUsedCache<String, ReviewBodyState>>,
> = LazyLock::new(|| tokio::sync::Mutex::new(cache::LeastRecentlyUsedCache::new(1000)));

#[derive(Copy, Clone, Debug)]
enum ReviewBodyState {
    Present,
    Absent,
}

impl cache::EstimatedSize for ReviewBodyState {
    fn estimated_size(&self) -> usize {
        std::mem::size_of::<Self>()
    }
}

/// Checks if this event is a PR review creation and adds in the body (if there is one)
/// a link our `gh-changes-since` endpoint to view changes since this review.
pub(crate) async fn handle(
    ctx: &Context,
    host: &str,
    event: &Event,
    _config: &ReviewChangesSinceConfig,
) -> anyhow::Result<()> {
    // Match on each review and top-level review comment
    if let Event::IssueComment(
        event @ IssueCommentEvent {
            action: IssueCommentAction::Created,
            issue:
                Issue {
                    pull_request: Some(_),
                    merged: false,
                    ..
                },
            comment:
                Comment {
                    in_reply_to_id: None,
                    ..
                },
            ..
        },
    ) = event
        && (
            // review
            event.comment.pr_review_state.is_some()
            // review comments
            || event.comment.pull_request_review_id.is_some()
        )
    {
        let issue_repo = event.issue.repository();
        let pr_num = event.issue.number;

        let base = &event.issue.base.as_ref().context("no base")?.sha;
        let head = &event.issue.head.as_ref().context("no head")?.sha;

        let link = format!("https://{host}/gh-changes-since/{issue_repo}/{pr_num}/{base}..{head}");

        if event.comment.pr_review_state.is_some() {
            // this is a review (not a review comment)

            {
                // first let's store it's review body state in the cache to avoid future api calls
                // when the review comments webhook arrives (a few milliseconds after)
                let cache_key = format!(
                    "{}/{}/{}",
                    &event.repository.full_name, event.issue.number, event.comment.id
                );
                REVIEW_BODY_CACHE.lock().await.put(
                    cache_key,
                    Arc::new(if event.comment.body.is_empty() {
                        ReviewBodyState::Absent
                    } else {
                        ReviewBodyState::Present
                    }),
                );
            }

            if !event.comment.body.is_empty() {
                // the review body is not empty, we can add to it the link to
                // our gh-changes-since endpoint
                let new_body = format!(
                    "{}\n\n*[View changes since this review]({link})*",
                    event.comment.body,
                );

                event
                    .issue
                    .edit_review(&ctx.github, event.comment.id, &new_body)
                    .await
                    .context("failed to update the review body")?;
            }
        } else if !event.comment.body.is_empty()
            && let Some(review_id) = event.comment.pull_request_review_id
        {
            // this is a review comment (not a review), we need to check if the parent
            // review already has a body (and as such a link)

            // fetch the parent review body state, first look into the cache
            let review_body_state = {
                let cache_key = format!(
                    "{}/{}/{}",
                    &event.repository.full_name, event.issue.number, review_id
                );
                match { REVIEW_BODY_CACHE.lock().await.get(&cache_key) } {
                    Some(state) => *state,
                    None => {
                        let review = event
                            .issue
                            .get_review(&ctx.github, review_id)
                            .await
                            .context("unable to fetch the parent review")?;
                        let state = if review.body.is_empty() {
                            ReviewBodyState::Absent
                        } else {
                            ReviewBodyState::Present
                        };
                        REVIEW_BODY_CACHE
                            .lock()
                            .await
                            .put(cache_key, Arc::new(state));
                        state
                    }
                }
            };

            if let ReviewBodyState::Absent = review_body_state {
                // parent review is empty, let's add the link to the review comment instead

                let new_body = format!(
                    "{}\n\n*[View changes since the review]({link})*",
                    event.comment.body
                );

                event
                    .issue
                    .edit_review_comment(&ctx.github, event.comment.id, &new_body)
                    .await
                    .context("failed to update the review comment body")?;
            }
        }
    }

    Ok(())
}
