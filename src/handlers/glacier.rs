//! Allows team members to directly create a glacier PR with the code provided.

use crate::{
    config::GlacierConfig,
    github::Event,
    handlers::{Context, Handler},
};

use futures::future::{BoxFuture, FutureExt};
use parser::command::glacier::GlacierCommand;
use parser::command::{Command, Input};

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

async fn handle_input(_ctx: &Context, _event: &Event, _cmd: GlacierCommand) -> anyhow::Result<()> {
    Ok(())
}
