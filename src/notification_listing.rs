use std::fmt::Write;
use std::sync::Arc;

use anyhow::Context as _;
use axum::{
    extract::{Query, State},
    response::{Html, IntoResponse, Response},
};
use hyper::StatusCode;
use serde::Deserialize;

use crate::{db::notifications::get_notifications, errors::AppError, handlers::Context};

#[derive(Deserialize)]
pub struct NotificationsQuery {
    user: Option<String>,
}

pub async fn notifications(
    Query(query): Query<NotificationsQuery>,
    State(ctx): State<Arc<Context>>,
) -> axum::response::Result<Response, AppError> {
    let Some(user) = query.user else {
        return Ok((
            StatusCode::BAD_REQUEST,
            "Please provide `?user=<username>` query param on URL.",
        )
            .into_response());
    };

    let notifications = get_notifications(&*ctx.db.get().await, &user)
        .await
        .context("getting notifications")?;

    let mut out = String::new();
    out.push_str("<html>");
    out.push_str("<head>");
    out.push_str("<meta charset=\"utf-8\">");
    out.push_str("<title>Triagebot Notification Data</title>");
    out.push_str("</head>");
    out.push_str("<body>");

    _ = write!(out, "<h3>Pending notifications for {user}</h3>");

    if notifications.is_empty() {
        out.push_str("<p><em>You have no pending notifications! :)</em></p>");
    } else {
        out.push_str("<ol>");
        for notification in notifications {
            out.push_str("<li>");
            _ = write!(
                out,
                "<a href='{}'>{}</a>",
                notification.origin_url,
                notification
                    .short_description
                    .as_ref()
                    .unwrap_or(&notification.origin_url)
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;")
                    .replace('"', "&quot;")
                    .replace('\'', "&#39;"),
            );
            if let Some(metadata) = &notification.metadata {
                _ = write!(
                    out,
                    "<ul><li>{}</li></ul>",
                    metadata
                        .replace('&', "&amp;")
                        .replace('<', "&lt;")
                        .replace('>', "&gt;")
                        .replace('"', "&quot;")
                        .replace('\'', "&#39;"),
                );
            }
            out.push_str("</li>");
        }
        out.push_str("</ol>");

        out.push_str(
            "<p><em>You can acknowledge a notification by sending </em><code>ack &lt;idx&gt;</code><em> \
            to </em><strong><code>@triagebot</code></strong><em> on Zulip, or you can acknowledge \
            all notifications by sending </em><code>ack all</code><em>. Read about the other notification commands \
            <a href=\"https://forge.rust-lang.org/platforms/zulip/triagebot.html#issue-notifications\">here</a>.</em></p>"
        );
    }

    out.push_str("</body>");
    out.push_str("</html>");

    Ok(Html(out).into_response())
}
