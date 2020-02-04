use anyhow::Context as _;
pub use tokio_postgres::Client as DbClient;

pub mod notifications;

pub async fn run_migrations(client: &DbClient) -> anyhow::Result<()> {
    client
        .execute(
            "CREATE TABLE IF NOT EXISTS database_versions (
                zero INTEGER PRIMARY KEY,
                migration_counter INTEGER
            );",
            &[],
        )
        .await
        .context("creating database versioning table")?;

    client
        .execute(
            "INSERT INTO database_versions (zero, migration_counter)
        VALUES (0, 0)
        ON CONFLICT DO NOTHING",
            &[],
        )
        .await
        .context("inserting initial database_versions")?;

    let migration_idx: i32 = client
        .query_one("SELECT migration_counter FROM database_versions", &[])
        .await
        .context("getting migration counter")?
        .get(0);
    let migration_idx = migration_idx as usize;

    for (idx, migration) in MIGRATIONS.iter().enumerate() {
        if idx >= migration_idx {
            client
                .execute(*migration, &[])
                .await
                .with_context(|| format!("executing {}th migration", idx))?;
            client
                .execute(
                    "UPDATE database_versions SET migration_counter = $1",
                    &[&(idx as i32 + 1)],
                )
                .await
                .with_context(|| format!("updating migration counter to {}", idx))?;
        }
    }

    Ok(())
}

static MIGRATIONS: &[&str] = &[
    "
CREATE TABLE notifications (
    notification_id BIGSERIAL PRIMARY KEY,
    user_id BIGINT,
    origin_url TEXT NOT NULL,
    origin_html TEXT,
    time TIMESTAMP WITH TIME ZONE
);
",
    "
CREATE TABLE users (
    user_id BIGINT PRIMARY KEY,
    username TEXT NOT NULL
);
",
    "ALTER TABLE notifications ADD COLUMN short_description TEXT;",
    "ALTER TABLE notifications ADD COLUMN team_name TEXT;",
    "ALTER TABLE notifications ADD COLUMN idx INTEGER;",
];
