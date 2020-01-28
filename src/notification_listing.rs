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
    out.push_str("<title>Triagebot Notification Data</title>");
    out.push_str("</head>");
    out.push_str("<body>");

    out.push_str(&format!("<h3>Pending notifications for {}</h3>", user));

    out.push_str("<ul>");
    for notification in notifications {
        out.push_str(&format!(
            "<li><a href='{}'>{}</a></li>",
            notification.origin_url, notification.origin_url,
        ));
    }
    out.push_str("</ul>");

    out.push_str("</body>");
    out.push_str("</html>");

    out
}
