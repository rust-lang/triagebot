use crate::{
    config::MajorChangeConfig,
    github::{self, Event},
    handlers::{Context, Handler},
    interactions::ErrorComment,
};
use futures::future::{BoxFuture, FutureExt};
use parser::command::second::SecondCommand;
use parser::command::{Command, Input};

pub(super) enum Invocation {
    Second,
    NewProposal,
}

pub(super) struct MajorChangeHandler;

impl Handler for MajorChangeHandler {
    type Input = Invocation;
    type Config = MajorChangeConfig;

    fn parse_input(
        &self,
        ctx: &Context,
        event: &Event,
        _: Option<&Self::Config>,
    ) -> Result<Option<Self::Input>, String> {
        let body = if let Some(b) = event.comment_body() {
            b
        } else {
            // not interested in other events
            return Ok(None);
        };

        match event {
            Event::Issue(e) => {
                if e.action != github::IssuesAction::Opened {
                    return Ok(None);
                }
            }
            Event::IssueComment(e) => {
                if e.action != github::IssueCommentAction::Created {
                    return Ok(None);
                }
            }
        }

        if let Event::Issue(e) = event {
            if e.issue.labels().iter().any(|l| l.name == "major-change") {
                return Ok(Some(Invocation::NewProposal));
            }
        }

        let mut input = Input::new(&body, &ctx.username);
        match input.parse_command() {
            Command::Second(Ok(SecondCommand)) => Ok(Some(Invocation::Second)),
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
    config: &MajorChangeConfig,
    event: &Event,
    cmd: Invocation,
) -> anyhow::Result<()> {
    let issue = event.issue().unwrap();

    let (zulip_msg, label_to_add) = match cmd {
        Invocation::Second => {
            if !issue.labels().iter().any(|l| l.name == "major-change") {
                let cmnt = ErrorComment::new(
                    &issue,
                    "This is not a major change (it lacks the `major-change` label).",
                );
                cmnt.post(&ctx.github).await?;
                return Ok(());
            }

            if !issue.labels().iter().any(|l| l.name == "major-change") {
                let cmnt = ErrorComment::new(
                    &issue,
                    "This is not a major change (it lacks the `major-change` label).",
                );
                cmnt.post(&ctx.github).await?;
                return Ok(());
            }
            let is_team_member =
                if let Err(_) | Ok(false) = event.user().is_team_member(&ctx.github).await {
                    false
                } else {
                    true
                };

            if !is_team_member {
                let cmnt = ErrorComment::new(&issue, "Only team members can second issues.");
                cmnt.post(&ctx.github).await?;
                return Ok(());
            }

            (format!(
                "@*T-compiler*: Proposal [#{}]({}) has been seconded, and will be approved in 10 days if no objections are raised.",
                issue.number,
                event.html_url().unwrap()
            ), config.second_label.clone())
        }
        Invocation::NewProposal => {
            if !issue.labels().iter().any(|l| l.name == "major-change") {
                let cmnt = ErrorComment::new(
                    &issue,
                    "This is not a major change (it lacks the `major-change` label).",
                );
                cmnt.post(&ctx.github).await?;
                return Ok(());
            }
            (format!(
                "A new proposal has been announced [#{}]({}). It will be brought up at the next meeting.",
                issue.number,
                event.html_url().unwrap()
            ), config.meeting_label.clone())
        }
    };

    let mut labels = issue.labels().to_owned();
    labels.push(github::Label { name: label_to_add });
    let github_req = issue.set_labels(&ctx.github, labels);

    let mut zulip_topic = format!("compiler-team#{} {}", issue.number, issue.title);
    zulip_topic.truncate(60); // Zulip limitation

    let zulip_stream = config.zulip_stream.to_string();
    let zulip_req = crate::zulip::MessageApiRequest {
        type_: "stream",
        to: &zulip_stream,
        topic: Some(&zulip_topic),
        content: &zulip_msg,
    };

    let zulip_req = zulip_req.send(&ctx.github.raw());

    let (gh_res, zulip_res) = futures::join!(github_req, zulip_req);
    gh_res?;
    zulip_res?;
    Ok(())
}
