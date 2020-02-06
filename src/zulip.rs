use crate::db::notifications::add_metadata;
use crate::db::notifications::{self, delete_ping, move_indices, record_ping, Identifier};
use crate::github::{self, GithubClient};
use crate::handlers::Context;
use anyhow::Context as _;
use std::convert::TryInto;

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
    sender_full_name: String,
}

#[derive(serde::Serialize)]
struct Response<'a> {
    content: &'a str,
}

pub async fn to_github_id(client: &GithubClient, zulip_id: usize) -> anyhow::Result<Option<i64>> {
    let url = format!("{}/zulip-map.json", rust_team_data::v1::BASE_URL);
    let map: rust_team_data::v1::ZulipMapping = client
        .json(client.raw().get(&url))
        .await
        .context("could not get team data")?;
    Ok(map.users.get(&zulip_id).map(|v| *v as i64))
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
        Ok(Some(gh_id)) => gh_id,
        Ok(None) => {
            return serde_json::to_string(&Response {
                content: &format!(
                "Unknown Zulip user. Please add `zulip-id = {}` to your file in rust-lang/team.",
                req.message.sender_id),
            })
            .unwrap();
        }
        Err(e) => {
            return serde_json::to_string(&Response {
                content: &format!("Failed to query team API: {:?}", e),
            })
            .unwrap();
        }
    };

    handle_command(ctx, gh_id, &req.data).await
}

fn handle_command<'a>(
    ctx: &'a Context,
    gh_id: i64,
    words: &'a str,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = String> + Send + 'a>> {
    Box::pin(async move {
        let mut words = words.split_whitespace();
        match words.next() {
            Some("as") => match execute_for_other_user(&ctx, gh_id, words).await {
                Ok(r) => r,
                Err(e) => serde_json::to_string(&Response {
                    content: &format!(
                        "Failed to parse; expected `as <username> <command...>`: {:?}.",
                        e
                    ),
                })
                .unwrap(),
            },
            Some("acknowledge") => match acknowledge(&ctx, gh_id, words).await {
                Ok(r) => r,
                Err(e) => serde_json::to_string(&Response {
                    content: &format!(
                        "Failed to parse acknowledgement, expected `acknowledge <identifier>`: {:?}.",
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
            _ => serde_json::to_string(&Response {
                content: "Unknown command.",
            })
            .unwrap(),
        }
    })
}

async fn execute_for_other_user(
    ctx: &Context,
    _original_id: i64,
    mut words: impl Iterator<Item = &str>,
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
    Ok(handle_command(ctx, user_id, &command).await)
}

async fn acknowledge(
    ctx: &Context,
    gh_id: i64,
    mut words: impl Iterator<Item = &str>,
) -> anyhow::Result<String> {
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
    match delete_ping(
        &mut Context::make_db_client(&ctx.github.raw()).await?,
        gh_id,
        ident,
    )
    .await
    {
        Ok(()) => Ok(serde_json::to_string(&Response {
            content: &format!("Acknowledged {}.", url),
        })
        .unwrap()),
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
    match add_metadata(
        &mut Context::make_db_client(&ctx.github.raw()).await?,
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
    match move_indices(
        &mut Context::make_db_client(&ctx.github.raw()).await?,
        gh_id,
        from,
        to,
    )
    .await
    {
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
