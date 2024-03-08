//! Allows team members to directly create a glacier PR with the code provided.

use crate::{config::GlacierConfig, github::Event, handlers::Context};
use models::repos::Object;
use octocrab::models;
use octocrab::params::repos::Reference;
use parser::command::glacier::GlacierCommand;
use tracing as log;

pub(super) async fn handle_command(
    ctx: &Context,
    _config: &GlacierConfig,
    event: &Event,
    cmd: GlacierCommand,
) -> anyhow::Result<()> {
    let is_team_member = event
        .user()
        .is_team_member(&ctx.github)
        .await
        .unwrap_or(false);

    if !is_team_member {
        return Ok(());
    };

    let body = ctx
        .github
        .raw_gist_from_url(&cmd.source, "playground.rs")
        .await?;

    let number = event.issue().unwrap().number;
    let user = event.user();

    let octocrab = &ctx.octocrab;

    let fork = octocrab.repos(&ctx.username, "glacier");
    let base = octocrab.repos("rust-lang", "glacier");

    let master = base
        .get_ref(&Reference::Branch("master".to_string()))
        .await?
        .object;
    let master = if let Object::Commit { sha, .. } = master {
        sha
    } else {
        log::error!("invalid commit sha - master {:?}", master);
        unreachable!()
    };

    fork.create_ref(
        &Reference::Branch(format!("triagebot-ice-{}", number)),
        master,
    )
    .await?;
    fork.create_file(
        format!("ices/{}.rs", number),
        format!("Add ICE reproduction for issue rust-lang/rust#{}.", number),
        body,
    )
    .branch(format!("triagebot-ice-{}", number))
    .send()
    .await?;

    octocrab
        .pulls("rust-lang", "glacier")
        .create(
            format!("ICE - rust-lang/rust#{}", number),
            format!("{}:triagebot-ice-{}", ctx.username, number),
            "master",
        )
        .body(format!(
            "Automatically created by @{} in issue rust-lang/rust#{}",
            user.login, number
        ))
        .send()
        .await?;
    Ok(())
}
