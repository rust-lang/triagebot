//! Allows to lock/unlock an issue or a PR

use crate::{config::LockConfig, errors::user_error, github::Event, handlers::Context};
use parser::command::lock::LockCommand;

pub(super) async fn handle_command(
    ctx: &Context,
    _config: &LockConfig,
    event: &Event,
    cmd: LockCommand,
) -> anyhow::Result<()> {
    let Some(issue) = event.issue() else {
        return user_error!("Can only lock/unlock issues and pull-request.");
    };

    let is_team_member = ctx
        .team
        .is_team_member(&event.user().login)
        .await
        .unwrap_or(false);

    if !is_team_member {
        return user_error!("Only team members can lock/unlock issues and pull-request.");
    }

    match cmd {
        LockCommand::Lock => {
            issue.lock(&ctx.github, None).await?;
        }
        LockCommand::Unlock => {
            issue.unlock(&ctx.github).await?;
        }
    }

    Ok(())
}
