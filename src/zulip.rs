use crate::db::notifications::add_metadata;
use crate::db::notifications::{self, delete_ping, move_indices, record_ping, Identifier};
use crate::github::{self, GithubClient};
use crate::handlers::Context;
use anyhow::Context as _;
use std::convert::TryInto;
use std::env;
use std::io::Write as _;

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
    sender_id: usize,
    sender_short_name: String,
    sender_full_name: String,
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
    let gh_id = match to_github_id(&ctx.github, req.message.sender_id).await {
        Ok(Some(gh_id)) => Ok(gh_id),
        Ok(None) => Err(serde_json::to_string(&Response {
            content: &format!(
                "Unknown Zulip user. Please add `zulip-id = {}` to your file in rust-lang/team.",
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
        let mut words = words.split_whitespace();
        let next = words.next();

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
            Some("acknowledge") | Some("ack") => match acknowledge(gh_id, words).await {
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
            Some("move") => match move_notification(gh_id, words).await {
                Ok(r) => r,
                Err(e) => serde_json::to_string(&Response {
                    content: &format!(
                        "Failed to parse movement, expected `move <from> <to>`: {:?}.",
                        e
                    ),
                })
                .unwrap(),
            },
            Some("meta") => match add_meta_notification(gh_id, words).await {
                Ok(r) => r,
                Err(e) => serde_json::to_string(&Response {
                    content: &format!(
                        "Failed to parse movement, expected `move <idx> <meta...>`: {:?}.",
                        e
                    ),
                })
                .unwrap(),
            },
            _ => serde_json::to_string(&Response {
                content: "Unknown command.",
            })
            .unwrap(),
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
        Ok(Some(id)) => id as i64,
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

    let user_email = match members.iter().find(|m| m.user_id == zulip_user_id) {
        Some(m) => &m.email,
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
    // output fromt he command as well as the command itself).

    let message = format!(
        "{} ({}) ran `{}` with output `{}` as you.",
        message_data.sender_full_name, message_data.sender_short_name, command, output_msg
    );

    let res = MessageApiRequest {
        type_: "private",
        to: &user_email,
        topic: None,
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
    user_id: i64,
}

#[derive(serde::Serialize)]
pub struct MessageApiRequest<'a> {
    #[serde(rename = "type")]
    pub type_: &'a str,
    pub to: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<&'a str>,
    pub content: &'a str,
}

impl MessageApiRequest<'_> {
    // FIXME: support private links too
    pub fn url(&self) -> String {
        // See
        // https://github.com/zulip/zulip/blob/46247623fc279/zerver/lib/url_encoding.py#L9
        // ALWAYS_SAFE from
        // https://github.com/python/cpython/blob/113e2b0a07c/Lib/urllib/parse.py#L772-L775
        let always_safe = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_.-~";
        let mut encoded_topic = Vec::new();
        for ch in self.topic.expect("topic").bytes() {
            if !(always_safe.contains(ch as char) || ch == b'.') {
                write!(encoded_topic, "%{:02X}", ch).unwrap();
            } else {
                encoded_topic.push(ch);
            }
        }
        let mut encoded_topic = String::from_utf8(encoded_topic).unwrap();
        encoded_topic = encoded_topic.replace("%", ".");

        format!(
            "https://rust-lang.zulipchat.com/#narrow/stream/{}-xxx/topic/{}",
            self.to, encoded_topic,
        )
    }

    pub async fn send(&self, client: &reqwest::Client) -> anyhow::Result<reqwest::Response> {
        if self.type_ == "stream" {
            assert!(self.topic.is_some());
        }
        let bot_api_token = env::var("ZULIP_API_TOKEN").expect("ZULIP_API_TOKEN");

        Ok(client
            .post("https://rust-lang.zulipchat.com/api/v1/messages")
            .basic_auth(BOT_EMAIL, Some(&bot_api_token))
            .form(&self)
            .send()
            .await?)
    }
}

async fn acknowledge(gh_id: i64, mut words: impl Iterator<Item = &str>) -> anyhow::Result<String> {
    let url = match words.next() {
        Some(url) => {
            if words.next().is_some() {
                anyhow::bail!("too many words");
            }
            url
        }
        None => anyhow::bail!("not enough words"),
    };
    let ident = if let Ok(number) = url.parse::<usize>() {
        Identifier::Index(
            std::num::NonZeroUsize::new(number)
                .ok_or_else(|| anyhow::anyhow!("index must be at least 1"))?,
        )
    } else {
        Identifier::Url(url)
    };
    match delete_ping(&mut crate::db::make_client().await?, gh_id, ident).await {
        Ok(deleted) => {
            let mut resp = format!("Acknowledged:\n");
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
            Ok(serde_json::to_string(&Response { content: &resp }).unwrap())
        }
        Err(e) => Ok(serde_json::to_string(&Response {
            content: &format!("Failed to acknowledge {}: {:?}.", url, e),
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
    match record_ping(
        &ctx.db,
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
    match add_metadata(
        &mut crate::db::make_client().await?,
        gh_id,
        idx,
        description.as_deref(),
    )
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
    match move_indices(&mut crate::db::make_client().await?, gh_id, from, to).await {
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
