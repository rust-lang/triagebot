use crate::github::{
    self, GithubClient, IssueCommentAction, IssueCommentEvent, IssuesAction, IssuesEvent, User,
};
use crate::github::{Event, Issue};
use crate::jobs::Job;
use crate::zulip::to_zulip_id;
use anyhow::Context as _;
use async_trait::async_trait;
use chrono::Utc;
use tracing::{self as log};

use super::Context;

const MAX_ZULIP_TOPIC: usize = 60;
const RUST_PROJECT_GOALS_REPO: &'static str = "rust-lang/rust-project-goals";
const GOALS_STREAM: u64 = 435869; // #project-goals
const C_TRACKING_ISSUE: &str = "C-tracking-issue";

const MESSAGE: &str = r#"
Dear $OWNERS, it's been $DAYS days since the last update to your goal *$GOAL*. Please comment on the github tracking issue goals#$GOALNUM with an update at your earliest convenience. Thanks! <3

Here is a suggested template for updates (feel free to drop the items that don't apply):

* **Key developments:** *What has happened since the last time. It's perfectly ok to list "nothing" if that's the truth, we know people get busy.*
* **Blockers:** *List any Rust teams you are waiting on and what you are waiting for.*
* **Help wanted:** *Are there places where you are looking for contribution or feedback from the broader community?*
"#;

pub struct ProjectGoalsUpdateJob;

#[async_trait]
impl Job for ProjectGoalsUpdateJob {
    fn name(&self) -> &'static str {
        "project_goals_update_job"
    }

    async fn run(&self, ctx: &super::Context, _metadata: &serde_json::Value) -> anyhow::Result<()> {
        ping_project_goals_owners(&ctx.github, false).await
    }
}

/// Returns true if the user with the given github id is allowed to ping all group people
/// and do other "project group adminstrative" tasks.
pub async fn check_project_goal_acl(_gh: &GithubClient, gh_id: u64) -> anyhow::Result<bool> {
    /// Github ID of the user allowed to ping all group people.
    ///
    /// FIXME: We should create a team for the person/people managing the goals program
    /// and check that the zulip person is on it, but I'm too
    const GOAL_OWNER_GH_ID: u64 = 155238; // nikomatsakis

    Ok(gh_id == GOAL_OWNER_GH_ID)
}

pub async fn ping_project_goals_owners(gh: &GithubClient, dry_run: bool) -> anyhow::Result<()> {
    let goals_repo = gh.repository(&RUST_PROJECT_GOALS_REPO).await?;

    let tracking_issues_query = github::Query {
        filters: vec![("state", "open"), ("is", "issue")],
        include_labels: vec!["C-tracking-issue"],
        exclude_labels: vec![],
    };
    let issues = goals_repo
        .get_issues(&gh, &tracking_issues_query)
        .await
        .with_context(|| "Unable to get issues.")?;

    for issue in issues {
        let comments = issue.comments.unwrap_or(0);

        // Find the time of the last comment posted.
        let days_since_last_comment = (Utc::now() - issue.updated_at).num_days();

        // Start pinging 3 weeks after the last update.
        // As a special case, if the last update was within a day of creation, that means no initial update, so ping anyway.
        log::debug!(
            "issue #{}: days_since_last_comment = {} days, number of comments = {}",
            issue.number,
            days_since_last_comment,
            comments,
        );
        if days_since_last_comment < 21 && comments > 1 {
            continue;
        }

        let zulip_topic_name = zulip_topic_name(&issue);
        let Some(zulip_owners) = zulip_owners(gh, &issue).await? else {
            log::debug!("no owners assigned");
            continue;
        };

        let message = MESSAGE
            .replace("$OWNERS", &zulip_owners)
            .replace(
                "$DAYS",
                &if comments <= 1 {
                    "∞".to_string()
                } else {
                    days_since_last_comment.to_string()
                },
            )
            .replace("$GOALNUM", &issue.number.to_string())
            .replace("$GOAL", &issue.title);

        let zulip_req = crate::zulip::MessageApiRequest {
            recipient: crate::zulip::Recipient::Stream {
                id: GOALS_STREAM,
                topic: &zulip_topic_name,
            },
            content: &message,
        };

        log::debug!("zulip_topic_name = {zulip_topic_name:#?}");
        log::debug!("message = {message:#?}");

        if !dry_run {
            zulip_req.send(&gh.raw()).await?;
        } else {
            log::debug!("skipping zulip send because dry run");
        }
    }

    Ok(())
}

fn zulip_topic_name(issue: &Issue) -> String {
    let goal_number = format!("(goals#{})", issue.number);
    let mut title = String::new();
    for word in issue.title.split_whitespace() {
        if title.len() + word.len() + 1 + goal_number.len() >= MAX_ZULIP_TOPIC {
            break;
        }
        title.push_str(word);
        title.push(' ');
    }
    title.push_str(&goal_number);
    assert!(title.len() < MAX_ZULIP_TOPIC);
    title
}

async fn zulip_owners(gh: &GithubClient, issue: &Issue) -> anyhow::Result<Option<String>> {
    use std::fmt::Write;

    Ok(match &issue.assignees[..] {
        [] => None,
        [string0] => Some(owner_string(gh, string0).await?),
        [string0, string1] => Some(format!(
            "{} and {}",
            owner_string(gh, string0).await?,
            owner_string(gh, string1).await?
        )),
        [string0 @ .., string1] => {
            let mut out = String::new();
            for s in string0 {
                write!(out, "{}, ", owner_string(gh, s).await?).unwrap();
            }
            write!(out, "{}, ", owner_string(gh, string1).await?).unwrap();
            Some(out)
        }
    })
}

async fn owner_string(gh: &GithubClient, assignee: &User) -> anyhow::Result<String> {
    if let Some(zulip_id) = to_zulip_id(gh, assignee.id).await? {
        Ok(format!("@**|{zulip_id}**"))
    } else {
        // No zulip-id? Fallback to github user name.
        Ok(format!(
            "@{login} ([register your zulip-id here to get a real ping!](https://github.com/rust-lang/team/tree/master/people/{login}.toml))",
            login = assignee.login,
        ))
    }
}

pub async fn handle(ctx: &Context, event: &Event) -> anyhow::Result<()> {
    let gh = &ctx.github;

    if event.repo().full_name != RUST_PROJECT_GOALS_REPO {
        return Ok(());
    }

    match event {
        // When a new issue is opened that is tagged as a tracking issue,
        // automatically create a Zulip topic for it and post a comment to the issue.
        Event::Issue(IssuesEvent {
            action: IssuesAction::Opened,
            issue,
            ..
        }) => {
            if issue.labels.iter().any(|l| l.name == C_TRACKING_ISSUE) {
                return Ok(());
            }
            let zulip_topic_name = zulip_topic_name(issue);
            let zulip_owners = match zulip_owners(gh, issue).await? {
                Some(names) => names,
                None => format!("(no owners assigned)"),
            };
            let title = &issue.title;
            let goalnum = issue.number;
            let zulip_req = crate::zulip::MessageApiRequest {
                recipient: crate::zulip::Recipient::Stream {
                    id: GOALS_STREAM,
                    topic: &zulip_topic_name,
                },
                content: &format!(
                    r#"New tracking issue goals#{goalnum}.\n* Goal title: {title}\n* Goal owners: {zulip_owners}"#
                ),
            };
            zulip_req.send(&gh.raw()).await?;
            Ok(())
        }

        // When a new comment is posted on a tracking issue, post it to Zulip.
        Event::IssueComment(IssueCommentEvent {
            action,
            issue,
            comment,
            ..
        }) => {
            // Only comments on tracking issues should be forwarded to Zulip.
            if !issue.labels.iter().any(|l| l.name == C_TRACKING_ISSUE) {
                return Ok(());
            }

            let number = issue.number;
            let action_str = match action {
                IssueCommentAction::Created => "posted",
                IssueCommentAction::Edited => "edited",
                IssueCommentAction::Deleted => "deleted",
            };
            let zulip_topic_name = zulip_topic_name(issue);
            let url = &comment.html_url;
            let text = &comment.body;
            let zulip_author = owner_string(gh, &comment.user).await?;

            let mut ticks = "````".to_string();
            while text.contains(&ticks) {
                ticks.push('`');
            }

            match action {
                IssueCommentAction::Created | IssueCommentAction::Edited => {
                    let zulip_req = crate::zulip::MessageApiRequest {
                        recipient: crate::zulip::Recipient::Stream {
                            id: GOALS_STREAM,
                            topic: &zulip_topic_name,
                        },
                        content: &format!(
                            "[Comment {action_str}]({url}) on goals#{number} by {zulip_author}:\n\
                            {ticks}quote\n\
                            {text}\n\
                            {ticks}"
                        ),
                    };
                    zulip_req.send(&gh.raw()).await?;
                }

                IssueCommentAction::Deleted => {
                    // Do we really care?
                }
            }

            Ok(())
        }

        _ => {
            /* No action for other cases */
            Ok(())
        }
    }
}
