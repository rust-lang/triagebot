use crate::db::notifications::delete_ping;
use crate::github::GithubClient;
use crate::handlers::Context;
use anyhow::Context as _;

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

    match two_words(&req.data) {
        Some(["acknowledge", url]) => match delete_ping(&ctx.db, gh_id, url).await {
            Ok(()) => serde_json::to_string(&Response {
                content: &format!("Acknowledged {}.", url),
            })
            .unwrap(),
            Err(e) => serde_json::to_string(&Response {
                content: &format!("Failed to acknowledge {}: {:?}.", url, e),
            })
            .unwrap(),
        },
        _ => serde_json::to_string(&Response {
            content: "Unknown command.",
        })
        .unwrap(),
    }
}

fn two_words(s: &str) -> Option<[&str; 2]> {
    let mut iter = s.split_whitespace();
    let first = iter.next()?;
    let second = iter.next()?;
    if iter.next().is_some() {
        return None;
    }

    return Some([first, second]);
}
