//! Handler that reports GitHub bans and unbans to a Zulip channel.

use crate::github::{OrgBlockAction, OrgBlockEvent, UserComment};
use crate::handlers::Context;
use crate::zulip::MessageApiRequest;
use crate::zulip::api::Recipient;
use tracing as log;

// The #mods channel on Zulip
const ZULIP_STREAM_ID: u64 = 464799;

/// Maximum number of recent comments to include in ban reports.
const MAX_RECENT_COMMENTS: usize = 10;

/// Maximum length of a comment snippet in the report.
const MAX_COMMENT_SNIPPET_LEN: usize = 200;

pub async fn handle(ctx: &Context, event: &OrgBlockEvent) -> anyhow::Result<()> {
    let topic = format!("github user {}", event.blocked_user.login);

    let action_text = match event.action {
        OrgBlockAction::Blocked => "banned",
        OrgBlockAction::Unblocked => "unbanned",
    };

    let mut message = format!(
        "User `{blocked_user}` was {action} from the `{org}` organization by `{sender}`.\n\n\
        [View user profile](https://github.com/{blocked_user})",
        blocked_user = event.blocked_user.login,
        action = action_text,
        org = event.organization.login,
        sender = event.sender.login,
    );

    // For bans, fetch and include the user's recent comments
    if event.action == OrgBlockAction::Blocked {
        let username = &event.blocked_user.login;
        let org = &event.organization.login;
        match ctx
            .github
            .user_comments_in_org(username, org, MAX_RECENT_COMMENTS)
            .await
        {
            Ok(comments) if !comments.is_empty() => {
                message.push_str("\n\n**Recent comments:**\n");
                for comment in comments {
                    message.push_str(&format_user_comment(&comment));
                }
            }
            Ok(_) => {
                message.push_str("\n\n*No recent comments found in this organization.*");
            }
            Err(err) => {
                log::warn!(
                    "Failed to fetch recent comments for {}: {err:?}",
                    event.blocked_user.login
                );
                message.push_str("\n\n*Could not fetch recent comments.*");
            }
        }
    }

    let recipient = Recipient::Stream {
        id: ZULIP_STREAM_ID,
        topic: &topic,
    };

    let req = MessageApiRequest {
        recipient,
        content: &message,
    }
    .send(&ctx.zulip)
    .await;

    if let Err(err) = req {
        log::error!("Failed to send user block notification to Zulip: {err}");
        return Err(err);
    }

    log::info!(
        "Posted user block notification: {} was {action_text} from {}",
        event.blocked_user.login,
        event.organization.login
    );

    Ok(())
}

/// Formats user's comment for display in the Zulip message.
fn format_user_comment(comment: &UserComment) -> String {
    let snippet = truncate_comment(&comment.body, MAX_COMMENT_SNIPPET_LEN);
    let date = comment
        .created_at
        .map(|dt| dt.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| "unknown date".to_string());

    format!(
        "- [{title}]({comment_url}) ({date}):\n  > {snippet}\n",
        title = truncate_comment(&comment.issue_title, 60),
        comment_url = comment.comment_url,
    )
}

/// Truncates a comment to the specified length, adding ellipsis if needed.
fn truncate_comment(text: &str, max_len: usize) -> String {
    let normalized: String = text.split_whitespace().collect::<Vec<_>>().join(" ");

    if normalized.len() <= max_len {
        normalized
    } else {
        let truncated: String = normalized.chars().take(max_len - 3).collect();
        format!("{truncated}...")
    }
}
