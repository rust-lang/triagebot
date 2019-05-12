use crate::github::{Event, GithubClient};
use failure::Error;

macro_rules! handlers {
    ($($name:ident = $handler:expr,)*) => {
        $(mod $name;)*

        pub fn handle(ctx: &Context, event: &Event) -> Result<(), Error> {
            $(if let Some(input) = Handler::parse_input(&$handler, ctx, event)? {
                let config = crate::config::get(&ctx.github, event.repo_name())?;
                if let Some(config) = &config.$name {
                    Handler::handle_input(&$handler, ctx, config, event, input)?;
                } else {
                    failure::bail!(
                        "The feature `{}` is not enabled in this repository.\n\
                         To enable it add its section in the `triagebot.toml` \
                         in the root of the repository.",
                        stringify!($name)
                    );
                }
            })*
            Ok(())
        }
    }
}

handlers! {
    assign = assign::AssignmentHandler,
    relabel = relabel::RelabelHandler,
    //tracking_issue = tracking_issue::TrackingIssueHandler,
}

pub struct Context {
    pub github: GithubClient,
    pub username: String,
}

pub trait Handler: Sync + Send {
    type Input;
    type Config;

    fn parse_input(&self, ctx: &Context, event: &Event) -> Result<Option<Self::Input>, Error>;

    fn handle_input(
        &self,
        ctx: &Context,
        config: &Self::Config,
        event: &Event,
        input: Self::Input,
    ) -> Result<(), Error>;
}
