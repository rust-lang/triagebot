use crate::{
    config::PrioritizeConfig,
    github::{self, Event},
    handlers::{Context, Handler},
    interactions::ErrorComment,
};
use futures::future::{BoxFuture, FutureExt};
use parser::command::prioritize::PrioritizeCommand;
use parser::command::{Command, Input};


pub(super) struct PrioritizeHandler;

impl Handler for PrioritizeHandler {
    type Input = PrioritizeCommand;
    type Config = PrioritizeConfig;

    fn parse_input(&self, ctx: &Context, event: &Event) -> Result<Option<Self::Input>, String> {
        let body = if let Some(b) = event.comment_body() {
            b
        } else {
            // not interested in other events
            return Ok(None);
        };

        if let Event::Issue(e) = event {
            if e.action != github::IssuesAction::Opened {
                log::debug!("skipping event, issue was {:?}", e.action);
                // skip events other than opening the issue to avoid retriggering commands in the
                // issue body
                return Ok(None);
            }
        }

        let mut input = Input::new(&body, &ctx.username);
        match input.parse_command() {
            Command::Prioritize(Ok(cmd)) => Ok(Some(cmd)),
            _ => Ok(None),
        }
    }

    fn handle_input<'a>(
        &self,
        ctx: &'a Context,
        config: &'a Self::Config,
        event: &'a Event,
        _cmd: Self::Input,
    ) -> BoxFuture<'a, anyhow::Result<()>> {
        handle_input(ctx, config, event).boxed()
    }
}

async fn handle_input(ctx: &Context, config: &PrioritizeConfig, event: &Event) -> anyhow::Result<()> {
    let is_team_member = if let Err(_) | Ok(false) = event.user().is_team_member(&ctx.github).await {
        false
    } else {
        true
    };

    let issue = event.issue().unwrap();
    if !is_team_member {
        let cmnt = ErrorComment::new(&issue, "Only Rust team members can prioritize issues.");
        cmnt.post(&ctx.github).await?;
        return Ok(());
    }

    if issue.labels().iter().any(|l| l.name == "I-prioritize") {
        let cmnt = ErrorComment::new(&issue, "This issue is already prioritized!");
        cmnt.post(&ctx.github).await?;
        return Ok(());
    }

    let mut labels = issue.labels().to_owned();
    labels.push(github::Label {
        name: "I-prioritize".to_string(),
    });
    let github_req = issue.set_labels(&ctx.github, labels);

    let mut zulip_topic = format!("I-pri #{} {}", issue.number, issue.title);
    zulip_topic.truncate(60); // Zulip limitation
    let client = reqwest::Client::new(); // TODO: have a Zulip Client akin to GithubClient
    let zulip_req = client.post("https://rust-lang.zulipchat.com/api/v1/messages")
        .form(&[
            ("type", "stream"),
            ("to", config.zulip_stream.to_string().as_str()),
            ("topic", &zulip_topic),
            ("content", "@*WG-prioritization*"),
        ])
        .send();
    
    let (gh_res, zulip_res) = futures::join!(github_req, zulip_req);
    gh_res?;
    zulip_res?;
    Ok(())
}
