use crate::{
    config::MajorChangeConfig,
    github::{Event, Issue, IssuesAction, IssuesEvent, Label, ZulipGitHubReference},
    handlers::Context,
    interactions::ErrorComment,
};
use anyhow::Context as _;
use parser::command::second::SecondCommand;
use tracing as log;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Invocation {
    NewProposal,
    AcceptedProposal,
    Rename { prev_issue: ZulipGitHubReference },
}

pub(super) async fn parse_input(
    _ctx: &Context,
    event: &IssuesEvent,
    _config: Option<&MajorChangeConfig>,
) -> Result<Option<Invocation>, String> {
    if event.action == IssuesAction::Edited {
        if let Some(changes) = &event.changes {
            if let Some(previous_title) = &changes.title {
                let prev_issue = ZulipGitHubReference {
                    number: event.issue.number,
                    title: previous_title.from.clone(),
                    repository: event.issue.repository().clone(),
                };
                if event
                    .issue
                    .labels()
                    .iter()
                    .any(|l| l.name == "major-change")
                {
                    return Ok(Some(Invocation::Rename { prev_issue }));
                } else {
                    // Ignore renamed issues without major-change label
                    // to avoid warning about the major-change feature not being
                    // enabled.
                    return Ok(None);
                }
            }
        } else {
            log::warn!("Did not note changes in edited issue?");
            return Ok(None);
        }
    }

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
    // issue is freshly opened, we only want to trigger once;
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
            "A new proposal has been announced: [{} #{}]({}). It will be \
            announced at the next meeting to try and draw attention to it, \
            but usually MCPs are not discussed during triage meetings. If \
            you think this would benefit from discussion amongst the \
            team, consider proposing a design meeting.",
            event.issue.title, event.issue.number, event.issue.html_url,
        ),
        Invocation::AcceptedProposal => format!(
            "This proposal has been accepted: [#{}]({}).",
            event.issue.number, event.issue.html_url,
        ),
        Invocation::Rename { prev_issue } => {
            let issue = &event.issue;

            let prev_topic = zulip_topic_from_issue(&prev_issue);
            let partial_issue = issue.to_zulip_github_reference();
            let new_topic = zulip_topic_from_issue(&partial_issue);

            let zulip_send_req = crate::zulip::MessageApiRequest {
                recipient: crate::zulip::Recipient::Stream {
                    id: config.zulip_stream,
                    topic: &prev_topic,
                },
                content: "The associated GitHub issue has been renamed. Renaming this Zulip topic.",
            };
            let zulip_send_res = zulip_send_req
                .send(&ctx.github.raw())
                .await
                .context("zulip post failed")?;

            let zulip_send_res: crate::zulip::MessageApiResponse = zulip_send_res.json().await?;

            let zulip_update_req = crate::zulip::UpdateMessageApiRequest {
                message_id: zulip_send_res.message_id,
                topic: Some(&new_topic),
                propagate_mode: Some("change_all"),
                content: None,
            };
            zulip_update_req
                .send(&ctx.github.raw())
                .await
                .context("zulip message update failed")?;

            return Ok(());
        }
    };
    handle(
        ctx,
        config,
        &event.issue,
        zulip_msg,
        config.meeting_label.clone(),
        Some(cmd),
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
        None,
    )
    .await
}

async fn handle(
    ctx: &Context,
    config: &MajorChangeConfig,
    issue: &Issue,
    zulip_msg: String,
    label_to_add: String,
    cmd: Option<Invocation>,
) -> anyhow::Result<()> {
    let github_req = issue.add_labels(&ctx.github, vec![Label { name: label_to_add }]);

    let partial_issue = issue.to_zulip_github_reference();
    let zulip_topic = zulip_topic_from_issue(&partial_issue);

    let zulip_req = crate::zulip::MessageApiRequest {
        recipient: crate::zulip::Recipient::Stream {
            id: config.zulip_stream,
            topic: &zulip_topic,
        },
        content: &zulip_msg,
    };

    if matches!(cmd, Some(Invocation::NewProposal | Invocation::Rename { .. })) {
        let topic_url = zulip_req.url();
        let comment = match cmd {
            Some(Invocation::NewProposal) =>
                format!(
                    "This issue is not meant to be used for technical discussion. \
                There is a Zulip [stream] for that. Use this issue to leave \
                procedural comments, such as volunteering to review, indicating that you \
                second the proposal (or third, etc), or raising a concern that you would \
                like to be addressed. \
                \n\n{} \
                \n\n[stream]: {}",
                    config.open_extra_text.as_deref().unwrap_or_default(),
                    topic_url
                ),
            Some(Invocation::Rename { .. }) =>
                format!(
                    "This issue has been renamed, please see the renamed Zulip [stream] for technical discussion. \
                \n\n{} \
                \n\n[stream]: {}",
                    config.open_extra_text.as_deref().unwrap_or_default(),
                    topic_url
                ),
            _ => unreachable!(),
        };
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

fn zulip_topic_from_issue(issue: &ZulipGitHubReference) -> String {
    // Concatenate the issue title and the topic reference, truncating such that
    // the overall length does not exceed 60 characters (a Zulip limitation).
    let topic_ref = issue.zulip_topic_reference();
    // Skip chars until the last characters that can be written:
    // Maximum 60, minus the reference, minus the elipsis and the space
    let mut chars = issue
        .title
        .char_indices()
        .skip(60 - topic_ref.chars().count() - 2);
    match chars.next() {
        Some((len, _)) if chars.next().is_some() => {
            format!("{}… {}", &issue.title[..len], topic_ref)
        }
        _ => format!("{} {}", issue.title, topic_ref),
    }
}
