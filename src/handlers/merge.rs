use anyhow::Context as _;
use octocrab::models::pulls::MergeableState;
use parser::command::merge::MergeCommand;

use crate::{
    config::MergeConfig,
    db::issue_data::IssueData,
    errors::user_error,
    github::{Event, IssueCommentEvent, UserId as GitHubUserId, client::GraphQlErrors},
    handlers::Context,
};

/// Key for the state in the database
const DELEGATIONS_KEY: &str = "merge-delegations";

/// State stored in the database
#[derive(Debug, Default, serde::Deserialize, serde::Serialize, Clone, PartialEq)]
struct DelegationsState {
    // List of all the user IDs who have been delegated the power to merge the PR.
    delegations: Vec<GitHubUserId>,
}

pub(super) async fn handle_command(
    ctx: &Context,
    config: &MergeConfig,
    event: &Event,
    cmd: MergeCommand,
) -> anyhow::Result<()> {
    let Event::IssueComment(issue_comment) = event else {
        return user_error!(
            "`merge` and `delegate` commands can only be issued from a pull-request comment/review"
        );
    };
    if issue_comment.issue.pull_request.is_none() {
        return user_error!(
            "`merge` and `delegate` commands can only be issued on an pull-request"
        );
    };

    let has_write_permissions = {
        let perm = issue_comment
            .issue
            .repository()
            .collaborator_permission(&ctx.github, &issue_comment.comment.user.login)
            .await
            .context("failed to get the user repository permission")?;

        perm.permission.has_write_permissions()
    };

    match cmd {
        MergeCommand::Merge => {
            if !has_write_permissions {
                let has_delegation_rights = was_delegated_to_commenter(ctx, &issue_comment)
                    .await
                    .context(
                    "Couldn't determine if the user has merge rights via delegation",
                )?;

                if !has_delegation_rights {
                    return user_error!(
                        "Unauthorized, only user with `write` permissions in this repository can merge PRs."
                    );
                }
            }

            merge_pr(ctx, config, issue_comment).await?;
        }
        MergeCommand::Delegate { login } => {
            if !has_write_permissions {
                return user_error!(
                    "Unauthorized, only user with `write` permissions in this repository can delegate PRs."
                );
            };

            let login = login.trim_start_matches('@');

            if login.is_empty() {
                return user_error!("Cannot delegate to an empty login.");
            }

            delegate_to(ctx, issue_comment, login).await?;
        }
        MergeCommand::DelegateToAuthor => {
            if !has_write_permissions {
                return user_error!(
                    "Unauthorized, only user with `write` permissions in this repository can delegate PRs."
                );
            };

            delegate_to(ctx, issue_comment, &issue_comment.issue.user.login).await?;
        }
    }

    Ok(())
}

async fn merge_pr(
    ctx: &Context,
    config: &MergeConfig,
    issue_comment: &IssueCommentEvent,
) -> anyhow::Result<()> {
    match config.type_ {
        crate::config::MergeType::MergeQueue => {
            merge_pr_with_merge_queue(ctx, issue_comment).await?;
        }
    }

    Ok(())
}

async fn merge_pr_with_merge_queue(
    ctx: &Context,
    issue_comment: &IssueCommentEvent,
) -> anyhow::Result<()> {
    let pr = ctx
        .github
        .pull_request(issue_comment.issue.repository(), issue_comment.issue.number)
        .await
        .context("failed to fetch the PR to merge")?;

    tracing::info!(
        "pr mergeable status={:?} ({:?})",
        &pr.mergeable,
        &pr.mergeable_state
    );

    // Unless the mergeable state is "clean" we enable auto merge, if it's "clean"
    // we can't use auto merge (GitHub blocks it), so let's enqueue the PR in the
    // merge queue, since we know by the "clean" state that we can.
    if pr.mergeable_state != Some(MergeableState::Clean) {
        if let Err(err) = issue_comment.issue.enable_auto_merge(&ctx.github).await {
            if let Some(graphql_errors) = err.downcast_ref::<GraphQlErrors>()
                && let [error] = &*graphql_errors.errors
                && error.path == &["enablePullRequestAutoMerge"]
                && error.type_ == "UNPROCESSABLE"
            {
                return user_error!(error.message.to_string());
            }

            anyhow::bail!(err);
        }
    } else {
        if let Err(err) = issue_comment
            .issue
            .enqueue_to_merge_queue(&ctx.github)
            .await
        {
            if let Some(graphql_errors) = err.downcast_ref::<GraphQlErrors>()
                && let [error] = &*graphql_errors.errors
                && error.path == &["enqueuePullRequest"]
                && error.type_ == "UNPROCESSABLE"
            {
                return user_error!(error.message.to_string());
            }

            anyhow::bail!(err);
        }
    }

    Ok(())
}

async fn delegate_to(
    ctx: &Context,
    issue_comment: &IssueCommentEvent,
    delegate_to: &str,
) -> anyhow::Result<()> {
    let delegatee = ctx.github.user_info(delegate_to).await?;

    let mut db = ctx.db.get().await;
    let mut state: IssueData<'_, DelegationsState> =
        IssueData::load(&mut db, &issue_comment.issue, DELEGATIONS_KEY).await?;

    if state.data.delegations.contains(&delegatee.id) {
        return user_error!(format!(
            "Merge rights have already been delegated to @{}.",
            &delegatee.login
        ));
    }

    state.data.delegations.push(delegatee.id);

    state
        .save()
        .await
        .context("Couldn't save the new delegations")?;

    issue_comment
        .issue
        .post_comment(&ctx.github, &format!(r#":v: @{delegatee}, you can now merge this pull request!

If @{delegator} told you to merge after making some further change, then please make that change and post `@{bot_prefix} merge`.

[View changes since this delegation](https://triagebot.infra.rust-lang.org/gh-changes-since/{org_repo}/{pr_num}/{base_sha}..{head_sha}).
"#,
delegatee = &delegatee.login,
bot_prefix = ctx.username,
delegator = &issue_comment.comment.user.login,
org_repo = &issue_comment.repository.full_name,
pr_num = issue_comment.issue.number,
base_sha = &issue_comment.issue.base.as_ref().context("no base sha")?.sha,
head_sha = &issue_comment.issue.head.as_ref().context("no head sha")?.sha,
))
        .await
        .context("Couldn't post the delegation confirmation comment")?;

    Ok(())
}

async fn was_delegated_to_commenter(
    ctx: &Context,
    issue_comment: &IssueCommentEvent,
) -> anyhow::Result<bool> {
    let mut db = ctx.db.get().await;
    let state: IssueData<'_, DelegationsState> =
        IssueData::load(&mut db, &issue_comment.issue, DELEGATIONS_KEY)
            .await
            .context("failed to load the delegations state")?;

    for delegated_user_id in &state.data.delegations {
        if delegated_user_id == &issue_comment.comment.user.id {
            return Ok(true);
        }
    }

    Ok(false)
}
