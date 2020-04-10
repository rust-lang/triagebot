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
    let issue = event.issue().unwrap();

    if issue.labels().iter().any(|l| l.name == config.label) {
        let cmnt = ErrorComment::new(&issue, "This issue has already been requested for prioritization.");
        cmnt.post(&ctx.github).await?;
        return Ok(());
    }

    let mut labels = issue.labels().to_owned();
    labels.push(github::Label {
        name: config.label.clone(),
    });
    let github_req = issue.set_labels(&ctx.github, labels);

    let mut zulip_topic = format!("{} #{} {}", config.label, issue.number, issue.title);
    zulip_topic.truncate(60); // Zulip limitation

    let zulip_req = ctx.github.raw().post("https://rust-lang.zulipchat.com/api/v1/messages")
        .form(&[
            ("type", "stream"),
            ("to", &config.zulip_stream.to_string()),
            ("topic", &zulip_topic),
            ("content", &format!(
                "@*WG-prioritization* issue [#{}]({}) has been requested for prioritization.",
                issue.number, event.html_url().unwrap()
            )),
        ])
        .send();
    
    let (gh_res, zulip_res) = futures::join!(github_req, zulip_req);
    gh_res?;
    zulip_res?;
    Ok(())
}
