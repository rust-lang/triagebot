use std::sync::{Arc, LazyLock};

use anyhow::Context as _;
use async_trait::async_trait;
use chrono::{Duration, Utc};

use crate::{
    cache,
    config::ReviewChangesSinceConfig,
    github::{Comment, Event, Issue, IssueCommentAction, IssueCommentEvent, IssueRepository},
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
                    merged_at: None,
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

                // unfortunately, review comments are not updated by GitHub in the UI, which
                // creates friction for contributors as they might want to edit the review
                // comment, but get blocked since we modified it in the mean time
                //
                // we therefore defer adding the link by one minute

                let pr_repo = event.issue.repository();
                let args = AddReviewChangesSinceLinkJobArgs {
                    org: pr_repo.organization.clone(),
                    repo: pr_repo.repository.clone(),
                    pr_id: event.issue.number,
                    review_comment_id: event.comment.id,
                    link,
                };

                crate::db::schedule_job(
                    &*ctx.db.get().await,
                    ADD_REVIEW_CHANGES_SINCE_LINK_JOB_NAME,
                    serde_json::to_value(args)?,
                    Utc::now() + Duration::minutes(5),
                ).await.context("failed to setup the job to add the review changes since link to the review comment")?;
            }
        }
    }

    Ok(())
}

#[derive(serde::Serialize, serde::Deserialize)]
struct AddReviewChangesSinceLinkJobArgs {
    org: String,
    repo: String,
    pr_id: u64,
    review_comment_id: u64,
    link: String,
}

pub(crate) struct AddReviewChangesSinceLinkJob;

const ADD_REVIEW_CHANGES_SINCE_LINK_JOB_NAME: &str = "add_review_changes_since_link_job";

#[async_trait]
impl crate::jobs::Job for AddReviewChangesSinceLinkJob {
    fn name(&self) -> &str {
        ADD_REVIEW_CHANGES_SINCE_LINK_JOB_NAME
    }

    async fn run(&self, ctx: &Context, metadata: &serde_json::Value) -> anyhow::Result<()> {
        let inner = async {
            let args: AddReviewChangesSinceLinkJobArgs = serde_json::from_value(metadata.clone())
                .with_context(|| {
                format!("failed to deserialize the metadata {metadata:?} into args")
            })?;

            let pr_repo = IssueRepository {
                organization: args.org.clone(),
                repository: args.repo.clone(),
            };

            let pr = ctx
                .github
                .pull_request(&pr_repo, args.pr_id)
                .await
                .context("failed to get the pr")?;

            let review_comment = pr
                .get_review_comment(&ctx.github, args.review_comment_id)
                .await
                .context("couldn't get the review comment")?;

            let new_body = format!(
                "{}\n\n*[View changes since the review]({})*",
                review_comment.body, &args.link
            );

            pr.edit_review_comment(&ctx.github, args.review_comment_id, &new_body)
                .await
                .context("failed to update the review comment body")?;

            anyhow::Ok(())
        };

        // If the inner block fails, print the error but don't bubble it up
        if let Err(err) = inner.await {
            tracing::error!("{ADD_REVIEW_CHANGES_SINCE_LINK_JOB_NAME} failed (no retry): {err:#?}");
        }

        Ok(())
    }
}
