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
use anyhow::Context as _;
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

    fn parse_input(
        &self,
        ctx: &Context,
        event: &Event,
        _: Option<&AssignConfig>,
    ) -> Result<Option<Self::Input>, String> {
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
    ) -> BoxFuture<'a, anyhow::Result<()>> {
        handle_input(ctx, event, cmd).boxed()
    }
}

async fn handle_input(ctx: &Context, event: &Event, cmd: AssignCommand) -> anyhow::Result<()> {
    let is_team_member = if let Err(_) | Ok(false) = event.user().is_team_member(&ctx.github).await
    {
        false
    } else {
        true
    };

    let issue = event.issue().unwrap();
    if issue.is_pr() {
        let username = match &cmd {
            AssignCommand::Own => event.user().login.clone(),
            AssignCommand::User { username } => username.clone(),
            AssignCommand::Release => {
                log::trace!(
                    "ignoring release on PR {:?}, must always have assignee",
                    issue.global_id()
                );
                return Ok(());
            }
        };
        // Don't re-assign if aleady assigned, e.g. on comment edit
        if issue.contain_assignee(&username) {
            log::trace!(
                "ignoring assign PR {} to {}, already assigned",
                issue.global_id(),
                username,
            );
            return Ok(());
        }
        if let Err(err) = issue
            .set_assignee(&ctx.github, &username)
            .await
        {
            log::warn!(
                "failed to set assignee of PR {} to {}: {:?}",
                issue.global_id(),
                username,
                err
            );
        }
        return Ok(());
    }

    let e = EditIssueBody::new(&issue, "ASSIGN");

    let to_assign = match cmd {
        AssignCommand::Own => event.user().login.clone(),
        AssignCommand::User { username } => {
            if !is_team_member && username != event.user().login {
                anyhow::bail!("Only Rust team members can assign other users");
            }
            username.clone()
        }
        AssignCommand::Release => {
            if let Some(AssignData {
                user: Some(current),
            }) = e.current_data()
            {
                if current == event.user().login || is_team_member {
                    issue
                        .remove_assignees(&ctx.github, Selection::All)
                        .await?;
                    e.apply(&ctx.github, String::new(), AssignData { user: None })
                        .await?;
                    return Ok(());
                } else {
                    anyhow::bail!("Cannot release another user's assignment");
                }
            } else {
                let current = &event.user().login;
                if issue.contain_assignee(current) {
                    issue
                        .remove_assignees(&ctx.github, Selection::One(&current))
                        .await?;
                    e.apply(&ctx.github, String::new(), AssignData { user: None })
                        .await?;
                    return Ok(());
                } else {
                    anyhow::bail!("Cannot release unassigned issue");
                }
            };
        }
    };
    // Don't re-assign if aleady assigned, e.g. on comment edit
    if issue.contain_assignee(&to_assign) {
        log::trace!(
            "ignoring assign issue {} to {}, already assigned",
            issue.global_id(),
            to_assign,
        );
        return Ok(());
    }
    let data = AssignData {
        user: Some(to_assign.clone()),
    };

    e.apply(&ctx.github, String::new(), &data).await?;

    match issue
        .set_assignee(&ctx.github, &to_assign)
        .await
    {
        Ok(()) => return Ok(()), // we are done
        Err(github::AssignmentError::InvalidAssignee) => {
            issue
                .set_assignee(&ctx.github, &ctx.username)
                .await
                .context("self-assignment failed")?;
            let cmt_body = format!(
                "This issue has been assigned to @{} via [this comment]({}).",
                to_assign,
                event.html_url().unwrap()
            );
            e.apply(&ctx.github, cmt_body, &data).await?;
        }
        Err(e) => return Err(e.into()),
    }

    Ok(())
}
