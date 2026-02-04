//! Handler that reports GitHub bans and unbans to a Zulip channel.

use crate::github::{OrgBlockAction, OrgBlockEvent};
use crate::handlers::Context;
use crate::zulip::MessageApiRequest;
use crate::zulip::api::Recipient;
use tracing as log;

// The #mods channel
const REPORT_STREAM_URL: u64 = 464799;

pub async fn handle(ctx: &Context, event: &OrgBlockEvent) -> anyhow::Result<()> {
    let topic = format!("github user {}", event.blocked_user.login);

    let action_text = match event.action {
        OrgBlockAction::Blocked => "banned",
        OrgBlockAction::Unblocked => "unbanned",
    };

    let message = format!(
        "User `{blocked_user}` was {action} from the `{org}` organization by `{sender}`.\n\n\
        [View user profile](https://github.com/{blocked_user})",
        blocked_user = event.blocked_user.login,
        action = action_text,
        org = event.organization.login,
        sender = event.sender.login,
    );

    let recipient = Recipient::Stream {
        id: REPORT_STREAM_URL,
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
