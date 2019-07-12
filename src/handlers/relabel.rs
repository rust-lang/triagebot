//! Purpose: Allow any user to modify issue labels on GitHub via comments.
//!
//! Labels are checked against the labels in the project; the bot does not support creating new
//! labels.
//!
//! Parsing is done in the `parser::command::relabel` module.
//!
//! If the command was successful, there will be no feedback beyond the label change to reduce
//! notification noise.

use crate::{
    config::RelabelConfig,
    github::{self, Event, GithubClient},
    handlers::{Context, Handler},
    interactions::ErrorComment,
};
use failure::Error;
use futures::future::{BoxFuture, FutureExt};
use parser::command::relabel::{LabelDelta, RelabelCommand};
use parser::command::{Command, Input};

pub(super) struct RelabelHandler;

impl Handler for RelabelHandler {
    type Input = RelabelCommand;
    type Config = RelabelConfig;

    fn parse_input(&self, ctx: &Context, event: &Event) -> Result<Option<Self::Input>, Error> {
        let body = if let Some(b) = event.comment_body() {
            b
        } else {
            // not interested in other events
            return Ok(None);
        };

        if let Event::Issue(e) = event {
            if e.action != github::IssuesAction::Opened {
                // skip events other than opening the issue to avoid retriggering commands in the
                // issue body
                return Ok(None);
            }
        }

        let mut input = Input::new(&body, &ctx.username);
        match input.parse_command() {
            Command::Relabel(Ok(command)) => Ok(Some(command)),
            Command::Relabel(Err(err)) => {
                failure::bail!(
                    "Parsing label command in [comment]({}) failed: {}",
                    event.html_url().expect("has html url"),
                    err
                );
            }
            _ => Ok(None),
        }
    }

    fn handle_input<'a>(
        &self,
        ctx: &'a Context,
        config: &'a RelabelConfig,
        event: &'a Event,
        input: RelabelCommand,
    ) -> BoxFuture<'a, Result<(), Error>> {
        handle_input(ctx, config, event, input).boxed()
    }
}

async fn handle_input(
    ctx: &Context,
    config: &RelabelConfig,
    event: &Event,
    input: RelabelCommand,
) -> Result<(), Error> {
    let mut issue_labels = event.issue().unwrap().labels().to_owned();
    let mut changed = false;
    for delta in &input.0 {
        let name = delta.label().as_str();
        if let Err(msg) = check_filter(name, config, &event.user(), &ctx.github).await {
            let cmnt = ErrorComment::new(&event.issue().unwrap(), msg.to_string());
            cmnt.post(&ctx.github).await?;
            return Ok(());
        }
        match delta {
            LabelDelta::Add(label) => {
                if !issue_labels.iter().any(|l| l.name == label.as_str()) {
                    changed = true;
                    issue_labels.push(github::Label {
                        name: label.to_string(),
                    });
                }
            }
            LabelDelta::Remove(label) => {
                if let Some(pos) = issue_labels.iter().position(|l| l.name == label.as_str()) {
                    changed = true;
                    issue_labels.remove(pos);
                }
            }
        }
    }

    if changed {
        event
            .issue()
            .unwrap()
            .set_labels(&ctx.github, issue_labels)
            .await?;
    }

    Ok(())
}

async fn check_filter(
    label: &str,
    config: &RelabelConfig,
    user: &github::User,
    client: &GithubClient,
) -> Result<(), Error> {
    let is_team_member;
    match user.is_team_member(client).await {
        Ok(true) => return Ok(()),
        Ok(false) => {
            is_team_member = Ok(());
        }
        Err(err) => {
            eprintln!("failed to check team membership: {:?}", err);
            is_team_member = Err(());
            // continue on; if we failed to check their membership assume that they are not members.
        }
    }
    for pattern in &config.allow_unauthenticated {
        let pattern = glob::Pattern::new(pattern)?;
        if pattern.matches(label) {
            return Ok(());
        }
    }
    if is_team_member.is_ok() {
        failure::bail!("Label {} can only be set by Rust team members", label);
    } else {
        failure::bail!(
            "Label {} can only be set by Rust team members;\
             we were unable to check if you are a team member.",
            label
        );
    }
}
