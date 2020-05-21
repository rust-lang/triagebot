//! Allows team members to directly create a glacier PR with the code provided.

use crate::{
    config::GlacierConfig,
    github::Event,
    handlers::{Context, Handler},
};

use futures::future::{BoxFuture, FutureExt};
use parser::command::glacier::GlacierCommand;
use parser::command::{Command, Input};
use octocrab::params::repos::Reference;
use octocrab::models::Object;

pub(super) struct GlacierHandler;

impl Handler for GlacierHandler {
    type Input = GlacierCommand;
    type Config = GlacierConfig;

    fn parse_input(
        &self,
        ctx: &Context,
        event: &Event,
        _: Option<&GlacierConfig>,
    ) -> Result<Option<Self::Input>, String> {
        let body = if let Some(b) = event.comment_body() {
            b
        } else {
            // not interested in other events
            return Ok(None);
        };

        match event {
            Event::IssueComment(_) => (),
            _ => {return Ok(None);}
        };

        let mut input = Input::new(&body, &ctx.username);
        match input.parse_command() {
            Command::Glacier(Ok(command)) => Ok(Some(command)),
            Command::Glacier(Err(err)) => {
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
        _config: &'a GlacierConfig,
        event: &'a Event,
        cmd: GlacierCommand,
    ) -> BoxFuture<'a, anyhow::Result<()>> {
        handle_input(ctx, event, cmd).boxed()
    }
}

async fn handle_input(ctx: &Context, event: &Event, cmd: GlacierCommand) -> anyhow::Result<()> {
    let is_team_member = if let Err(_) | Ok(false) = event.user().is_team_member(&ctx.github).await
    {
        false
    } else {
        true
    };

    if !is_team_member {
        return Ok(())
    };

    let url = cmd.source;
    let response = reqwest::get(&format!("{}{}", url.replace("github", "githubusercontent"), "/playground.rs")).await?;
    let body = response.text().await?;

    let number = event.issue().unwrap().number;
    let user = event.user();

    let octocrab = octocrab::instance();

    let fork = octocrab.repos("rust-lang", "glacier");
    let base = octocrab.repos("rust-lang", "glacier");

    let master = base.get_ref(&Reference::Branch("master".to_string())).await?.object;
    let master = if let Object::Commit { sha, ..} = master {
        sha
    } else {
        panic!()
    };

    fork.create_ref(&Reference::Branch(format!("triagebot-ice-{}", number)), master).await?;
    fork.create_file(format!("ices/{}.rs", number), format!("This PR was created by {} on issue {}.", user.login, number), body)
        .branch("triagebot-ice")
        .send()
        .await?;

    octocrab.pulls("rust-lang", "glacier")
        .create(format!("ICE - {}", number), format!("triagebot-ice-{}", number), "master")
        .body("This is a fake new catastrophic avalanche of ICE!")
        .send()
        .await?;
    Ok(())
}
