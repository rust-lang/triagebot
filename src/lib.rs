#![allow(clippy::new_without_default)]

#[macro_use]
extern crate lazy_static;

use anyhow::Context;
use handlers::HandlerError;
use interactions::ErrorComment;
use std::fmt;

pub mod actions;
pub mod agenda;
pub mod config;
pub mod db;
pub mod github;
pub mod handlers;
pub mod interactions;
pub mod logger;
pub mod notification_listing;
pub mod payload;
pub mod team;
mod team_data;
pub mod zulip;

#[derive(Debug)]
pub enum EventName {
    PullRequest,
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
            "pull_request" => EventName::PullRequest,
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
                EventName::PullRequest => "pull_request",
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
    let mut deserializer = serde_json::Deserializer::from_str(&v);
    let res: Result<T, _> = serde_path_to_error::deserialize(&mut deserializer);
    match res {
        Ok(r) => Ok(r),
        Err(e) => {
            let ctx = format!("at {:?}", e.path());
            Err(e.into_inner()).context(ctx)
        }
    }
}

pub async fn webhook(
    event: EventName,
    payload: String,
    ctx: &handlers::Context,
) -> Result<bool, WebhookError> {
    let event = match event {
        EventName::PullRequestReview => {
            let payload = deserialize_payload::<github::PullRequestReviewEvent>(&payload)
                .context("PullRequestReview failed to deserialize")
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
                changes: payload.changes,
                issue: payload.pull_request,
                comment: payload.review,
                repository: payload.repository,
            })
        }
        EventName::PullRequestReviewComment => {
            let payload = deserialize_payload::<github::PullRequestReviewComment>(&payload)
                .context("PullRequestReview(Comment) failed to deserialize")
                .map_err(anyhow::Error::from)?;

            log::info!("handling pull request review comment {:?}", payload);

            // Treat pull request review comments exactly like pull request
            // review comments.
            github::Event::IssueComment(github::IssueCommentEvent {
                action: payload.action,
                changes: payload.changes,
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
        EventName::Issue | EventName::PullRequest => {
            let payload = deserialize_payload::<github::IssuesEvent>(&payload)
                .context(format!("{:?} failed to deserialize", event))
                .map_err(anyhow::Error::from)?;

            log::info!("handling issue event {:?}", payload);

            github::Event::Issue(payload)
        }
        // Other events need not be handled
        EventName::Other => {
            return Ok(false);
        }
    };
    let errors = handlers::handle(&ctx, &event).await;
    let mut other_error = false;
    let mut message = String::new();
    for err in errors {
        match err {
            HandlerError::Message(msg) => {
                if !message.is_empty() {
                    message.push_str("\n\n");
                }
                message.push_str(&msg);
            }
            HandlerError::Other(err) => {
                log::error!("handling event failed: {:?}", err);
                other_error = true;
            }
        }
    }
    if !message.is_empty() {
        if let Some(issue) = event.issue() {
            let cmnt = ErrorComment::new(issue, message);
            cmnt.post(&ctx.github).await?;
        }
    }
    if other_error {
        Err(WebhookError(anyhow::anyhow!(
            "handling failed, error logged",
        )))
    } else {
        Ok(true)
    }
}
