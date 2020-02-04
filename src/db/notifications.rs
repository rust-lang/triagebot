use anyhow::Context as _;
use chrono::{DateTime, FixedOffset};
use tokio_postgres::Client as DbClient;

pub struct Notification {
    pub user_id: i64,
    pub username: String,
    pub origin_url: String,
    pub origin_html: String,
    pub short_description: Option<String>,
    pub time: DateTime<FixedOffset>,

    /// If this is Some, then the notification originated in a team-wide ping
    /// (e.g., @rust-lang/libs). The String is the team name (e.g., libs).
    pub team_name: Option<String>,
}

pub async fn record_ping(db: &DbClient, notification: &Notification) -> anyhow::Result<()> {
    db.execute(
        "INSERT INTO users (user_id, username) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        &[&notification.user_id, &notification.username],
    )
    .await
    .context("inserting user id / username")?;

    db.execute("INSERT INTO notifications (user_id, origin_url, origin_html, time, short_description, team_name) VALUES ($1, $2, $3, $4, $5, $6)",
        &[&notification.user_id, &notification.origin_url, &notification.origin_html, &notification.time, &notification.short_description, &notification.team_name],
        ).await.context("inserting notification")?;

    Ok(())
}

pub async fn delete_ping(db: &DbClient, user_id: i64, origin_url: &str) -> anyhow::Result<()> {
    db.execute(
        "DELETE FROM notifications WHERE user_id = $1 and origin_url = $2",
        &[&user_id, &origin_url],
    )
    .await
    .context("delete notification query")?;

    Ok(())
}

#[derive(Debug)]
pub struct NotificationData {
    pub origin_url: String,
    pub origin_text: String,
    pub short_description: Option<String>,
    pub time: DateTime<FixedOffset>,
}

pub async fn get_notifications(
    db: &DbClient,
    username: &str,
) -> anyhow::Result<Vec<NotificationData>> {
    let notifications = db
        .query(
            "
        select username, origin_url, origin_html, time, short_description
        from notifications
        join users on notifications.user_id = users.user_id
        where username = $1
        order by time desc;",
            &[&username],
        )
        .await
        .context("Getting notification data")?;

    let mut data = Vec::new();
    for notification in notifications {
        let origin_url: String = notification.get(1);
        let origin_text: String = notification.get(2);
        let time: DateTime<FixedOffset> = notification.get(3);
        let short_description: Option<String> = notification.get(4);

        data.push(NotificationData {
            origin_url,
            origin_text,
            short_description,
            time,
        });
    }

    Ok(data)
}
