//! This module updates the PR workqueue of the Rust project contributors
//! Runs after a PR has been assigned or unassigned
//!
//! Purpose:
//!
//! - Adds the PR to the workqueue of one team member (after the PR has been assigned or reopened)
//! - Removes the PR from the workqueue of one team member (after the PR has been unassigned or closed)

use crate::github::PullRequestNumber;
use crate::github::{User, UserId};
use crate::{
    config::ReviewPrefsConfig,
    github::{IssuesAction, IssuesEvent},
    handlers::Context,
};
use std::collections::{HashMap, HashSet};
use tracing as log;

/// Maps users to a set of currently assigned open non-draft pull requests.
/// We store this map in memory, rather than in the DB, because it can get desynced when webhooks
/// are missed.
/// It is thus reloaded when triagebot starts and also periodically, so it is not needed to store it
/// in the DB.
#[derive(Debug, Default)]
pub struct ReviewerWorkqueue {
    reviewers: HashMap<UserId, HashSet<PullRequestNumber>>,
}

impl ReviewerWorkqueue {
    pub fn new(reviewers: HashMap<UserId, HashSet<PullRequestNumber>>) -> Self {
        Self { reviewers }
    }
}

pub(super) enum ReviewPrefsInput {
    Assigned { assignee: User },
    Unassigned { assignee: User },
    Reopened,
    Closed,
}

pub(super) async fn parse_input(
    _ctx: &Context,
    event: &IssuesEvent,
    config: Option<&ReviewPrefsConfig>,
) -> Result<Option<ReviewPrefsInput>, String> {
    // NOTE: this config check MUST exist. Else, the triagebot will emit an error
    // about this feature not being enabled
    if config.is_none() {
        return Ok(None);
    };

    // Execute this handler only if this is a PR ...
    if !event.issue.is_pr() {
        return Ok(None);
    }

    // ... and if the action is an assignment or unassignment with an assignee
    match &event.action {
        IssuesAction::Assigned { assignee } => Ok(Some(ReviewPrefsInput::Assigned {
            assignee: assignee.clone(),
        })),
        IssuesAction::Unassigned { assignee } => Ok(Some(ReviewPrefsInput::Unassigned {
            assignee: assignee.clone(),
        })),
        // We don't need to handle Opened explicitly, because that will trigger the Assigned event
        IssuesAction::Reopened => Ok(Some(ReviewPrefsInput::Reopened)),
        IssuesAction::Closed | IssuesAction::Deleted | IssuesAction::Transferred => {
            Ok(Some(ReviewPrefsInput::Closed))
        }
        _ => Ok(None),
    }
}

pub(super) async fn handle_input<'a>(
    ctx: &Context,
    _config: &ReviewPrefsConfig,
    event: &IssuesEvent,
    input: ReviewPrefsInput,
) -> anyhow::Result<()> {
    log::info!("Handling event action {:?} in PR tracking", event.action);

    match input {
        // This handler is reached also when assigning a PR using the Github UI
        // (i.e. from the "Assignees" dropdown menu).
        // We need to also check assignee availability here.
        ReviewPrefsInput::Assigned { assignee } => {
            let pr_number = event.issue.number;
            log::info!(
                "Adding PR {pr_number} from workqueue of {} because they were assigned.",
                assignee.login
            );

            upsert_pr_into_workqueue(ctx, assignee.id, pr_number).await;
        }
        ReviewPrefsInput::Unassigned { assignee } => {
            let pr_number = event.issue.number;
            log::info!(
                "Removing PR {pr_number} from workqueue of {} because they were unassigned.",
                assignee.login
            );
            delete_pr_from_workqueue(ctx, assignee.id, pr_number).await;
        }
        ReviewPrefsInput::Closed => {
            for assignee in &event.issue.assignees {
                let pr_number = event.issue.number;
                log::info!(
                    "Removing PR {pr_number} from workqueue of {} because it was closed or merged.",
                    assignee.login
                );
                delete_pr_from_workqueue(ctx, assignee.id, pr_number).await;
            }
        }
        ReviewPrefsInput::Reopened => {
            for assignee in &event.issue.assignees {
                let pr_number = event.issue.number;
                log::info!(
                    "Re-adding PR {pr_number} to workqueue of {} because it was (re)opened.",
                    assignee.login
                );
                upsert_pr_into_workqueue(ctx, assignee.id, pr_number).await;
            }
        }
    }

    Ok(())
}

/// Get pull request assignments for a team member
pub async fn get_assigned_prs(ctx: &Context, user_id: UserId) -> HashSet<PullRequestNumber> {
    ctx.workqueue
        .read()
        .await
        .reviewers
        .get(&user_id)
        .cloned()
        .unwrap_or_default()
}

/// Add a PR to the workqueue of a team member.
/// Ensures no accidental PR duplicates.
async fn upsert_pr_into_workqueue(ctx: &Context, user_id: UserId, pr: PullRequestNumber) {
    ctx.workqueue
        .write()
        .await
        .reviewers
        .entry(user_id)
        .or_default()
        .insert(pr);
}

/// Delete a PR from the workqueue of a team member
async fn delete_pr_from_workqueue(ctx: &Context, user_id: UserId, pr: PullRequestNumber) {
    let mut queue = ctx.workqueue.write().await;
    if let Some(reviewer) = queue.reviewers.get_mut(&user_id) {
        reviewer.remove(&pr);
    }
}

#[cfg(test)]
mod tests {
    use crate::config::Config;
    use crate::github::PullRequestNumber;
    use crate::github::{Issue, IssuesAction, IssuesEvent, Repository, User};
    use crate::handlers::pr_tracking::{handle_input, parse_input, upsert_pr_into_workqueue};
    use crate::tests::github::{default_test_user, issue, pull_request, user};
    use crate::tests::{run_test, TestContext};

    #[tokio::test]
    async fn add_pr_to_workqueue_on_assign() {
        run_test(|ctx| async move {
            let user = user("Martin", 2);

            run_handler(
                &ctx,
                IssuesAction::Assigned {
                    assignee: user.clone(),
                },
                pull_request().number(10).call(),
            )
            .await;

            check_assigned_prs(&ctx, &user, &[10]).await;

            Ok(ctx)
        })
        .await;
    }

    #[tokio::test]
    async fn remove_pr_from_workqueue_on_unassign() {
        run_test(|ctx| async move {
            let user = user("Martin", 2);
            set_assigned_prs(&ctx, &user, &[10]).await;

            run_handler(
                &ctx,
                IssuesAction::Unassigned {
                    assignee: user.clone(),
                },
                pull_request().number(10).call(),
            )
            .await;

            check_assigned_prs(&ctx, &user, &[]).await;

            Ok(ctx)
        })
        .await;
    }

    #[tokio::test]
    async fn remove_pr_from_workqueue_on_pr_closed() {
        run_test(|ctx| async move {
            let user = user("Martin", 2);
            set_assigned_prs(&ctx, &user, &[10]).await;

            run_handler(
                &ctx,
                IssuesAction::Closed,
                pull_request()
                    .number(10)
                    .assignees(vec![user.clone()])
                    .call(),
            )
            .await;

            check_assigned_prs(&ctx, &user, &[]).await;

            Ok(ctx)
        })
        .await;
    }

    #[tokio::test]
    async fn add_pr_to_workqueue_on_pr_reopen() {
        run_test(|ctx| async move {
            let user = user("Martin", 2);
            set_assigned_prs(&ctx, &user, &[42]).await;

            run_handler(
                &ctx,
                IssuesAction::Reopened,
                pull_request()
                    .number(10)
                    .assignees(vec![user.clone()])
                    .call(),
            )
            .await;

            check_assigned_prs(&ctx, &user, &[10, 42]).await;

            Ok(ctx)
        })
        .await;
    }

    // Make sure that we only consider pull requests, not issues.
    #[tokio::test]
    async fn ignore_issue_assignments() {
        run_test(|ctx| async move {
            let user = user("Martin", 2);

            run_handler(
                &ctx,
                IssuesAction::Assigned {
                    assignee: user.clone(),
                },
                issue().number(10).call(),
            )
            .await;

            check_assigned_prs(&ctx, &user, &[]).await;

            Ok(ctx)
        })
        .await;
    }

    async fn check_assigned_prs(
        ctx: &TestContext,
        user: &User,
        expected_prs: &[PullRequestNumber],
    ) {
        let mut assigned = ctx
            .handler_ctx()
            .workqueue
            .read()
            .await
            .reviewers
            .get(&user.id)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect::<Vec<_>>();
        assigned.sort();
        assert_eq!(assigned, expected_prs);
    }

    async fn set_assigned_prs(ctx: &TestContext, user: &User, prs: &[PullRequestNumber]) {
        for &pr in prs {
            upsert_pr_into_workqueue(ctx.handler_ctx(), user.id, pr).await;
        }
        check_assigned_prs(&ctx, user, prs).await;
    }

    async fn run_handler(ctx: &TestContext, action: IssuesAction, issue: Issue) {
        let handler_ctx = ctx.handler_ctx();
        let config = create_config().pr_tracking;

        let event = IssuesEvent {
            action,
            issue,
            changes: None,
            repository: Repository {
                full_name: "rust-lang-test/triagebot-test".to_string(),
                default_branch: "main".to_string(),
                fork: false,
                parent: None,
            },
            sender: default_test_user(),
        };

        let input = parse_input(&handler_ctx, &event, config.as_ref())
            .await
            .unwrap();
        if let Some(input) = input {
            handle_input(&handler_ctx, &config.unwrap(), &event, input)
                .await
                .unwrap()
        }
    }

    fn create_config() -> Config {
        toml::from_str::<Config>(
            r#"
[pr-tracking]
"#,
        )
        .unwrap()
    }
}
