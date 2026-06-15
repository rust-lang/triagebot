use anyhow::Context as _;
use parser::command::merge::MergeCommand;

use crate::{
    config::MergeConfig,
    errors::user_error,
    github::{Event, client::GraphQlErrors},
    handlers::Context,
};

pub(super) async fn handle_command(
    ctx: &Context,
    config: &MergeConfig,
    event: &Event,
    _cmd: MergeCommand,
) -> anyhow::Result<()> {
    let Event::IssueComment(issue_comment) = event else {
        return user_error!(
            "`merge` command can only be issued from a pull-request comment/review"
        );
    };
    if issue_comment.issue.pull_request.is_none() {
        return user_error!("`merge` command can only be issued on an pull-request");
    };

    let has_perm = {
        let perm = issue_comment
            .issue
            .repository()
            .collaborator_permission(&ctx.github, &issue_comment.comment.user.login)
            .await
            .context("failed to get the user repository permission")?;

        perm.permission.has_write_permissions()
    };

    if !has_perm {
        return user_error!(
            "Unauthorized, only user with `write` permissions on this repository can merge PRs."
        );
    };

    match config.type_ {
        crate::config::MergeType::MergeQueue => {
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
    }

    Ok(())
}
