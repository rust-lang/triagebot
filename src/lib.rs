#![feature(async_await)]
#![allow(clippy::new_without_default)]

use failure::{Error, ResultExt};

use interactions::ErrorComment;

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

#[derive(Debug)]
pub struct WebhookError(Error);

impl From<Error> for WebhookError {
    fn from(e: Error) -> WebhookError {
        WebhookError(e)
    }
}

pub fn deserialize_payload<T: serde::de::DeserializeOwned>(v: &str) -> Result<T, Error> {
    Ok(serde_json::from_str(&v).with_context(|_| format!("input: {:?}", v))?)
}

pub async fn webhook(
    event: EventName,
    payload: String,
    ctx: &handlers::Context,
) -> Result<(), WebhookError> {
    match event {
        EventName::IssueComment => {
            let payload = deserialize_payload::<github::IssueCommentEvent>(&payload)
                .context("IssueCommentEvent failed to deserialize")
                .map_err(Error::from)?;

            let event = github::Event::IssueComment(payload);
            if let Err(err) = handlers::handle(&ctx, &event).await {
                if let Some(issue) = event.issue() {
                    let cmnt = ErrorComment::new(issue, err.to_string());
                    cmnt.post(&ctx.github).await?;
                }
                return Err(err.into());
            }
        }
        EventName::Issue => {
            let payload = deserialize_payload::<github::IssuesEvent>(&payload)
                .context("IssuesEvent failed to deserialize")
                .map_err(Error::from)?;

            let event = github::Event::Issue(payload);
            if let Err(err) = handlers::handle(&ctx, &event).await {
                if let Some(issue) = event.issue() {
                    let cmnt = ErrorComment::new(issue, err.to_string());
                    cmnt.post(&ctx.github).await?;
                }
                return Err(err.into());
            }
        }
        // Other events need not be handled
        EventName::Other => {}
    }
    Ok(())
}
