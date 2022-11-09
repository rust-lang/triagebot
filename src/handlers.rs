use crate::config::{self, Config, ConfigurationError};
use crate::github::{Event, GithubClient, IssueCommentAction, IssuesAction, IssuesEvent};
use octocrab::Octocrab;
use parser::command::{assign::AssignCommand, Command, Input};
use std::fmt;
use std::sync::Arc;
use tracing as log;

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
mod close;
mod decision;
pub mod docs_update;
mod github_releases;
mod glacier;
pub mod jobs;
mod major_change;
mod mentions;
mod milestone_prs;
mod no_merges;
mod nominate;
mod note;
mod notification;
mod notify_zulip;
mod ping;
mod prioritize;
mod relabel;
mod review_submitted;
mod rfc_helper;
pub mod rustc_commits;
mod shortcut;

pub async fn handle(ctx: &Context, event: &Event) -> Vec<HandlerError> {
    let config = config::get(&ctx.github, event.repo()).await;
    if let Err(e) = &config {
        log::warn!("configuration error {}: {e}", event.repo().full_name);
    }
    let mut errors = Vec::new();

    if let (Ok(config), Event::Issue(event)) = (config.as_ref(), event) {
        handle_issue(ctx, event, config, &mut errors).await;
    }

    if let Some(body) = event.comment_body() {
        handle_command(ctx, event, &config, body, &mut errors).await;
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

    if let Err(e) = milestone_prs::handle(ctx, event).await {
        log::error!(
            "failed to process event {:?} with milestone_prs handler: {:?}",
            event,
            e
        );
    }

    if let Err(e) = rfc_helper::handle(ctx, event).await {
        log::error!(
            "failed to process event {:?} with rfc_helper handler: {:?}",
            event,
            e
        );
    }

    if let Some(config) = config
        .as_ref()
        .ok()
        .and_then(|c| c.review_submitted.as_ref())
    {
        if let Err(e) = review_submitted::handle(ctx, event, config).await {
            log::error!(
                "failed to process event {:?} with review_submitted handler: {:?}",
                event,
                e
            )
        }
    }

    if let Some(ghr_config) = config
        .as_ref()
        .ok()
        .and_then(|c| c.github_releases.as_ref())
    {
        if let Err(e) = github_releases::handle(ctx, event, ghr_config).await {
            log::error!(
                "failed to process event {:?} with github_releases handler: {:?}",
                event,
                e
            );
        }
    }

    errors
}

macro_rules! issue_handlers {
    ($($name:ident,)*) => {
        async fn handle_issue(
            ctx: &Context,
            event: &IssuesEvent,
            config: &Arc<Config>,
            errors: &mut Vec<HandlerError>,
        ) {
            $(
            match $name::parse_input(ctx, event, config.$name.as_ref()).await {
                Err(err) => errors.push(HandlerError::Message(err)),
                Ok(Some(input)) => {
                    if let Some(config) = &config.$name {
                        $name::handle_input(ctx, config, event, input).await.unwrap_or_else(|err| errors.push(HandlerError::Other(err)));
                    } else {
                        errors.push(HandlerError::Message(format!(
                            "The feature `{}` is not enabled in this repository.\n\
                            To enable it add its section in the `triagebot.toml` \
                            in the root of the repository.",
                            stringify!($name)
                        )));
                    }
                }
                Ok(None) => {}
            })*
        }
    }
}

// Handle events that happened on issues
//
// This is for events that happen only on issues (e.g. label changes).
// Each module in the list must contain the functions `parse_input` and `handle_input`.
issue_handlers! {
    assign,
    autolabel,
    major_change,
    mentions,
    no_merges,
    notify_zulip,
}

macro_rules! command_handlers {
    ($($name:ident: $enum:ident,)*) => {
        async fn handle_command(
            ctx: &Context,
            event: &Event,
            config: &Result<Arc<Config>, ConfigurationError>,
            body: &str,
            errors: &mut Vec<HandlerError>,
        ) {
            match event {
                Event::Issue(e) => if !matches!(e.action, IssuesAction::Opened | IssuesAction::Edited) {
                    // no change in issue's body for these events, so skip
                    log::debug!("skipping event, issue was {:?}", e.action);
                    return;
                }
                Event::IssueComment(e) => if e.action == IssueCommentAction::Deleted {
                    // don't execute commands again when comment is deleted
                    log::debug!("skipping event, comment was {:?}", e.action);
                    return;
                }
                Event::Push(_) | Event::Create(_) => {
                    log::debug!("skipping unsupported event");
                    return;
                }
            }

            let input = Input::new(&body, vec![&ctx.username, "triagebot"]);
            let commands = if let Some(previous) = event.comment_from() {
                let prev_commands = Input::new(&previous, vec![&ctx.username, "triagebot"]).collect::<Vec<_>>();
                input.filter(|cmd| !prev_commands.contains(cmd)).collect::<Vec<_>>()
            } else {
                input.collect()
            };

            log::info!("Comment parsed to {:?}", commands);

            if commands.is_empty() {
                return;
            }

            let config = match config {
                Ok(config) => config,
                Err(e @ ConfigurationError::Missing) => {
                    // r? is conventionally used to mean "hey, can you review"
                    // even if the repo doesn't have a triagebot.toml. In that
                    // case, just ignore it.
                    if commands
                        .iter()
                        .all(|cmd| matches!(cmd, Command::Assign(Ok(AssignCommand::ReviewName { .. }))))
                    {
                        return;
                    }
                    return errors.push(HandlerError::Message(e.to_string()));
                }
                Err(e @ ConfigurationError::Toml(_)) => {
                    return errors.push(HandlerError::Message(e.to_string()));
                }
                Err(e @ ConfigurationError::Http(_)) => {
                    return errors.push(HandlerError::Other(e.clone().into()));
                }
            };

            for command in commands {
                match command {
                    $(
                    Command::$enum(Ok(command)) => {
                        if let Some(config) = &config.$name {
                            $name::handle_command(ctx, config, event, command)
                                .await
                                .unwrap_or_else(|err| errors.push(HandlerError::Other(err)));
                        } else {
                            errors.push(HandlerError::Message(format!(
                                "The feature `{}` is not enabled in this repository.\n\
                                To enable it add its section in the `triagebot.toml` \
                                in the root of the repository.",
                                stringify!($name)
                            )));
                        }
                    }
                    Command::$enum(Err(err)) => {
                        errors.push(HandlerError::Message(format!(
                            "Parsing {} command in [comment]({}) failed: {}",
                            stringify!($name),
                            event.html_url().expect("has html url"),
                            err
                        )));
                    })*
                }
            }
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
    shortcut: Shortcut,
    close: Close,
    note: Note,
    decision: Decision,
}

pub struct Context {
    pub github: GithubClient,
    pub db: crate::db::ClientPool,
    pub username: String,
    pub octocrab: Octocrab,
}
