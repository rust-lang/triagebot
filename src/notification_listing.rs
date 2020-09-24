use crate::db::notifications::get_notifications;
use crate::db::DbClient;

pub async fn render(db: &DbClient, user: &str) -> String {
    let notifications = match get_notifications(db, user).await {
        Ok(n) => n,
        Err(e) => {
            return format!("{:?}", e.context("getting notifications"));
        }
    };

    let mut out = String::new();
    out.push_str("<html>");
    out.push_str("<head>");
    out.push_str("<meta charset=\"utf-8\">");
    out.push_str("<title>Triagebot Notification Data</title>");
    out.push_str("</head>");
    out.push_str("<body>");

    out.push_str(&format!("<h3>Pending notifications for {}</h3>", user));

    if notifications.is_empty() {
        out.push_str("<p><em>You have no pending notifications! :)</em></p>");
    } else {
        out.push_str("<ol>");
        for notification in notifications {
            out.push_str("<li>");
            out.push_str(&format!(
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
            ));
            if let Some(metadata) = &notification.metadata {
                out.push_str(&format!(
                    "<ul><li>{}</li></ul>",
                    metadata
                        .replace('&', "&amp;")
                        .replace('<', "&lt;")
                        .replace('>', "&gt;")
                        .replace('"', "&quot;")
                        .replace('\'', "&#39;"),
                ));
            }
            out.push_str("</li>");
        }
        out.push_str("</ol>");

        out.push_str(
            "<p><em>You can acknowledge notifications by sending <code>ack &lt;idx&gt;</code> \
            to <b>@triagebot</b> on Zulip. Read about the other notification commands \
            <a href=\"https://forge.rust-lang.org/platforms/zulip/triagebot.html#issue-notifications\">here</a>.</em></p>"
        );
    }

    out.push_str("</body>");
    out.push_str("</html>");

    out
}
