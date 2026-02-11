pub mod api;
pub mod client;
mod commands;

use crate::db::notifications::add_metadata;
use crate::db::notifications::{self, Identifier, delete_ping, move_indices, record_ping};
use crate::db::review_prefs::{
    ReviewPreferences, RotationMode, get_review_prefs, get_review_prefs_batch,
    upsert_repo_review_prefs, upsert_team_review_prefs, upsert_user_review_prefs,
};
use crate::github::{User, UserComment};
use crate::handlers::Context;
use crate::handlers::docs_update::docs_update;
use crate::handlers::pr_tracking::{ReviewerWorkqueue, get_assigned_prs};
use crate::handlers::project_goals::{self, ping_project_goals_owners};
use crate::interactions::ErrorComment;
use crate::utils::pluralize;
use crate::zulip::api::{MessageApiResponse, Recipient};
use crate::zulip::client::ZulipClient;
use crate::zulip::commands::{
    BackportChannelArgs, BackportVerbArgs, ChatCommand, LookupCmd, PingGoalsArgs, StreamCommand,
    WorkqueueCmd, WorkqueueLimit, parse_cli,
};
use anyhow::{Context as _, format_err};
use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::response::IntoResponse;
use commands::BackportArgs;
use itertools::Itertools;
use octocrab::Octocrab;
use rust_team_data::v1::{TeamKind, TeamMember};
use secrecy::{ExposeSecret, SecretString};
use std::cmp::{Ordering, Reverse};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;
use subtle::ConstantTimeEq;
use tracing::log;

fn get_text_backport_approved(
    channel: &BackportChannelArgs,
    verb: &BackportVerbArgs,
    zulip_link: &str,
) -> String {
    format!("
{channel} backport {verb} as per compiler team [on Zulip]({zulip_link}). A backport PR will be authored by the release team at the end of the current development cycle. Backport labels are handled by them.

@rustbot label +{channel}-accepted")
}

fn get_text_backport_declined(
    channel: &BackportChannelArgs,
    verb: &BackportVerbArgs,
    zulip_link: &str,
) -> String {
    format!(
        "
{channel} backport {verb} as per compiler team [on Zulip]({zulip_link}).

@rustbot label -{channel}-nominated -{channel}-accepted"
    )
}

#[derive(Debug, serde::Deserialize)]
pub struct Request {
    /// Markdown body of the sent message.
    data: String,

    /// Metadata about this request.
    message: Message,

    /// Authentication token. The same for all Zulip messages.
    token: SecretString,
}

#[derive(Clone, Debug, serde::Deserialize)]
struct Message {
    sender_id: u64,
    /// A unique ID for the set of users receiving the message (either a
    /// stream or group of users). Useful primarily for hashing.
    #[allow(unused)]
    recipient_id: u64,
    sender_full_name: String,
    sender_email: String,
    /// The ID of the stream.
    ///
    /// `None` if it is a private message.
    stream_id: Option<u64>,
    /// The topic of the incoming message. Not the stream name.
    ///
    /// Not currently set for private messages (though Zulip may change this in
    /// the future if it adds topics to private messages).
    subject: Option<String>,
    /// The type of the message: stream or private.
    #[allow(unused)]
    #[serde(rename = "type")]
    type_: String,
}

impl Message {
    /// Creates a `Recipient` that will be addressed to the sender of this message.
    fn sender_to_recipient(&self) -> Recipient<'_> {
        match self.stream_id {
            Some(id) => Recipient::Stream {
                id,
                topic: self
                    .subject
                    .as_ref()
                    .expect("stream messages should have a topic"),
            },
            None => Recipient::Private {
                id: self.sender_id,
                email: &self.sender_email,
            },
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Response {
    content: String,
}

/// Top-level handler for Zulip webhooks.
///
/// Returns a JSON response or a 400 with an error message.
pub async fn webhook(
    State(ctx): State<Arc<Context>>,
    req: Result<Json<Request>, JsonRejection>,
) -> axum::response::Response {
    let Json(req) = match req {
        Ok(req) => req,
        Err(rejection) => {
            tracing::error!(?rejection);
            return Json(Response {
                content: ErrorComment::markdown(
                    "unable to handle this Zulip request: invalid JSON input",
                )
                .expect("creating a error message without fail"),
            })
            .into_response();
        }
    };

    tracing::info!(?req);
    let response = process_zulip_request(ctx, req).await;
    tracing::info!(?response);

    match response {
        Ok(None) => Json(ResponseNotRequired {
            response_not_required: true,
        })
        .into_response(),
        Ok(Some(content)) => Json(Response { content }).into_response(),
        Err(err) => {
            // We are mixing network errors and "logic" error (like clap errors)
            // so don't return a 500. Long term we should decouple those.

            // Reply with a 200 and reply only with outermost error
            Json(Response {
                content: err.to_string(),
            })
            .into_response()
        }
    }
}

pub fn get_token_from_env() -> Result<SecretString, anyhow::Error> {
    #[expect(clippy::bind_instead_of_map, reason = "`.map_err` is suggested, but we don't really map the error")]
    // ZULIP_WEBHOOK_SECRET is preferred, ZULIP_TOKEN is kept for retrocompatibility but will be deprecated
    std::env::var("ZULIP_WEBHOOK_SECRET")
        .or_else(|_| std::env::var("ZULIP_TOKEN"))
        .or_else(|_| {
            log::error!(
                "Cannot communicate with Zulip: neither ZULIP_WEBHOOK_SECRET or ZULIP_TOKEN are set."
            );
            Err(anyhow::anyhow!("Cannot communicate with Zulip."))
        })
        .map(|v| v.into())
}

/// Processes a Zulip webhook.
///
/// Returns a string of the response, or None if no response is needed.
async fn process_zulip_request(ctx: Arc<Context>, req: Request) -> anyhow::Result<Option<String>> {
    let expected_token = get_token_from_env()?;
    if !bool::from(
        req.token
            .expose_secret()
            .as_bytes()
            .ct_eq(expected_token.expose_secret().as_bytes()),
    ) {
        anyhow::bail!("Invalid authorization.");
    }

    // Zulip commands are only available to users in the team database
    let gh_id = match ctx.team.zulip_to_github_id(req.message.sender_id).await {
        Ok(Some(gh_id)) => gh_id,
        Ok(None) => {
            return Err(anyhow::anyhow!(
                "Unknown Zulip user. Please add `zulip-id = {}` to your file in \
                [rust-lang/team](https://github.com/rust-lang/team).",
                req.message.sender_id
            ));
        }
        Err(e) => anyhow::bail!("Failed to query team API: {e:?}"),
    };

    handle_command(ctx, gh_id, &req.data, &req.message).await
}

async fn handle_command<'a>(
    ctx: Arc<Context>,
    mut gh_id: u64,
    message: &'a str,
    message_data: &'a Message,
) -> anyhow::Result<Option<String>> {
    log::trace!("handling zulip message {:?}", message);

    // Missing stream means that this is a direct message
    if message_data.stream_id.is_none() {
        let mut words: Vec<&str> = message.split_whitespace().collect();

        // Parse impersonation
        let mut impersonated = false;
        #[expect(clippy::get_first, reason = "for symmetry with `get(1)`")]
        if let Some(&"as") = words.get(0) {
            if let Some(username) = words.get(1) {
                let impersonated_gh_id = ctx
                    .team
                    .get_gh_id_from_username(username)
                    .await
                    .context("getting ID of github user")?
                    .context("Can only authorize for other GitHub users.")?;

                // Impersonate => change actual gh_id
                if impersonated_gh_id != gh_id {
                    impersonated = true;
                    gh_id = impersonated_gh_id;
                }

                // Skip the first two arguments for the rest of CLI parsing
                words = words[2..].to_vec();
            } else {
                return Err(anyhow::anyhow!(
                    "Failed to parse command; expected `as <username> <command...>`."
                ));
            }
        }

        let cmd = parse_cli::<ChatCommand, _>(words.into_iter())?;
        let impersonation_mode = get_cmd_impersonation_mode(&cmd);
        if impersonated && matches!(impersonation_mode, ImpersonationMode::Disabled) {
            return Err(anyhow::anyhow!(
                "This command cannot be used with impersonation. Remove the `as <user>` prefix."
            ));
        }

        tracing::info!("command parsed to {cmd:?} (impersonated: {impersonated})");

        let output = match &cmd {
            ChatCommand::Acknowledge { identifier } => {
                acknowledge(&ctx, gh_id, identifier.into()).await
            }
            ChatCommand::Add { url, description } => {
                add_notification(&ctx, gh_id, url, &description.join(" ")).await
            }
            ChatCommand::Move { from, to } => move_notification(&ctx, gh_id, *from, *to).await,
            ChatCommand::Meta { index, description } => {
                add_meta_notification(&ctx, gh_id, *index, &description.join(" ")).await
            }
            ChatCommand::Whoami => whoami_cmd(&ctx, gh_id).await,
            ChatCommand::Lookup(cmd) => lookup_cmd(&ctx, cmd).await,
            ChatCommand::Work(cmd) => workqueue_commands(&ctx, gh_id, cmd).await,
            ChatCommand::PingGoals(args) => {
                ping_goals_cmd(ctx.clone(), gh_id, message_data, args).await
            }
            ChatCommand::DocsUpdate => trigger_docs_update(message_data, &ctx.zulip),
            ChatCommand::Comments {
                username,
                organization,
            } => recent_comments_cmd(&ctx, gh_id, username, &organization)
                .await
                .map(Some),
            ChatCommand::TeamStats { name, repo } => {
                let repo = normalize_repo(repo);
                team_status_cmd(&ctx, name, &repo).await
            }
        };

        let output = output?;

        // Let the impersonated person know about the impersonation if we should notify
        if impersonated && matches!(impersonation_mode, ImpersonationMode::Notify) {
            let impersonated_zulip_id =
                ctx.team.github_to_zulip_id(gh_id).await?.ok_or_else(|| {
                    anyhow::anyhow!("Zulip user for GitHub ID {gh_id} was not found")
                })?;
            let users = ctx.zulip.get_zulip_users().await?;
            let user = users
                .iter()
                .find(|m| m.user_id == impersonated_zulip_id)
                .ok_or_else(|| format_err!("Could not find Zulip user email."))?;

            let sender = &message_data.sender_full_name;
            let message = format!(
                "{sender} ran `{message}` on your behalf. Output:\n{}",
                output.as_deref().unwrap_or("<empty>")
            );

            MessageApiRequest {
                recipient: Recipient::Private {
                    id: user.user_id,
                    email: &user.email,
                },
                content: &message,
            }
            .send(&ctx.zulip)
            .await?;
        }

        Ok(output)
    } else {
        // We are in a stream, where someone wrote `@**triagebot** <command(s)>`
        //
        // Yet we need to process each lines separately as the command can only be
        // one line.
        for line in message.lines() {
            let words: Vec<&str> = line.split_whitespace().collect();

            // Try to find the ping, continue to the next line if we don't find it here.
            let Some(cmd_index) = words.iter().position(|w| *w == "@**triagebot**") else {
                continue;
            };

            // Skip @**triagebot**
            let cmd_index = cmd_index + 1;

            // Error on unexpected end-of-line
            if cmd_index >= words.len() {
                return Ok(Some(
                    "Error parsing command: unexpected end-of-line".to_string(),
                ));
            }

            let cmd = parse_cli::<StreamCommand, _>(words[cmd_index..].iter().copied())?;
            tracing::info!("command parsed to {cmd:?}");

            // Process the command and early return (we don't expect multi-commands in the
            // same message)
            return match cmd {
                StreamCommand::EndTopic => {
                    post_waiter(&ctx, message_data, WaitingMessage::end_topic())
                        .await
                        .map_err(|e| format_err!("Failed to await at this time: {e:?}"))
                }
                StreamCommand::EndMeeting => {
                    post_waiter(&ctx, message_data, WaitingMessage::end_meeting())
                        .await
                        .map_err(|e| format_err!("Failed to await at this time: {e:?}"))
                }
                StreamCommand::Read => {
                    post_waiter(&ctx, message_data, WaitingMessage::start_reading())
                        .await
                        .map_err(|e| format_err!("Failed to await at this time: {e:?}"))
                }
                StreamCommand::PingGoals(args) => {
                    ping_goals_cmd(ctx, gh_id, message_data, &args).await
                }
                StreamCommand::DocsUpdate => trigger_docs_update(message_data, &ctx.zulip),
                StreamCommand::Backport(args) => {
                    accept_decline_backport(message_data, &ctx.octocrab, &ctx.zulip, &args).await
                }
                StreamCommand::Comments {
                    username,
                    organization,
                } => recent_comments_cmd(&ctx, gh_id, &username, &organization)
                    .await
                    .map(Some),
            };
        }

        tracing::warn!("no command found, yet we were pinged, weird");
        Ok(Some("Unknown command".to_string()))
    }
}

// TODO: shorter variant of this command (f.e. `backport accept` or even `accept`) that infers everything from the Message payload
async fn accept_decline_backport(
    message_data: &Message,
    octo_client: &Octocrab,
    zulip_client: &ZulipClient,
    args_data: &BackportArgs,
) -> anyhow::Result<Option<String>> {
    let message = message_data.clone();
    let args = args_data.clone();
    let stream_id = message.stream_id.unwrap();
    let subject = message.subject.unwrap();

    // Repository owner and name are hardcoded
    // This command is only used in this repository
    let repo_owner = std::env::var("MAIN_GH_REPO_OWNER").unwrap_or("rust-lang".to_string());
    let repo_name = std::env::var("MAIN_GH_REPO_NAME").unwrap_or("rust".to_string());

    // TODO: factor out the Zulip "URL encoder" to make it practical to use
    let zulip_send_req = crate::zulip::MessageApiRequest {
        recipient: Recipient::Stream {
            id: stream_id,
            topic: &subject,
        },
        content: "",
    };

    // NOTE: the Zulip Message API cannot yet pin exactly a single message so the link in the GitHub comment will be to the whole topic
    // See: https://rust-lang.zulipchat.com/#narrow/channel/122653-zulip/topic/.22near.22.20parameter.20in.20payload.20of.20send.20message.20API
    let zulip_link = zulip_send_req.url(zulip_client);

    let message_body = match args.verb {
        BackportVerbArgs::Accept
        | BackportVerbArgs::Accepted
        | BackportVerbArgs::Approve
        | BackportVerbArgs::Approved => {
            get_text_backport_approved(&args.channel, &args.verb, &zulip_link)
        }
        BackportVerbArgs::Decline | BackportVerbArgs::Declined => {
            get_text_backport_declined(&args.channel, &args.verb, &zulip_link)
        }
    };

    let _ = octo_client
        .issues(repo_owner, repo_name)
        .create_comment(args.pr_num, &message_body)
        .await
        .with_context(|| anyhow::anyhow!("unable to post comment on #{}", args.pr_num))?;

    Ok(None)
}

async fn ping_goals_cmd(
    ctx: Arc<Context>,
    gh_id: u64,
    message: &Message,
    args: &PingGoalsArgs,
) -> anyhow::Result<Option<String>> {
    if project_goals::check_project_goal_acl(&ctx.team, gh_id).await? {
        let args = args.clone();
        let message = message.clone();
        tokio::spawn(async move {
            let res = ping_project_goals_owners(
                &ctx.github,
                &ctx.zulip,
                &ctx.team,
                false,
                args.threshold as i64,
                &format!("on {}", args.next_update),
            )
            .await;

            let status = match res {
                Ok(_res) => "OK".to_string(),
                Err(err) => {
                    tracing::error!("ping_project_goals_owners: {err:?}");
                    format!("ERROR\n\n```\n{err:#?}\n```\n")
                }
            };

            let res = MessageApiRequest {
                recipient: message.sender_to_recipient(),
                content: &format!("End pinging project groups owners: {status}"),
            }
            .send(&ctx.zulip)
            .await;

            if let Err(err) = res {
                tracing::error!(
                    "error sending project goals ping reply: {err:?} for status: {status}"
                );
            }
        });

        Ok(Some("Started pinging project groups owners...".to_string()))
    } else {
        Err(format_err!(
            "That command is only permitted for those running the project-goal program.",
        ))
    }
}

/// Output recent GitHub comments made by a given user in a given organization.
/// This command can only be used by team members.
async fn recent_comments_cmd(
    ctx: &Context,
    gh_id: u64,
    username: &str,
    organization: &str,
) -> anyhow::Result<String> {
    const RECENT_COMMENTS_LIMIT: usize = 10;

    let user = User {
        login: ctx
            .team
            .username_from_gh_id(gh_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Username for GitHub user {gh_id} not found"))?,
        id: gh_id,
    };
    if !user.is_team_member(&ctx.team).await? {
        return Err(anyhow::anyhow!(
            "This command is only available to team members."
        ));
    }

    if ctx.team.repos().await?.repos.get(organization).is_none() {
        return Err(anyhow::anyhow!(
            "Organization `{organization}` is not managed by the team database."
        ));
    }

    let comments = ctx
        .github
        .user_comments_in_org(username, organization, RECENT_COMMENTS_LIMIT)
        .await
        .context("Cannot load recent comments")?;

    if comments.is_empty() {
        return Ok(format!(
            "No recent comments found for **{username}** in the `{organization}` organization."
        ));
    }

    let mut message = format!("**Recent comments by {username} in `{organization}`:**\n");
    for comment in &comments {
        message.push_str(&format_user_comment(comment));
    }
    Ok(message)
}

async fn team_status_cmd(
    ctx: &Context,
    team_name: &str,
    repo: &str,
) -> anyhow::Result<Option<String>> {
    use std::fmt::Write;

    let Some(team) = ctx.team.get_team(team_name).await? else {
        return Ok(Some(format!("Team {team_name} not found")));
    };

    let mut members = team.members;
    members.sort_by(|a, b| a.github.cmp(&b.github));

    let usernames: Vec<&str> = members
        .iter()
        .map(|member| member.github.as_str())
        .collect();

    let db = ctx.db.get().await;
    let review_prefs = get_review_prefs_batch(&db, &usernames)
        .await
        .context("cannot load review preferences")?;

    // If a repository name was provided then check for an adhoc group named after that team in
    // that repository's `triagebot.toml` and require that team members be in that list to be
    // considered on rotation.
    let adhoc_group: Option<Vec<String>> = {
        let repo = ctx
            .github
            .repository(repo)
            .await
            .context("failed retrieving the repository informations")?;
        let config = crate::config::get(&ctx.github, &repo)
            .await
            .context("failed to get triagebot configuration")?;
        if let Some(adhoc_group) = config
            .assign
            .as_ref()
            .and_then(|a| a.adhoc_groups.get(team_name))
        {
            Some(
                adhoc_group
                    .into_iter()
                    .map(|reviewer| {
                        // Adhoc groups reviewers are by convention prefixed with `@`, let's
                        // strip it to avoid issues with unprefixed GitHub handles.
                        //
                        // Also lowercase the reviewer, in case it has a different
                        // casing between our different sources.
                        reviewer
                            .strip_prefix('@')
                            .unwrap_or(reviewer)
                            .to_lowercase()
                    })
                    .collect(),
            )
        } else {
            None
        }
    };

    let workqueue_arc = ctx
        .workqueue_map
        .get(repo)
        .unwrap_or_else(|| Arc::new(tokio::sync::RwLock::new(ReviewerWorkqueue::default())));
    let workqueue = workqueue_arc.read().await;
    let total_assigned: u64 = members
        .iter()
        .map(|member| workqueue.assigned_pr_count(member.github_id))
        .sum();

    let mut available = vec![];
    let mut full_capacity = vec![];
    let mut off_rotation = vec![];
    for member in &members {
        let status = get_reviewer_status(
            member,
            &review_prefs,
            repo,
            team_name,
            adhoc_group.as_ref(),
            &workqueue,
        );

        match status {
            ReviewerStatus::Available => available.push((member, status)),
            ReviewerStatus::FullCapacity => full_capacity.push((member, status)),
            status => off_rotation.push((member, status)),
        }
    }
    available.sort_by_key(|(member, _)| Reverse(workqueue.assigned_pr_count(member.github_id)));
    full_capacity.sort_by_key(|(member, _)| Reverse(workqueue.assigned_pr_count(member.github_id)));
    off_rotation.sort_by(|(member_a, status_a), (member_b, status_b)| {
        // Order by off rotation reason first
        match (status_a, status_b) {
            (ReviewerStatus::OffRotationGlobally, ReviewerStatus::OffRotationThroughTeam) => {
                return Ordering::Less;
            }
            (ReviewerStatus::OffRotationThroughTeam, ReviewerStatus::OffRotationGlobally) => {
                return Ordering::Greater;
            }
            _ => {}
        }

        // Then by assigned PR count
        workqueue
            .assigned_pr_count(member_a.github_id)
            .cmp(&workqueue.assigned_pr_count(member_b.github_id))
            .reverse()
    });

    let format_row = |(member, status): (&TeamMember, ReviewerStatus)| {
        let review_prefs = review_prefs.get(member.github.as_str());
        let max_capacity = review_prefs
            .as_ref()
            .and_then(|prefs| prefs.repo_review_prefs.get(repo))
            .and_then(|prefs| prefs.max_assigned_prs)
            .map(|c| c.to_string());
        let max_capacity = max_capacity.as_deref().unwrap_or("unlimited");
        let assigned_prs = workqueue.assigned_pr_count(member.github_id);
        let status = match status {
            ReviewerStatus::Available => format_args!(":check:"),
            ReviewerStatus::FullCapacity => format_args!(":stop_sign:"),
            ReviewerStatus::NotInAdhocGroup => format_args!("Not in adhoc group"),
            ReviewerStatus::OffRotationGlobally => format_args!("Off (global)"),
            ReviewerStatus::OffRotationThroughTeam => format_args!("Off ({team_name})"),
        };

        format!(
            "| `{}` | {} | `{assigned_prs}` | `{max_capacity}` | {status} |",
            member.github, member.name
        )
    };
    let write_table = |msg: &mut String,
                       members: Vec<(&TeamMember, ReviewerStatus)>,
                       title: &str,
                       align_left: bool| {
        if members.is_empty() {
            return;
        }
        let rows = members.into_iter().map(format_row).collect::<Vec<_>>();
        writeln!(
            msg,
            r"### {title} ({})
| Username | Name | Assigned PRs | Review capacity | Status |
|----------|------|-------------:|----------------:|:--------{}|",
            rows.len(),
            if align_left { "" } else { ":" }
        )
        .unwrap();
        writeln!(msg, "{}\n", rows.join("\n")).unwrap();
    };

    // e.g. 2 members, 5 PRs assigned
    let mut msg = format!(
        "{} {}, {} {} assigned\n\n",
        members.len(),
        pluralize("member", members.len()),
        total_assigned,
        pluralize("PR", total_assigned as usize)
    );
    write_table(&mut msg, available, "Available", false);
    write_table(&mut msg, full_capacity, "Full capacity", false);
    write_table(&mut msg, off_rotation, "Off rotation", true);

    Ok(Some(msg))
}

enum ReviewerStatus {
    Available,
    FullCapacity,
    NotInAdhocGroup,
    OffRotationGlobally,
    OffRotationThroughTeam,
}

fn get_reviewer_status(
    member: &TeamMember,
    review_prefs: &HashMap<&str, ReviewPreferences>,
    repo: &str,
    team_name: &str,
    adhoc_group: Option<&Vec<String>>,
    workqueue: &ReviewerWorkqueue,
) -> ReviewerStatus {
    let prefs = review_prefs.get(member.github.as_str());
    let rotation_mode = prefs.map(|prefs| prefs.rotation_mode).unwrap_or_default();
    let team_rotation_mode = prefs
        .and_then(|prefs| prefs.team_review_prefs.get(team_name))
        .map(|prefs| prefs.rotation_mode)
        .unwrap_or_default();
    let in_adhoc_group = adhoc_group
        .as_ref()
        .map(|reviewers| reviewers.contains(&member.github.to_lowercase()))
        .unwrap_or(true);
    let capacity_is_full = if let Some(capacity) = prefs
        .and_then(|prefs| prefs.repo_review_prefs.get(repo))
        .and_then(|prefs| prefs.max_assigned_prs)
        && capacity <= workqueue.assigned_pr_count(member.github_id) as u32
    {
        true
    } else {
        false
    };
    if !in_adhoc_group {
        ReviewerStatus::NotInAdhocGroup
    } else if !matches!(team_rotation_mode, RotationMode::OnRotation) {
        ReviewerStatus::OffRotationThroughTeam
    } else if !matches!(rotation_mode, RotationMode::OnRotation) {
        ReviewerStatus::OffRotationGlobally
    } else if capacity_is_full {
        ReviewerStatus::FullCapacity
    } else {
        ReviewerStatus::Available
    }
}

/// How does impersonation work for a given command.
enum ImpersonationMode {
    /// Impersonation is enabled, but the impersonated user will not be notified.
    /// Should only be used for commands that are "read-only".
    Silent,
    /// Impersonation is enabled and the impersonated user will be notified.
    Notify,
    /// Impersonation is disabled.
    /// Should be used for commands where impersonation doesn't make sense or if there are some
    /// specific permissions required to run the command.
    Disabled,
}

/// Returns the impersonation mode of the command.
fn get_cmd_impersonation_mode(cmd: &ChatCommand) -> ImpersonationMode {
    match cmd {
        ChatCommand::Acknowledge { .. }
        | ChatCommand::Add { .. }
        | ChatCommand::Move { .. }
        | ChatCommand::Meta { .. }
        | ChatCommand::DocsUpdate
        | ChatCommand::PingGoals(_)
        | ChatCommand::Comments { .. }
        | ChatCommand::TeamStats { .. }
        | ChatCommand::Lookup(_) => ImpersonationMode::Disabled,
        ChatCommand::Whoami => ImpersonationMode::Silent,
        ChatCommand::Work(cmd) => match cmd {
            WorkqueueCmd::Show { .. } => ImpersonationMode::Silent,
            WorkqueueCmd::SetPrLimit { .. }
            | WorkqueueCmd::SetRotationMode { .. }
            | WorkqueueCmd::SetTeamRotationMode { .. } => ImpersonationMode::Notify,
        },
    }
}

/// Commands for working with the workqueue, e.g. showing how many PRs are assigned
/// or modifying the PR review assignment limit.
async fn workqueue_commands(
    ctx: &Context,
    gh_id: u64,
    cmd: &WorkqueueCmd,
) -> anyhow::Result<Option<String>> {
    let db_client = ctx.db.get().await;

    let gh_username =
        ctx.team.username_from_gh_id(gh_id).await?.ok_or_else(|| {
            anyhow::anyhow!("Cannot find your GitHub username in the team database")
        })?;
    let user = User {
        login: gh_username.clone(),
        id: gh_id,
    };
    let review_prefs = get_review_prefs(&db_client, gh_id)
        .await
        .context("Unable to retrieve your review preferences.")?;

    fn format_rotation_mode(mode: RotationMode) -> &'static str {
        match mode {
            RotationMode::OnRotation => "on rotation",
            RotationMode::OffRotation => "off rotation",
        }
    }

    let response = match cmd {
        WorkqueueCmd::Show { repo } => {
            let repo = normalize_repo(repo);
            // Currently hardcoded to rust-lang/rust workqueue
            let mut assigned_prs = get_assigned_prs(ctx, &repo, gh_id)
                .await
                .into_iter()
                .collect::<Vec<_>>();
            assigned_prs.sort_by_key(|(pr_number, _)| *pr_number);

            let capacity = match review_prefs
                .repo_review_prefs
                .get(&repo)
                .map(|p| p.max_assigned_prs)
                .unwrap_or_default()
            {
                Some(max) => max.to_string(),
                None => String::from("Not set (i.e. unlimited)"),
            };
            let rotation_mode = format_rotation_mode(review_prefs.rotation_mode);

            let mut response = if assigned_prs.is_empty() {
                format!("There are no PRs in your `{repo}` review queue\n")
            } else {
                let prs = assigned_prs
                    .iter()
                    .map(|(pr_number, pr)| {
                        format!(
                            "- [#{pr_number}](https://github.com/{repo}/pull/{pr_number}) {}",
                            pr.title
                        )
                    })
                    .format("\n");
                format!(
                    "`{repo}` PRs in your review queue ({} {}):\n{prs}\n\n",
                    assigned_prs.len(),
                    pluralize("PR", assigned_prs.len())
                )
            };

            writeln!(response, "Review capacity: `{capacity}`\n")?;
            writeln!(response, "Rotation mode: *{rotation_mode}*\n")?;
            for (team, team_prefs) in &review_prefs.team_review_prefs {
                writeln!(
                    response,
                    "Team `{team}` rotation mode: *{}*\n",
                    format_rotation_mode(team_prefs.rotation_mode)
                )?;
            }

            writeln!(
                response,
                "*Note that only certain PRs that are assigned to you are included in your review queue.*"
            )?;
            response
        }
        WorkqueueCmd::SetPrLimit { limit, repo } => {
            let repo = normalize_repo(repo);
            let max_assigned_prs = match limit {
                WorkqueueLimit::Unlimited => None,
                WorkqueueLimit::Limit(limit) => Some(*limit),
            };
            upsert_repo_review_prefs(&db_client, user, &repo, max_assigned_prs)
                .await
                .context("Error occurred while setting review preferences.")?;
            tracing::info!(
                "Setting max assigned PRs of `{gh_username}` in `{repo}` to {max_assigned_prs:?}"
            );
            format!(
                "Review capacity in `{repo}` set to {}",
                match max_assigned_prs {
                    Some(v) => v.to_string(),
                    None => "unlimited".to_string(),
                }
            )
        }
        WorkqueueCmd::SetRotationMode { rotation_mode } => {
            let rotation_mode = rotation_mode.0;
            upsert_user_review_prefs(&db_client, user, rotation_mode)
                .await
                .context("Error occurred while setting review preferences.")?;
            tracing::info!("Setting rotation mode `{gh_username}` to {rotation_mode:?}");
            format!(
                "Rotation mode set to *{}*.",
                format_rotation_mode(rotation_mode)
            )
        }
        WorkqueueCmd::SetTeamRotationMode {
            team,
            rotation_mode,
        } => {
            let teams = ctx.team.teams().await?;
            if teams.teams.get(team).is_none() {
                return Err(anyhow::anyhow!(
                    "Team `{team}` not found in the team database."
                ));
            }

            let rotation_mode = rotation_mode.0;
            upsert_team_review_prefs(&db_client, user, team, rotation_mode)
                .await
                .context("Error occurred while setting team review preferences.")?;
            tracing::info!(
                "Setting team rotation mode of `{gh_username}` for team `{team}` to {rotation_mode:?}"
            );
            let mut response = format!(
                "Rotation mode for team `{team}` set to *{}*.",
                format_rotation_mode(rotation_mode)
            );
            if rotation_mode == RotationMode::OnRotation
                && review_prefs.rotation_mode == RotationMode::OffRotation
            {
                writeln!(
                    response,
                    "\n\n**Warning**: your global rotation mode is still off, so no PRs will currently be assigned to you automatically.\nSend `work set-rotation-mode on` to resume the review rotation."
                )?;
            }

            response
        }
    };

    Ok(Some(response))
}

/// Add `rust-lang` prefix to repository name if it does not contain a slash.
fn normalize_repo(repo: &str) -> String {
    if repo.contains('/') {
        repo.to_owned()
    } else {
        format!("rust-lang/{repo}")
    }
}

/// The `whoami` command displays the user's membership in Rust teams.
async fn whoami_cmd(ctx: &Context, gh_id: u64) -> anyhow::Result<Option<String>> {
    let gh_username =
        ctx.team.username_from_gh_id(gh_id).await?.ok_or_else(|| {
            anyhow::anyhow!("Cannot find your GitHub username in the team database")
        })?;
    let teams = ctx
        .team
        .teams()
        .await
        .context("cannot load team information")?;
    let mut entries = teams
        .teams
        .iter()
        .flat_map(|(_, team)| {
            team.members
                .iter()
                .filter(|member| member.github_id == gh_id)
                .map(move |member| (team, member))
        })
        .map(|(team, member)| {
            let main_role = if member.is_lead { "lead" } else { "member" };
            let mut entry = format!(
                "**{}** ({}): {main_role}",
                team.name,
                match team.kind {
                    TeamKind::Team => "team",
                    TeamKind::WorkingGroup => "working group",
                    TeamKind::ProjectGroup => "project group",
                    TeamKind::MarkerTeam => "marker team",
                    TeamKind::Unknown => "unknown team kind",
                }
            );
            if !member.roles.is_empty() {
                write!(entry, " (roles: {})", member.roles.join(", ")).unwrap();
            }
            entry
        })
        .collect::<Vec<String>>();
    entries.sort();

    let mut output = format!("You are **{gh_username}**.");
    if entries.is_empty() {
        output.push_str(" You are not a member of any Rust team.");
    } else {
        writeln!(output, " You are a member of the following Rust teams:")?;
        for entry in entries {
            writeln!(output, "- {entry}")?;
        }
    }
    Ok(Some(output))
}

async fn lookup_cmd(ctx: &Context, cmd: &LookupCmd) -> anyhow::Result<Option<String>> {
    let username = match &cmd {
        LookupCmd::Zulip { github_username } => github_username.clone(),
        // Usernames could contain spaces, so rejoin everything to serve as the username.
        LookupCmd::GitHub { zulip_username } => zulip_username.join(" "),
    };

    // The username could be a mention, which looks like this: `@**<username>**`, so strip the
    // extra sigils.
    let username = username.trim_matches(['@', '*']);

    match cmd {
        LookupCmd::GitHub { .. } => Ok(Some(lookup_github_username(ctx, username).await?)),
        LookupCmd::Zulip { .. } => Ok(Some(lookup_zulip_username(ctx, username).await?)),
    }
}

/// Tries to find a GitHub username from a Zulip username.
async fn lookup_github_username(ctx: &Context, zulip_username: &str) -> anyhow::Result<String> {
    let username_lowercase = zulip_username.to_lowercase();

    let users = ctx
        .zulip
        .get_zulip_users()
        .await
        .context("Cannot get Zulip users")?;
    let Some(zulip_user) = users
        .iter()
        .find(|user| user.name.to_lowercase() == username_lowercase)
    else {
        return Ok(format!(
            "Zulip user {zulip_username} was not found on Zulip"
        ));
    };

    // Prefer what is configured on Zulip. If there is nothing, try to lookup the GitHub username
    // from the team database.
    let github_username = if let Some(name) = zulip_user.get_github_username() {
        name.to_string()
    } else {
        let zulip_id = zulip_user.user_id;
        let Some(gh_id) = ctx.team.zulip_to_github_id(zulip_id).await? else {
            return Ok(format!(
                "Zulip user {zulip_username} was not found in team Zulip mapping. Maybe they do not have zulip-id configured in team."
            ));
        };
        let Some(username) = ctx.team.username_from_gh_id(gh_id).await? else {
            return Ok(format!(
                "Zulip user {zulip_username} was not found in the team database."
            ));
        };
        username
    };

    Ok(format!(
        "{}'s GitHub profile is [{github_username}](https://github.com/{github_username}).",
        render_zulip_username(zulip_user.user_id)
    ))
}

pub fn render_zulip_username(zulip_id: u64) -> String {
    // Rendering the username directly was running into some encoding issues, so we use
    // the special `|<user-id>` syntax instead.
    // @**|<zulip-id>** is Zulip syntax that will render as the username (and a link) of the user
    // with the given Zulip ID.
    format!("@**|{zulip_id}**")
}

/// Tries to find a Zulip username from a GitHub username.
async fn lookup_zulip_username(ctx: &Context, gh_username: &str) -> anyhow::Result<String> {
    async fn lookup_zulip_id_from_zulip(
        zulip: &ZulipClient,
        gh_username: &str,
    ) -> anyhow::Result<Option<u64>> {
        let username_lowercase = gh_username.to_lowercase();
        let users = zulip.get_zulip_users().await?;
        Ok(users
            .into_iter()
            .find(|user| {
                user.get_github_username().map(str::to_lowercase).as_deref()
                    == Some(username_lowercase.as_str())
            })
            .map(|u| u.user_id))
    }

    async fn lookup_zulip_id_from_team(
        ctx: &Context,
        gh_username: &str,
    ) -> anyhow::Result<Option<u64>> {
        let people = ctx.team.people().await?.people;

        // Lookup the person in the team DB
        let Some(person) = people.get(gh_username).or_else(|| {
            let username_lowercase = gh_username.to_lowercase();
            people
                .keys()
                .find(|key| key.to_lowercase() == username_lowercase)
                .and_then(|key| people.get(key))
        }) else {
            return Ok(None);
        };

        let Some(zulip_id) = ctx.team.github_to_zulip_id(person.github_id).await? else {
            return Ok(None);
        };
        Ok(Some(zulip_id))
    }

    let zulip_id = match lookup_zulip_id_from_team(ctx, gh_username).await? {
        Some(id) => id,
        None => match lookup_zulip_id_from_zulip(&ctx.zulip, gh_username).await? {
            Some(id) => id,
            None => {
                return Ok(format!(
                    "No Zulip account found for GitHub username `{gh_username}`."
                ));
            }
        },
    };
    Ok(format!(
        "The GitHub user `{gh_username}` has the following Zulip account: {}",
        render_zulip_username(zulip_id)
    ))
}

#[derive(serde::Serialize)]
pub(crate) struct MessageApiRequest<'a> {
    pub(crate) recipient: Recipient<'a>,
    pub(crate) content: &'a str,
}

impl MessageApiRequest<'_> {
    pub fn url(&self, zulip: &ZulipClient) -> String {
        self.recipient.url(zulip)
    }

    pub(crate) async fn send(&self, client: &ZulipClient) -> anyhow::Result<MessageApiResponse> {
        client.send_message(self.recipient, self.content).await
    }
}

#[derive(Debug)]
pub struct UpdateMessageApiRequest<'a> {
    pub message_id: u64,
    pub topic: Option<&'a str>,
    pub propagate_mode: Option<&'a str>,
    pub content: Option<&'a str>,
}

impl UpdateMessageApiRequest<'_> {
    pub async fn send(&self, client: &ZulipClient) -> anyhow::Result<()> {
        client
            .update_message(
                self.message_id,
                self.topic,
                self.propagate_mode,
                self.content,
            )
            .await
    }
}

async fn acknowledge(
    ctx: &Context,
    gh_id: u64,
    ident: Identifier<'_>,
) -> anyhow::Result<Option<String>> {
    let mut db = ctx.db.get().await;
    let deleted = delete_ping(&mut db, gh_id, ident)
        .await
        .map_err(|e| format_err!("Failed to acknowledge {ident:?}: {e:?}."))?;

    let resp = if deleted.is_empty() {
        format!("No notifications matched `{ident:?}`, so none were deleted.")
    } else {
        let mut resp = String::from("Acknowledged:\n");
        for deleted in deleted {
            _ = writeln!(
                resp,
                " * [{}]({}){}",
                deleted
                    .short_description
                    .as_deref()
                    .unwrap_or(&deleted.origin_url),
                deleted.origin_url,
                deleted
                    .metadata
                    .map_or(String::new(), |m| format!(" ({m})")),
            );
        }
        resp
    };

    Ok(Some(resp))
}

async fn add_notification(
    ctx: &Context,
    gh_id: u64,
    url: &str,
    description: &str,
) -> anyhow::Result<Option<String>> {
    let description = description.trim();
    let description = if description.is_empty() {
        None
    } else {
        Some(description.to_string())
    };
    match record_ping(
        &*ctx.db.get().await,
        &notifications::Notification {
            user_id: gh_id,
            origin_url: url.to_owned(),
            origin_html: String::new(),
            short_description: description,
            time: chrono::Utc::now().into(),
            team_name: None,
        },
    )
    .await
    {
        Ok(()) => Ok(Some("Created!".to_string())),
        Err(e) => Err(format_err!("Failed to create: {e:?}")),
    }
}

async fn add_meta_notification(
    ctx: &Context,
    gh_id: u64,
    idx: u32,
    description: &str,
) -> anyhow::Result<Option<String>> {
    let idx = idx
        .checked_sub(1)
        .ok_or_else(|| anyhow::anyhow!("1-based indexes"))?;
    let description = if description.is_empty() {
        None
    } else {
        Some(description.to_string())
    };
    let mut db = ctx.db.get().await;
    match add_metadata(&mut db, gh_id, idx, description.as_deref()).await {
        Ok(()) => Ok(Some("Added metadata!".to_string())),
        Err(e) => Err(format_err!("Failed to add: {e:?}")),
    }
}

async fn move_notification(
    ctx: &Context,
    gh_id: u64,
    from: u32,
    to: u32,
) -> anyhow::Result<Option<String>> {
    let from = from
        .checked_sub(1)
        .ok_or_else(|| anyhow::anyhow!("1-based indexes"))?;
    let to = to
        .checked_sub(1)
        .ok_or_else(|| anyhow::anyhow!("1-based indexes"))?;
    match move_indices(&mut *ctx.db.get().await, gh_id, from, to).await {
        Ok(()) => {
            // to 1-base indices
            Ok(Some(format!("Moved {} to {}.", from + 1, to + 1)))
        }
        Err(e) => Err(format_err!("Failed to move: {e:?}.")),
    }
}

#[derive(serde::Serialize, Debug)]
struct ResponseNotRequired {
    response_not_required: bool,
}

#[derive(serde::Serialize, Debug, Copy, Clone)]
struct AddReaction<'a> {
    message_id: u64,
    emoji_name: &'a str,
}

impl AddReaction<'_> {
    pub async fn send(self, client: &ZulipClient) -> anyhow::Result<()> {
        client.add_reaction(self.message_id, self.emoji_name).await
    }
}

struct WaitingMessage<'a> {
    primary: &'a str,
    emoji: &'a [&'a str],
}

impl WaitingMessage<'static> {
    fn end_topic() -> Self {
        WaitingMessage {
            primary: "Does anyone have something to add on the current topic?\n\
                  React with :working_on_it: if you have something to say.\n\
                  React with :all_good: if not.",
            emoji: &["working_on_it", "all_good"],
        }
    }

    fn end_meeting() -> Self {
        WaitingMessage {
            primary: "Does anyone have something to bring up?\n\
                  React with :working_on_it: if you have something to say.\n\
                  React with :all_good: if you're ready to end the meeting.",
            emoji: &["working_on_it", "all_good"],
        }
    }
    fn start_reading() -> Self {
        WaitingMessage {
            primary: "Click on the :book: when you start reading (and leave it clicked).\n\
                      Click on the :checkered_flag: when you finish reading.",
            emoji: &["book", "checkered_flag"],
        }
    }
}

async fn post_waiter(
    ctx: &Context,
    message: &Message,
    waiting: WaitingMessage<'_>,
) -> anyhow::Result<Option<String>> {
    let posted = MessageApiRequest {
        recipient: Recipient::Stream {
            id: message
                .stream_id
                .ok_or_else(|| format_err!("private waiting not supported, missing stream id"))?,
            topic: message
                .subject
                .as_deref()
                .ok_or_else(|| format_err!("private waiting not supported, missing topic"))?,
        },
        content: waiting.primary,
    }
    .send(&ctx.zulip)
    .await?;

    for reaction in waiting.emoji {
        AddReaction {
            message_id: posted.message_id,
            emoji_name: reaction,
        }
        .send(&ctx.zulip)
        .await
        .context("emoji reaction failed")?;
    }

    Ok(None)
}

fn trigger_docs_update(message: &Message, zulip: &ZulipClient) -> anyhow::Result<Option<String>> {
    let message = message.clone();
    // The default Zulip timeout of 10 seconds can be too short, so process in
    // the background.
    let zulip = zulip.clone();
    tokio::task::spawn(async move {
        let response = match docs_update().await {
            Ok(None) => "No updates found.".to_string(),
            Ok(Some(pr)) => format!("Created docs update PR <{}>", pr.html_url),
            Err(e) => {
                // Don't send errors to Zulip since they may contain sensitive data.
                log::error!("Docs update via Zulip failed: {e:?}");
                "Docs update failed, please check the logs for more details.".to_string()
            }
        };
        let recipient = message.sender_to_recipient();
        let message = MessageApiRequest {
            recipient,
            content: &response,
        };
        if let Err(e) = message.send(&zulip).await {
            log::error!("failed to send Zulip response: {e:?}\nresponse was:\n{response}");
        }
    });
    Ok(Some(
        "Docs update in progress, I'll let you know when I'm finished.".to_string(),
    ))
}

/// Formats user's GitHub comment for display in the Zulip message.
pub fn format_user_comment(comment: &UserComment) -> String {
    // Limit the size of the comment to avoid running into Zulip max message size limits
    let snippet = truncate_text(&comment.body, 300);
    let date = comment
        .created_at
        .map(|dt| dt.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| "unknown date".to_string());

    format!(
        "- [{title}]({comment_url}) ({date}):\n  > {snippet}\n",
        title = truncate_text(&comment.issue_title, 60),
        comment_url = comment.comment_url,
    )
}

/// Truncates the given text to the specified length, adding ellipsis if needed.
fn truncate_text(text: &str, max_len: usize) -> String {
    let normalized: String = text.split_whitespace().collect::<Vec<_>>().join(" ");

    if normalized.len() <= max_len {
        normalized
    } else {
        let truncated: String = normalized.chars().take(max_len - 3).collect();
        format!("{truncated}...")
    }
}
