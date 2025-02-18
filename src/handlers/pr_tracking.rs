//! This module updates the PR workqueue of the Rust project contributors
//! Runs after a PR has been assigned or unassigned
//!
//! Purpose:
//!
//! - Adds the PR to the workqueue of one team member (after the PR has been assigned)
//! - Removes the PR from the workqueue of one team member (after the PR has been unassigned or closed)

use super::assign::{FindReviewerError, REVIEWER_HAS_NO_CAPACITY, SELF_ASSIGN_HAS_NO_CAPACITY};
use crate::github::User;
use crate::{
    config::ReviewPrefsConfig,
    db::notifications::record_username,
    github::{IssuesAction, IssuesEvent},
    handlers::Context,
    ReviewPrefs,
};
use anyhow::Context as _;
use tokio_postgres::Client as DbClient;
use tracing as log;

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
    let db_client = ctx.db.get().await;

    log::info!("Handling event action {:?} in PR tracking", event.action);

    match input {
        // This handler is reached also when assigning a PR using the Github UI
        // (i.e. from the "Assignees" dropdown menu).
        // We need to also check assignee availability here.
        ReviewPrefsInput::Assigned { assignee } => {
            let work_queue = has_user_capacity(&db_client, &assignee.login)
                .await
                .context("Failed to retrieve user work queue");

            // if user has no capacity, revert the PR assignment (GitHub has already assigned it)
            // and post a comment suggesting what to do
            if let Err(_) = work_queue {
                log::warn!(
                    "[#{}] DB reported that user {} has no review capacity. Ignoring.",
                    event.issue.number,
                    &assignee.login
                );

                // NOTE: disabled for now, just log
                // event
                //     .issue
                //     .remove_assignees(&ctx.github, crate::github::Selection::One(&assignee.login))
                //     .await?;
                // let msg = if assignee.login.to_lowercase() == event.issue.user.login.to_lowercase() {
                //     SELF_ASSIGN_HAS_NO_CAPACITY.replace("{username}", &assignee.login)
                // } else {
                //     REVIEWER_HAS_NO_CAPACITY.replace("{username}", &assignee.login)
                // };
                // event.issue.post_comment(&ctx.github, &msg).await?;
            }

            log::info!(
                "Adding PR {} from workqueue of {} because they were assigned.",
                event.issue.number,
                assignee.login
            );

            upsert_pr_into_workqueue(&db_client, &assignee, event.issue.number)
                .await
                .context("Failed to add PR to work queue")?;
        }
        ReviewPrefsInput::Unassigned { assignee } => {
            let pr_number = event.issue.number;
            log::info!(
                "Removing PR {pr_number} from workqueue of {} because they were unassigned.",
                assignee.login
            );
            delete_pr_from_workqueue(&db_client, assignee.id, pr_number)
                .await
                .context("Failed to remove PR from work queue")?;
        }
        ReviewPrefsInput::Closed => {
            for assignee in &event.issue.assignees {
                let pr_number = event.issue.number;
                log::info!(
                    "Removing PR {pr_number} from workqueue of {} because it was closed or merged.",
                    assignee.login
                );
                delete_pr_from_workqueue(&db_client, assignee.id, pr_number)
                    .await
                    .context("Failed to to remove PR from work queue")?;
            }
        }
        ReviewPrefsInput::Reopened => {
            for assignee in &event.issue.assignees {
                let pr_number = event.issue.number;
                log::info!(
                    "Re-adding PR {pr_number} to workqueue of {} because it was (re)opened.",
                    assignee.login
                );
                upsert_pr_into_workqueue(&db_client, &assignee, pr_number)
                    .await
                    .context("Failed to add PR to work queue")?;
            }
        }
    }

    Ok(())
}

// Check user review capacity.
// Returns error if SQL query fails or user has no capacity
pub async fn has_user_capacity(
    db: &crate::db::PooledClient,
    assignee: &str,
) -> anyhow::Result<ReviewPrefs, FindReviewerError> {
    let q = "
SELECT username, r.*
FROM review_prefs r
JOIN users ON users.user_id = r.user_id
WHERE username = $1
AND CARDINALITY(r.assigned_prs) < LEAST(COALESCE(r.max_assigned_prs,1000000));";
    let rec = db.query_one(q, &[&assignee]).await;
    if let Err(_) = rec {
        return Err(FindReviewerError::ReviewerHasNoCapacity {
            username: assignee.to_string(),
        });
    }
    Ok(rec.unwrap().into())
}

/// Add a PR to the workqueue of a team member.
/// Ensures no accidental PR duplicates.
async fn upsert_pr_into_workqueue(
    db: &DbClient,
    user: &User,
    pr: u64,
) -> anyhow::Result<u64, anyhow::Error> {
    // Ensure the user has entry in the `users` table
    record_username(db, user.id, &user.login)
        .await
        .context("failed to record username")?;

    let q = "
INSERT INTO review_prefs
(user_id, assigned_prs) VALUES ($1, $2)
ON CONFLICT (user_id)
DO UPDATE SET assigned_prs = uniq(sort(array_append(review_prefs.assigned_prs, $3)));";
    db.execute(q, &[&(user.id as i64), &vec![pr as i32], &(pr as i32)])
        .await
        .context("Upsert DB error")
}

/// Delete a PR from the workqueue of a team member
async fn delete_pr_from_workqueue(
    db: &DbClient,
    user_id: u64,
    pr: u64,
) -> anyhow::Result<u64, anyhow::Error> {
    let q = "
UPDATE review_prefs r
SET assigned_prs = array_remove(r.assigned_prs, $2)
WHERE r.user_id = $1;";
    db.execute(q, &[&(user_id as i64), &(pr as i32)])
        .await
        .context("Update DB error")
}

#[cfg(test)]
mod tests {
    use crate::config::Config;
    use crate::github::{Issue, IssuesAction, IssuesEvent, Repository, User};
    use crate::handlers::pr_tracking::{handle_input, parse_input, upsert_pr_into_workqueue};
    use crate::tests::github::{default_test_user, issue, pull_request, user};
    use crate::tests::{run_test, TestContext};
    use tokio_postgres::GenericClient;

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

    async fn check_assigned_prs(ctx: &TestContext, user: &User, expected_prs: &[i32]) {
        let results = ctx
            .db_client()
            .await
            .query(
                "SELECT assigned_prs FROM review_prefs WHERE user_id = $1",
                &[&(user.id as i64)],
            )
            .await
            .unwrap();
        assert!(results.len() < 2);
        let mut assigned = results
            .get(0)
            .map(|row| row.get::<_, Vec<i32>>(0))
            .unwrap_or_default();
        assigned.sort();
        assert_eq!(assigned, expected_prs);
    }

    async fn set_assigned_prs(ctx: &TestContext, user: &User, prs: &[i32]) {
        for &pr in prs {
            upsert_pr_into_workqueue(&ctx.db_client().await.client(), user, pr as u64)
                .await
                .unwrap();
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
