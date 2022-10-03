//! The `jobs` table provides a way to have scheduled jobs
use anyhow::{Context as _, Result};
use chrono::{DateTime, Duration, FixedOffset};
use postgres_types::{FromSql, ToSql};
use serde::{Deserialize, Serialize};
use tokio_postgres::Client as DbClient;
use uuid::Uuid;

#[derive(Serialize, Deserialize, Debug)]
pub struct Job {
    pub id: Uuid,
    pub name: String,
    pub expected_time: DateTime<FixedOffset>,
    pub frequency: Option<i32>,
    pub frequency_unit: Option<FrequencyUnit>,
    pub metadata: serde_json::Value,
    pub executed_at: Option<DateTime<FixedOffset>>,
    pub error_message: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, ToSql, FromSql)]
#[postgres(name = "frequency_unit")]
pub enum FrequencyUnit {
    #[postgres(name = "days")]
    Days,
    #[postgres(name = "hours")]
    Hours,
    #[postgres(name = "minutes")]
    Minutes,
    #[postgres(name = "seconds")]
    Seconds,
}

pub async fn insert_job(
    db: &DbClient,
    name: &String,
    expected_time: &DateTime<FixedOffset>,
    frequency: &Option<i32>,
    frequency_unit: &Option<FrequencyUnit>,
    metadata: &serde_json::Value,
) -> Result<()> {
    tracing::trace!("insert_job(name={})", name);

    db.execute(
        "INSERT INTO jobs (name, expected_time, frequency, frequency_unit, metadata) VALUES ($1, $2, $3, $4, $5) 
            ON CONFLICT (name, expected_time) DO UPDATE SET metadata = EXCLUDED.metadata",
        &[&name, &expected_time, &frequency, &frequency_unit, &metadata],
    )
    .await
    .context("Inserting job")?;

    Ok(())
}

pub async fn delete_job(db: &DbClient, id: &Uuid) -> Result<()> {
    tracing::trace!("delete_job(id={})", id);

    db.execute("DELETE FROM jobs WHERE id = $1", &[&id])
        .await
        .context("Deleting job")?;

    Ok(())
}

pub async fn update_job_error_message(db: &DbClient, id: &Uuid, message: &String) -> Result<()> {
    tracing::trace!("update_job_error_message(id={})", id);

    db.execute(
        "UPDATE jobs SET error_message = $2 WHERE id = $1",
        &[&id, &message],
    )
    .await
    .context("Updating job error message")?;

    Ok(())
}

pub async fn update_job_executed_at(db: &DbClient, id: &Uuid) -> Result<()> {
    tracing::trace!("update_job_executed_at(id={})", id);

    db.execute("UPDATE jobs SET executed_at = now() WHERE id = $1", &[&id])
        .await
        .context("Updating job executed at")?;

    Ok(())
}

// Selects all jobs with:
//  - expected_time in the past
//  - error_message is null or executed_at is at least 60 minutes ago (intended to make repeat executions rare enough)
pub async fn get_jobs_to_execute(db: &DbClient) -> Result<Vec<Job>> {
    let jobs = db
        .query(
            "
        SELECT * FROM jobs WHERE expected_time <= now() AND (error_message IS NULL OR executed_at <= now() - INTERVAL '60 minutes')",
            &[],
        )
        .await
        .context("Getting jobs data")?;

    let mut data = Vec::with_capacity(jobs.len());
    for job in jobs {
        let id: Uuid = job.get(0);
        let name: String = job.get(1);
        let expected_time: DateTime<FixedOffset> = job.get(2);
        let frequency: Option<i32> = job.get(3);
        let frequency_unit: Option<FrequencyUnit> = job.get(4);
        let metadata: serde_json::Value = job.get(5);
        let executed_at: Option<DateTime<FixedOffset>> = job.get(6);
        let error_message: Option<String> = job.get(7);

        data.push(Job {
            id,
            name,
            expected_time,
            frequency,
            frequency_unit,
            metadata,
            executed_at,
            error_message,
        });
    }

    Ok(data)
}

pub fn get_duration_from_cron(cron_period: i32, cron_unit: &FrequencyUnit) -> Duration {
    match cron_unit {
        FrequencyUnit::Days => Duration::days(cron_period as i64),
        FrequencyUnit::Hours => Duration::hours(cron_period as i64),
        FrequencyUnit::Minutes => Duration::minutes(cron_period as i64),
        FrequencyUnit::Seconds => Duration::seconds(cron_period as i64),
    }
}
