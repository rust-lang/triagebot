use super::{Commit, Identifier, Job, Notification, NotificationData};
use crate::db::{Connection, ConnectionManager, ManagedConnection, Transaction};
use anyhow::Context as _;
use anyhow::Result;
use chrono::Utc;
use chrono::{DateTime, FixedOffset};
use native_tls::{Certificate, TlsConnector};
use postgres_native_tls::MakeTlsConnector;
use tokio_postgres::types::Json;
use tokio_postgres::{GenericClient, TransactionBuilder};
use tracing::trace;
use uuid::Uuid;

pub struct Postgres(String, std::sync::Once);

impl Postgres {
    pub fn new(url: String) -> Self {
        Postgres(url, std::sync::Once::new())
    }
}

const CERT_URL: &str = "https://s3.amazonaws.com/rds-downloads/rds-ca-2019-root.pem";

lazy_static::lazy_static! {
    static ref CERTIFICATE_PEM: Vec<u8> = {
        let client = reqwest::blocking::Client::new();
        let resp = client
            .get(CERT_URL)
            .send()
            .expect("failed to get RDS cert");
         resp.bytes().expect("failed to get RDS cert body").to_vec()
    };
}

async fn make_client(db_url: &str) -> Result<tokio_postgres::Client> {
    if db_url.contains("rds.amazonaws.com") {
        let cert = &CERTIFICATE_PEM[..];
        let cert = Certificate::from_pem(&cert).context("made certificate")?;
        let connector = TlsConnector::builder()
            .add_root_certificate(cert)
            .build()
            .context("built TlsConnector")?;
        let connector = MakeTlsConnector::new(connector);

        let (db_client, connection) = match tokio_postgres::connect(&db_url, connector).await {
            Ok(v) => v,
            Err(e) => {
                anyhow::bail!("failed to connect to DB: {}", e);
            }
        };
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                eprintln!("database connection error: {}", e);
            }
        });

        Ok(db_client)
    } else {
        eprintln!("Warning: Non-TLS connection to non-RDS DB");
        let (db_client, connection) =
            match tokio_postgres::connect(&db_url, tokio_postgres::NoTls).await {
                Ok(v) => v,
                Err(e) => {
                    anyhow::bail!("failed to connect to DB: {}", e);
                }
            };
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                eprintln!("database connection error: {}", e);
            }
        });

        Ok(db_client)
    }
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
    "ALTER TABLE notifications ADD COLUMN metadata TEXT;",
    "
CREATE TABLE rustc_commits (
    sha TEXT PRIMARY KEY,
    parent_sha TEXT NOT NULL,
    time TIMESTAMP WITH TIME ZONE
);
",
    "ALTER TABLE rustc_commits ADD COLUMN pr INTEGER;",
    "
CREATE TABLE issue_data (
    repo TEXT,
    issue_number INTEGER,
    key TEXT,
    data JSONB,
    PRIMARY KEY (repo, issue_number, key)
);
",
    "
CREATE TABLE jobs (
    id UUID DEFAULT gen_random_uuid() PRIMARY KEY,
    name TEXT NOT NULL,
    scheduled_at TIMESTAMP WITH TIME ZONE NOT NULL,
    metadata JSONB,
    executed_at TIMESTAMP WITH TIME ZONE,
    error_message TEXT
);
",
    "
CREATE UNIQUE INDEX jobs_name_scheduled_at_unique_index
    ON jobs (
        name, scheduled_at
    );
",
];

#[async_trait::async_trait]
impl ConnectionManager for Postgres {
    type Connection = PostgresConnection;
    async fn open(&self) -> Self::Connection {
        let client = make_client(&self.0).await.unwrap();
        let mut should_init = false;
        self.1.call_once(|| {
            should_init = true;
        });
        if should_init {
            run_migrations(&client).await.unwrap();
        }
        PostgresConnection::new(client).await
    }
    async fn is_valid(&self, conn: &mut Self::Connection) -> bool {
        !conn.conn.is_closed()
    }
}

pub async fn run_migrations(client: &tokio_postgres::Client) -> Result<()> {
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

#[async_trait::async_trait]
impl<'a> Transaction for PostgresTransaction<'a> {
    async fn commit(self: Box<Self>) -> Result<(), anyhow::Error> {
        Ok(self.conn.commit().await?)
    }
    async fn finish(self: Box<Self>) -> Result<(), anyhow::Error> {
        Ok(self.conn.rollback().await?)
    }
    fn conn(&mut self) -> &mut dyn Connection {
        self
    }
    fn conn_ref(&self) -> &dyn Connection {
        self
    }
}

pub struct PostgresTransaction<'a> {
    conn: tokio_postgres::Transaction<'a>,
}

pub struct PostgresConnection {
    conn: tokio_postgres::Client,
}

impl Into<tokio_postgres::Client> for PostgresConnection {
    fn into(self) -> tokio_postgres::Client {
        self.conn
    }
}

pub trait PClient {
    type Client: Send + Sync + tokio_postgres::GenericClient;
    fn conn(&self) -> &Self::Client;
    fn conn_mut(&mut self) -> &mut Self::Client;
    fn build_transaction(&mut self) -> TransactionBuilder;
}

impl<'a> PClient for PostgresTransaction<'a> {
    type Client = tokio_postgres::Transaction<'a>;
    fn conn(&self) -> &Self::Client {
        &self.conn
    }
    fn conn_mut(&mut self) -> &mut Self::Client {
        &mut self.conn
    }
    fn build_transaction(&mut self) -> TransactionBuilder {
        panic!("nested transactions not supported");
    }
}

impl PClient for ManagedConnection<PostgresConnection> {
    type Client = tokio_postgres::Client;
    fn conn(&self) -> &Self::Client {
        &(&**self).conn
    }
    fn conn_mut(&mut self) -> &mut Self::Client {
        &mut (&mut **self).conn
    }
    fn build_transaction(&mut self) -> TransactionBuilder {
        self.conn_mut().build_transaction()
    }
}

impl PostgresConnection {
    pub async fn new(conn: tokio_postgres::Client) -> Self {
        PostgresConnection { conn }
    }
}

#[async_trait::async_trait]
impl<P> Connection for P
where
    P: Send + Sync + PClient,
{
    async fn transaction(&mut self) -> Box<dyn Transaction + '_> {
        let tx = self.conn_mut().transaction().await.unwrap();
        Box::new(PostgresTransaction { conn: tx })
    }

    async fn record_username(&mut self, user_id: i64, username: String) -> Result<()> {
        self.conn()
            .execute(
                "INSERT INTO users (user_id, username) VALUES ($1, $2) ON CONFLICT DO NOTHING",
                &[&user_id, &username],
            )
            .await
            .context("inserting user id / username")?;
        Ok(())
    }

    async fn record_ping(&mut self, notification: &Notification) -> Result<()> {
        self.conn()
            .execute(
                "INSERT INTO notifications
                    (user_id, origin_url, origin_html, time, short_description, team_name, idx)
                VALUES (
                    $1, $2, $3, $4, $5, $6,
                    (SELECT max(notifications.idx) + 1 from notifications
                        where notifications.user_id = $1)
                )",
                &[
                    &notification.user_id,
                    &notification.origin_url,
                    &notification.origin_html,
                    &notification.time,
                    &notification.short_description,
                    &notification.team_name,
                ],
            )
            .await
            .context("inserting notification")?;

        Ok(())
    }

    async fn get_missing_commits(&mut self) -> Result<Vec<String>> {
        let missing = self
            .conn()
            .query(
                "
                SELECT parent_sha
                FROM rustc_commits
                WHERE parent_sha NOT IN (
                    SELECT sha
                    FROM rustc_commits
                )",
                &[],
            )
            .await
            .context("fetching missing commits")?;
        Ok(missing.into_iter().map(|row| row.get(0)).collect())
    }

    async fn record_commit(&mut self, commit: &Commit) -> Result<()> {
        trace!("record_commit(sha={})", commit.sha);
        let pr = commit.pr.expect("commit has pr");
        self.conn()
            .execute(
                "INSERT INTO rustc_commits (sha, parent_sha, time, pr)
                    VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
                &[&commit.sha, &commit.parent_sha, &commit.time, &(pr as i32)],
            )
            .await
            .context("inserting commit")?;
        Ok(())
    }

    async fn has_commit(&mut self, sha: &str) -> Result<bool> {
        self.conn()
            .query("SELECT 1 FROM rustc_commits WHERE sha = $1", &[&sha])
            .await
            .context("selecting from rustc_commits")
            .map(|commits| !commits.is_empty())
    }

    async fn get_commits_with_artifacts(&mut self) -> Result<Vec<Commit>> {
        let commits = self
            .conn()
            .query(
                "
                select sha, parent_sha, time, pr
                from rustc_commits
                where time >= current_date - interval '168 days'
                order by time desc;",
                &[],
            )
            .await
            .context("Getting commit data")?;

        let mut data = Vec::with_capacity(commits.len());
        for commit in commits {
            let sha: String = commit.get(0);
            let parent_sha: String = commit.get(1);
            let time: DateTime<FixedOffset> = commit.get(2);
            let pr: Option<i32> = commit.get(3);

            data.push(Commit {
                sha,
                parent_sha,
                time,
                pr: pr.map(|n| n as u32),
            });
        }

        Ok(data)
    }

    async fn get_notifications(&mut self, username: &str) -> Result<Vec<NotificationData>> {
        let notifications = self
            .conn()
            .query(
                "SELECT username, origin_url, origin_html, time, short_description, idx, metadata
                FROM notifications
                JOIN users ON notifications.user_id = users.user_id
                WHERE username = $1
                ORDER BY notifications.idx ASC NULLS LAST;",
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
            let metadata: Option<String> = notification.get(6);

            data.push(NotificationData {
                origin_url,
                origin_text,
                short_description,
                time,
                metadata,
            });
        }

        Ok(data)
    }

    async fn delete_ping(
        &mut self,
        user_id: i64,
        identifier: Identifier<'_>,
    ) -> Result<Vec<NotificationData>> {
        match identifier {
            Identifier::Url(origin_url) => {
                let rows = self
                    .conn()
                    .query(
                        "DELETE FROM notifications WHERE user_id = $1 and origin_url = $2
                        RETURNING origin_html, time, short_description, metadata",
                        &[&user_id, &origin_url],
                    )
                    .await
                    .context("delete notification query")?;
                Ok(rows
                    .into_iter()
                    .map(|row| {
                        let origin_text: String = row.get(0);
                        let time: DateTime<FixedOffset> = row.get(1);
                        let short_description: Option<String> = row.get(2);
                        let metadata: Option<String> = row.get(3);
                        NotificationData {
                            origin_url: origin_url.to_owned(),
                            origin_text,
                            time,
                            short_description,
                            metadata,
                        }
                    })
                    .collect())
            }
            Identifier::Index(idx) => loop {
                let t = self
                    .build_transaction()
                    .isolation_level(tokio_postgres::IsolationLevel::Serializable)
                    .start()
                    .await
                    .context("begin transaction")?;

                let notifications = t
                    .query(
                        "SELECT notification_id, idx, user_id
                        FROM notifications
                        WHERE user_id = $1
                        ORDER BY idx ASC NULLS LAST;",
                        &[&user_id],
                    )
                    .await
                    .context("failed to get ordering")?;

                let notification_id: i64 = notifications
                    .get(idx.get() - 1)
                    .ok_or_else(|| {
                        anyhow::anyhow!("No such notification with index {}", idx.get())
                    })?
                    .get(0);

                let row = t
                    .query_one(
                        "DELETE FROM notifications WHERE notification_id = $1
                            RETURNING origin_url, origin_html, time, short_description, metadata",
                        &[&notification_id],
                    )
                    .await
                    .context(format!(
                        "Failed to delete notification with id {}",
                        notification_id
                    ))?;

                let origin_url: String = row.get(0);
                let origin_text: String = row.get(1);
                let time: DateTime<FixedOffset> = row.get(2);
                let short_description: Option<String> = row.get(3);
                let metadata: Option<String> = row.get(4);
                let deleted_notification = NotificationData {
                    origin_url,
                    origin_text,
                    time,
                    short_description,
                    metadata,
                };

                if let Err(e) = t.commit().await {
                    if e.code().map_or(false, |c| {
                        *c == tokio_postgres::error::SqlState::T_R_SERIALIZATION_FAILURE
                    }) {
                        trace!("serialization failure, restarting deletion");
                        continue;
                    } else {
                        return Err(e).context("transaction commit failure");
                    }
                } else {
                    return Ok(vec![deleted_notification]);
                }
            },
            Identifier::All => {
                let rows = self
                    .conn()
                    .query(
                        "DELETE FROM notifications WHERE user_id = $1
                            RETURNING origin_url, origin_html, time, short_description, metadata",
                        &[&user_id],
                    )
                    .await
                    .context("delete all notifications query")?;
                Ok(rows
                    .into_iter()
                    .map(|row| {
                        let origin_url: String = row.get(0);
                        let origin_text: String = row.get(1);
                        let time: DateTime<FixedOffset> = row.get(2);
                        let short_description: Option<String> = row.get(3);
                        let metadata: Option<String> = row.get(4);
                        NotificationData {
                            origin_url,
                            origin_text,
                            time,
                            short_description,
                            metadata,
                        }
                    })
                    .collect())
            }
        }
    }

    async fn add_metadata(
        &mut self,
        user_id: i64,
        idx: usize,
        metadata: Option<&str>,
    ) -> Result<()> {
        loop {
            let t = self
                .build_transaction()
                .isolation_level(tokio_postgres::IsolationLevel::Serializable)
                .start()
                .await
                .context("begin transaction")?;

            let notifications = t
                .query(
                    "SELECT notification_id, idx, user_id
                    FROM notifications
                    WHERE user_id = $1
                    ORDER BY idx ASC NULLS LAST;",
                    &[&user_id],
                )
                .await
                .context("failed to get initial ordering")?;

            let notifications = notifications
                .into_iter()
                .map(|n| n.get(0))
                .collect::<Vec<i64>>();

            match notifications.get(idx) {
                None => anyhow::bail!(
                    "index not present, must be less than {}",
                    notifications.len()
                ),
                Some(id) => {
                    t.execute(
                        "UPDATE notifications SET metadata = $2
                        WHERE notification_id = $1",
                        &[&id, &metadata],
                    )
                    .await
                    .context("update notification id")?;
                }
            }

            if let Err(e) = t.commit().await {
                if e.code().map_or(false, |c| {
                    *c == tokio_postgres::error::SqlState::T_R_SERIALIZATION_FAILURE
                }) {
                    trace!("serialization failure, restarting index movement");
                    continue;
                } else {
                    return Err(e).context("transaction commit failure");
                }
            } else {
                break;
            }
        }

        Ok(())
    }

    async fn move_indices(&mut self, user_id: i64, from: usize, to: usize) -> Result<()> {
        loop {
            let t = self
                .build_transaction()
                .isolation_level(tokio_postgres::IsolationLevel::Serializable)
                .start()
                .await
                .context("begin transaction")?;

            let notifications = t
                .query(
                    "SELECT notification_id, idx, user_id
                    FROM notifications
                    WHERE user_id = $1
                    ORDER BY idx ASC NULLS LAST;",
                    &[&user_id],
                )
                .await
                .context("failed to get initial ordering")?;

            let mut notifications = notifications
                .into_iter()
                .map(|n| n.get(0))
                .collect::<Vec<i64>>();

            if notifications.get(from).is_none() {
                anyhow::bail!(
                    "`from` index not present, must be less than {}",
                    notifications.len()
                );
            }

            if notifications.get(to).is_none() {
                anyhow::bail!(
                    "`to` index not present, must be less than {}",
                    notifications.len()
                );
            }

            if from < to {
                notifications[from..=to].rotate_left(1);
            } else if to < from {
                notifications[to..=from].rotate_right(1);
            }

            for (idx, id) in notifications.into_iter().enumerate() {
                t.execute(
                    "UPDATE notifications SET idx = $2
                     WHERE notification_id = $1",
                    &[&id, &(idx as i32)],
                )
                .await
                .context("update notification id")?;
            }

            if let Err(e) = t.commit().await {
                if e.code().map_or(false, |c| {
                    *c == tokio_postgres::error::SqlState::T_R_SERIALIZATION_FAILURE
                }) {
                    trace!("serialization failure, restarting index movement");
                    continue;
                } else {
                    return Err(e).context("transaction commit failure");
                }
            } else {
                break;
            }
        }

        Ok(())
    }

    async fn insert_job(
        &mut self,
        name: &str,
        scheduled_at: &DateTime<Utc>,
        metadata: &serde_json::Value,
    ) -> Result<()> {
        tracing::trace!("insert_job(name={})", name);

        self.conn()
            .execute(
                "INSERT INTO jobs (name, scheduled_at, metadata) VALUES ($1, $2, $3)
                ON CONFLICT (name, scheduled_at) DO UPDATE SET metadata = EXCLUDED.metadata",
                &[&name, &scheduled_at, &metadata],
            )
            .await
            .context("Inserting job")?;

        Ok(())
    }

    async fn delete_job(&mut self, id: &Uuid) -> Result<()> {
        tracing::trace!("delete_job(id={})", id);

        self.conn()
            .execute("DELETE FROM jobs WHERE id = $1", &[id])
            .await
            .context("Deleting job")?;

        Ok(())
    }

    async fn update_job_error_message(&mut self, id: &Uuid, message: &str) -> Result<()> {
        tracing::trace!("update_job_error_message(id={})", id);

        self.conn()
            .execute(
                "UPDATE jobs SET error_message = $2 WHERE id = $1",
                &[&id, &message],
            )
            .await
            .context("Updating job error message")?;

        Ok(())
    }

    async fn update_job_executed_at(&mut self, id: &Uuid) -> Result<()> {
        tracing::trace!("update_job_executed_at(id={})", id);

        self.conn()
            .execute("UPDATE jobs SET executed_at = now() WHERE id = $1", &[&id])
            .await
            .context("Updating job executed at")?;

        Ok(())
    }

    async fn get_job_by_name_and_scheduled_at(
        &mut self,
        name: &str,
        scheduled_at: &DateTime<Utc>,
    ) -> Result<Job> {
        tracing::trace!(
            "get_job_by_name_and_scheduled_at(name={}, scheduled_at={})",
            name,
            scheduled_at
        );

        let job = self
            .conn()
            .query_one(
                "SELECT * FROM jobs WHERE name = $1 AND scheduled_at = $2",
                &[&name, &scheduled_at],
            )
            .await
            .context("Select job by name and scheduled at")?;

        deserialize_job(&job)
    }

    async fn get_jobs_to_execute(&mut self) -> Result<Vec<Job>> {
        let jobs = self
            .conn()
            .query(
                "SELECT * FROM jobs WHERE scheduled_at <= now()
                    AND (error_message IS NULL OR executed_at <= now() - INTERVAL '60 minutes')",
                &[],
            )
            .await
            .context("Getting jobs data")?;

        let mut data = Vec::with_capacity(jobs.len());
        for job in jobs {
            let serialized_job = deserialize_job(&job);
            data.push(serialized_job.unwrap());
        }

        Ok(data)
    }

    async fn lock_and_load_issue_data(
        &mut self,
        repo: &str,
        issue_number: i32,
        key: &str,
    ) -> Result<(Box<dyn Transaction + '_>, Option<serde_json::Value>)> {
        let transaction = self.conn_mut().transaction().await?;
        transaction
            .execute("LOCK TABLE issue_data", &[])
            .await
            .context("locking issue data")?;
        let data = transaction
            .query_opt(
                "SELECT data FROM issue_data WHERE \
                 repo = $1 AND issue_number = $2 AND key = $3",
                &[&repo, &issue_number, &key],
            )
            .await
            .context("selecting issue data")?
            .map(|row| row.get::<usize, Json<serde_json::Value>>(0).0);
        Ok((Box::new(PostgresTransaction { conn: transaction }), data))
    }

    async fn save_issue_data(
        &mut self,
        repo: &str,
        issue_number: i32,
        key: &str,
        data: &serde_json::Value,
    ) -> Result<()> {
        self.conn()
            .execute(
                "INSERT INTO issue_data (repo, issue_number, key, data) \
                 VALUES ($1, $2, $3, $4) \
                 ON CONFLICT (repo, issue_number, key) DO UPDATE SET data=EXCLUDED.data",
                &[&repo, &issue_number, &key, &Json(&data)],
            )
            .await
            .context("inserting issue data")?;
        Ok(())
    }
}

fn deserialize_job(row: &tokio_postgres::row::Row) -> Result<Job> {
    let id: Uuid = row.try_get(0)?;
    let name: String = row.try_get(1)?;
    let scheduled_at: DateTime<Utc> = row.try_get(2)?;
    let metadata: serde_json::Value = row.try_get(3)?;
    let executed_at: Option<DateTime<Utc>> = row.try_get(4)?;
    let error_message: Option<String> = row.try_get(5)?;

    Ok(Job {
        id,
        name,
        scheduled_at,
        metadata,
        executed_at,
        error_message,
    })
}
