use crate::{
    config::PrioritizeConfig,
    github::{self, Event},
    handlers::{Context, Handler},
};
use futures::future::{BoxFuture, FutureExt};
use parser::command::prioritize::PrioritizeCommand;
use parser::command::{Command, Input};

pub(super) struct PrioritizeHandler;

pub(crate) enum Prioritize {
    Label,
    Start,
    End,
}

impl Handler for PrioritizeHandler {
    type Input = Prioritize;
    type Config = PrioritizeConfig;

    fn parse_input(
        &self,
        ctx: &Context,
        event: &Event,
        config: Option<&Self::Config>,
    ) -> Result<Option<Self::Input>, String> {
        let body = if let Some(b) = event.comment_body() {
            b
        } else {
            // not interested in other events
            return Ok(None);
        };

        if let Event::Issue(e) = event {
            if e.action == github::IssuesAction::Labeled {
                if let Some(config) = config {
                    if e.label.as_ref().expect("label").name == config.label {
                        // We need to take the exact same action in this case.
                        return Ok(Some(Prioritize::Start));
                    } else {
                        match glob::Pattern::new(&config.priority_labels) {
                            Ok(glob) => {
                                let issue_labels = event.issue().unwrap().labels();
                                let label_name = &e.label.as_ref().expect("label").name;

                                if issue_labels.iter().all(|l| !glob.matches(&l.name))
                                    && config.prioritize_on.iter().any(|l| l == label_name)
                                {
                                    return Ok(Some(Prioritize::Label));
                                }
                            }
                            Err(error) => log::error!("Invalid glob pattern: {}", error),
                        }
                    }
                }
            }

            if e.action == github::IssuesAction::Unlabeled {
                if let Some(config) = config {
                    if e.label.as_ref().expect("label").name == config.label {
                        // We need to take the exact same action in this case.
                        return Ok(Some(Prioritize::End));
                    }
                }
            }

            if e.action != github::IssuesAction::Opened {
                log::debug!("skipping event, issue was {:?}", e.action);
                // skip events other than opening the issue to avoid retriggering commands in the
                // issue body
                return Ok(None);
            }
        }

        let mut input = Input::new(&body, &ctx.username);
        match input.parse_command() {
            Command::Prioritize(Ok(PrioritizeCommand)) => Ok(Some(Prioritize::Label)),
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
    cmd: Prioritize,
) -> anyhow::Result<()> {
    let issue = event.issue().unwrap();

    let mut labels = issue.labels().to_owned();
    let content = match cmd {
        Prioritize::Label => {
            // Don't add the label if it's already there
            if !labels.iter().any(|l| l.name == config.label) {
                labels.push(github::Label {
                    name: config.label.clone(),
                });
            } else {
                // TODO maybe send a GitHub message if the label is already there,
                // i.e. the issue has already been requested for prioritization?
                return Ok(());
            }
            None
        }
        Prioritize::Start => {
            Some(format!(
                "@*WG-prioritization* issue [#{}]({}) has been requested for prioritization.",
                issue.number,
                event.html_url().unwrap()
            ))
        }
        Prioritize::End => {
            // Shouldn't be necessary in practice as we only end on label
            // removal, but if we add support in the future let's be sure to do
            // the right thing.
            if let Some(idx) = labels.iter().position(|l| l.name == config.label) {
                labels.remove(idx);
            }
            Some(format!(
                "Issue [#{}]({})'s prioritization request has been removed.",
                issue.number,
                event.html_url().unwrap()
            ))
        }
    };

    let github_req = issue.set_labels(&ctx.github, labels);

    if let Some(content) = content {
        let mut zulip_topic = format!(
            "{} {} {}",
            config.label,
            issue.zulip_topic_reference(),
            issue.title
        );
        zulip_topic.truncate(60); // Zulip limitation

        let zulip_stream = config.zulip_stream.to_string();
        let zulip_req = crate::zulip::MessageApiRequest {
            type_: "stream",
            to: &zulip_stream,
            topic: Some(&zulip_topic),
            content: &content,
        };
        let zulip_req = zulip_req.send(&ctx.github.raw());
        let (gh_res, zulip_res) = futures::join!(github_req, zulip_req);
        gh_res?;
        zulip_res?;
        Ok(())
    } else {
        github_req.await?;
        Ok(())
    }
}
