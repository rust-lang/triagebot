//! Triage command
//!
//! Removes I-nominated tag and adds priority label.

use crate::{
    config::TriageConfig,
    github::{self, Event},
    handlers::{Context, Handler},
};
use failure::Error;
use parser::command::triage::{Priority, TriageCommand};
use parser::command::{Command, Input};

pub(super) struct TriageHandler;

impl Handler for TriageHandler {
    type Input = TriageCommand;
    type Config = TriageConfig;

    fn parse_input(&self, ctx: &Context, event: &Event) -> Result<Option<Self::Input>, Error> {
        #[allow(irrefutable_let_patterns)]
        let event = if let Event::IssueComment(e) = event {
            e
        } else {
            // not interested in other events
            return Ok(None);
        };

        let mut input = Input::new(&event.comment.body, &ctx.username);
        match input.parse_command() {
            Command::Triage(Ok(command)) => Ok(Some(command)),
            Command::Triage(Err(err)) => {
                failure::bail!(
                    "Parsing triage command in [comment]({}) failed: {}",
                    event.comment.html_url,
                    err
                );
            }
            _ => Ok(None),
        }
    }

    fn handle_input(
        &self,
        ctx: &Context,
        config: &TriageConfig,
        event: &Event,
        cmd: TriageCommand,
    ) -> Result<(), Error> {
        #[allow(irrefutable_let_patterns)]
        let event = if let Event::IssueComment(e) = event {
            e
        } else {
            // not interested in other events
            return Ok(());
        };

        let is_team_member =
            if let Err(_) | Ok(false) = event.comment.user.is_team_member(&ctx.github) {
                false
            } else {
                true
            };
        if !is_team_member {
            failure::bail!("Cannot use triage command as non-team-member");
        }

        let mut labels = event.issue.labels().to_owned();
        let add = match cmd.priority {
            Priority::High => &config.high,
            Priority::Medium => &config.medium,
            Priority::Low => &config.low,
        };
        labels.push(github::Label { name: add.clone() });
        labels.retain(|label| !config.remove.contains(&label.name));
        if &labels[..] != event.issue.labels() {
            event.issue.set_labels(&ctx.github, labels)?;
        }

        Ok(())
    }
}
