use anyhow::Context;
use tokio_postgres::Client as DbClient;

/// Add a new user (if not existing)
pub async fn record_username(db: &DbClient, user_id: u64, username: &str) -> anyhow::Result<()> {
    db.execute(
        r"
INSERT INTO users (user_id, username) VALUES ($1, $2)
ON CONFLICT (user_id)
DO UPDATE SET username = $2",
        &[&(user_id as i64), &username],
    )
    .await
    .context("inserting user id / username")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::db::users::record_username;
    use crate::tests::run_test;

    #[tokio::test]
    async fn update_username_on_conflict() {
        run_test(|ctx| async {
            let db = ctx.db_client().await;

            record_username(&db, 1, "Foo").await?;
            record_username(&db, 1, "Bar").await?;

            let row = db
                .query_one("SELECT username FROM users WHERE user_id = 1", &[])
                .await
                .unwrap();
            let name: &str = row.get(0);
            assert_eq!(name, "Bar");

            Ok(ctx)
        })
        .await;
    }
}
