use super::{Commit, Identifier, Notification, NotificationData};
use crate::db::{Connection, ConnectionManager, Job, ManagedConnection, Transaction};
use anyhow::{Context, Result};
use chrono::DateTime;
use chrono::Utc;
use rusqlite::params;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::Once;
use uuid::Uuid;

pub struct SqliteTransaction<'a> {
    conn: &'a mut SqliteConnection,
    finished: bool,
}

#[async_trait::async_trait]
impl<'a> Transaction for SqliteTransaction<'a> {
    async fn commit(mut self: Box<Self>) -> Result<(), anyhow::Error> {
        self.finished = true;
        Ok(self.conn.raw().execute_batch("COMMIT")?)
    }

    async fn finish(mut self: Box<Self>) -> Result<(), anyhow::Error> {
        self.finished = true;
        Ok(self.conn.raw().execute_batch("ROLLBACK")?)
    }
    fn conn(&mut self) -> &mut dyn Connection {
        &mut *self.conn
    }
    fn conn_ref(&self) -> &dyn Connection {
        &*self.conn
    }
}

impl std::ops::Deref for SqliteTransaction<'_> {
    type Target = dyn Connection;
    fn deref(&self) -> &Self::Target {
        &*self.conn
    }
}

impl std::ops::DerefMut for SqliteTransaction<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.conn
    }
}

impl Drop for SqliteTransaction<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.conn.raw().execute_batch("ROLLBACK").unwrap();
        }
    }
}

pub struct Sqlite(PathBuf, Once);

impl Sqlite {
    pub fn new(path: PathBuf) -> Self {
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent).unwrap();
            }
        }
        Sqlite(path, Once::new())
    }
}

struct Migration {
    /// One or more SQL statements, each terminated by a semicolon.
    sql: &'static str,

    /// If false, indicates that foreign key checking should be delayed until after execution of
    /// the migration SQL, and foreign key `ON UPDATE` and `ON DELETE` actions disabled completely.
    foreign_key_constraints_enabled: bool,
}

impl Migration {
    /// Returns a `Migration` with foreign key constraints enabled during execution.
    const fn new(sql: &'static str) -> Migration {
        Migration {
            sql,
            foreign_key_constraints_enabled: true,
        }
    }

    /// Returns a `Migration` with foreign key checking delayed until after execution, and foreign
    /// key `ON UPDATE` and `ON DELETE` actions disabled completely.
    ///
    /// SQLite has limited `ALTER TABLE` capabilities, so some schema alterations require the
    /// approach of replacing a table with a new one having the desired schema. Because there might
    /// be other tables with foreign key constraints on the table, these constraints need to be
    /// disabled during execution of such migration SQL, and reenabled after. Otherwise, dropping
    /// the old table may trigger `ON DELETE` actions in the referencing tables. See [SQLite
    /// documentation](https://www.sqlite.org/lang_altertable.html) for more information.
    #[allow(dead_code)]
    const fn without_foreign_key_constraints(sql: &'static str) -> Migration {
        Migration {
            sql,
            foreign_key_constraints_enabled: false,
        }
    }

    fn execute(&self, conn: &mut rusqlite::Connection, migration_id: i32) {
        if self.foreign_key_constraints_enabled {
            let tx = conn.transaction().unwrap();
            tx.execute_batch(&self.sql).unwrap();
            tx.pragma_update(None, "user_version", &migration_id)
                .unwrap();
            tx.commit().unwrap();
            return;
        }

        // The following steps are reproduced from https://www.sqlite.org/lang_altertable.html,
        // from the section titled, "Making Other Kinds Of Table Schema Changes".

        // 1.  If foreign key constraints are enabled, disable them using PRAGMA foreign_keys=OFF.
        conn.pragma_update(None, "foreign_keys", &"OFF").unwrap();

        // 2.  Start a transaction.
        let tx = conn.transaction().unwrap();

        // The migration SQL is responsible for steps 3 through 9.

        // 3.  Remember the format of all indexes, triggers, and views associated with table X.
        //     This information will be needed in step 8 below. One way to do this is to run a
        //     query like the following: SELECT type, sql FROM sqlite_schema WHERE tbl_name='X'.
        //
        // 4.  Use CREATE TABLE to construct a new table "new_X" that is in the desired revised
        //     format of table X. Make sure that the name "new_X" does not collide with any
        //     existing table name, of course.
        //
        // 5.  Transfer content from X into new_X using a statement like: INSERT INTO new_X SELECT
        //     ... FROM X.
        //
        // 6.  Drop the old table X: DROP TABLE X.
        //
        // 7.  Change the name of new_X to X using: ALTER TABLE new_X RENAME TO X.
        //
        // 8.  Use CREATE INDEX, CREATE TRIGGER, and CREATE VIEW to reconstruct indexes, triggers,
        //     and views associated with table X. Perhaps use the old format of the triggers,
        //     indexes, and views saved from step 3 above as a guide, making changes as appropriate
        //     for the alteration.
        //
        // 9.  If any views refer to table X in a way that is affected by the schema change, then
        //     drop those views using DROP VIEW and recreate them with whatever changes are
        //     necessary to accommodate the schema change using CREATE VIEW.
        tx.execute_batch(&self.sql).unwrap();

        // 10. If foreign key constraints were originally enabled then run PRAGMA foreign_key_check
        //     to verify that the schema change did not break any foreign key constraints.
        tx.pragma_query(None, "foreign_key_check", |row| {
            let table: String = row.get_unwrap(0);
            let row_id: Option<i64> = row.get_unwrap(1);
            let foreign_table: String = row.get_unwrap(2);
            let fk_idx: i64 = row.get_unwrap(3);

            tx.query_row::<(), _, _>(
                "select * from pragma_foreign_key_list(?) where id = ?",
                params![&table, &fk_idx],
                |row| {
                    let col: String = row.get_unwrap(3);
                    let foreign_col: String = row.get_unwrap(4);
                    panic!(
                        "Foreign key violation encountered during migration\n\
                            table: {},\n\
                            column: {},\n\
                            row_id: {:?},\n\
                            foreign table: {},\n\
                            foreign column: {}\n\
                            migration ID: {}\n",
                        table, col, row_id, foreign_table, foreign_col, migration_id,
                    );
                },
            )
            .unwrap();
            Ok(())
        })
        .unwrap();

        tx.pragma_update(None, "user_version", &migration_id)
            .unwrap();

        // 11. Commit the transaction started in step 2.
        tx.commit().unwrap();

        // 12. If foreign keys constraints were originally enabled, reenable them now.
        conn.pragma_update(None, "foreign_keys", &"ON").unwrap();
    }
}

static MIGRATIONS: &[Migration] = &[
    Migration::new(""),
    Migration::new(
        r#"
CREATE TABLE notifications (
    notification_id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
    user_id BIGINT,
    origin_url TEXT NOT NULL,
    origin_html TEXT,
    time TEXT NOT NULL,
    short_description TEXT,
    team_name TEXT,
    idx INTEGER,
    metadata TEXT
);
        "#,
    ),
    Migration::new(
        r#"
CREATE TABLE users (
    user_id BIGINT PRIMARY KEY,
    username TEXT NOT NULL
);
        "#,
    ),
    Migration::new(
        r#"
CREATE TABLE rustc_commits (
    sha TEXT PRIMARY KEY,
    parent_sha TEXT NOT NULL,
    time TEXT NOT NULL,
    pr INTEGER
);
        "#,
    ),
    Migration::new(
        r#"
CREATE TABLE issue_data (
    repo TEXT,
    issue_number INTEGER,
    key TEXT,
    data JSONB,
    PRIMARY KEY (repo, issue_number, key)
);
        "#,
    ),
    Migration::new(
        r#"
CREATE TABLE jobs (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    scheduled_at TIMESTAMP WITH TIME ZONE NOT NULL,
    metadata JSONB,
    executed_at TIMESTAMP WITH TIME ZONE,
    error_message TEXT
);
        "#,
    ),
    Migration::new(
        r#"
CREATE UNIQUE INDEX jobs_name_scheduled_at_unique_index
    ON jobs (
        name, scheduled_at
    );
        "#,
    ),
];

#[async_trait::async_trait]
impl ConnectionManager for Sqlite {
    type Connection = Mutex<rusqlite::Connection>;
    async fn open(&self) -> Self::Connection {
        let mut conn = rusqlite::Connection::open(&self.0).unwrap();
        conn.pragma_update(None, "cache_size", &-128000).unwrap();
        conn.pragma_update(None, "journal_mode", &"WAL").unwrap();
        conn.pragma_update(None, "foreign_keys", &"ON").unwrap();

        self.1.call_once(|| {
            let version: i32 = conn
                .query_row(
                    "select user_version from pragma_user_version;",
                    params![],
                    |row| row.get(0),
                )
                .unwrap();
            for mid in (version as usize + 1)..MIGRATIONS.len() {
                MIGRATIONS[mid].execute(&mut conn, mid as i32);
            }
        });

        Mutex::new(conn)
    }
    async fn is_valid(&self, conn: &mut Self::Connection) -> bool {
        conn.get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .execute_batch("")
            .is_ok()
    }
}

pub struct SqliteConnection {
    conn: ManagedConnection<Mutex<rusqlite::Connection>>,
}

#[async_trait::async_trait]
impl Connection for SqliteConnection {
    async fn transaction(&mut self) -> Box<dyn Transaction + '_> {
        Box::new(self.raw_transaction())
    }

    async fn record_username(&mut self, user_id: i64, username: String) -> Result<()> {
        self.raw().execute(
            "INSERT INTO users (user_id, username) VALUES (?, ?) ON CONFLICT DO NOTHING",
            params![user_id, username],
        )?;
        Ok(())
    }

    async fn record_ping(&mut self, notification: &Notification) -> Result<()> {
        self.raw().execute(
            "INSERT INTO notifications
                    (user_id, origin_url, origin_html, time, short_description, team_name, idx)
                VALUES (
                    ?, ?, ?, ?, ?, ?,
                    (SELECT ifnull(max(notifications.idx), 0) + 1 from notifications
                        where notifications.user_id = ?)
                )",
            params![
                notification.user_id,
                notification.origin_url,
                notification.origin_html,
                notification.time,
                notification.short_description,
                notification.team_name,
                notification.user_id,
            ],
        )?;
        Ok(())
    }

    async fn get_missing_commits(&mut self) -> Result<Vec<String>> {
        let commits = self
            .raw()
            .prepare(
                "
                SELECT parent_sha
                FROM rustc_commits
                WHERE parent_sha NOT IN (
                    SELECT sha
                    FROM rustc_commits
                )",
            )?
            .query_map([], |row| row.get(0))?
            .collect::<std::result::Result<_, rusqlite::Error>>()?;
        Ok(commits)
    }

    async fn record_commit(&mut self, commit: &Commit) -> Result<()> {
        let pr = commit.pr.expect("commit has pr");
        // let time = commit.time.timestamp();
        self.raw().execute(
            "INSERT INTO rustc_commits (sha, parent_sha, time, pr) \
            VALUES (?, ?, ?, ?) ON CONFLICT DO NOTHING",
            params![commit.sha, commit.parent_sha, commit.time, pr],
        )?;
        Ok(())
    }

    async fn has_commit(&mut self, sha: &str) -> Result<bool> {
        Ok(self
            .raw()
            .prepare("SELECT 1 FROM rustc_commits WHERE sha = ?")?
            .query([sha])?
            .next()?
            .is_some())
    }

    async fn get_commits_with_artifacts(&mut self) -> Result<Vec<Commit>> {
        let commits = self
            .raw()
            .prepare(
                "SELECT sha, parent_sha, time, pr
                    FROM rustc_commits
                    WHERE time >= datetime('now', '-168 days')
                    ORDER BY time DESC;",
            )?
            .query_map([], |row| {
                let c = Commit {
                    sha: row.get(0)?,
                    parent_sha: row.get(1)?,
                    time: row.get(2)?,
                    pr: row.get(3)?,
                };
                Ok(c)
            })?
            .collect::<std::result::Result<_, rusqlite::Error>>()?;
        Ok(commits)
    }

    async fn get_notifications(&mut self, username: &str) -> Result<Vec<NotificationData>> {
        let notifications = self
            .raw()
            .prepare(
                "SELECT username, origin_url, origin_html, time, short_description, idx, metadata
                FROM notifications
                JOIN users ON notifications.user_id = users.user_id
                WHERE username = ?
                ORDER BY notifications.idx ASC NULLS LAST;",
            )?
            .query_map([username], |row| {
                let n = NotificationData {
                    origin_url: row.get(1)?,
                    origin_text: row.get(2)?,
                    time: row.get(3)?,
                    short_description: row.get(4)?,
                    metadata: row.get(6)?,
                };
                Ok(n)
            })?
            .collect::<std::result::Result<_, rusqlite::Error>>()?;
        Ok(notifications)
    }

    async fn delete_ping(
        &mut self,
        user_id: i64,
        identifier: Identifier<'_>,
    ) -> Result<Vec<NotificationData>> {
        match identifier {
            Identifier::Url(origin_url) => {
                let rows = self
                    .raw()
                    .prepare(
                        "DELETE FROM notifications WHERE user_id = ? and origin_url = ?
                        RETURNING origin_html, time, short_description, metadata",
                    )?
                    .query_map(params![user_id, origin_url], |row| {
                        let n = NotificationData {
                            origin_url: origin_url.to_string(),
                            origin_text: row.get(0)?,
                            time: row.get(1)?,
                            short_description: row.get(2)?,
                            metadata: row.get(3)?,
                        };
                        Ok(n)
                    })?
                    .collect::<std::result::Result<_, rusqlite::Error>>()?;
                Ok(rows)
            }
            Identifier::Index(idx) => {
                let deleted_notifications: Vec<_> = self
                    .raw()
                    .prepare(
                        "DELETE FROM notifications WHERE notification_id = (
                            SELECT notification_id FROM notifications
                                WHERE user_id = ?
                                ORDER BY idx ASC NULLS LAST
                                LIMIT 1 OFFSET ?
                        )
                        RETURNING origin_url, origin_html, time, short_description, metadata",
                    )?
                    .query_map(params![user_id, idx.get() - 1], |row| {
                        let n = NotificationData {
                            origin_url: row.get(0)?,
                            origin_text: row.get(1)?,
                            time: row.get(2)?,
                            short_description: row.get(3)?,
                            metadata: row.get(4)?,
                        };
                        Ok(n)
                    })?
                    .collect::<std::result::Result<_, rusqlite::Error>>()?;
                if deleted_notifications.is_empty() {
                    anyhow::bail!("No such notification with index {}", idx.get());
                }
                return Ok(deleted_notifications);
            }
            Identifier::All => {
                let rows = self
                    .raw()
                    .prepare(
                        "DELETE FROM notifications WHERE user_id = ?
                        RETURNING origin_url, origin_html, time, short_description, metadata",
                    )?
                    .query_map([&user_id], |row| {
                        let n = NotificationData {
                            origin_url: row.get(0)?,
                            origin_text: row.get(1)?,
                            time: row.get(2)?,
                            short_description: row.get(3)?,
                            metadata: row.get(4)?,
                        };
                        Ok(n)
                    })?
                    .collect::<std::result::Result<_, rusqlite::Error>>()?;
                Ok(rows)
            }
        }
    }

    async fn add_metadata(
        &mut self,
        user_id: i64,
        idx: usize,
        metadata: Option<&str>,
    ) -> Result<()> {
        let t = self.raw().transaction()?;

        let notifications = t
            .prepare(
                "SELECT notification_id
                FROM notifications
                WHERE user_id = ?
                ORDER BY idx ASC NULLS LAST",
            )?
            .query_map([user_id], |row| row.get(0))
            .context("failed to get initial ordering")?
            .collect::<std::result::Result<Vec<i64>, rusqlite::Error>>()?;

        match notifications.get(idx) {
            None => anyhow::bail!(
                "index not present, must be less than {}",
                notifications.len()
            ),
            Some(id) => {
                t.prepare(
                    "UPDATE notifications SET metadata = ?
                    WHERE notification_id = ?",
                )?
                .execute(params![metadata, id])
                .context("update notification id")?;
            }
        }
        t.commit()?;

        Ok(())
    }

    async fn move_indices(&mut self, user_id: i64, from: usize, to: usize) -> Result<()> {
        let t = self.raw().transaction()?;

        let mut notifications = t
            .prepare(
                "SELECT notification_id
                    FROM notifications
                    WHERE user_id = ?
                    ORDER BY idx ASC NULLS LAST;",
            )?
            .query_map([user_id], |row| row.get(0))
            .context("failed to get initial ordering")?
            .collect::<std::result::Result<Vec<i64>, rusqlite::Error>>()?;

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
            t.prepare(
                "UPDATE notifications SET idx = ?
                     WHERE notification_id = ?",
            )?
            .execute(params![idx, id])
            .context("update notification id")?;
        }
        t.commit()?;

        Ok(())
    }

    async fn insert_job(
        &mut self,
        name: &str,
        scheduled_at: &DateTime<Utc>,
        metadata: &serde_json::Value,
    ) -> Result<()> {
        tracing::trace!("insert_job(name={})", name);

        let id = Uuid::new_v4();
        self.raw()
            .execute(
                "INSERT INTO jobs (id, name, scheduled_at, metadata) VALUES (?, ?, ?, ?)
                ON CONFLICT (name, scheduled_at) DO UPDATE SET metadata = EXCLUDED.metadata",
                params![id, name, scheduled_at, metadata],
            )
            .context("Inserting job")?;

        Ok(())
    }

    async fn delete_job(&mut self, id: &Uuid) -> Result<()> {
        tracing::trace!("delete_job(id={})", id);

        self.raw()
            .execute("DELETE FROM jobs WHERE id = ?", [id])
            .context("Deleting job")?;

        Ok(())
    }

    async fn update_job_error_message(&mut self, id: &Uuid, message: &str) -> Result<()> {
        tracing::trace!("update_job_error_message(id={})", id);

        self.raw()
            .execute(
                "UPDATE jobs SET error_message = ? WHERE id = ?",
                params![message, id],
            )
            .context("Updating job error message")?;

        Ok(())
    }

    async fn update_job_executed_at(&mut self, id: &Uuid) -> Result<()> {
        tracing::trace!("update_job_executed_at(id={})", id);

        self.raw()
            .execute(
                "UPDATE jobs SET executed_at = datetime('now') WHERE id = ?",
                [id],
            )
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
            .raw()
            .query_row(
                "SELECT * FROM jobs WHERE name = ? AND scheduled_at = ?",
                params![name, scheduled_at],
                |row| deserialize_job(row),
            )
            .context("Select job by name and scheduled at")?;
        Ok(job)
    }

    async fn get_jobs_to_execute(&mut self) -> Result<Vec<Job>> {
        let jobs = self
            .raw()
            .prepare(
                "SELECT * FROM jobs WHERE scheduled_at <= datetime('now')
                    AND (error_message IS NULL OR executed_at <= datetime('now', '-60 minutes'))",
            )?
            .query_map([], |row| deserialize_job(row))?
            .collect::<std::result::Result<_, rusqlite::Error>>()?;
        Ok(jobs)
    }

    async fn lock_and_load_issue_data(
        &mut self,
        repo: &str,
        issue_number: i32,
        key: &str,
    ) -> Result<(Box<dyn Transaction + '_>, Option<serde_json::Value>)> {
        let transaction = self.raw_transaction();
        let data = match transaction
            .conn
            .raw()
            .prepare(
                "SELECT data FROM issue_data WHERE \
                 repo = ? AND issue_number = ? AND key = ?",
            )?
            .query_row(params![repo, issue_number, key], |row| row.get(0))
        {
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(e) => return Err(e.into()),
            Ok(d) => Some(d),
        };
        Ok((Box::new(transaction), data))
    }

    async fn save_issue_data(
        &mut self,
        repo: &str,
        issue_number: i32,
        key: &str,
        data: &serde_json::Value,
    ) -> Result<()> {
        self.raw()
            .execute(
                "INSERT INTO issue_data (repo, issue_number, key, data) \
                 VALUES (?, ?, ?, ?) \
                 ON CONFLICT (repo, issue_number, key) DO UPDATE SET data=EXCLUDED.data",
                params![repo, issue_number, key, data],
            )
            .context("inserting issue data")?;
        Ok(())
    }
}

fn assert_sync<T: Sync>() {}

impl SqliteConnection {
    pub fn new(conn: ManagedConnection<Mutex<rusqlite::Connection>>) -> Self {
        assert_sync::<Self>();
        Self { conn }
    }

    pub fn raw(&mut self) -> &mut rusqlite::Connection {
        self.conn.get_mut().unwrap_or_else(|e| e.into_inner())
    }
    pub fn raw_ref(&self) -> std::sync::MutexGuard<rusqlite::Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn raw_transaction(&mut self) -> SqliteTransaction<'_> {
        self.raw().execute_batch("BEGIN DEFERRED").unwrap();
        SqliteTransaction {
            conn: self,
            finished: false,
        }
    }
}

fn deserialize_job(row: &rusqlite::Row<'_>) -> std::result::Result<Job, rusqlite::Error> {
    Ok(Job {
        id: row.get(0)?,
        name: row.get(1)?,
        scheduled_at: row.get(2)?,
        metadata: row.get(3)?,
        executed_at: row.get(4)?,
        error_message: row.get(5)?,
    })
}
