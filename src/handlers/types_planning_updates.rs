use crate::github;
use crate::jobs::Job;
use crate::zulip::BOT_EMAIL;
use crate::zulip::{to_zulip_id, MembersApiResponse};
use anyhow::{format_err, Context as _};
use async_trait::async_trait;
use chrono::{Duration, Utc};

pub struct TypesPlanningUpdatesJob;

#[async_trait]
impl Job for TypesPlanningUpdatesJob {
    fn name(&self) -> &'static str {
        "types_planning_updates"
    }

    async fn run(&self, ctx: &super::Context, _metadata: &serde_json::Value) -> anyhow::Result<()> {
        request_updates(ctx).await?;
        Ok(())
    }
}

const TYPES_REPO: &'static str = "rust-lang/types-team";

pub async fn request_updates(ctx: &super::Context) -> anyhow::Result<()> {
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

    for issue in issues {
        let comments = issue.get_first100_comments(gh).await?;
        if comments.len() >= 100 {
            anyhow::bail!(
                "Encountered types tracking issue with 100 or more comments; needs implementation."
            );
        }
        let older_than_28_days = comments
            .last()
            .map_or(true, |c| c.updated_at < (Utc::now() - Duration::days(28)));
        if !older_than_28_days {
            continue;
        }
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
    }

    Ok(())
}

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
