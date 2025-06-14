use std::fmt::Display;

use crate::jobs::Job;
use crate::zulip::api::Recipient;
use crate::{
    config::MajorChangeConfig,
    github::{Event, Issue, IssuesAction, IssuesEvent, Label, ZulipGitHubReference},
    handlers::Context,
    interactions::ErrorComment,
};
use anyhow::Context as _;
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use parser::command::second::SecondCommand;
use serde::{Deserialize, Serialize};
use tracing as log;

#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) enum Invocation {
    NewProposal,
    AcceptedProposal,
    Rename { prev_issue: ZulipGitHubReference },
    ConcernsAdded,
    ConcernsResolved,
}

pub(super) async fn parse_input(
    _ctx: &Context,
    event: &IssuesEvent,
    config: Option<&MajorChangeConfig>,
) -> Result<Option<Invocation>, String> {
    let config = if let Some(config) = config {
        config
    } else {
        return Ok(None);
    };
    let enabling_label = config.enabling_label.as_str();

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
                    .any(|l| l.name == enabling_label)
                {
                    return Ok(Some(Invocation::Rename { prev_issue }));
                } else {
                    // Ignore renamed issues without primary label (e.g., major-change)
                    // to avoid warning about the feature not being enabled.
                    return Ok(None);
                }
            }
        } else {
            log::warn!("Did not note changes in edited issue?");
            return Ok(None);
        }
    }

    // If we were labeled with accepted, then issue that event
    if matches!(&event.action, IssuesAction::Labeled { label } if label.name == config.accept_label)
    {
        return Ok(Some(Invocation::AcceptedProposal));
    }

    // If the concerns label was added, then considered that the
    // major change is blocked
    if matches!(&event.action, IssuesAction::Labeled { label } if Some(&label.name) == config.concerns_label.as_ref())
    {
        return Ok(Some(Invocation::ConcernsAdded));
    }

    // If the concerns label was removed, then considered that
    // all concerns have been resolved; the major change is no
    // longer blocked.
    if matches!(&event.action, IssuesAction::Unlabeled { label: Some(label) } if Some(&label.name) == config.concerns_label.as_ref())
    {
        return Ok(Some(Invocation::ConcernsResolved));
    }

    // Opening an issue with a label assigned triggers both
    // "Opened" and "Labeled" events.
    //
    // We want to treat reopened issues as new proposals but if the
    // issue is freshly opened, we only want to trigger once;
    // currently we do so on the label event.
    if matches!(event.action, IssuesAction::Reopened if event.issue.labels().iter().any(|l| l.name == enabling_label))
        || matches!(&event.action, IssuesAction::Labeled { label } if label.name == enabling_label)
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
        .any(|l| l.name == config.enabling_label)
    {
        let cmnt = ErrorComment::new(
            &event.issue,
            format!(
                "This issue is not ready for proposals; it lacks the `{}` label.",
                config.enabling_label
            ),
        );
        cmnt.post(&ctx.github).await?;
        return Ok(());
    }
    let (zulip_msg, label_to_add) = match cmd {
        Invocation::NewProposal => (
            format!(
                "A new proposal has been announced: [{} #{}]({}). It will be \
                announced at the next meeting to try and draw attention to it, \
                but usually MCPs are not discussed during triage meetings. If \
                you think this would benefit from discussion amongst the \
                team, consider proposing a design meeting.",
                event.issue.title, event.issue.number, event.issue.html_url,
            ),
            Some(&config.meeting_label),
        ),
        Invocation::AcceptedProposal => (
            format!(
                "This proposal has been accepted: [#{}]({}).",
                event.issue.number, event.issue.html_url,
            ),
            Some(&config.meeting_label),
        ),
        Invocation::Rename { prev_issue } => {
            let issue = &event.issue;

            let prev_topic = zulip_topic_from_issue(&prev_issue);
            let partial_issue = issue.to_zulip_github_reference();
            let new_topic = zulip_topic_from_issue(&partial_issue);

            let zulip_send_req = crate::zulip::MessageApiRequest {
                recipient: Recipient::Stream {
                    id: config.zulip_stream,
                    topic: &prev_topic,
                },
                content: "The associated GitHub issue has been renamed. Renaming this Zulip topic.",
            };
            let zulip_send_res = zulip_send_req
                .send(&ctx.zulip)
                .await
                .context("zulip post failed")?;

            let zulip_update_req = crate::zulip::UpdateMessageApiRequest {
                message_id: zulip_send_res.message_id,
                topic: Some(&new_topic),
                propagate_mode: Some("change_all"),
                content: None,
            };
            zulip_update_req
                .send(&ctx.zulip)
                .await
                .context("zulip message update failed")?;

            // after renaming the zulip topic, post an additional comment under the old topic with a url to the new, renamed topic
            // this is necessary due to the lack of topic permalinks, see https://github.com/zulip/zulip/issues/15290
            let new_topic_url = Recipient::Stream {
                id: config.zulip_stream,
                topic: &new_topic,
            }
            .url(&ctx.zulip);
            let breadcrumb_comment = format!(
                "The associated GitHub issue has been renamed. Please see the [renamed Zulip topic]({}).",
                new_topic_url
            );
            let zulip_send_breadcrumb_req = crate::zulip::MessageApiRequest {
                recipient: Recipient::Stream {
                    id: config.zulip_stream,
                    topic: &prev_topic,
                },
                content: &breadcrumb_comment,
            };
            zulip_send_breadcrumb_req
                .send(&ctx.zulip)
                .await
                .context("zulip post failed")?;

            return Ok(());
        }
        Invocation::ConcernsAdded => (
            // Ideally, we would remove the `enabled_label` (if present) and add it back once all concerns are resolved.
            //
            // However, since this handler is stateless, we can't track when to re-add it, it's also a bit unclear if it
            // should be re-added at all. Also historically the `enable_label` wasn't removed either, so we don't touch it.
            format!(
                "Concern(s) have been raised on the [associated GitHub issue]({}). This proposal is now blocked until those concerns are fully resolved.",
                event.issue.html_url
            ),
            None,
        ),
        Invocation::ConcernsResolved => (
            if event.issue.labels().contains(&Label {
                name: config.second_label.to_string(),
            }) {
                // Re-schedule acceptance job to automaticaly close the MCP
                schedule_acceptance_job(ctx, config, &event.issue).await?;

                format!(
                    "All concerns on the [associated GitHub issue]({}) have been resolved, this proposal is no longer blocked, and will be approved in {} days if no (new) objections are raised.",
                    event.issue.html_url, config.waiting_period
                )
            } else {
                format!(
                    "All concerns on the [associated GitHub issue]({}) have been resolved, this proposal is no longer blocked.",
                    event.issue.html_url
                )
            },
            None,
        ),
    };

    handle(
        ctx,
        config,
        &event.issue,
        zulip_msg,
        label_to_add.cloned(),
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

    if !issue
        .labels()
        .iter()
        .any(|l| l.name == config.enabling_label)
    {
        let cmnt = ErrorComment::new(
            &issue,
            &format!(
                "This issue cannot be seconded; it lacks the `{}` label.",
                config.enabling_label
            ),
        );
        cmnt.post(&ctx.github).await?;
        return Ok(());
    }

    let is_team_member = event
        .user()
        .is_team_member(&ctx.team)
        .await
        .ok()
        .unwrap_or(false);

    if !is_team_member {
        let cmnt = ErrorComment::new(&issue, "Only team members can second issues.");
        cmnt.post(&ctx.github).await?;
        return Ok(());
    }

    let has_concerns = if let Some(concerns_label) = &config.concerns_label {
        issue.labels().iter().any(|l| &l.name == concerns_label)
    } else {
        false
    };

    let zulip_msg = if !has_concerns {
        format!(
            "@*{}*: Proposal [#{}]({}) has been seconded, and will be approved in {} days if no objections are raised.",
            config.zulip_ping,
            issue.number,
            event.html_url().unwrap(),
            config.waiting_period,
        )
    } else {
        format!(
            "@*{}*: Proposal [#{}]({}) has been seconded, but there are unresolved concerns preventing approval, use `@{} resolve concern-name` in the GitHub thread to resolve them.",
            config.zulip_ping,
            issue.number,
            event.html_url().unwrap(),
            &ctx.username,
        )
    };

    handle(
        ctx,
        config,
        issue,
        zulip_msg,
        Some(config.second_label.clone()),
        false,
    )
    .await
    .context("unable to process second command")?;

    if !has_concerns {
        // Schedule acceptance job to automaticaly close the MCP
        schedule_acceptance_job(ctx, config, issue).await?;
    }

    Ok(())
}

async fn schedule_acceptance_job(
    ctx: &Context,
    config: &MajorChangeConfig,
    issue: &Issue,
) -> anyhow::Result<()> {
    if config.auto_closing {
        let seconded_at = Utc::now();
        let accept_at = if issue.repository().full_repo_name() == "rust-lang/triagebot" {
            // Hack for the triagebot repo, so we can test more quickly
            seconded_at + Duration::minutes(5)
        } else {
            seconded_at + Duration::days(config.waiting_period.into())
        };

        let major_change_seconded = MajorChangeSeconded {
            repo: issue.repository().full_repo_name(),
            issue: issue.number,
            seconded_at,
            accept_at,
        };

        tracing::info!(
            "major_change inserting to acceptence queue: {:?}",
            &major_change_seconded
        );

        crate::db::schedule_job(
            &*ctx.db.get().await,
            MAJOR_CHANGE_ACCEPTENCE_JOB_NAME,
            serde_json::to_value(major_change_seconded)
                .context("unable to serialize the major change metadata")?,
            accept_at,
        )
        .await
        .context("failed to add the major change to the automatic acceptance queue")?;
    }

    Ok(())
}

async fn handle(
    ctx: &Context,
    config: &MajorChangeConfig,
    issue: &Issue,
    zulip_msg: String,
    label_to_add: Option<String>,
    new_proposal: bool,
) -> anyhow::Result<()> {
    let github_req = label_to_add
        .map(|label_to_add| issue.add_labels(&ctx.github, vec![Label { name: label_to_add }]));

    let partial_issue = issue.to_zulip_github_reference();
    let zulip_topic = zulip_topic_from_issue(&partial_issue);

    let zulip_req = crate::zulip::MessageApiRequest {
        recipient: Recipient::Stream {
            id: config.zulip_stream,
            topic: &zulip_topic,
        },
        content: &zulip_msg,
    };

    if new_proposal {
        let topic_url = zulip_req.url(&ctx.zulip);
        let comment = format!(
            r#"> [!IMPORTANT]
> This issue is *not meant to be used for technical discussion*. There is a **Zulip [stream]** for that.
> Use this issue to leave procedural comments, such as volunteering to review, indicating that you second the proposal (or third, etc), or raising a concern that you would like to be addressed.

<details>
<summary>Concerns or objections can formally be registered here by adding a comment.</summary>
<p>

```
@rustbot concern reason-for-concern
<description of the concern>
```
Concerns can be lifted with:
```
@rustbot resolve reason-for-concern
```
See documentation at [https://forge.rust-lang.org](https://forge.rust-lang.org/compiler/proposals-and-stabilization.html#what-kinds-of-comments-should-go-on-a-mcp-in-the-compiler-team-repo)

</p>
</details>
{}

[stream]: {}"#,
            config.open_extra_text.as_deref().unwrap_or_default(),
            topic_url
        );
        issue
            .post_comment(&ctx.github, &comment)
            .await
            .context("post major change comment")?;
    }

    let zulip_req = zulip_req.send(&ctx.zulip);

    if let Some(github_req) = github_req {
        let (gh_res, zulip_res) = futures::join!(github_req, zulip_req);
        zulip_res.context("zulip post failed")?;
        gh_res.context("label setting failed")?;
    } else {
        zulip_req.await.context("zulip post failed")?;
    }
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

#[derive(Debug)]
enum SecondedLogicError {
    NotYetAcceptenceTime {
        accept_at: DateTime<Utc>,
        now: DateTime<Utc>,
    },
    IssueNotReady {
        draft: bool,
        open: bool,
    },
    NotAMajorChange,
    SecondLabelAbsent,
    ConcernsLabelPresent,
    NoMajorChangeConfig,
}

impl std::error::Error for SecondedLogicError {}

impl Display for SecondedLogicError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SecondedLogicError::NotYetAcceptenceTime { accept_at, now } => {
                write!(f, "not yet acceptence time ({accept_at} > {now})")
            }
            SecondedLogicError::IssueNotReady { draft, open } => {
                write!(f, "issue is not ready (draft: {draft}; open: {open})")
            }
            SecondedLogicError::NotAMajorChange => write!(f, "not a major change"),
            SecondedLogicError::SecondLabelAbsent => write!(f, "second label is absent"),
            SecondedLogicError::ConcernsLabelPresent => write!(f, "concerns label set"),
            SecondedLogicError::NoMajorChangeConfig => write!(f, "no `[major_change]` config"),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(test, derive(PartialEq, Eq, Clone))]
struct MajorChangeSeconded {
    repo: String,
    issue: u64,
    seconded_at: DateTime<Utc>,
    accept_at: DateTime<Utc>,
}

const MAJOR_CHANGE_ACCEPTENCE_JOB_NAME: &str = "major_change_acceptence";

pub(crate) struct MajorChangeAcceptenceJob;

#[async_trait]
impl Job for MajorChangeAcceptenceJob {
    fn name(&self) -> &'static str {
        MAJOR_CHANGE_ACCEPTENCE_JOB_NAME
    }

    async fn run(&self, ctx: &super::Context, metadata: &serde_json::Value) -> anyhow::Result<()> {
        let major_change: MajorChangeSeconded = serde_json::from_value(metadata.clone())
            .context("unable to deserialize the metadata in major change acceptence job")?;

        let now = Utc::now();

        match process_seconded(&ctx, &major_change, now).await {
            Ok(()) => {
                tracing::info!(
                    "{}: major change ({:?}) as been accepted",
                    self.name(),
                    &major_change,
                );
            }
            Err(err) if err.downcast_ref::<SecondedLogicError>().is_some() => {
                tracing::error!(
                    "{}: major change ({:?}) has a logical error (no retry): {err}",
                    self.name(),
                    &major_change,
                );
                // exit job succesfully, so it's not retried
            }
            Err(err) => {
                tracing::error!(
                    "{}: major change ({:?}) is in error: {err}",
                    self.name(),
                    &major_change,
                );
                return Err(err); // so it is retried
            }
        }

        Ok(())
    }
}

async fn process_seconded(
    ctx: &super::Context,
    major_change: &MajorChangeSeconded,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    if major_change.accept_at > now {
        anyhow::bail!(SecondedLogicError::NotYetAcceptenceTime {
            accept_at: major_change.accept_at,
            now
        });
    }

    let repo = ctx
        .github
        .repository(&major_change.repo)
        .await
        .context("failed retrieving the repository informations")?;

    let config = crate::config::get(&ctx.github, &repo)
        .await
        .context("failed to get triagebot configuration")?;

    let config = config
        .major_change
        .as_ref()
        .ok_or(SecondedLogicError::NoMajorChangeConfig)?;

    let issue = repo
        .get_issue(&ctx.github, major_change.issue)
        .await
        .context("unable to get the associated issue")?;

    if !issue.labels.iter().any(|l| l.name == config.enabling_label) {
        anyhow::bail!(SecondedLogicError::NotAMajorChange);
    }

    if !issue.labels.iter().any(|l| l.name == config.second_label) {
        anyhow::bail!(SecondedLogicError::SecondLabelAbsent);
    }

    let concerns_label = config.concerns_label.as_ref();
    if issue.labels.iter().any(|l| Some(&l.name) == concerns_label) {
        anyhow::bail!(SecondedLogicError::ConcernsLabelPresent);
    }

    if !issue.is_open() || issue.draft {
        anyhow::bail!(SecondedLogicError::IssueNotReady {
            draft: issue.draft,
            open: issue.is_open()
        });
    }

    if !issue.labels.iter().any(|l| l.name == config.accept_label) {
        // Only post the comment if the accept_label isn't set yet, we may be in a retry
        issue
            .post_comment(
                &ctx.github,
                &*format!(
r#"The final comment period is now complete, this major change is now **accepted**.

As the automated representative, I would like to thank the author for their work and everyone else who contributed to this major change proposal.

*If you think this major change shouldn't have been accepted, feel free to remove the `{}` label and reopen this issue.*"#,
                    &config.accept_label,
                ),
            )
            .await
            .context("unable to post the acceptance comment")?;
    }
    issue
        .add_labels(
            &ctx.github,
            vec![Label {
                name: config.accept_label.clone(),
            }],
        )
        .await
        .context("unable to add the accept label")?;
    issue
        .remove_label(&ctx.github, &config.second_label)
        .await
        .context("unable to remove the second label")?;
    issue
        .close(&ctx.github)
        .await
        .context("unable to close the issue")?;

    Ok(())
}

#[test]
fn major_change_queue_serialize() {
    let original = MajorChangeSeconded {
        repo: "rust-lang/rust".to_string(),
        issue: 1245,
        seconded_at: Utc::now(),
        accept_at: Utc::now(),
    };

    let value = serde_json::to_value(original.clone()).unwrap();

    let deserialized = serde_json::from_value(value).unwrap();

    assert_eq!(original, deserialized);
}
