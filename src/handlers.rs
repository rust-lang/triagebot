use crate::config::{self, ConfigurationError};
use crate::github::{Event, GithubClient};
use futures::future::BoxFuture;
use std::fmt;
use tokio_postgres::Client as DbClient;

#[derive(Debug)]
pub enum HandlerError {
    Message(String),
    Other(anyhow::Error),
}

impl std::error::Error for HandlerError {}

impl fmt::Display for HandlerError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            HandlerError::Message(msg) => write!(f, "{}", msg),
            HandlerError::Other(_) => write!(f, "An internal error occurred."),
        }
    }
}

mod notification;
mod rustc_commits;

macro_rules! handlers {
    ($($name:ident = $handler:expr,)*) => {
        $(mod $name;)*

        pub async fn handle(ctx: &Context, event: &Event) -> Result<(), HandlerError> {
            let config = config::get(&ctx.github, event.repo_name()).await;

            $(
            if let Some(input) = Handler::parse_input(
                &$handler, ctx, event, config.as_ref().ok().and_then(|c| c.$name.as_ref()),
            ).map_err(HandlerError::Message)? {
                let config = match &config {
                    Ok(config) => config,
                    Err(e @ ConfigurationError::Missing) => {
                        return Err(HandlerError::Message(e.to_string()));
                    }
                    Err(e @ ConfigurationError::Toml(_)) => {
                        return Err(HandlerError::Message(e.to_string()));
                    }
                    Err(e @ ConfigurationError::Http(_)) => {
                        return Err(HandlerError::Other(e.clone().into()));
                    }
                };
                if let Some(config) = &config.$name {
                    Handler::handle_input(&$handler, ctx, config, event, input).await.map_err(HandlerError::Other)?;
                } else {
                    return Err(HandlerError::Message(format!(
                        "The feature `{}` is not enabled in this repository.\n\
                         To enable it add its section in the `triagebot.toml` \
                         in the root of the repository.",
                        stringify!($name)
                    )));
                }
            })*

            if let Err(e) = notification::handle(ctx, event).await {
                log::error!("failed to process event {:?} with notification handler: {:?}", event, e);
            }

            if let Err(e) = rustc_commits::handle(ctx, event).await {
                log::error!("failed to process event {:?} with rustc_commits handler: {:?}", event, e);
            }

            Ok(())
        }
    }
}

handlers! {
    assign = assign::AssignmentHandler,
    relabel = relabel::RelabelHandler,
    ping = ping::PingHandler,
    nominate = nominate::NominateHandler,
    prioritize = prioritize::PrioritizeHandler,
    major_change = major_change::MajorChangeHandler,
    //tracking_issue = tracking_issue::TrackingIssueHandler,
    glacier = glacier::GlacierHandler,
}

pub struct Context {
    pub github: GithubClient,
    pub db: DbClient,
    pub username: String,
}

pub trait Handler: Sync + Send {
    type Input;
    type Config;

    fn parse_input(
        &self,
        ctx: &Context,
        event: &Event,
        config: Option<&Self::Config>,
    ) -> Result<Option<Self::Input>, String>;

    fn handle_input<'a>(
        &self,
        ctx: &'a Context,
        config: &'a Self::Config,
        event: &'a Event,
        input: Self::Input,
    ) -> BoxFuture<'a, anyhow::Result<()>>;
}
