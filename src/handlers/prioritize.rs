use crate::{
    config::PrioritizeConfig,
    github::{self, Event},
    handlers::{Context, Handler},
};
use futures::future::{BoxFuture, FutureExt};
use parser::command::prioritize::PrioritizeCommand;
use parser::command::{Command, Input};

pub(super) struct PrioritizeHandler;

impl Handler for PrioritizeHandler {
    type Input = PrioritizeCommand;
    type Config = PrioritizeConfig;

    fn parse_input(
        &self,
        ctx: &Context,
        event: &Event,
        _config: Option<&Self::Config>,
    ) -> Result<Option<Self::Input>, String> {
        let body = if let Some(b) = event.comment_body() {
            b
        } else {
            // not interested in other events
            return Ok(None);
        };
        
        if let Event::Issue(e) = event {
            if !matches!(e.action, github::IssuesAction::Opened | github::IssuesAction::Edited) {
                // skip events other than opening or editing the issue to avoid retriggering commands in the
                // issue body
                return Ok(None);
            }
        }

        let mut input = Input::new(&body, &ctx.username);
        let command = input.parse_command();
        
        if let Some(previous) = event.comment_from() {
            let mut prev_input = Input::new(&previous, &ctx.username);
            let prev_command = prev_input.parse_command();
            if command == prev_command {
                return Ok(None);
            }
        }
        
        match command {
            Command::Prioritize(Ok(PrioritizeCommand)) => Ok(Some(PrioritizeCommand)),
            _ => Ok(None),
        }
    }

    fn handle_input<'a>(
        &self,
        ctx: &'a Context,
        config: &'a Self::Config,
        event: &'a Event,
        cmd: Self::Input,
    ) -> BoxFuture<'a, anyhow::Result<()>> {
        handle_input(ctx, config, event, cmd).boxed()
    }
}

async fn handle_input(
    ctx: &Context,
    config: &PrioritizeConfig,
    event: &Event,
    _: PrioritizeCommand,
) -> anyhow::Result<()> {
    let issue = event.issue().unwrap();
    let mut labels = issue.labels().to_owned();

    // Don't add the label if it's already there
    if !labels.iter().any(|l| l.name == config.label) {
        labels.push(github::Label { name: config.label.to_owned() });
    }

    issue.set_labels(&ctx.github, labels).await?;
    Ok(())
}
