use crate::db::notifications::{Identifier, Notification};
use crate::github::{self, GithubClient};
use crate::handlers::Context;
use anyhow::Context as _;
use std::convert::TryInto;
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

#[derive(Debug, serde::Deserialize)]
struct Message {
    sender_id: u64,
    #[allow(unused)]
    recipient_id: u64,
    sender_short_name: Option<String>,
    sender_full_name: String,
    stream_id: Option<u64>,
    // The topic of the incoming message. Not the stream name.
    subject: Option<String>,
    #[allow(unused)]
    #[serde(rename = "type")]
    type_: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Response<'a> {
    content: &'a str,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ResponseOwned {
    content: String,
}

pub const BOT_EMAIL: &str = "triage-rust-lang-bot@zulipchat.com";

pub async fn to_github_id(client: &GithubClient, zulip_id: usize) -> anyhow::Result<Option<i64>> {
    let map = crate::team_data::zulip_map(client).await?;
    Ok(map.users.get(&zulip_id).map(|v| *v as i64))
}

pub async fn to_zulip_id(client: &GithubClient, github_id: i64) -> anyhow::Result<Option<usize>> {
    let map = crate::team_data::zulip_map(client).await?;
    Ok(map
        .users
        .iter()
        .find(|(_, github)| **github == github_id as usize)
        .map(|v| *v.0))
}

pub async fn respond(ctx: &Context, req: Request) -> String {
    let expected_token = std::env::var("ZULIP_TOKEN").expect("`ZULIP_TOKEN` set for authorization");

    if !openssl::memcmp::eq(req.token.as_bytes(), expected_token.as_bytes()) {
        return serde_json::to_string(&Response {
            content: "Invalid authorization.",
        })
        .unwrap();
    }

    log::trace!("zulip hook: {:?}", req);
    let gh_id = match to_github_id(&ctx.github, req.message.sender_id as usize).await {
        Ok(Some(gh_id)) => Ok(gh_id),
        Ok(None) => Err(serde_json::to_string(&Response {
            content: &format!(
                "Unknown Zulip user. Please add `zulip-id = {}` to your file in \
                [rust-lang/team](https://github.com/rust-lang/team).",
                req.message.sender_id
            ),
        })
        .unwrap()),
        Err(e) => {
            return serde_json::to_string(&Response {
                content: &format!("Failed to query team API: {:?}", e),
            })
            .unwrap();
        }
    };

    handle_command(ctx, gh_id, &req.data, &req.message).await
}

fn handle_command<'a>(
    ctx: &'a Context,
    gh_id: Result<i64, String>,
    words: &'a str,
    message_data: &'a Message,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = String> + Send + 'a>> {
    Box::pin(async move {
        log::trace!("handling zulip command {:?}", words);
        let mut words = words.split_whitespace();
        let mut next = words.next();

        if let Some("as") = next {
            return match execute_for_other_user(&ctx, words, message_data).await {
                Ok(r) => r,
                Err(e) => serde_json::to_string(&Response {
                    content: &format!(
                        "Failed to parse; expected `as <username> <command...>`: {:?}.",
                        e
                    ),
                })
                .unwrap(),
            };
        }
        let gh_id = match gh_id {
            Ok(id) => id,
            Err(e) => return e,
        };

        match next {
            Some("acknowledge") | Some("ack") => match acknowledge(&ctx, gh_id, words).await {
                Ok(r) => r,
                Err(e) => serde_json::to_string(&Response {
                    content: &format!(
                        "Failed to parse acknowledgement, expected `(acknowledge|ack) <identifier>`: {:?}.",
                        e
                    ),
                })
                .unwrap(),
            },
            Some("add") => match add_notification(&ctx, gh_id, words).await {
                Ok(r) => r,
                Err(e) => serde_json::to_string(&Response {
                    content: &format!(
                        "Failed to parse description addition, expected `add <url> <description (multiple words)>`: {:?}.",
                        e
                    ),
                })
                .unwrap(),
            },
            Some("move") => match move_notification(&ctx, gh_id, words).await {
                Ok(r) => r,
                Err(e) => serde_json::to_string(&Response {
                    content: &format!(
                        "Failed to parse movement, expected `move <from> <to>`: {:?}.",
                        e
                    ),
                })
                .unwrap(),
            },
            Some("meta") => match add_meta_notification(&ctx, gh_id, words).await {
                Ok(r) => r,
                Err(e) => serde_json::to_string(&Response {
                    content: &format!(
                        "Failed to parse movement, expected `move <idx> <meta...>`: {:?}.",
                        e
                    ),
                })
                .unwrap(),
            },
            _ => {
                while let Some(word) = next {
                    if word == "@**triagebot**" {
                        let next = words.next();
                        match next {
                            Some("end-topic") | Some("await") => return match post_waiter(&ctx, message_data, WaitingMessage::end_topic()).await {
                                Ok(r) => r,
                                Err(e) => serde_json::to_string(&Response {
                                    content: &format!("Failed to await at this time: {:?}", e),
                                })
                                .unwrap(),
                            },
                            Some("end-meeting") => return match post_waiter(&ctx, message_data, WaitingMessage::end_meeting()).await {
                                Ok(r) => r,
                                Err(e) => serde_json::to_string(&Response {
                                    content: &format!("Failed to await at this time: {:?}", e),
                                })
                                .unwrap(),
                            },
                            Some("read") => return match post_waiter(&ctx, message_data, WaitingMessage::start_reading()).await {
                                Ok(r) => r,
                                Err(e) => serde_json::to_string(&Response {
                                    content: &format!("Failed to await at this time: {:?}", e),
                                })
                                .unwrap(),
                            },
                            _ => {}
                        }
                    }
                    next = words.next();
                }

                serde_json::to_string(&Response {
                    content: "Unknown command.",
                })
                .unwrap()
            },
        }
    })
}

// This does two things:
//  * execute the command for the other user
//  * tell the user executed for that a command was run as them by the user
//    given.
async fn execute_for_other_user(
    ctx: &Context,
    mut words: impl Iterator<Item = &str>,
    message_data: &Message,
) -> anyhow::Result<String> {
    // username is a GitHub username, not a Zulip username
    let username = match words.next() {
        Some(username) => username,
        None => anyhow::bail!("no username provided"),
    };
    let user_id = match (github::User {
        login: username.to_owned(),
        id: None,
    })
    .get_id(&ctx.github)
    .await
    .context("getting ID of github user")?
    {
        Some(id) => id.try_into().unwrap(),
        None => {
            return Ok(serde_json::to_string(&Response {
                content: "Can only authorize for other GitHub users.",
            })
            .unwrap());
        }
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
        .await;
    let members = match members {
        Ok(members) => members,
        Err(e) => {
            return Ok(serde_json::to_string(&Response {
                content: &format!("Failed to get list of zulip users: {:?}.", e),
            })
            .unwrap());
        }
    };
    let members = members.json::<MembersApiResponse>().await;
    let members = match members {
        Ok(members) => members.members,
        Err(e) => {
            return Ok(serde_json::to_string(&Response {
                content: &format!("Failed to get list of zulip users: {:?}.", e),
            })
            .unwrap());
        }
    };

    // Map GitHub `user_id` to `zulip_user_id`.
    let zulip_user_id = match to_zulip_id(&ctx.github, user_id).await {
        Ok(Some(id)) => id as u64,
        Ok(None) => {
            return Ok(serde_json::to_string(&Response {
                content: &format!("Could not find Zulip ID for GitHub ID: {}", user_id),
            })
            .unwrap());
        }
        Err(e) => {
            return Ok(serde_json::to_string(&Response {
                content: &format!("Could not find Zulip ID for GitHub id {}: {:?}", user_id, e),
            })
            .unwrap());
        }
    };

    let user = match members.iter().find(|m| m.user_id == zulip_user_id) {
        Some(m) => m,
        None => {
            return Ok(serde_json::to_string(&Response {
                content: &format!("Could not find Zulip user email."),
            })
            .unwrap());
        }
    };

    let output = handle_command(ctx, Ok(user_id as i64), &command, message_data).await;
    let output_msg: ResponseOwned =
        serde_json::from_str(&output).expect("result should always be JSON");
    let output_msg = output_msg.content;

    // At this point, the command has been run (FIXME: though it may have
    // errored, it's hard to determine that currently, so we'll just give the
    // output from the command as well as the command itself).

    let sender = match &message_data.sender_short_name {
        Some(short_name) => format!("{} ({})", message_data.sender_full_name, short_name),
        None => message_data.sender_full_name.clone(),
    };
    let message = format!(
        "{} ran `{}` with output `{}` as you.",
        sender, command, output_msg
    );

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

    Ok(output)
}

#[derive(serde::Deserialize)]
struct MembersApiResponse {
    members: Vec<Member>,
}

#[derive(serde::Deserialize)]
struct Member {
    email: String,
    user_id: u64,
}

#[derive(serde::Serialize)]
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
    gh_id: i64,
    mut words: impl Iterator<Item = &str>,
) -> anyhow::Result<String> {
    let filter = match words.next() {
        Some(filter) => {
            if words.next().is_some() {
                anyhow::bail!("too many words");
            }
            filter
        }
        None => anyhow::bail!("not enough words"),
    };
    let ident = if let Ok(number) = filter.parse::<usize>() {
        Identifier::Index(
            std::num::NonZeroUsize::new(number)
                .ok_or_else(|| anyhow::anyhow!("index must be at least 1"))?,
        )
    } else if filter == "all" || filter == "*" {
        Identifier::All
    } else {
        Identifier::Url(filter)
    };
    let mut connection = ctx.db.connection().await;
    match connection.delete_ping(gh_id, ident).await {
        Ok(deleted) => {
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

            Ok(serde_json::to_string(&Response { content: &resp }).unwrap())
        }
        Err(e) => Ok(serde_json::to_string(&Response {
            content: &format!("Failed to acknowledge {}: {:?}.", filter, e),
        })
        .unwrap()),
    }
}

async fn add_notification(
    ctx: &Context,
    gh_id: i64,
    mut words: impl Iterator<Item = &str>,
) -> anyhow::Result<String> {
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
    let mut connection = ctx.db.connection().await;
    match connection
        .record_ping(&Notification {
            user_id: gh_id,
            origin_url: url.to_owned(),
            origin_html: String::new(),
            short_description: description,
            time: chrono::Utc::now().into(),
            team_name: None,
        })
        .await
    {
        Ok(()) => Ok(serde_json::to_string(&Response {
            content: "Created!",
        })
        .unwrap()),
        Err(e) => Ok(serde_json::to_string(&Response {
            content: &format!("Failed to create: {:?}", e),
        })
        .unwrap()),
    }
}

async fn add_meta_notification(
    ctx: &Context,
    gh_id: i64,
    mut words: impl Iterator<Item = &str>,
) -> anyhow::Result<String> {
    let idx = match words.next() {
        Some(idx) => idx,
        None => anyhow::bail!("idx not present"),
    };
    let idx = idx
        .parse::<usize>()
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
    let mut connection = ctx.db.connection().await;
    match connection
        .add_metadata(gh_id, idx, description.as_deref())
        .await
    {
        Ok(()) => Ok(serde_json::to_string(&Response {
            content: "Added metadata!",
        })
        .unwrap()),
        Err(e) => Ok(serde_json::to_string(&Response {
            content: &format!("Failed to add: {:?}", e),
        })
        .unwrap()),
    }
}

async fn move_notification(
    ctx: &Context,
    gh_id: i64,
    mut words: impl Iterator<Item = &str>,
) -> anyhow::Result<String> {
    let from = match words.next() {
        Some(idx) => idx,
        None => anyhow::bail!("from idx not present"),
    };
    let to = match words.next() {
        Some(idx) => idx,
        None => anyhow::bail!("from idx not present"),
    };
    let from = from
        .parse::<usize>()
        .context("from index")?
        .checked_sub(1)
        .ok_or_else(|| anyhow::anyhow!("1-based indexes"))?;
    let to = to
        .parse::<usize>()
        .context("to index")?
        .checked_sub(1)
        .ok_or_else(|| anyhow::anyhow!("1-based indexes"))?;
    let mut connection = ctx.db.connection().await;
    match connection.move_indices(gh_id, from, to).await {
        Ok(()) => Ok(serde_json::to_string(&Response {
            // to 1-base indices
            content: &format!("Moved {} to {}.", from + 1, to + 1),
        })
        .unwrap()),
        Err(e) => Ok(serde_json::to_string(&Response {
            content: &format!("Failed to move: {:?}.", e),
        })
        .unwrap()),
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
) -> anyhow::Result<String> {
    let posted = MessageApiRequest {
        recipient: Recipient::Stream {
            id: message.stream_id.ok_or_else(|| {
                anyhow::format_err!("private waiting not supported, missing stream id")
            })?,
            topic: message.subject.as_deref().ok_or_else(|| {
                anyhow::format_err!("private waiting not supported, missing topic")
            })?,
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

    Ok(serde_json::to_string(&ResponseNotRequired {
        response_not_required: true,
    })
    .unwrap())
}
