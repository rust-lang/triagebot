//! Purpose: Allow any user to ping a pre-selected group of people on GitHub via comments.
//!
//! The set of "teams" which can be pinged is intentionally restricted via configuration.
//!
//! Parsing is done in the `parser::command::ping` module.

use crate::{
    config::PingConfig,
    github::{self, Event},
    handlers::{Context, Handler},
    interactions::ErrorComment,
};
use futures::future::{BoxFuture, FutureExt};
use parser::command::ping::PingCommand;
use parser::command::{Command, Input};

pub(super) struct PingHandler;

impl Handler for PingHandler {
    type Input = PingCommand;
    type Config = PingConfig;

    fn parse_input(&self, ctx: &Context, event: &Event) -> Result<Option<Self::Input>, String> {
        let body = if let Some(b) = event.comment_body() {
            b
        } else {
            // not interested in other events
            return Ok(None);
        };

        if let Event::Issue(e) = event {
            if e.action != github::IssuesAction::Opened {
                // skip events other than opening the issue to avoid retriggering commands in the
                // issue body
                return Ok(None);
            }
        }

        let mut input = Input::new(&body, &ctx.username);
        match input.parse_command() {
            Command::Ping(Ok(command)) => Ok(Some(command)),
            Command::Ping(Err(err)) => {
                return Err(format!(
                    "Parsing ping command in [comment]({}) failed: {}",
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
        config: &'a PingConfig,
        event: &'a Event,
        input: PingCommand,
    ) -> BoxFuture<'a, anyhow::Result<()>> {
        handle_input(ctx, config, event, input.team).boxed()
    }
}

async fn handle_input(
    ctx: &Context,
    config: &PingConfig,
    event: &Event,
    team_name: String,
) -> anyhow::Result<()> {
    let is_team_member = if let Err(_) | Ok(false) = event.user().is_team_member(&ctx.github).await
    {
        false
    } else {
        true
    };

    if !is_team_member {
        let cmnt = ErrorComment::new(
            &event.issue().unwrap(),
            format!("Only Rust team members can ping teams."),
        );
        cmnt.post(&ctx.github).await?;
        return Ok(());
    }

    let (gh_team, config) = match config.get_by_name(&team_name) {
        Some(v) => v,
        None => {
            let cmnt = ErrorComment::new(
                &event.issue().unwrap(),
                format!(
                    "This team (`{}`) cannot be pinged via this command;\
                 it may need to be added to `triagebot.toml` on the master branch.",
                    team_name,
                ),
            );
            cmnt.post(&ctx.github).await?;
            return Ok(());
        }
    };
    let team = github::get_team(&ctx.github, &gh_team).await?;
    let team = match team {
        Some(team) => team,
        None => {
            let cmnt = ErrorComment::new(
                &event.issue().unwrap(),
                format!(
                    "This team (`{}`) does not exist in the team repository.",
                    team_name,
                ),
            );
            cmnt.post(&ctx.github).await?;
            return Ok(());
        }
    };

    if let Some(label) = config.label.clone() {
        let issue_labels = event.issue().unwrap().labels();
        if !issue_labels.iter().any(|l| l.name == label) {
            let mut issue_labels = issue_labels.to_owned();
            issue_labels.push(github::Label { name: label });
            event
                .issue()
                .unwrap()
                .set_labels(&ctx.github, issue_labels)
                .await?;
        }
    }

    let mut users = Vec::new();

    if let Some(gh) = team.github {
        let repo = event.issue().expect("has issue").repository();
        // Ping all github teams associated with this team repo team that are in this organization.
        // We cannot ping across organizations, but this should not matter, as teams should be
        // sync'd to the org for which triagebot is configured.
        for gh_team in gh.teams.iter().filter(|t| t.org == repo.organization) {
            users.push(format!("@{}/{}", gh_team.org, gh_team.name));
        }
    } else {
        for member in &team.members {
            users.push(format!("@{}", member.github));
        }
    }

    let ping_msg = if users.is_empty() {
        format!("no known users to ping?")
    } else {
        format!("cc {}", users.join(" "))
    };
    let comment = format!("{}\n\n{}", config.message, ping_msg);
    event
        .issue()
        .expect("issue")
        .post_comment(&ctx.github, &comment)
        .await?;

    Ok(())
}
