use crate::config::{self, Config, ConfigurationError};
use crate::gha_logs::GitHubActionLogsCache;
use crate::github::{Event, GithubClient, IssueCommentAction, IssuesAction, IssuesEvent};
use crate::handlers::pr_tracking::ReviewerWorkqueue;
use crate::team_data::TeamClient;
use crate::zulip::client::ZulipClient;
use octocrab::Octocrab;
use parser::command::{Command, Input, assign::AssignCommand};
use std::fmt;
use std::sync::Arc;
use tracing as log;

macro_rules! inform {
    ($err:expr $(,)?) => {
        anyhow::bail!(crate::handlers::UserError($err.into()))
    };
}

#[macro_use]
mod macros;

mod assign;
mod autolabel;
mod backport;
mod bot_pull_requests;
mod check_commits;
mod close;
mod concern;
pub mod docs_update;
mod github_releases;
mod issue_links;
pub(crate) mod major_change;
mod mentions;
mod merge_conflicts;
mod milestone_prs;
mod nominate;
mod note;
mod notification;
mod notify_zulip;
mod ping;
pub mod pr_tracking;
mod prioritize;
pub mod project_goals;
pub mod pull_requests_assignment_update;
mod relabel;
mod relnotes;
mod rendered_link;
mod review_changes_since;
mod review_requested;
mod review_submitted;
pub mod rustc_commits;
mod shortcut;
mod transfer;
pub mod types_planning_updates;

pub struct Context {
    pub github: GithubClient,
    pub zulip: ZulipClient,
    pub team: TeamClient,
    pub db: crate::db::ClientPool,
    pub username: String,
    pub octocrab: Octocrab,
    /// Represents the workqueue (assigned open PRs) of individual reviewers.
    /// tokio's RwLock is used to avoid deadlocks, since we run on a single-threaded tokio runtime.
    pub workqueue: Arc<tokio::sync::RwLock<ReviewerWorkqueue>>,
    pub gha_logs: Arc<tokio::sync::RwLock<GitHubActionLogsCache>>,
}

// Handle events that happened on issues
//
// This is for events that happen only on issues or pull requests (e.g. label changes or assignments).
// Each module in the list must contain the functions `parse_input` and `handle_input`.
issue_handlers! {
    assign,
    autolabel,
    backport,
    issue_links,
    major_change,
    mentions,
    notify_zulip,
    review_requested,
    pr_tracking,
}

// Handle commands in comments/issues body
//
// This is for handlers for commands parsed by the `parser` crate.
// Each variant of `parser::command::Command` must be in this list,
// preceded by the module containing the corresponding `handle_command` function
command_handlers! {
    assign: Assign,
    nominate: Nominate,
    ping: Ping,
    prioritize: Prioritize,
    relabel: Relabel,
    major_change: Second,
    shortcut: Shortcut,
    close: Close,
    note: Note,
    concern: Concern,
    transfer: Transfer,
}

pub async fn handle(ctx: &Context, host: &str, event: &Event) -> Vec<HandlerError> {
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

    // custom handlers (prefer issue_handlers! for issue event handler)
    custom_handlers! { errors ->
        project_goals: project_goals::handle(ctx, event).await,
        notification: notification::handle(ctx, event).await,
        rustc_commits: rustc_commits::handle(ctx, event).await,
        milestone_prs: milestone_prs::handle(ctx, event).await,
        relnotes: relnotes::handle(ctx, event).await,
        check_commits: {
            if let Ok(config) = &config {
                check_commits::handle(ctx, host, event, &config).await?;
            }
            Ok(())
        },
        rendered_link: {
            if let Some(rendered_link_config) = config.as_ref().ok().and_then(|c| c.rendered_link.as_ref())
            {
                rendered_link::handle(ctx, event, rendered_link_config).await?
            }
            Ok(())
        },
        bot_pull_requests: {
            if config.as_ref().is_ok_and(|c| c.bot_pull_requests.is_some()) {
                bot_pull_requests::handle(ctx, event).await?;
            }
            Ok(())
        },
        review_submitted: {
            if let Some(config) = config.as_ref().ok().and_then(|c| c.review_submitted.as_ref()) {
                review_submitted::handle(ctx, event, config).await?;
            }
            Ok(())
        },
        review_changes_since: {
            if let Some(config) = config.as_ref().ok().and_then(|c| c.review_changes_since.as_ref()) {
                review_changes_since::handle(ctx, host, event, config).await?;
            }
            Ok(())
        },
        github_releases: {
            if let Some(ghr_config) = config.as_ref().ok().and_then(|c| c.github_releases.as_ref()) {
                github_releases::handle(ctx, event, ghr_config).await?;
            }
            Ok(())
        },
        merge_conflicts: {
            if let Some(conflict_config) = config.as_ref().ok().and_then(|c| c.merge_conflicts.as_ref()) {
                merge_conflicts::handle(ctx, event, conflict_config).await?;
            }
            Ok(())
        },
    };

    errors
}

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

#[derive(Debug)]
pub struct UserError(String);

impl std::error::Error for UserError {}

impl fmt::Display for UserError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.0)
    }
}
