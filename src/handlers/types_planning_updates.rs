use crate::db::schedule_job;
use crate::github;
use crate::jobs::Job;
use crate::zulip::BOT_EMAIL;
use crate::zulip::{to_zulip_id, MembersApiResponse};
use anyhow::{format_err, Context as _};
use async_trait::async_trait;
use chrono::{Datelike, Duration, NaiveTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

const TYPES_REPO: &'static str = "rust-lang/types-team";

pub struct TypesPlanningMeetingThreadOpenJob;

#[async_trait]
impl Job for TypesPlanningMeetingThreadOpenJob {
    fn name(&self) -> &'static str {
        "types_planning_meeting_thread_open"
    }

    async fn run(&self, ctx: &super::Context, _metadata: &serde_json::Value) -> anyhow::Result<()> {
        // On the last week of the month, we open a thread on zulip for the next Monday
        let today = chrono::Utc::now().date().naive_utc();
        let first_monday = today + chrono::Duration::days(7);
        let meeting_date_string = first_monday.format("%Y-%m-%d").to_string();
        let message = format!("\
            Hello @*T-types/meetings*. Monthly planning meeting in one week.\n\
            This is a reminder to update the current [roadmap tracking issues](https://github.com/rust-lang/types-team/issues?q=is%3Aissue+is%3Aopen+label%3Aroadmap-tracking-issue).\n\
            Extra reminders will be sent later this week.");
        let zulip_req = crate::zulip::MessageApiRequest {
            recipient: crate::zulip::Recipient::Stream {
                id: 326132,
                topic: &format!("{meeting_date_string} planning meeting"),
            },
            content: &message,
        };
        zulip_req.send(&ctx.github.raw()).await?;

        // Then, we want to schedule the next Thursday after this
        let mut thursday = today;
        while thursday.weekday().num_days_from_monday() != 3 {
            thursday = thursday.succ();
        }
        let thursday_at_noon =
            Utc.from_utc_datetime(&thursday.and_time(NaiveTime::from_hms(12, 0, 0)));
        let metadata = serde_json::value::to_value(PlanningMeetingUpdatesPingMetadata {
            date_string: meeting_date_string,
        })
        .unwrap();
        schedule_job(
            &*ctx.db.get().await,
            TypesPlanningMeetingUpdatesPing.name(),
            metadata,
            thursday_at_noon,
        )
        .await?;

        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
pub struct PlanningMeetingUpdatesPingMetadata {
    pub date_string: String,
}

pub struct TypesPlanningMeetingUpdatesPing;

#[async_trait]
impl Job for TypesPlanningMeetingUpdatesPing {
    fn name(&self) -> &'static str {
        "types_planning_meeting_updates_ping"
    }

    async fn run(&self, ctx: &super::Context, metadata: &serde_json::Value) -> anyhow::Result<()> {
        let metadata = serde_json::from_value(metadata.clone())?;
        // On the thursday before the first monday, we want to ping for updates
        request_updates(ctx, metadata).await?;
        Ok(())
    }
}

pub async fn request_updates(
    ctx: &super::Context,
    metadata: PlanningMeetingUpdatesPingMetadata,
) -> anyhow::Result<()> {
    let gh = &ctx.github;
    let types_repo = gh.repository(TYPES_REPO).await?;

    let tracking_issues_query = github::Query {
        filters: vec![("state", "open"), ("is", "issue")],
        include_labels: vec!["roadmap-tracking-issue"],
        exclude_labels: vec![],
    };
    let issues = types_repo
        .get_issues(&gh, &tracking_issues_query)
        .await
        .with_context(|| "Unable to get issues.")?;

    let mut issues_needs_updates = vec![];
    for issue in issues {
        // Github doesn't have a nice way to get the *last* comment; we would have to paginate all comments to get it.
        // For now, just bail out if there are more than 100 comments (if this ever becomes a problem, we will have to fix).
        let comments = issue.get_first100_comments(gh).await?;
        if comments.len() >= 100 {
            anyhow::bail!(
                "Encountered types tracking issue with 100 or more comments; needs implementation."
            );
        }

        // If there are any comments in the past 7 days, we consider this "updated". We *could* be more clever, but
        // this is fine under the assumption that tracking issues should only contain updates.
        let older_than_7_days = comments
            .last()
            .map_or(true, |c| c.updated_at < (Utc::now() - Duration::days(7)));
        if !older_than_7_days {
            continue;
        }
        // In the future, we should reach out to specific people in charge of specific issues. For now, because our tracking
        // method is crude and will over-estimate the issues that need updates.
        /*
        let mut dmed_assignee = false;
        for assignee in issue.assignees {
            let zulip_id_and_email = zulip_id_and_email(ctx, assignee.id.unwrap()).await?;
            let (zulip_id, email) = match zulip_id_and_email {
                Some(id) => id,
                None => continue,
            };
            let message = format!(
                "Type team tracking issue needs an update. [Issue #{}]({})",
                issue.number, issue.html_url
            );
            let zulip_req = crate::zulip::MessageApiRequest {
                recipient: crate::zulip::Recipient::Private {
                    id: zulip_id,
                    email: &email,
                },
                content: &message,
            };
            zulip_req.send(&ctx.github.raw()).await?;
            dmed_assignee = true;
        }
        if !dmed_assignee {
            let message = format!(
                "Type team tracking issue needs an update, and was unable to reach an assignee. \
                [Issue #{}]({})",
                issue.number, issue.html_url
            );
            let zulip_req = crate::zulip::MessageApiRequest {
                recipient: crate::zulip::Recipient::Stream {
                    id: 144729,
                    topic: "tracking issue updates",
                },
                content: &message,
            };
            zulip_req.send(&ctx.github.raw()).await?;
        }
        */
        issues_needs_updates.push(format!("- [Issue #{}]({})", issue.number, issue.html_url));
    }

    let issue_list = issues_needs_updates.join("\n");

    let message = format!("The following issues still need updates:\n\n{issue_list}");

    let meeting_date_string = metadata.date_string;
    let zulip_req = crate::zulip::MessageApiRequest {
        recipient: crate::zulip::Recipient::Stream {
            id: 326132,
            topic: &format!("{meeting_date_string} planning meeting"),
        },
        content: &message,
    };
    zulip_req.send(&ctx.github.raw()).await?;

    Ok(())
}

#[allow(unused)] // Needed for commented out bit above
async fn zulip_id_and_email(
    ctx: &super::Context,
    github_id: i64,
) -> anyhow::Result<Option<(u64, String)>> {
    let bot_api_token = std::env::var("ZULIP_API_TOKEN").expect("ZULIP_API_TOKEN");

    let members = ctx
        .github
        .raw()
        .get("https://rust-lang.zulipchat.com/api/v1/users")
        .basic_auth(BOT_EMAIL, Some(&bot_api_token))
        .send()
        .await
        .map_err(|e| format_err!("Failed to get list of zulip users: {e:?}."))?;
    let members = members
        .json::<MembersApiResponse>()
        .await
        .map_err(|e| format_err!("Failed to get list of zulip users: {e:?}."))?;

    let zulip_id = match to_zulip_id(&ctx.github, github_id).await {
        Ok(Some(id)) => id as u64,
        Ok(None) => return Ok(None),
        Err(e) => anyhow::bail!("Could not find Zulip ID for GitHub id {github_id}: {e:?}"),
    };

    let user = members.members.iter().find(|m| m.user_id == zulip_id);

    Ok(user.map(|m| (m.user_id, m.email.clone())))
}
