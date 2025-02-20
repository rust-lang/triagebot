use anyhow::Context;
use tokio_postgres::Client as DbClient;

/// Add a new user (if not existing)
pub async fn record_username(db: &DbClient, user_id: u64, username: &str) -> anyhow::Result<()> {
    db.execute(
        "INSERT INTO users (user_id, username) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        &[&(user_id as i64), &username],
    )
    .await
    .context("inserting user id / username")?;
    Ok(())
}
