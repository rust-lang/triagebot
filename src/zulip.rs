use crate::db::notifications::add_metadata;
use crate::db::notifications::{self, delete_ping, move_indices, record_ping, Identifier};
use crate::github::{get_id_for_username, GithubClient};
use crate::handlers::docs_update::docs_update;
use crate::handlers::pull_requests_assignment_update::get_review_prefs;
use crate::handlers::Context;
use anyhow::{format_err, Context as _};
use std::env;
use std::fmt::Write as _;
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

pub const BOT_EMAIL: &str = "triage-rust-lang-bot@zulipchat.com";

pub async fn to_github_id(client: &GithubClient, zulip_id: u64) -> anyhow::Result<Option<u64>> {
    let map = crate::team_data::zulip_map(client).await?;
    Ok(map.users.get(&zulip_id).copied())
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

/// Processes a Zulip webhook.
///
/// Returns a string of the response, or None if no response is needed.
async fn process_zulip_request(ctx: &Context, req: Request) -> anyhow::Result<Option<String>> {
    let expected_token = std::env::var("ZULIP_TOKEN").expect("`ZULIP_TOKEN` set for authorization");

    if !openssl::memcmp::eq(req.token.as_bytes(), expected_token.as_bytes()) {
        anyhow::bail!("Invalid authorization.");
    }

    log::trace!("zulip hook: {:?}", req);
    let gh_id = match to_github_id(&ctx.github, req.message.sender_id).await {
        Ok(Some(gh_id)) => Ok(gh_id),
        Ok(None) => Err(format_err!(
            "Unknown Zulip user. Please add `zulip-id = {}` to your file in \
                [rust-lang/team](https://github.com/rust-lang/team).",
            req.message.sender_id
        )),
        Err(e) => anyhow::bail!("Failed to query team API: {e:?}"),
    };

    handle_command(ctx, gh_id, &req.data, &req.message).await
}

fn handle_command<'a>(
    ctx: &'a Context,
    gh_id: anyhow::Result<u64>,
    words: &'a str,
    message_data: &'a Message,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<Option<String>>> + Send + 'a>>
{
    Box::pin(async move {
        log::trace!("handling zulip command {:?}", words);
        let mut words = words.split_whitespace();
        let mut next = words.next();

        if let Some("as") = next {
            return execute_for_other_user(&ctx, words, message_data)
                .await
                .map_err(|e| {
                    format_err!("Failed to parse; expected `as <username> <command...>`: {e:?}.")
                });
        }
        let gh_id = gh_id?;

        match next {
            Some("acknowledge") | Some("ack") => acknowledge(&ctx, gh_id, words).await
                .map_err(|e| format_err!("Failed to parse acknowledgement, expected `(acknowledge|ack) <identifier>`: {e:?}.")),
            Some("add") => add_notification(&ctx, gh_id, words).await
                .map_err(|e| format_err!("Failed to parse description addition, expected `add <url> <description (multiple words)>`: {e:?}.")),
            Some("move") => move_notification(&ctx, gh_id, words).await
                .map_err(|e| format_err!("Failed to parse movement, expected `move <from> <to>`: {e:?}.")),
            Some("meta") => add_meta_notification(&ctx, gh_id, words).await
                .map_err(|e| format_err!("Failed to parse `meta` command. Synopsis: meta <num> <text>: Add <text> to your notification identified by <num> (>0)\n\nError: {e:?}")),
            Some("work") => query_pr_assignments(&ctx, gh_id, words).await
                                                                    .map_err(|e| format_err!("Failed to parse `work` command. Synopsis: work <show>: shows your current PRs assignment\n\nError: {e:?}")),
            _ => {
                while let Some(word) = next {
                    if word == "@**triagebot**" {
                        let next = words.next();
                        match next {
                            Some("end-topic") | Some("await") => {
                                return post_waiter(&ctx, message_data, WaitingMessage::end_topic())
                                    .await
                                    .map_err(|e| {
                                        format_err!("Failed to await at this time: {e:?}")
                                    })
                            }
                            Some("end-meeting") => {
                                return post_waiter(
                                    &ctx,
                                    message_data,
                                    WaitingMessage::end_meeting(),
                                )
                                .await
                                .map_err(|e| format_err!("Failed to await at this time: {e:?}"))
                            }
                            Some("read") => {
                                return post_waiter(
                                    &ctx,
                                    message_data,
                                    WaitingMessage::start_reading(),
                                )
                                .await
                                .map_err(|e| format_err!("Failed to await at this time: {e:?}"))
                            }
                            Some("docs-update") => return trigger_docs_update(message_data),
                            _ => {}
                        }
                    }
                    next = words.next();
                }

                Ok(Some(String::from("Unknown command")))
            }
        }
    })
}

async fn query_pr_assignments(
    ctx: &&Context,
    gh_id: u64,
    mut words: impl Iterator<Item = &str>,
) -> anyhow::Result<Option<String>> {
    let subcommand = match words.next() {
        Some(subcommand) => subcommand,
        None => anyhow::bail!("no subcommand provided"),
    };

    let db_client = ctx.db.get().await;

    let record = match subcommand {
        "show" => {
            let rec = get_review_prefs(&db_client, gh_id).await;
            if rec.is_err() {
                anyhow::bail!("No preferences set.")
            }
            rec?
        }
        _ => anyhow::bail!("Invalid subcommand."),
    };

    Ok(Some(record.to_string()))
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
    let bot_api_token = env::var("ZULIP_API_TOKEN").expect("ZULIP_API_TOKEN");

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

    // Map GitHub `user_id` to `zulip_user_id`.
    let zulip_user_id = match to_zulip_id(&ctx.github, user_id).await {
        Ok(Some(id)) => id as u64,
        Ok(None) => anyhow::bail!("Could not find Zulip ID for GitHub ID: {user_id}"),
        Err(e) => anyhow::bail!("Could not find Zulip ID for GitHub id {user_id}: {e:?}"),
    };

    let user = members
        .members
        .iter()
        .find(|m| m.user_id == zulip_user_id)
        .ok_or_else(|| format_err!("Could not find Zulip user email."))?;

    let output = handle_command(ctx, Ok(user_id), &command, message_data)
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
    .send(ctx.github.raw())
    .await;

    match res {
        Ok(resp) => {
            if !resp.status().is_success() {
                log::error!(
                    "Failed to notify real user about command: response: {:?}",
                    resp
                );
            }
        }
        Err(err) => {
            log::error!("Failed to notify real user about command: {:?}", err);
        }
    }

    Ok(Some(output))
}

#[derive(serde::Deserialize)]
pub struct MembersApiResponse {
    pub members: Vec<Member>,
}

#[derive(serde::Deserialize)]
pub struct Member {
    pub email: String,
    pub user_id: u64,
}

#[derive(Copy, Clone, serde::Serialize)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum Recipient<'a> {
    Stream {
        #[serde(rename = "to")]
        id: u64,
        topic: &'a str,
    },
    Private {
        #[serde(skip)]
        id: u64,
        #[serde(rename = "to")]
        email: &'a str,
    },
}

impl Recipient<'_> {
    pub fn narrow(&self) -> String {
        match self {
            Recipient::Stream { id, topic } => {
                // See
                // https://github.com/zulip/zulip/blob/46247623fc279/zerver/lib/url_encoding.py#L9
                // ALWAYS_SAFE without `.` from
                // https://github.com/python/cpython/blob/113e2b0a07c/Lib/urllib/parse.py#L772-L775
                //
                // ALWAYS_SAFE doesn't contain `.` because Zulip actually encodes them to be able
                // to use `.` instead of `%` in the encoded strings
                const ALWAYS_SAFE: &str =
                    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-~";

                let mut encoded_topic = String::new();
                for ch in topic.bytes() {
                    if !(ALWAYS_SAFE.contains(ch as char)) {
                        write!(encoded_topic, ".{:02X}", ch).unwrap();
                    } else {
                        encoded_topic.push(ch as char);
                    }
                }
                format!("stream/{}-xxx/topic/{}", id, encoded_topic)
            }
            Recipient::Private { id, .. } => format!("pm-with/{}-xxx", id),
        }
    }

    pub fn url(&self) -> String {
        format!("https://rust-lang.zulipchat.com/#narrow/{}", self.narrow())
    }
}

#[cfg(test)]
fn check_encode(topic: &str, expected: &str) {
    const PREFIX: &str = "stream/0-xxx/topic/";
    let computed = Recipient::Stream { id: 0, topic }.narrow();
    assert_eq!(&computed[..PREFIX.len()], PREFIX);
    assert_eq!(&computed[PREFIX.len()..], expected);
}

#[test]
fn test_encode() {
    check_encode("some text with spaces", "some.20text.20with.20spaces");
    check_encode(
        " !\"#$%&'()*+,-./",
        ".20.21.22.23.24.25.26.27.28.29.2A.2B.2C-.2E.2F",
    );
    check_encode("0123456789:;<=>?", "0123456789.3A.3B.3C.3D.3E.3F");
    check_encode(
        "@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_",
        ".40ABCDEFGHIJKLMNOPQRSTUVWXYZ.5B.5C.5D.5E_",
    );
    check_encode(
        "`abcdefghijklmnopqrstuvwxyz{|}~",
        ".60abcdefghijklmnopqrstuvwxyz.7B.7C.7D~.7F",
    );
    check_encode("áé…", ".C3.A1.C3.A9.E2.80.A6");
}

#[derive(serde::Serialize)]
pub struct MessageApiRequest<'a> {
    pub recipient: Recipient<'a>,
    pub content: &'a str,
}

impl<'a> MessageApiRequest<'a> {
    pub fn url(&self) -> String {
        self.recipient.url()
    }

    pub async fn send(&self, client: &reqwest::Client) -> anyhow::Result<reqwest::Response> {
        let bot_api_token = env::var("ZULIP_API_TOKEN").expect("ZULIP_API_TOKEN");

        #[derive(serde::Serialize)]
        struct SerializedApi<'a> {
            #[serde(rename = "type")]
            type_: &'static str,
            to: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            topic: Option<&'a str>,
            content: &'a str,
        }

        Ok(client
            .post("https://rust-lang.zulipchat.com/api/v1/messages")
            .basic_auth(BOT_EMAIL, Some(&bot_api_token))
            .form(&SerializedApi {
                type_: match self.recipient {
                    Recipient::Stream { .. } => "stream",
                    Recipient::Private { .. } => "private",
                },
                to: match self.recipient {
                    Recipient::Stream { id, .. } => id.to_string(),
                    Recipient::Private { email, .. } => email.to_string(),
                },
                topic: match self.recipient {
                    Recipient::Stream { topic, .. } => Some(topic),
                    Recipient::Private { .. } => None,
                },
                content: self.content,
            })
            .send()
            .await?)
    }
}

#[derive(serde::Deserialize)]
pub struct MessageApiResponse {
    #[serde(rename = "id")]
    pub message_id: u64,
}

#[derive(Debug)]
pub struct UpdateMessageApiRequest<'a> {
    pub message_id: u64,
    pub topic: Option<&'a str>,
    pub propagate_mode: Option<&'a str>,
    pub content: Option<&'a str>,
}

impl<'a> UpdateMessageApiRequest<'a> {
    pub async fn send(&self, client: &reqwest::Client) -> anyhow::Result<reqwest::Response> {
        let bot_api_token = env::var("ZULIP_API_TOKEN").expect("ZULIP_API_TOKEN");

        #[derive(serde::Serialize)]
        struct SerializedApi<'a> {
            #[serde(skip_serializing_if = "Option::is_none")]
            pub topic: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            pub propagate_mode: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            pub content: Option<&'a str>,
        }

        Ok(client
            .patch(&format!(
                "https://rust-lang.zulipchat.com/api/v1/messages/{}",
                self.message_id
            ))
            .basic_auth(BOT_EMAIL, Some(&bot_api_token))
            .form(&SerializedApi {
                topic: self.topic,
                propagate_mode: self.propagate_mode,
                content: self.content,
            })
            .send()
            .await?)
    }
}

async fn acknowledge(
    ctx: &Context,
    gh_id: u64,
    mut words: impl Iterator<Item = &str>,
) -> anyhow::Result<Option<String>> {
    let filter = match words.next() {
        Some(filter) => {
            if words.next().is_some() {
                anyhow::bail!("too many words");
            }
            filter
        }
        None => anyhow::bail!("not enough words"),
    };
    let ident = if let Ok(number) = filter.parse::<u32>() {
        Identifier::Index(
            std::num::NonZeroU32::new(number)
                .ok_or_else(|| anyhow::anyhow!("index must be at least 1"))?,
        )
    } else if filter == "all" || filter == "*" {
        Identifier::All
    } else {
        Identifier::Url(filter)
    };
    let mut db = ctx.db.get().await;
    let deleted = delete_ping(&mut *db, gh_id, ident)
        .await
        .map_err(|e| format_err!("Failed to acknowledge {filter}: {e:?}."))?;

    let resp = if deleted.is_empty() {
        format!(
            "No notifications matched `{}`, so none were deleted.",
            filter
        )
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
    mut words: impl Iterator<Item = &str>,
) -> anyhow::Result<Option<String>> {
    let url = match words.next() {
        Some(idx) => idx,
        None => anyhow::bail!("url not present"),
    };
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

#[derive(serde::Deserialize, Debug)]
struct SentMessage {
    id: u64,
}

#[derive(serde::Serialize, Debug, Copy, Clone)]
struct AddReaction<'a> {
    message_id: u64,
    emoji_name: &'a str,
}

impl<'a> AddReaction<'a> {
    pub async fn send(self, client: &reqwest::Client) -> anyhow::Result<reqwest::Response> {
        let bot_api_token = env::var("ZULIP_API_TOKEN").expect("ZULIP_API_TOKEN");

        Ok(client
            .post(&format!(
                "https://rust-lang.zulipchat.com/api/v1/messages/{}/reactions",
                self.message_id
            ))
            .basic_auth(BOT_EMAIL, Some(&bot_api_token))
            .form(&self)
            .send()
            .await?)
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
    .send(ctx.github.raw())
    .await?;
    let body = posted.text().await?;
    let message_id = serde_json::from_str::<SentMessage>(&body)
        .with_context(|| format!("{:?} did not deserialize as SentMessage", body))?
        .id;

    for reaction in waiting.emoji {
        AddReaction {
            message_id,
            emoji_name: reaction,
        }
        .send(&ctx.github.raw())
        .await
        .context("emoji reaction failed")?;
    }

    Ok(None)
}

fn trigger_docs_update(message: &Message) -> anyhow::Result<Option<String>> {
    let message = message.clone();
    // The default Zulip timeout of 10 seconds can be too short, so process in
    // the background.
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
        if let Err(e) = message.send(&reqwest::Client::new()).await {
            log::error!("failed to send Zulip response: {e:?}\nresponse was:\n{response}");
        }
    });
    Ok(Some(
        "Docs update in progress, I'll let you know when I'm finished.".to_string(),
    ))
}
