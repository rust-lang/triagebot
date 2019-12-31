#![allow(clippy::new_without_default)]

use anyhow::Context;
use handlers::HandlerError;
use interactions::ErrorComment;
use std::fmt;

pub mod config;
pub mod github;
pub mod handlers;
pub mod interactions;
pub mod payload;
pub mod team;

pub enum EventName {
    IssueComment,
    Issue,
    Other,
}

impl std::str::FromStr for EventName {
    type Err = std::convert::Infallible;
    fn from_str(s: &str) -> Result<EventName, Self::Err> {
        Ok(match s {
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
