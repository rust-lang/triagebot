//! Allows to close an issue or a PR

use crate::{config::CloseConfig, github::Event, handlers::Context};
use parser::command::close::CloseCommand;

pub(super) async fn handle_command(
    ctx: &Context,
    _config: &CloseConfig,
    event: &Event,
    _cmd: CloseCommand,
) -> anyhow::Result<()> {
    let is_team_member = event
        .user()
        .is_team_member(&ctx.github)
        .await
        .unwrap_or(false);
    let issue = event.issue().unwrap();

    unimplemented!();
}
