use std::{fmt, sync::Arc};

use axum::{extract::State, response::IntoResponse};
use axum_extra::extract::Host;
use hmac::{Hmac, Mac};
use hyper::HeaderMap;
use sha2::Sha256;
use tracing::debug;

use crate::{handlers::HandlerError, interactions::ErrorComment};

use super::*;

/// The name of a webhook event.
#[derive(Debug)]
pub enum EventName {
    /// Pull request activity.
    ///
    /// This gets translated to [`github::Event::Issue`] when sent to a handler.
    ///
    /// <https://docs.github.com/en/developers/webhooks-and-events/webhooks/webhook-events-and-payloads#pull_request>
    PullRequest,
    /// Pull request review activity.
    ///
    /// This gets translated to [`github::Event::IssueComment`] when sent to a handler.
    ///
    /// <https://docs.github.com/en/developers/webhooks-and-events/webhooks/webhook-events-and-payloads#pull_request_review>
    PullRequestReview,
    /// A comment on a pull request review.
    ///
    /// This gets translated to [`github::Event::IssueComment`] when sent to a handler.
    ///
    /// <https://docs.github.com/en/developers/webhooks-and-events/webhooks/webhook-events-and-payloads#pull_request_review_comment>
    PullRequestReviewComment,
    /// An issue or PR comment.
    ///
    /// This gets translated to [`github::Event::IssueComment`] when sent to a handler.
    ///
    /// <https://docs.github.com/en/developers/webhooks-and-events/webhooks/webhook-events-and-payloads#issue_comment>
    IssueComment,
    /// Issue activity.
    ///
    /// This gets translated to [`github::Event::Issue`] when sent to a handler.
    ///
    /// <https://docs.github.com/en/developers/webhooks-and-events/webhooks/webhook-events-and-payloads#issues>
    Issue,
    /// One or more commits are pushed to a repository branch or tag.
    ///
    /// This gets translated to [`github::Event::Push`] when sent to a handler.
    ///
    /// <https://docs.github.com/en/developers/webhooks-and-events/webhooks/webhook-events-and-payloads#push>
    Push,
    /// A Git branch or tag is created.
    ///
    /// This gets translated to [`github::Event::Create`] when sent to a handler.
    ///
    /// <https://docs.github.com/en/developers/webhooks-and-events/webhooks/webhook-events-and-payloads#create>
    Create,
    /// All other unhandled webhooks.
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
            "push" => EventName::Push,
            "create" => EventName::Create,
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
                EventName::Push => "push",
                EventName::Create => "create",
                EventName::Other => "other",
            }
        )
    }
}

pub fn deserialize_payload<T: serde::de::DeserializeOwned>(v: &str) -> anyhow::Result<T> {
    let mut deserializer = serde_json::Deserializer::from_str(v);
    let res: Result<T, _> = serde_path_to_error::deserialize(&mut deserializer);
    match res {
        Ok(r) => Ok(r),
        Err(e) => Err(anyhow::anyhow!("webhook payload: {v}").context(e)),
    }
}

pub async fn webhook(
    headers: HeaderMap,
    State(ctx): State<Arc<crate::handlers::Context>>,
    Host(host): Host,
    body: Bytes,
) -> axum::response::Response {
    // Extract X-GitHub-Event header
    let Some(ev) = headers.get("X-GitHub-Event") else {
        tracing::error!("X-GitHub-Event header must be set");
        return (StatusCode::BAD_REQUEST, "X-GitHub-Event header must be set").into_response();
    };
    let Ok(ev) = ev.to_str() else {
        tracing::error!("X-GitHub-Event header must be UTF-8 encoded");
        return (
            StatusCode::BAD_REQUEST,
            "X-GitHub-Event header must be UTF-8 encoded",
        )
            .into_response();
    };
    let Ok(event) = ev.parse::<EventName>();

    debug!("event={event}");

    // Extract X-Hub-Signature-256 header
    let Some(sig) = headers.get("X-Hub-Signature-256") else {
        tracing::error!("X-Hub-Signature-256 header must be set");
        return (
            StatusCode::BAD_REQUEST,
            "X-Hub-Signature-256 header must be set",
        )
            .into_response();
    };
    let Ok(signature) = sig.to_str() else {
        tracing::error!("X-Hub-Signature-256 header must be UTF-8 encoded");
        return (
            StatusCode::BAD_REQUEST,
            "X-Hub-Signature-256 header must be UTF-8 encoded",
        )
            .into_response();
    };

    debug!("signature={signature}");

    // Check signature on body
    if let Err(err) = check_payload_signed(signature, &body) {
        tracing::error!("check_payload_signed: {err}");
        return (StatusCode::FORBIDDEN, "Wrong signature").into_response();
    }

    let Ok(payload) = str::from_utf8(&body) else {
        tracing::error!("payload not utf-8");
        return (StatusCode::BAD_REQUEST, "Payload must be UTF-8").into_response();
    };

    match process_payload(event, payload, &ctx, &host).await {
        Ok(true) => ("processed request",).into_response(),
        Ok(false) => ("ignored request",).into_response(),
        Err(err) => {
            tracing::error!("{err:?}");
            let body = format!("request failed: {err:?}");
            (StatusCode::INTERNAL_SERVER_ERROR, body).into_response()
        }
    }
}

async fn process_payload(
    event: EventName,
    payload: &str,
    ctx: &crate::handlers::Context,
    host: &str,
) -> anyhow::Result<bool> {
    let event = match event {
        EventName::PullRequestReview => {
            let mut payload = deserialize_payload::<PullRequestReviewEvent>(payload)
                .context("failed to deserialize to PullRequestReviewEvent")?;

            log::info!("handling pull request review comment {payload:?}");
            payload.pull_request.pull_request = Some(PullRequestDetails::new());

            // Treat pull request review comments exactly like pull request
            // comments.
            Event::IssueComment(IssueCommentEvent {
                action: match payload.action {
                    PullRequestReviewAction::Submitted => IssueCommentAction::Created,
                    PullRequestReviewAction::Edited => IssueCommentAction::Edited,
                    PullRequestReviewAction::Dismissed => IssueCommentAction::Deleted,
                },
                changes: payload.changes,
                issue: payload.pull_request,
                comment: payload.review,
                repository: payload.repository,
            })
        }
        EventName::PullRequestReviewComment => {
            let mut payload = deserialize_payload::<PullRequestReviewComment>(payload)
                .context("failed to deserialize to PullRequestReviewComment")?;

            payload.issue.pull_request = Some(PullRequestDetails::new());

            log::info!("handling pull request review comment {payload:?}");

            // Treat pull request review comments exactly like pull request
            // review comments.
            Event::IssueComment(IssueCommentEvent {
                action: payload.action,
                changes: payload.changes,
                issue: payload.issue,
                comment: payload.comment,
                repository: payload.repository,
            })
        }
        EventName::IssueComment => {
            let payload = deserialize_payload::<IssueCommentEvent>(payload)
                .context("failed to deserialize IssueCommentEvent")?;

            log::info!("handling issue comment {payload:?}");

            Event::IssueComment(payload)
        }
        EventName::Issue | EventName::PullRequest => {
            let mut payload = deserialize_payload::<IssuesEvent>(payload)
                .context("failed to deserialize IssuesEvent")?;

            if matches!(event, EventName::PullRequest) {
                payload.issue.pull_request = Some(PullRequestDetails::new());
            }

            log::info!("handling issue event {payload:?}");

            Event::Issue(payload)
        }
        EventName::Push => {
            let payload = deserialize_payload::<PushEvent>(payload)
                .context("failed to deserialize to PushEvent")?;

            log::info!("handling push event {payload:?}");

            Event::Push(payload)
        }
        EventName::Create => {
            let payload = deserialize_payload::<CreateEvent>(payload)
                .context("failed to deserialize to CreateEvent")?;

            log::info!("handling create event {payload:?}");

            Event::Create(payload)
        }
        // Other events need not be handled
        EventName::Other => {
            return Ok(false);
        }
    };
    let errors = crate::handlers::handle(ctx, host, &event).await;
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
                log::error!("handling event failed: {err:?}");
                other_error = true;
            }
        }
    }
    if !message.is_empty() {
        log::info!("user error: {}", message);
        if let Some(issue) = event.issue() {
            let cmnt = ErrorComment::new(issue, message);
            cmnt.post(&ctx.github).await?;
        }
    }
    if other_error {
        Err(anyhow::anyhow!("handling failed, error logged"))
    } else {
        Ok(true)
    }
}

#[derive(Debug)]
pub struct SignedPayloadError;

impl fmt::Display for SignedPayloadError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "failed to validate payload")
    }
}

impl std::error::Error for SignedPayloadError {}

pub fn check_payload_signed(signature: &str, payload: &[u8]) -> Result<(), SignedPayloadError> {
    let signature = signature
        .strip_prefix("sha256=")
        .ok_or(SignedPayloadError)?;
    let signature = match hex::decode(signature) {
        Ok(e) => e,
        Err(e) => {
            tracing::trace!("hex decode failed for {signature:?}: {e:?}");
            return Err(SignedPayloadError);
        }
    };

    let mut mac = Hmac::<Sha256>::new_from_slice(
        std::env::var("GITHUB_WEBHOOK_SECRET")
            .expect("Missing GITHUB_WEBHOOK_SECRET")
            .as_bytes(),
    )
    .unwrap();
    mac.update(payload);
    mac.verify_slice(&signature).map_err(|_| SignedPayloadError)
}
