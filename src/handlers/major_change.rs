use crate::{
    config::MajorChangeConfig,
    github::{Event, Issue, IssuesAction, IssuesEvent, Label},
    handlers::Context,
    interactions::ErrorComment,
};
use anyhow::Context as _;
use parser::command::second::SecondCommand;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Invocation {
    NewProposal,
    AcceptedProposal,
}

pub(super) fn parse_input(
    _ctx: &Context,
    event: &IssuesEvent,
    _config: Option<&MajorChangeConfig>,
) -> Result<Option<Invocation>, String> {
    // If we were labeled with accepted, then issue that event
    if event.action == IssuesAction::Labeled
        && event
            .label
            .as_ref()
            .map_or(false, |l| l.name == "major-change-accepted")
    {
        return Ok(Some(Invocation::AcceptedProposal));
    }

    // Opening an issue with a label assigned triggers both
    // "Opened" and "Labeled" events.
    //
    // We want to treat reopened issues as new proposals but if the
    // issues is freshly opened, we only want to trigger once;
    // currently we do so on the label event.
    if (event.action == IssuesAction::Reopened
        && event
            .issue
            .labels()
            .iter()
            .any(|l| l.name == "major-change"))
        || (event.action == IssuesAction::Labeled
            && event
                .label
                .as_ref()
                .map_or(false, |l| l.name == "major-change"))
    {
        return Ok(Some(Invocation::NewProposal));
    }

    // All other issue events are ignored
    return Ok(None);
}

pub(super) async fn handle_input(
    ctx: &Context,
    config: &MajorChangeConfig,
    event: &IssuesEvent,
    cmd: Invocation,
) -> anyhow::Result<()> {
    if !event
        .issue
        .labels()
        .iter()
        .any(|l| l.name == "major-change")
    {
        let cmnt = ErrorComment::new(
            &event.issue,
            "This is not a major change (it lacks the `major-change` label).",
        );
        cmnt.post(&ctx.github).await?;
        return Ok(());
    }
    let zulip_msg = match cmd {
        Invocation::NewProposal => format!(
            "A new proposal has been announced: [#{}]({}). It will be \
            announced at the next meeting to try and draw attention to it, \
            but usually MCPs are not discussed during triage meetings. If \
            you think this would benefit from discussion amongst the \
            team, consider proposing a design meeting.",
            event.issue.number, event.issue.html_url,
        ),
        Invocation::AcceptedProposal => format!(
            "This proposal has been accepted: [#{}]({}).",
            event.issue.number, event.issue.html_url,
        ),
    };
    handle(
        ctx,
        config,
        &event.issue,
        zulip_msg,
        config.meeting_label.clone(),
        cmd == Invocation::NewProposal,
    )
    .await
}

pub(super) async fn handle_command(
    ctx: &Context,
    config: &MajorChangeConfig,
    event: &Event,
    _cmd: SecondCommand,
) -> anyhow::Result<()> {
    let issue = event.issue().unwrap();

    if !issue.labels().iter().any(|l| l.name == "major-change") {
        let cmnt = ErrorComment::new(
            &issue,
            "This is not a major change (it lacks the `major-change` label).",
        );
        cmnt.post(&ctx.github).await?;
        return Ok(());
    }

    let is_team_member = event
        .user()
        .is_team_member(&ctx.github)
        .await
        .ok()
        .unwrap_or(false);

    if !is_team_member {
        let cmnt = ErrorComment::new(&issue, "Only team members can second issues.");
        cmnt.post(&ctx.github).await?;
        return Ok(());
    }

    let zulip_msg = format!(
        "@*{}*: Proposal [#{}]({}) has been seconded, and will be approved in 10 days if no objections are raised.",
        config.zulip_ping,
        issue.number,
        event.html_url().unwrap()
    );

    handle(
        ctx,
        config,
        issue,
        zulip_msg,
        config.second_label.clone(),
        false,
    )
    .await
}

async fn handle(
    ctx: &Context,
    config: &MajorChangeConfig,
    issue: &Issue,
    zulip_msg: String,
    label_to_add: String,
    new_proposal: bool,
) -> anyhow::Result<()> {
    let mut labels = issue.labels().to_owned();
    labels.push(Label { name: label_to_add });
    let github_req = issue.set_labels(&ctx.github, labels);

    let mut zulip_topic = format!(" {}", issue.zulip_topic_reference());
    // We prepend the issue title, truncating such that the overall length does
    // not exceed 60 characters (a Zulip limitation).
    zulip_topic.insert_str(
        0,
        &issue.title[..std::cmp::min(issue.title.len(), 60 - zulip_topic.len())],
    );

    let zulip_req = crate::zulip::MessageApiRequest {
        recipient: crate::zulip::Recipient::Stream {
            id: config.zulip_stream,
            topic: &zulip_topic,
        },
        content: &zulip_msg,
    };

    if new_proposal {
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
    zulip_res.context("zulip post failed")?;
    gh_res.context("label setting failed")?;
    Ok(())
}
