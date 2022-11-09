use crate::handlers::jobs::handle_job;
use crate::{db::jobs::*, handlers::Context};
use anyhow::Context as _;
use chrono::Utc;
use native_tls::{Certificate, TlsConnector};
use postgres_native_tls::MakeTlsConnector;
use std::sync::{Arc, Mutex};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_postgres::Client as DbClient;

pub mod issue_data;
pub mod issue_decision_state;
pub mod jobs;
pub mod notifications;
pub mod rustc_commits;

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

pub struct ClientPool {
    connections: Arc<Mutex<Vec<tokio_postgres::Client>>>,
    permits: Arc<Semaphore>,
}

pub struct PooledClient {
    client: Option<tokio_postgres::Client>,
    #[allow(unused)] // only used for drop impl
    permit: OwnedSemaphorePermit,
    pool: Arc<Mutex<Vec<tokio_postgres::Client>>>,
}

impl Drop for PooledClient {
    fn drop(&mut self) {
        let mut clients = self.pool.lock().unwrap_or_else(|e| e.into_inner());
        clients.push(self.client.take().unwrap());
    }
}

impl std::ops::Deref for PooledClient {
    type Target = tokio_postgres::Client;

    fn deref(&self) -> &Self::Target {
        self.client.as_ref().unwrap()
    }
}

impl std::ops::DerefMut for PooledClient {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.client.as_mut().unwrap()
    }
}

impl ClientPool {
    pub fn new() -> ClientPool {
        ClientPool {
            connections: Arc::new(Mutex::new(Vec::with_capacity(16))),
            permits: Arc::new(Semaphore::new(16)),
        }
    }

    pub async fn get(&self) -> PooledClient {
        let permit = self.permits.clone().acquire_owned().await.unwrap();
        {
            let mut slots = self.connections.lock().unwrap_or_else(|e| e.into_inner());
            // Pop connections until we hit a non-closed connection (or there are no
            // "possibly open" connections left).
            while let Some(c) = slots.pop() {
                if !c.is_closed() {
                    return PooledClient {
                        client: Some(c),
                        permit,
                        pool: self.connections.clone(),
                    };
                }
            }
        }

        PooledClient {
            client: Some(make_client().await.unwrap()),
            permit,
            pool: self.connections.clone(),
        }
    }
}

async fn make_client() -> anyhow::Result<tokio_postgres::Client> {
    let db_url = std::env::var("DATABASE_URL").expect("needs DATABASE_URL");
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
        tokio::task::spawn(async move {
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

pub async fn schedule_jobs(db: &DbClient, jobs: Vec<JobSchedule>) -> anyhow::Result<()> {
    for job in jobs {
        let mut upcoming = job.schedule.upcoming(Utc).take(1);

        if let Some(scheduled_at) = upcoming.next() {
            if let Err(_) = get_job_by_name_and_scheduled_at(&db, &job.name, &scheduled_at).await {
                // mean there's no job already in the db with that name and scheduled_at
                insert_job(&db, &job.name, &scheduled_at, &job.metadata).await?;
            }
        }
    }

    Ok(())
}

pub async fn run_scheduled_jobs(ctx: &Context, db: &DbClient) -> anyhow::Result<()> {
    let jobs = get_jobs_to_execute(&db).await.unwrap();
    tracing::trace!("jobs to execute: {:#?}", jobs);

    for job in jobs.iter() {
        update_job_executed_at(&db, &job.id).await?;

        match handle_job(&ctx, &job.name, &job.metadata).await {
            Ok(_) => {
                tracing::trace!("job successfully executed (id={})", job.id);
                delete_job(&db, &job.id).await?;
            }
            Err(e) => {
                tracing::error!("job failed on execution (id={:?}, error={:?})", job.id, e);
                update_job_error_message(&db, &job.id, &e.to_string()).await?;
            }
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
    "
CREATE TYPE reversibility AS ENUM ('reversible', 'irreversible');
",
    "
CREATE TYPE resolution AS ENUM ('hold', 'merge');
",
    "CREATE TABLE issue_decision_state (
    issue_id BIGINT PRIMARY KEY,
    initiator TEXT NOT NULL,
    start_date TIMESTAMP WITH TIME ZONE NOT NULL,
    end_date TIMESTAMP WITH TIME ZONE NOT NULL,
    current JSONB NOT NULL,
    history JSONB,
    reversibility reversibility NOT NULL DEFAULT 'reversible',
    resolution resolution NOT NULL DEFAULT 'merge'
);",
];
