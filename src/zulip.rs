pub mod api;
pub mod client;
mod commands;

use crate::db::notifications::add_metadata;
use crate::db::notifications::{self, delete_ping, move_indices, record_ping, Identifier};
use crate::db::review_prefs::{get_review_prefs, upsert_review_prefs, RotationMode};
use crate::github::{get_id_for_username, GithubClient, User};
use crate::handlers::docs_update::docs_update;
use crate::handlers::pr_tracking::get_assigned_prs;
use crate::handlers::project_goals::{self, ping_project_goals_owners};
use crate::handlers::Context;
use crate::team_data::{people, teams};
use crate::utils::pluralize;
use crate::zulip::api::{MessageApiResponse, Recipient};
use crate::zulip::client::ZulipClient;
use crate::zulip::commands::{ChatCommand, LookupCmd, WorkqueueCmd, WorkqueueLimit};
use anyhow::{format_err, Context as _};
use clap::Parser;
use postgres_types::ToSql;
use rust_team_data::v1::TeamKind;
use std::fmt::Write as _;
use std::str::FromStr;
use subtle::ConstantTimeEq;
use tracing as log;

#[derive(Debug, serde::Deserialize)]
pub struct Request {
    /// Markdown body of the sent message.
    data: String,

    /// Metadata about this request.
    message: Message,

    /// Authentication token. The same for all Zulip messages.
    token: String,
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

pub async fn to_github_id(client: &GithubClient, zulip_id: u64) -> anyhow::Result<Option<u64>> {
    let map = crate::team_data::zulip_map(client).await?;
    Ok(map.users.get(&zulip_id).copied())
}

pub async fn username_from_gh_id(
    client: &GithubClient,
    gh_id: u64,
) -> anyhow::Result<Option<String>> {
    let people_map = crate::team_data::people(client).await?;
    Ok(people_map
        .people
        .into_iter()
        .filter(|(_, p)| p.github_id == gh_id)
        .map(|p| p.0)
        .next())
}

pub async fn to_zulip_id(client: &GithubClient, github_id: u64) -> anyhow::Result<Option<u64>> {
    let map = crate::team_data::zulip_map(client).await?;
    Ok(map
        .users
        .iter()
        .find(|&(_, &github)| github == github_id)
        .map(|v| *v.0))
}

/// Top-level handler for Zulip webhooks.
///
/// Returns a JSON response.
pub async fn respond(ctx: &Context, req: Request) -> String {
    let content = match process_zulip_request(ctx, req).await {
        Ok(None) => {
            return serde_json::to_string(&ResponseNotRequired {
                response_not_required: true,
            })
            .unwrap();
        }
        Ok(Some(s)) => s,
        Err(e) => format!("{:?}", e),
    };
    serde_json::to_string(&Response { content }).unwrap()
}

pub fn get_token_from_env() -> Result<String, anyhow::Error> {
    // ZULIP_WEBHOOK_SECRET is preferred, ZULIP_TOKEN is kept for retrocompatibility but will be deprecated
    match std::env::var("ZULIP_WEBHOOK_SECRET") {
        Ok(v) => return Ok(v),
        Err(_) => (),
    }

    match std::env::var("ZULIP_TOKEN") {
        Ok(v) => return Ok(v),
        Err(_) => (),
    }

    log::error!(
        "Cannot communicate with Zulip: neither ZULIP_WEBHOOK_SECRET or ZULIP_TOKEN are set."
    );
    anyhow::bail!("Cannot communicate with Zulip.");
}

/// Processes a Zulip webhook.
///
/// Returns a string of the response, or None if no response is needed.
async fn process_zulip_request(ctx: &Context, req: Request) -> anyhow::Result<Option<String>> {
    let expected_token = get_token_from_env()?;
    if !bool::from(req.token.as_bytes().ct_eq(expected_token.as_bytes())) {
        anyhow::bail!("Invalid authorization.");
    }

    log::trace!("zulip hook: {req:?}");

    // Zulip commands are only available to users in the team database
    let gh_id = match to_github_id(&ctx.github, req.message.sender_id).await {
        Ok(Some(gh_id)) => gh_id,
        Ok(None) => {
            return Err(anyhow::anyhow!(
                "Unknown Zulip user. Please add `zulip-id = {}` to your file in \
                [rust-lang/team](https://github.com/rust-lang/team).",
                req.message.sender_id
            ))
        }
        Err(e) => anyhow::bail!("Failed to query team API: {e:?}"),
    };

    handle_command(ctx, gh_id, &req.data, &req.message).await
}

fn handle_command<'a>(
    ctx: &'a Context,
    gh_id: u64,
    command: &'a str,
    message_data: &'a Message,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<Option<String>>> + Send + 'a>>
{
    Box::pin(async move {
        log::trace!("handling zulip command {:?}", command);
        let mut words = command.split_whitespace().peekable();
        let mut next = words.peek();

        if let Some(&"as") = next {
            words.next(); // skip `as`
            return execute_for_other_user(&ctx, words, message_data)
                .await
                .map_err(|e| {
                    format_err!("Failed to parse; expected `as <username> <command...>`: {e:?}.")
                });
        }

        // Missing stream means that this is a direct message
        if message_data.stream_id.is_none() {
            let cmd = ChatCommand::try_parse_from(words)?;
            match cmd {
                ChatCommand::Acknowledge { identifier } => {
                    acknowledge(&ctx, gh_id, identifier.into()).await
                }
                ChatCommand::Add { url, description } => {
                    add_notification(&ctx, gh_id, &url, &description.join(" ")).await
                }
                ChatCommand::Whoami => whoami_cmd(&ctx, gh_id).await,
                ChatCommand::Lookup(cmd) => lookup_cmd(&ctx, cmd).await,
                ChatCommand::Work(cmd) => workqueue_commands(ctx, gh_id, cmd).await,
            }
        } else {
            todo!()
        }

        // match next {
        //     Some("acknowledge") | Some("ack") => acknowledge(&ctx, gh_id, words).await
        //         .map_err(|e| format_err!("Failed to parse acknowledgement, expected `(acknowledge|ack) <identifier>`: {e:?}.")),
        //     Some("add") => add_notification(&ctx, gh_id, words).await
        //         .map_err(|e| format_err!("Failed to parse description addition, expected `add <url> <description (multiple words)>`: {e:?}.")),
        //     Some("move") => move_notification(&ctx, gh_id, words).await
        //         .map_err(|e| format_err!("Failed to parse movement, expected `move <from> <to>`: {e:?}.")),
        //     Some("meta") => add_meta_notification(&ctx, gh_id, words).await
        //         .map_err(|e| format_err!("Failed to parse `meta` command. Synopsis: meta <num> <text>: Add <text> to your notification identified by <num> (>0)\n\nError: {e:?}")),
        //     Some("whoami") => whoami_cmd(&ctx, gh_id, words).await
        //         .map_err(|e| format_err!("Failed to run the `whoami` command. Synopsis: whoami: Show to which Rust teams you are a part of\n\nError: {e:?}")),
        //     Some("lookup") => lookup_cmd(&ctx, words).await
        //         .map_err(|e| format_err!("Failed to run the `lookup` command. Synopsis: lookup (github <zulip-username>|zulip <github-username>): Show the GitHub username of a Zulip <user> or the Zulip username of a GitHub user\n\nError: {e:?}")),
        //     Some("work") => workqueue_commands(ctx, gh_id, words).await
        //                                                             .map_err(|e| format_err!("Failed to parse `work` command. Help: {WORKQUEUE_HELP}\n\nError: {e:?}")),
        //     _ => {
        //         while let Some(word) = next {
        //             if word == "@**triagebot**" {
        //                 let next = words.next();
        //                 match next {
        //                     Some("end-topic") | Some("await") => {
        //                         return post_waiter(&ctx, message_data, WaitingMessage::end_topic())
        //                             .await
        //                             .map_err(|e| {
        //                                 format_err!("Failed to await at this time: {e:?}")
        //                             })
        //                     }
        //                     Some("end-meeting") => {
        //                         return post_waiter(
        //                             &ctx,
        //                             message_data,
        //                             WaitingMessage::end_meeting(),
        //                         )
        //                         .await
        //                         .map_err(|e| format_err!("Failed to await at this time: {e:?}"))
        //                     }
        //                     Some("read") => {
        //                         return post_waiter(
        //                             &ctx,
        //                             message_data,
        //                             WaitingMessage::start_reading(),
        //                         )
        //                         .await
        //                         .map_err(|e| format_err!("Failed to await at this time: {e:?}"))
        //                     }
        //                     Some("ping-goals") => {
        //                         let usage_err = |description: &str| Err(format_err!(
        //                             "Error: {description}\n\
        //                             \n\
        //                             Usage: triagebot ping-goals D N, where:\n\
        //                             \n\
        //                              * D is the number of days before an update is considered stale\n\
        //                              * N is the date of next update, like \"Sep-5\"\n",
        //                         ));
        //
        //                         let Some(threshold) = words.next() else {
        //                             return usage_err("expected number of days");
        //                         };
        //                         let threshold = match i64::from_str(threshold) {
        //                             Ok(v) => v,
        //                             Err(e) => return usage_err(&format!("ill-formed number of days, {e}")),
        //                         };
        //
        //                         let Some(next_update) = words.next() else {
        //                             return usage_err("expected date of next update");
        //                         };
        //
        //                         if project_goals::check_project_goal_acl(&ctx.github, gh_id).await? {
        //                             ping_project_goals_owners(&ctx.github, &ctx.zulip, false, threshold, &format!("on {next_update}"))
        //                                 .await
        //                                 .map_err(|e| format_err!("Failed to await at this time: {e:?}"))?;
        //                             return Ok(None);
        //                         } else {
        //                             return Err(format_err!(
        //                                 "That command is only permitted for those running the project-goal program.",
        //                             ));
        //                         }
        //                     }
        //                     Some("docs-update") => return trigger_docs_update(message_data, &ctx.zulip),
        //                     _ => {}
        //                 }
        //             }
        //             next = words.next();
        //         }
        //
        //         Ok(Some(String::from("Unknown command")))
        //     }
        // }
    })
}

/// Commands for working with the workqueue, e.g. showing how many PRs are assigned
/// or modifying the PR review assignment limit.
async fn workqueue_commands(
    ctx: &Context,
    gh_id: u64,
    cmd: WorkqueueCmd,
) -> anyhow::Result<Option<String>> {
    let db_client = ctx.db.get().await;

    let gh_username = username_from_gh_id(&ctx.github, gh_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Cannot find your GitHub username in the team database"))?;
    let user = User {
        login: gh_username.clone(),
        id: gh_id,
    };
    let review_prefs = get_review_prefs(&db_client, gh_id)
        .await
        .context("Unable to retrieve your review preferences.")?;

    let response = match cmd {
        WorkqueueCmd::Show => {
            let mut assigned_prs = get_assigned_prs(ctx, gh_id)
                .await
                .into_iter()
                .collect::<Vec<_>>();
            assigned_prs.sort();

            let prs = assigned_prs
                .iter()
                .map(|pr| format!("#{pr}"))
                .collect::<Vec<String>>()
                .join(", ");

            let review_prefs = get_review_prefs(&db_client, gh_id)
                .await
                .context("cannot get review preferences")?;
            let capacity = match review_prefs.as_ref().and_then(|p| p.max_assigned_prs) {
                Some(max) => max.to_string(),
                None => String::from("Not set (i.e. unlimited)"),
            };
            let rotation_mode = review_prefs
                .as_ref()
                .map(|p| p.rotation_mode)
                .unwrap_or_default();
            let rotation_mode = match rotation_mode {
                RotationMode::OnRotation => "on rotation",
                RotationMode::OffRotation => "off rotation",
            };

            let mut response = format!(
                "`rust-lang/rust` PRs in your review queue: {prs} ({} {})\n",
                assigned_prs.len(),
                pluralize("PR", assigned_prs.len())
            );
            writeln!(response, "Review capacity: {capacity}\n")?;
            writeln!(response, "Rotation mode: *{rotation_mode}*\n")?;
            writeln!(response, "*Note that only certain PRs that are assigned to you are included in your review queue.*")?;
            response
        }
        WorkqueueCmd::SetPrLimit { limit } => {
            let max_assigned_prs = match limit {
                WorkqueueLimit::Unlimited => None,
                WorkqueueLimit::Limit(limit) => Some(limit),
            };
            upsert_review_prefs(
                &db_client,
                user,
                max_assigned_prs,
                review_prefs.map(|p| p.rotation_mode).unwrap_or_default(),
            )
            .await
            .context("Error occurred while setting review preferences.")?;
            tracing::info!("Setting max assignment PRs of `{gh_username}` to {max_assigned_prs:?}");
            format!(
                "Review capacity set to {}",
                match max_assigned_prs {
                    Some(v) => v.to_string(),
                    None => "unlimited".to_string(),
                }
            )
        }
        WorkqueueCmd::SetRotationMode { rotation_mode } => {
            let rotation_mode = rotation_mode.0;
            upsert_review_prefs(
                &db_client,
                user,
                review_prefs.and_then(|p| p.max_assigned_prs.map(|v| v as u32)),
                rotation_mode,
            )
            .await
            .context("Error occurred while setting review preferences.")?;
            tracing::info!("Setting rotation mode `{gh_username}` to {rotation_mode:?}");
            format!(
                "Rotation mode set to {}",
                match rotation_mode {
                    RotationMode::OnRotation => "*on rotation*",
                    RotationMode::OffRotation => "*off rotation*.",
                }
            )
        }
    };

    Ok(Some(response))
}

/// The `whoami` command displays the user's membership in Rust teams.
async fn whoami_cmd(ctx: &Context, gh_id: u64) -> anyhow::Result<Option<String>> {
    let gh_username = username_from_gh_id(&ctx.github, gh_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Cannot find your GitHub username in the team database"))?;
    let teams = teams(&ctx.github)
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

async fn lookup_cmd(ctx: &Context, cmd: LookupCmd) -> anyhow::Result<Option<String>> {
    let username = match &cmd {
        LookupCmd::Zulip { github_username } => github_username.clone(),
        // Usernames could contain spaces, so rejoin everything to serve as the username.
        LookupCmd::GitHub { zulip_username } => zulip_username.join(" "),
    };

    // The username could be a mention, which looks like this: `@**<username>**`, so strip the
    // extra sigils.
    let username = username.trim_matches(&['@', '*']);

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
    let github_username = match zulip_user.get_github_username() {
        Some(name) => name.to_string(),
        None => {
            let zulip_id = zulip_user.user_id;
            let Some(gh_id) = to_github_id(&ctx.github, zulip_id).await? else {
                return Ok(format!("Zulip user {zulip_username} was not found in team Zulip mapping. Maybe they do not have zulip-id configured in team."));
            };
            let Some(username) = username_from_gh_id(&ctx.github, gh_id).await? else {
                return Ok(format!(
                    "Zulip user {zulip_username} was not found in the team database."
                ));
            };
            username
        }
    };

    Ok(format!(
        "{}'s GitHub profile is [{github_username}](https://github.com/{github_username}).",
        render_zulip_username(zulip_user.user_id)
    ))
}

fn render_zulip_username(zulip_id: u64) -> String {
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
                user.get_github_username()
                    .map(|u| u.to_lowercase())
                    .as_deref()
                    == Some(username_lowercase.as_str())
            })
            .map(|u| u.user_id))
    }

    async fn lookup_zulip_id_from_team(
        ctx: &Context,
        gh_username: &str,
    ) -> anyhow::Result<Option<u64>> {
        let people = people(&ctx.github).await?.people;

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

        let Some(zulip_id) = to_zulip_id(&ctx.github, person.github_id).await? else {
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
                ))
            }
        },
    };
    Ok(format!(
        "The GitHub user `{gh_username}` has the following Zulip account: {}",
        render_zulip_username(zulip_id)
    ))
}

// This does two things:
//  * execute the command for the other user
//  * tell the user executed for that a command was run as them by the user
//    given.
async fn execute_for_other_user(
    ctx: &Context,
    mut words: impl Iterator<Item = &str>,
    message_data: &Message,
) -> anyhow::Result<Option<String>> {
    // username is a GitHub username, not a Zulip username
    let username = match words.next() {
        Some(username) => username,
        None => anyhow::bail!("no username provided"),
    };
    let user_id = match get_id_for_username(&ctx.github, username)
        .await
        .context("getting ID of github user")?
    {
        Some(id) => id.try_into().unwrap(),
        None => anyhow::bail!("Can only authorize for other GitHub users."),
    };
    let mut command = words.fold(String::new(), |mut acc, piece| {
        acc.push_str(piece);
        acc.push(' ');
        acc
    });
    let command = if command.is_empty() {
        anyhow::bail!("no command provided")
    } else {
        assert_eq!(command.pop(), Some(' ')); // pop trailing space
        command
    };

    let members = ctx
        .zulip
        .get_zulip_users()
        .await
        .map_err(|e| format_err!("Failed to get list of zulip users: {e:?}."))?;

    // Map GitHub `user_id` to `zulip_user_id`.
    let zulip_user_id = match to_zulip_id(&ctx.github, user_id).await {
        Ok(Some(id)) => id as u64,
        Ok(None) => anyhow::bail!("Could not find Zulip ID for GitHub ID: {user_id}"),
        Err(e) => anyhow::bail!("Could not find Zulip ID for GitHub id {user_id}: {e:?}"),
    };

    let user = members
        .iter()
        .find(|m| m.user_id == zulip_user_id)
        .ok_or_else(|| format_err!("Could not find Zulip user email."))?;

    let output = handle_command(ctx, user_id, &command, message_data)
        .await?
        .unwrap_or_default();

    // At this point, the command has been run.
    let sender = &message_data.sender_full_name;
    let message = format!("{sender} ran `{command}` with output `{output}` as you.");

    let res = MessageApiRequest {
        recipient: Recipient::Private {
            id: user.user_id,
            email: &user.email,
        },
        content: &message,
    }
    .send(&ctx.zulip)
    .await;

    if let Err(err) = res {
        log::error!("Failed to notify real user about command: {:?}", err);
    }

    Ok(Some(output))
}

#[derive(serde::Serialize)]
pub(crate) struct MessageApiRequest<'a> {
    pub(crate) recipient: Recipient<'a>,
    pub(crate) content: &'a str,
}

impl<'a> MessageApiRequest<'a> {
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

impl<'a> UpdateMessageApiRequest<'a> {
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
    ident: Identifier,
) -> anyhow::Result<Option<String>> {
    let mut db = ctx.db.get().await;
    let deleted = delete_ping(&mut *db, gh_id, ident)
        .await
        .map_err(|e| format_err!("Failed to acknowledge {filter}: {e:?}."))?;

    let resp = if deleted.is_empty() {
        format!("No notifications matched `{ident:?}`, so none were deleted.")
    } else {
        let mut resp = String::from("Acknowledged:\n");
        for deleted in deleted {
            resp.push_str(&format!(
                " * [{}]({}){}\n",
                deleted
                    .short_description
                    .as_deref()
                    .unwrap_or(&deleted.origin_url),
                deleted.origin_url,
                deleted
                    .metadata
                    .map_or(String::new(), |m| format!(" ({})", m)),
            ));
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
    mut words: impl Iterator<Item = &str>,
) -> anyhow::Result<Option<String>> {
    let idx = match words.next() {
        Some(idx) => idx,
        None => anyhow::bail!("idx not present"),
    };
    let idx = idx
        .parse::<u32>()
        .context("index")?
        .checked_sub(1)
        .ok_or_else(|| anyhow::anyhow!("1-based indexes"))?;
    let mut description = words.fold(String::new(), |mut acc, piece| {
        acc.push_str(piece);
        acc.push(' ');
        acc
    });
    let description = if description.is_empty() {
        None
    } else {
        assert_eq!(description.pop(), Some(' ')); // pop trailing space
        Some(description)
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
    mut words: impl Iterator<Item = &str>,
) -> anyhow::Result<Option<String>> {
    let from = match words.next() {
        Some(idx) => idx,
        None => anyhow::bail!("from idx not present"),
    };
    let to = match words.next() {
        Some(idx) => idx,
        None => anyhow::bail!("from idx not present"),
    };
    let from = from
        .parse::<u32>()
        .context("from index")?
        .checked_sub(1)
        .ok_or_else(|| anyhow::anyhow!("1-based indexes"))?;
    let to = to
        .parse::<u32>()
        .context("to index")?
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

impl<'a> AddReaction<'a> {
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
