use crate::{
    config::MajorChangeConfig,
    github::{self, Event, IssuesAction},
    handlers::{Context, Handler},
    interactions::ErrorComment,
};
use anyhow::Context as _;
use futures::future::{BoxFuture, FutureExt};
use parser::command::second::SecondCommand;
use parser::command::{Command, Input};

#[derive(Copy, Clone, PartialEq, Eq)]
pub(super) enum Invocation {
    Second,
    NewProposal,
    AcceptedProposal,
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
                // If we were labeled with accepted, then issue that event
                if e.action == IssuesAction::Labeled
                    && e.label.map_or(false, |l| l.name == "major-change-accepted")
                {
                    return Ok(Some(Invocation::AcceptedProposal));
                }

                // Opening an issue with a label assigned triggers both
                // "Opened" and "Labeled" events.
                //
                // We want to treat reopened issues as new proposals but if the
                // issues is freshly opened, we only want to trigger once;
                // currently we do so on the label event.
                if (e.action == IssuesAction::Reopened
                    && e.issue.labels().iter().any(|l| l.name == "major-change"))
                    || (e.action == IssuesAction::Labeled
                        && e.label.map_or(false, |l| l.name == "major-change"))
                {
                    return Ok(Some(Invocation::NewProposal));
                }

                // All other issue events are ignored
                return Ok(None);
            }
            Event::IssueComment(e) => {
                if e.action != github::IssueCommentAction::Created {
                    return Ok(None);
                }
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
                "@*{}*: Proposal [#{}]({}) has been seconded, and will be approved in 10 days if no objections are raised.",
                config.zulip_ping,
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
            (
                format!(
                    "A new proposal has been announced: [#{}]({}). It will be
                announced at the next meeting to try and draw attention to it,
                but usually MCPs are not discussed during triage meetings. If
                you think this would benefit from discussion amongst the
                team, consider proposing a design meeting.",
                    issue.number,
                    event.html_url().unwrap()
                ),
                config.meeting_label.clone(),
            )
        }
        Invocation::AcceptedProposal => {
            if !issue.labels().iter().any(|l| l.name == "major-change") {
                let cmnt = ErrorComment::new(
                    &issue,
                    "This is not a major change (it lacks the `major-change` label).",
                );
                cmnt.post(&ctx.github).await?;
                return Ok(());
            }
            (
                format!(
                    "This proposal has been accepted: [#{}]({}).",
                    issue.number,
                    event.html_url().unwrap()
                ),
                config.meeting_label.clone(),
            )
        }
    };

    let mut labels = issue.labels().to_owned();
    labels.push(github::Label { name: label_to_add });
    let github_req = issue.set_labels(&ctx.github, labels);

    let mut zulip_topic = format!(" {}", issue.zulip_topic_reference());
    // We prepend the issue title, truncating such that the overall length does
    // not exceed 60 characters (a Zulip limitation).
    zulip_topic.insert_str(
        0,
        &issue.title[..std::cmp::min(issue.title.len(), 60 - zulip_topic.len())],
    );

    let zulip_stream = config.zulip_stream.to_string();
    let zulip_req = crate::zulip::MessageApiRequest {
        type_: "stream",
        to: &zulip_stream,
        topic: Some(&zulip_topic),
        content: &zulip_msg,
    };

    if cmd == Invocation::NewProposal {
        let topic_url = zulip_req.url();
        let comment = format!(
            "This issue is not meant to be used for technical discussion. \
        There is a Zulip [stream] for that. Use this issue to leave \
        procedural comments, such as volunteering to review, indicating that you \
        second the proposal (or third, etc), or raising a concern that you would \
        like to be addressed. \
        \n\n[stream]: {}",
            topic_url
        );
        issue
            .post_comment(&ctx.github, &comment)
            .await
            .context("post major change comment")?;
    }

    let zulip_req = zulip_req.send(&ctx.github.raw());

    let (gh_res, zulip_res) = futures::join!(github_req, zulip_req);
    gh_res?;
    zulip_res?;
    Ok(())
}
