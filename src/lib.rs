#![allow(clippy::new_without_default)]

use anyhow::Context;
use handlers::HandlerError;
use interactions::ErrorComment;
use std::fmt;

pub mod config;
pub mod db;
pub mod github;
pub mod handlers;
pub mod interactions;
pub mod notification_listing;
pub mod payload;
pub mod team;
pub mod zulip;

pub enum EventName {
    PullRequestReview,
    PullRequestReviewComment,
    IssueComment,
    Issue,
    Other,
}

impl std::str::FromStr for EventName {
    type Err = std::convert::Infallible;
    fn from_str(s: &str) -> Result<EventName, Self::Err> {
        Ok(match s {
            "pull_request_review" => EventName::PullRequestReview,
            "pull_request_review_comment" => EventName::PullRequestReviewComment,
            "issue_comment" => EventName::IssueComment,
            "issues" => EventName::Issue,
            _ => EventName::Other,
        })
    }
}

impl fmt::Display for EventName {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                EventName::PullRequestReview => "pull_request_review",
                EventName::PullRequestReviewComment => "pull_request_review_comment",
                EventName::IssueComment => "issue_comment",
                EventName::Issue => "issues",
                EventName::Other => "other",
            }
        )
    }
}

#[derive(Debug)]
pub struct WebhookError(anyhow::Error);

impl From<anyhow::Error> for WebhookError {
    fn from(e: anyhow::Error) -> WebhookError {
        WebhookError(e)
    }
}

pub fn deserialize_payload<T: serde::de::DeserializeOwned>(v: &str) -> anyhow::Result<T> {
    Ok(serde_json::from_str(&v).with_context(|| format!("input: {:?}", v))?)
}

pub async fn webhook(
    event: EventName,
    payload: String,
    ctx: &handlers::Context,
) -> Result<(), WebhookError> {
    let event = match event {
        EventName::PullRequestReview => {
            let payload = deserialize_payload::<github::PullRequestReviewEvent>(&payload)
                .context("IssueCommentEvent failed to deserialize")
                .map_err(anyhow::Error::from)?;

            log::info!("handling pull request review comment {:?}", payload);

            // Treat pull request review comments exactly like pull request
            // review comments.
            github::Event::IssueComment(github::IssueCommentEvent {
                action: match payload.action {
                    github::PullRequestReviewAction::Submitted => {
                        github::IssueCommentAction::Created
                    }
                    github::PullRequestReviewAction::Edited => github::IssueCommentAction::Edited,
                    github::PullRequestReviewAction::Dismissed => {
                        github::IssueCommentAction::Deleted
                    }
                },
                issue: payload.pull_request,
                comment: payload.review,
                repository: payload.repository,
            })
        }
        EventName::PullRequestReviewComment => {
            let payload = deserialize_payload::<github::PullRequestReviewComment>(&payload)
                .context("IssueCommentEvent failed to deserialize")
                .map_err(anyhow::Error::from)?;

            log::info!("handling pull request review comment {:?}", payload);

            // Treat pull request review comments exactly like pull request
            // review comments.
            github::Event::IssueComment(github::IssueCommentEvent {
                action: payload.action,
                issue: payload.issue,
                comment: payload.comment,
                repository: payload.repository,
            })
        }
        EventName::IssueComment => {
            let payload = deserialize_payload::<github::IssueCommentEvent>(&payload)
                .context("IssueCommentEvent failed to deserialize")
                .map_err(anyhow::Error::from)?;

            log::info!("handling issue comment {:?}", payload);

            github::Event::IssueComment(payload)
        }
        EventName::Issue => {
            let payload = deserialize_payload::<github::IssuesEvent>(&payload)
                .context("IssuesEvent failed to deserialize")
                .map_err(anyhow::Error::from)?;

            log::info!("handling issue event {:?}", payload);

            github::Event::Issue(payload)
        }
        // Other events need not be handled
        EventName::Other => {
            return Ok(());
        }
    };
    if let Err(err) = handlers::handle(&ctx, &event).await {
        match err {
            HandlerError::Message(message) => {
                if let Some(issue) = event.issue() {
                    let cmnt = ErrorComment::new(issue, message);
                    cmnt.post(&ctx.github).await?;
                }
            }
            HandlerError::Other(err) => {
                log::error!("handling event failed: {:?}", err);
                return Err(WebhookError(anyhow::anyhow!(
                    "handling failed, error logged",
                )));
            }
        }
    }
    Ok(())
}
