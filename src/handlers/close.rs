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
    github::{self, Event, Selection},
    handlers::{Context, Handler},
    interactions::EditIssueBody,
};
use failure::{Error, ResultExt};
use futures::future::{BoxFuture, FutureExt};
use parser::command::assign::AssignCommand;
use parser::command::{Command, Input};

pub(super) struct AssignmentHandler;

#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct AssignData {
    user: Option<String>,
}

impl Handler for AssignmentHandler {
    type Input = AssignCommand;
    type Config = AssignConfig;

    fn parse_input(&self, ctx: &Context, event: &Event) -> Result<Option<Self::Input>, String> {
        let body = if let Some(b) = event.comment_body() {
            b
        } else {
            // not interested in other events
            return Ok(None);
        };

        if let Event::Issue(e) = event {
            if e.action != github::IssuesAction::Opened {
                log::debug!("skipping event, issue was {:?}", e.action);
                // skip events other than opening the issue to avoid retriggering commands in the
                // issue body
                return Ok(None);
            }
        }

        let mut input = Input::new(&body, &ctx.username);
        match input.parse_command() {
            Command::Assign(Ok(command)) => Ok(Some(command)),
            Command::Assign(Err(err)) => {
                return Err(format!(
                    "Parsing assign command in [comment]({}) failed: {}",
                    event.html_url().expect("has html url"),
                    err
                ));
            }
            _ => Ok(None),
        }
    }

    fn handle_input<'a>(
        &self,
        ctx: &'a Context,
        _config: &'a AssignConfig,
        event: &'a Event,
        cmd: AssignCommand,
    ) -> BoxFuture<'a, Result<(), Error>> {
        handle_input(ctx, event, cmd).boxed()
    }
}

async fn handle_input(ctx: &Context, event: &Event, cmd: AssignCommand) -> Result<(), Error> {
    let is_team_member = if let Err(_) | Ok(false) = event.user().is_team_member(&ctx.github).await
    {
        false
    } else {
        true
    };

    if event.issue().unwrap().is_pr() {
        let username = match &cmd {
            AssignCommand::Own => event.user().login.clone(),
            AssignCommand::User { username } => username.clone(),
            AssignCommand::Release => {
                log::trace!(
                    "ignoring release on PR {:?}, must always have assignee",
                    event.issue().unwrap().global_id()
                );
                return Ok(());
            }
        };
        if let Err(err) = event
            .issue()
            .unwrap()
            .set_assignee(&ctx.github, &username)
            .await
        {
            log::warn!(
                "failed to set assignee of PR {} to {}: {:?}",
                event.issue().unwrap().global_id(),
                username,
                err
            );
        }
        return Ok(());
    }

    let e = EditIssueBody::new(&event.issue().unwrap(), "ASSIGN");

    let to_assign = match cmd {
        AssignCommand::Own => event.user().login.clone(),
        AssignCommand::User { username } => {
            if !is_team_member && username != event.user().login {
                failure::bail!("Only Rust team members can assign other users");
            }
            username.clone()
        }
        AssignCommand::Release => {
            if let Some(AssignData {
                user: Some(current),
            }) = e.current_data()
            {
                if current == event.user().login || is_team_member {
                    event
                        .issue()
                        .unwrap()
                        .remove_assignees(&ctx.github, Selection::All)
                        .await?;
                    e.apply(&ctx.github, String::new(), AssignData { user: None })
                        .await?;
                    return Ok(());
                } else {
                    failure::bail!("Cannot release another user's assignment");
                }
            } else {
                let current = &event.user();
                if event.issue().unwrap().contain_assignee(current) {
                    event
                        .issue()
                        .unwrap()
                        .remove_assignees(&ctx.github, Selection::One(&current))
                        .await?;
                    e.apply(&ctx.github, String::new(), AssignData { user: None })
                        .await?;
                    return Ok(());
                } else {
                    failure::bail!("Cannot release unassigned issue");
                }
            };
        }
    };
    let data = AssignData {
        user: Some(to_assign.clone()),
    };

    e.apply(&ctx.github, String::new(), &data).await?;

    match event
        .issue()
        .unwrap()
        .set_assignee(&ctx.github, &to_assign)
        .await
    {
        Ok(()) => return Ok(()), // we are done
        Err(github::AssignmentError::InvalidAssignee) => {
            event
                .issue()
                .unwrap()
                .set_assignee(&ctx.github, &ctx.username)
                .await
                .context("self-assignment failed")?;
            e.apply(
                &ctx.github,
                format!(
                    "This issue has been assigned to @{} via [this comment]({}).",
                    to_assign,
                    event.html_url().unwrap()
                ),
                &data,
            )
            .await?;
        }
        Err(e) => return Err(e.into()),
    }

    Ok(())
}
