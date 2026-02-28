use crate::config::{self, Config, ConfigurationError};
use crate::gh_comments::GitHubCommentsCache;
use crate::gha_logs::GitHubActionLogsCache;
use crate::github::{Event, GithubClient, IssueCommentAction, IssuesAction, IssuesEvent};
use crate::handlers::pr_tracking::RepositoryWorkqueueMap;
use crate::team_data::TeamClient;
use crate::zulip::client::ZulipClient;
use octocrab::Octocrab;
use parser::command::{Command, Input, assign::AssignCommand};
use std::fmt;
use std::sync::Arc;
use tracing as log;

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
pub mod report_user_bans;
mod review_changes_since;
mod review_reminder;
mod review_requested;
mod review_submitted;
pub mod rustc_commits;
mod shortcut;
mod transfer;
pub mod types_planning_updates;
mod view_all_comments_link;

pub struct Context {
    pub github: GithubClient,
    pub zulip: ZulipClient,
    pub team: TeamClient,
    pub db: crate::db::ClientPool,
    pub username: String,
    pub octocrab: Octocrab,
    /// Represents the workqueue (assigned open PRs) of individual reviewers, per repository.
    pub workqueue_map: RepositoryWorkqueueMap,
    pub gha_logs: Arc<tokio::sync::RwLock<GitHubActionLogsCache>>,
    pub gh_comments: Arc<tokio::sync::RwLock<GitHubCommentsCache>>,
}

#[expect(
    clippy::collapsible_if,
    reason = "we check the preconditions in the outer if, and handle errors inside"
)]
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

    let prune_gh_comments = async {
        if let Some(issue) = event.issue() {
            let repo = issue.repository();

            let owner = &*repo.organization;
            let repo = &*repo.repository;
            let issue_id = issue.number;

            let cache_key = (owner.to_string(), repo.to_string(), issue_id);

            if ctx.gh_comments.write().await.prune(&cache_key) {
                log::info!("pruned {owner}/{repo}#{issue_id} from gh_comments cache");
            }
        }
        Ok(())
    };

    let check_commits = async {
        if let Ok(check_commits_config) = &config {
            check_commits::handle(ctx, host, event, check_commits_config)
                .await
                .map_err(|e| HandlerError::Other(e.context("check_commits handler failed")))
        } else {
            Ok(())
        }
    };

    let project_goals = async {
        project_goals::handle(ctx, event)
            .await
            .map_err(|e| HandlerError::Other(e.context("project_goals handler failed")))
    };

    let notification = async {
        notification::handle(ctx, event)
            .await
            .map_err(|e| HandlerError::Other(e.context("notification handler failed")))
    };

    let rustc_commits = async {
        rustc_commits::handle(ctx, event)
            .await
            .map_err(|e| HandlerError::Other(e.context("rustc_commits handler failed")))
    };

    let milestone_prs = async {
        milestone_prs::handle(ctx, event)
            .await
            .map_err(|e| HandlerError::Other(e.context("milestone_prs handler failed")))
    };

    let rendered_link = async {
        if let Some(rendered_link_config) =
            config.as_ref().ok().and_then(|c| c.rendered_link.as_ref())
        {
            rendered_link::handle(ctx, event, rendered_link_config)
                .await
                .map_err(|e| HandlerError::Other(e.context("rendered_link handler failed")))
        } else {
            Ok(())
        }
    };

    let view_all_comments_link = async {
        if let Some(view_all_comments_config) = config
            .as_ref()
            .ok()
            .and_then(|c| c.view_all_comments_link.as_ref())
        {
            view_all_comments_link::handle(ctx, event, host, view_all_comments_config)
                .await
                .map_err(|e| HandlerError::Other(e.context("view_all_comments handler failed")))
        } else {
            Ok(())
        }
    };

    let relnotes = async {
        relnotes::handle(ctx, event)
            .await
            .map_err(|e| HandlerError::Other(e.context("relnotes handler failed")))
    };

    let bot_pull_requests = async {
        if config.as_ref().is_ok_and(|c| c.bot_pull_requests.is_some()) {
            bot_pull_requests::handle(ctx, event)
                .await
                .map_err(|e| HandlerError::Other(e.context("bot_pull_requests handler failed")))
        } else {
            Ok(())
        }
    };

    let review_submitted = async {
        if let Some(review_submitted_config) = config
            .as_ref()
            .ok()
            .and_then(|c| c.review_submitted.as_ref())
        {
            review_submitted::handle(
                ctx,
                event,
                review_submitted_config,
                config.as_ref().ok().and_then(|c| c.shortcut.as_ref()),
            )
            .await
            .map_err(|e| HandlerError::Other(e.context("review_submitted handler failed")))
        } else {
            Ok(())
        }
    };

    let review_changes_since = async {
        if let Some(review_changes_since_config) = config
            .as_ref()
            .ok()
            .and_then(|c| c.review_changes_since.as_ref())
        {
            review_changes_since::handle(ctx, host, event, review_changes_since_config)
                .await
                .map_err(|e| HandlerError::Other(e.context("review_changes_since handler failed")))
        } else {
            Ok(())
        }
    };

    let github_releases = async {
        if let Some(github_releases_config) = config
            .as_ref()
            .ok()
            .and_then(|c| c.github_releases.as_ref())
        {
            github_releases::handle(ctx, event, github_releases_config)
                .await
                .map_err(|e| HandlerError::Other(e.context("github_releases handler failed")))
        } else {
            Ok(())
        }
    };

    let merge_conflicts = async {
        if let Some(merge_conflicts_config) = config
            .as_ref()
            .ok()
            .and_then(|c| c.merge_conflicts.as_ref())
        {
            merge_conflicts::handle(ctx, event, merge_conflicts_config)
                .await
                .map_err(|e| HandlerError::Other(e.context("merge_conflicts handler failed")))
        } else {
            Ok(())
        }
    };

    let (
        prune_gh_comments,
        check_commits,
        project_goals,
        notification,
        rustc_commits,
        milestone_prs,
        rendered_link,
        view_all_comments,
        relnotes,
        bot_pull_requests,
        review_submitted,
        review_changes_since,
        github_releases,
        merge_conflicts,
    ) = futures::join!(
        prune_gh_comments,
        check_commits,
        project_goals,
        notification,
        rustc_commits,
        milestone_prs,
        rendered_link,
        view_all_comments_link,
        relnotes,
        bot_pull_requests,
        review_submitted,
        review_changes_since,
        github_releases,
        merge_conflicts,
    );

    for result in [
        prune_gh_comments,
        check_commits,
        project_goals,
        notification,
        rustc_commits,
        milestone_prs,
        rendered_link,
        view_all_comments,
        relnotes,
        bot_pull_requests,
        review_submitted,
        review_changes_since,
        github_releases,
        merge_conflicts,
    ] {
        if let Err(e) = result {
            errors.push(e);
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
            // Process the issue handlers concurrently
            let results = futures::join!(
                $(
                    async {
                        match $name::parse_input(ctx, event, config.$name.as_ref()).await {
                            Err(err) => Err(HandlerError::Message(err)),
                            Ok(Some(input)) => {
                                if let Some(config) = &config.$name {
                                    $name::handle_input(ctx, config, event, input)
                                        .await
                                        .map_err(|mut e| {
                                            if let Some(err) = e.downcast_mut::<crate::errors::UserError>() {
                                                HandlerError::Message(err.to_string())
                                            } else {
                                                HandlerError::Other(e.context(format!(
                                                    "error when processing {} handler",
                                                    stringify!($name)
                                                )))
                                            }
                                        })
                                } else {
                                    Err(HandlerError::Message(format!(
                                        "The feature `{}` is not enabled in this repository.\n\
                                        To enable it add its section in the `triagebot.toml` \
                                        in the root of the repository.",
                                        stringify!($name)
                                    )))
                                }
                            }
                            Ok(None) => Ok(())
                        }
                    }
                ),*
            );

            // Destructure the results into named variables
            let ($($name,)*) = results;

            // Push errors for each handler
            $(
                if let Err(e) = $name {
                    errors.push(e);
                }
            )*
        }
    }
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
                // always handle new PRs / issues
                Event::Issue(IssuesEvent { action: IssuesAction::Opened, .. }) => {},
                Event::Issue(IssuesEvent { action: IssuesAction::Edited, .. }) => {
                    // if the issue was edited, but we don't get a `changes[body]` diff, it means only the title was edited, not the body.
                    // don't process the same commands twice.
                    if event.comment_from().is_none() {
                        log::debug!("skipping title-only edit event");
                        return;
                    }
                },
                Event::Issue(e) => {
                    // no change in issue's body for these events, so skip
                    log::debug!("skipping event, issue was {:?}", e.action);
                    return;
                }
                Event::IssueComment(e) => {
                    match e.action {
                        IssueCommentAction::Created => {}
                        IssueCommentAction::Edited => {
                            if event.comment_from().is_none() {
                                // We are not entirely sure why this happens.
                                // Sometimes when someone posts a PR review,
                                // GitHub sends an "edited" event with no
                                // changes just before the "created" event.
                                log::debug!("skipping issue comment edit without changes");
                                return;
                            }
                        }
                        IssueCommentAction::Deleted
                        | IssueCommentAction::Pinned
                        | IssueCommentAction::Unpinned => {
                            // don't execute commands again when comment is deleted, pinned or unpinned
                            log::debug!("skipping event, comment was {:?}", e.action);
                            return;
                        }
                    }
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

            log::info!("Comment parsed to {commands:?}");

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
                        .all(|cmd| matches!(cmd, Command::Assign(Ok(AssignCommand::RequestReview { .. }))))
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
                                .unwrap_or_else(|mut err| {
                                    if let Some(err) = err.downcast_mut::<crate::errors::UserError>() {
                                        errors.push(HandlerError::Message(err.to_string()));
                                    } else {
                                        errors.push(HandlerError::Message(format!(
                                            "`{}` handler unexpectedly failed in [this comment]({}): {err}",
                                            stringify!($name),
                                            event.html_url().expect("has html url"),
                                        )));
                                        errors.push(HandlerError::Other(err.context(format!(
                                            "error when processing {} command handler",
                                            stringify!($name)
                                        ))));
                                    }
                                });
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
                            "Parsing {} command in [comment]({}) failed: {err}",
                            stringify!($name),
                            event.html_url().expect("has html url"),
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

#[derive(Debug)]
pub enum HandlerError {
    Message(String),
    Other(anyhow::Error),
}

impl std::error::Error for HandlerError {}

impl fmt::Display for HandlerError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            HandlerError::Message(msg) => write!(f, "{msg}"),
            HandlerError::Other(_) => write!(f, "An internal error occurred."),
        }
    }
}
