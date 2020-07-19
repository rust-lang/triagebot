use crate::config::{self, Config, ConfigurationError};
use crate::github::{Event, GithubClient, IssuesAction, IssuesEvent};
use octocrab::Octocrab;
use parser::command::{Command, Input};
use std::fmt;
use std::sync::Arc;
use tokio_postgres::Client as DbClient;

#[derive(Debug)]
pub enum HandlerError {
    Message(String),
    Other(anyhow::Error),
}

impl std::error::Error for HandlerError {}

impl fmt::Display for HandlerError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            HandlerError::Message(msg) => write!(f, "{}", msg),
            HandlerError::Other(_) => write!(f, "An internal error occurred."),
        }
    }
}

mod assign;
mod autolabel;
mod glacier;
mod major_change;
mod nominate;
mod notification;
mod notify_zulip;
mod ping;
mod prioritize;
mod relabel;
mod rustc_commits;

// TODO: Return multiple handler errors ?
pub async fn handle(ctx: &Context, event: &Event) -> Result<(), HandlerError> {
    let config = config::get(&ctx.github, event.repo_name()).await;

    if let (Ok(config), Event::Issue(event)) = (config.as_ref(), event) {
        handle_issue(ctx, event, config).await?;
    }

    if let Some(body) = event.comment_body() {
        handle_command(ctx, event, &config, body).await?;
    }

    if let Err(e) = notification::handle(ctx, event).await {
        log::error!(
            "failed to process event {:?} with notification handler: {:?}",
            event,
            e
        );
    }

    if let Err(e) = rustc_commits::handle(ctx, event).await {
        log::error!(
            "failed to process event {:?} with rustc_commits handler: {:?}",
            event,
            e
        );
    }

    Ok(())
}

macro_rules! issue_handlers {
    ($($name:ident,)*) => {
        async fn handle_issue(ctx: &Context, event: &IssuesEvent, config: &Arc<Config>) -> Result<(), HandlerError> {
            $(
            if let Some(input) = $name::parse_input(
                ctx, event, config.$name.as_ref(),
            ).map_err(HandlerError::Message)? {
                if let Some(config) = &config.$name {
                    $name::handle_input(ctx, config, event, input).await.map_err(HandlerError::Other)?;
                } else {
                    return Err(HandlerError::Message(format!(
                        "The feature `{}` is not enabled in this repository.\n\
                        To enable it add its section in the `triagebot.toml` \
                        in the root of the repository.",
                        stringify!($name)
                    )));
                }
            })*
            Ok(())
        }
    }
}

// Handle events that happend on issues
//
// This is for events that happends only on issues (e.g. label changes).
// Each module in the list must contain the functions `parse_input` and `handle_input`.
issue_handlers! {
    autolabel,
    major_change,
    notify_zulip,
}

macro_rules! command_handlers {
    ($($name:ident: $enum:ident,)*) => {
        async fn handle_command(ctx: &Context, event: &Event, config: &Result<Arc<Config>, ConfigurationError>, body: &str) -> Result<(), HandlerError> {
            if let Event::Issue(e) = event {
                if !matches!(e.action, IssuesAction::Opened | IssuesAction::Edited) {
                    // no change in issue's body for these events, so skip
                    log::debug!("skipping event, issue was {:?}", e.action);
                    return Ok(());
                }
            }

            // TODO: parse multiple commands and diff them
            let mut input = Input::new(&body, &ctx.username);
            let command = input.parse_command();

            if let Some(previous) = event.comment_from() {
                let mut prev_input = Input::new(&previous, &ctx.username);
                let prev_command = prev_input.parse_command();
                if command == prev_command {
                    log::info!("skipping unmodified command: {:?} -> {:?}", prev_command, command);
                    return Ok(());
                } else {
                    log::debug!("executing modified command: {:?} -> {:?}", prev_command, command);
                }
            }

            if command == Command::None {
                return Ok(());
            }

            let config = match config {
                Ok(config) => config,
                Err(e @ ConfigurationError::Missing) => {
                    return Err(HandlerError::Message(e.to_string()));
                }
                Err(e @ ConfigurationError::Toml(_)) => {
                    return Err(HandlerError::Message(e.to_string()));
                }
                Err(e @ ConfigurationError::Http(_)) => {
                    return Err(HandlerError::Other(e.clone().into()));
                }
            };

            match command {
                $(
                Command::$enum(Ok(command)) => {
                    if let Some(config) = &config.$name {
                        $name::handle_command(ctx, config, event, command).await.map_err(HandlerError::Other)?;
                    } else {
                        return Err(HandlerError::Message(format!(
                            "The feature `{}` is not enabled in this repository.\n\
                            To enable it add its section in the `triagebot.toml` \
                            in the root of the repository.",
                            stringify!($name)
                        )));
                    }
                }
                Command::$enum(Err(err)) => {
                    return Err(HandlerError::Message(format!(
                        "Parsing {} command in [comment]({}) failed: {}",
                        stringify!($name),
                        event.html_url().expect("has html url"),
                        err
                    )));
                })*
                Command::None => unreachable!(),
            }
            Ok(())
        }
    }
}

// Handle commands in comments/issues body
//
// This is for handlers for commands parsed by the `parser` crate.
// Each variant of `parser::command::Command` must be in this list,
// preceded by the module containing the coresponding `handle_command` function
command_handlers! {
    assign: Assign,
    glacier: Glacier,
    nominate: Nominate,
    ping: Ping,
    prioritize: Prioritize,
    relabel: Relabel,
    major_change: Second,
}

pub struct Context {
    pub github: GithubClient,
    pub db: DbClient,
    pub username: String,
    pub octocrab: Octocrab,
}
