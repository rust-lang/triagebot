//! Permit assignment of any user to issues, without requiring "write" access to the repository.
//!
//! We need to fake-assign ourselves and add a 'claimed by' section to the top-level comment.
//!
//! Such assigned issues should also be placed in a queue to ensure that the user remains
//! active; the assigned user will be asked for a status report every 2 weeks (XXX: timing).
//!
//! If we're intending to ask for a status report but no comments from the assigned user have
//! been given for the past 2 weeks, the bot will de-assign the user. They can once more claim
//! the issue if necessary.
//!
//! Assign users with `@rustbot assign @gh-user` or `@rustbot claim` (self-claim).

use crate::{
    config::AssignConfig,
    github::{self, Event},
    handlers::{Context, Handler},
    interactions::EditIssueBody,
};
use failure::{Error, ResultExt};
use parser::command::assign::AssignCommand;
use parser::command::{Command, Input};

pub(super) struct AssignmentHandler;

impl Handler for AssignmentHandler {
    type Input = AssignCommand;
    type Config = AssignConfig;

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
            Command::Assign(Ok(command)) => Ok(Some(command)),
            Command::Assign(Err(err)) => {
                failure::bail!(
                    "Parsing assign command in [comment]({}) failed: {}",
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
        _config: &AssignConfig,
        event: &Event,
        cmd: AssignCommand,
    ) -> Result<(), Error> {
        #[allow(irrefutable_let_patterns)]
        let event = if let Event::IssueComment(e) = event {
            e
        } else {
            // not interested in other events
            return Ok(());
        };

        let to_assign = match cmd {
            AssignCommand::Own => event.comment.user.login.clone(),
            AssignCommand::User { username } => username.clone(),
        };

        let e = EditIssueBody::new(&event.issue, "ASSIGN", String::new());
        e.apply(&ctx.github)?;

        match event.issue.set_assignee(&ctx.github, &to_assign) {
            Ok(()) => return Ok(()), // we are done
            Err(github::AssignmentError::InvalidAssignee) => {
                event
                    .issue
                    .set_assignee(&ctx.github, &ctx.username)
                    .context("self-assignment failed")?;
                let e = EditIssueBody::new(
                    &event.issue,
                    "ASSIGN",
                    format!(
                        "This issue has been assigned to @{} via [this comment]({}).",
                        to_assign, event.comment.html_url
                    ),
                );
                e.apply(&ctx.github)?;
            }
            Err(e) => return Err(e.into()),
        }

        Ok(())
    }
}
