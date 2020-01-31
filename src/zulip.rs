use crate::db::notifications::delete_ping;
use crate::handlers::Context;

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

// Zulip User ID to GH User ID
//
// FIXME: replace with https://github.com/rust-lang/team/pull/222 once it lands
static MAPPING: &[(usize, i64)] = &[(116122, 5047365), (119235, 1940490)];

pub async fn respond(ctx: &Context, req: Request) -> String {
    let expected_token = std::env::var("ZULIP_TOKEN").expect("`ZULIP_TOKEN` set for authorization");

    if !openssl::memcmp::eq(req.token.as_bytes(), expected_token.as_bytes()) {
        return serde_json::to_string(&Response {
            content: "Invalid authorization.",
        })
        .unwrap();
    }

    log::trace!("zulip hook: {:?}", req);
    let gh_id = match MAPPING
        .iter()
        .find(|(zulip, _)| *zulip == req.message.sender_id)
    {
        Some((_, gh_id)) => *gh_id,
        None => {
            return serde_json::to_string(&Response {
                content: &format!(
                "Unknown Zulip user. Please add `zulip-id = {}` to your file in rust-lang/team.",
                req.message.sender_id),
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
