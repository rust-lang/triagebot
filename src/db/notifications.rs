use anyhow::Context as _;
use chrono::{DateTime, FixedOffset};
use tokio_postgres::Client as DbClient;

pub struct Notification {
    pub user_id: i64,
    pub username: String,
    pub origin_url: String,
    pub origin_html: String,
    pub time: DateTime<FixedOffset>,
}

pub async fn record_ping(db: &DbClient, notification: &Notification) -> anyhow::Result<()> {
    db.execute(
        "INSERT INTO users (user_id, username) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        &[&notification.user_id, &notification.username],
    )
    .await
    .context("inserting user id / username")?;

    db.execute("INSERT INTO notifications (user_id, origin_url, origin_html, time) VALUES ($1, $2, $3, $4)",
        &[&notification.user_id, &notification.origin_url, &notification.origin_html, &notification.time],
        ).await.context("inserting notification")?;

    Ok(())
}
