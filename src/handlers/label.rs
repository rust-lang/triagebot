//! Purpose: Allow any user to modify issue labels on GitHub via comments.
//!
//! Labels are checked against the labels in the project; the bot does not support creating new
//! labels.
//!
//! Parsing is done in the `parser::command::label` module.
//!
//! If the command was successful, there will be no feedback beyond the label change to reduce
//! notification noise.

use crate::{
    github::{self, Event, GithubClient},
    handlers::Context,
    interactions::ErrorComment,
    registry::Handler,
};
use failure::Error;
use parser::command::label::{LabelCommand, LabelDelta};
use parser::command::{Command, Input};

pub struct LabelHandler;

impl Handler for LabelHandler {
    fn handle_event(&self, ctx: &Context, event: &Event) -> Result<(), Error> {
        #[allow(irrefutable_let_patterns)]
        let event = if let Event::IssueComment(e) = event {
            e
        } else {
            // not interested in other events
            return Ok(());
        };

        let repo = &event.repository.full_name;
        let mut issue_labels = event.issue.labels().to_owned();

        let mut input = Input::new(&event.comment.body, &ctx.username);
        let deltas = match input.parse_command() {
            Command::Label(Ok(LabelCommand(deltas))) => deltas,
            Command::Label(Err(err)) => {
                ErrorComment::new(
                    &event.issue,
                    format!(
                        "Parsing label command in [comment]({}) failed: {}",
                        event.comment.html_url, err
                    ),
                )
                .post(&ctx.github)?;
                failure::bail!(
                    "label parsing failed for issue #{}, error: {:?}",
                    event.issue.number,
                    err
                );
            }
            _ => return Ok(()),
        };

        let mut changed = false;
        for delta in &deltas {
            let name = delta.label().as_str();
            if let Err(msg) = check_filter(name, repo, &event.comment.user, &ctx.github) {
                ErrorComment::new(&event.issue, msg.to_string()).post(&ctx.github)?;
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
            event.issue.set_labels(&ctx.github, issue_labels)?;
        }

        Ok(())
    }
}

fn check_filter(
    label: &str,
    repo: &str,
    user: &github::User,
    client: &GithubClient,
) -> Result<(), Error> {
    let is_team_member;
    match user.is_team_member(client) {
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
    let config = crate::config::get(client, repo)?;
    for pattern in &config.label.as_ref().unwrap().allow_unauthenticated {
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
