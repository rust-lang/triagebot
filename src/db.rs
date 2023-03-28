use self::jobs::Job;
use self::notifications::{Identifier, Notification, NotificationData};
use anyhow::Result;
use chrono::{DateTime, FixedOffset, Utc};
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use uuid::Uuid;

pub mod issue_data;
pub mod jobs;
pub mod notifications;
pub mod postgres;
pub mod sqlite;

/// A bors merge commit.
#[derive(Debug, Serialize)]
pub struct Commit {
    pub sha: String,
    pub parent_sha: String,
    pub time: DateTime<FixedOffset>,
    pub pr: Option<u32>,
}

#[async_trait::async_trait]
pub trait Connection: Send + Sync {
    async fn transaction(&mut self) -> Box<dyn Transaction + '_>;

    // Pings
    async fn record_username(&mut self, user_id: i64, username: String) -> Result<()>;
    async fn record_ping(&mut self, notification: &Notification) -> Result<()>;

    // Rustc commits
    async fn get_missing_commits(&mut self) -> Result<Vec<String>>;
    async fn record_commit(&mut self, commit: &Commit) -> Result<()>;
    async fn has_commit(&mut self, sha: &str) -> Result<bool>;
    async fn get_commits_with_artifacts(&mut self) -> Result<Vec<Commit>>;

    // Notifications
    async fn get_notifications(&mut self, username: &str) -> Result<Vec<NotificationData>>;
    async fn delete_ping(
        &mut self,
        user_id: i64,
        identifier: Identifier<'_>,
    ) -> Result<Vec<NotificationData>>;
    async fn add_metadata(
        &mut self,
        user_id: i64,
        idx: usize,
        metadata: Option<&str>,
    ) -> Result<()>;
    async fn move_indices(&mut self, user_id: i64, from: usize, to: usize) -> Result<()>;

    // Jobs
    async fn insert_job(
        &mut self,
        name: &str,
        scheduled_at: &DateTime<Utc>,
        metadata: &serde_json::Value,
    ) -> Result<()>;
    async fn delete_job(&mut self, id: &Uuid) -> Result<()>;
    async fn update_job_error_message(&mut self, id: &Uuid, message: &str) -> Result<()>;
    async fn update_job_executed_at(&mut self, id: &Uuid) -> Result<()>;
    async fn get_job_by_name_and_scheduled_at(
        &mut self,
        name: &str,
        scheduled_at: &DateTime<Utc>,
    ) -> Result<Job>;
    async fn get_jobs_to_execute(&mut self) -> Result<Vec<Job>>;

    // Issue data
    async fn lock_and_load_issue_data(
        &mut self,
        repo: &str,
        issue_number: i32,
        key: &str,
    ) -> Result<(Box<dyn Transaction + '_>, Option<serde_json::Value>)>;
    async fn save_issue_data(
        &mut self,
        repo: &str,
        issue_number: i32,
        key: &str,
        data: &serde_json::Value,
    ) -> Result<()>;
}

#[async_trait::async_trait]
pub trait Transaction: Send + Sync {
    fn conn(&mut self) -> &mut dyn Connection;
    fn conn_ref(&self) -> &dyn Connection;

    async fn commit(self: Box<Self>) -> Result<(), anyhow::Error>;
    async fn finish(self: Box<Self>) -> Result<(), anyhow::Error>;
}

#[async_trait::async_trait]
pub trait ConnectionManager {
    type Connection;
    async fn open(&self) -> Self::Connection;
    async fn is_valid(&self, c: &mut Self::Connection) -> bool;
}

pub struct ConnectionPool<M: ConnectionManager> {
    connections: Arc<Mutex<Vec<M::Connection>>>,
    permits: Arc<Semaphore>,
    manager: M,
}

pub struct ManagedConnection<T> {
    conn: Option<T>,
    connections: Arc<Mutex<Vec<T>>>,
    #[allow(unused)]
    permit: OwnedSemaphorePermit,
}

impl<T> std::ops::Deref for ManagedConnection<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.conn.as_ref().unwrap()
    }
}
impl<T> std::ops::DerefMut for ManagedConnection<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.conn.as_mut().unwrap()
    }
}

impl<T> Drop for ManagedConnection<T> {
    fn drop(&mut self) {
        let conn = self.conn.take().unwrap();
        self.connections
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(conn);
    }
}

impl<T, M> ConnectionPool<M>
where
    T: Send,
    M: ConnectionManager<Connection = T>,
{
    fn new(manager: M) -> Self {
        ConnectionPool {
            connections: Arc::new(Mutex::new(Vec::with_capacity(16))),
            permits: Arc::new(Semaphore::new(16)),
            manager,
        }
    }

    pub fn raw(&mut self) -> &mut M {
        &mut self.manager
    }

    async fn get(&self) -> ManagedConnection<T> {
        let permit = self.permits.clone().acquire_owned().await.unwrap();
        let conn = {
            let mut slots = self.connections.lock().unwrap_or_else(|e| e.into_inner());
            slots.pop()
        };
        if let Some(mut c) = conn {
            if self.manager.is_valid(&mut c).await {
                return ManagedConnection {
                    conn: Some(c),
                    permit,
                    connections: self.connections.clone(),
                };
            }
        }

        let conn = self.manager.open().await;
        ManagedConnection {
            conn: Some(conn),
            connections: self.connections.clone(),
            permit,
        }
    }
}

pub enum Pool {
    Sqlite(ConnectionPool<sqlite::Sqlite>),
    Postgres(ConnectionPool<postgres::Postgres>),
}

impl Pool {
    pub async fn connection(&self) -> Box<dyn Connection> {
        match self {
            Pool::Sqlite(p) => Box::new(sqlite::SqliteConnection::new(p.get().await)),
            Pool::Postgres(p) => Box::new(p.get().await),
        }
    }

    pub fn open(uri: &str) -> Pool {
        if uri.starts_with("postgres") {
            Pool::Postgres(ConnectionPool::new(postgres::Postgres::new(uri.into())))
        } else {
            Pool::Sqlite(ConnectionPool::new(sqlite::Sqlite::new(uri.into())))
        }
    }

    pub fn new_from_env() -> Pool {
        Self::open(&std::env::var("DATABASE_URL").expect("needs DATABASE_URL"))
    }
}
