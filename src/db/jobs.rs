//! The `jobs` table provides a way to have scheduled jobs

use crate::db::Connection;
use crate::handlers::jobs::handle_job;
use crate::handlers::Context;
use anyhow::Result;
use chrono::DateTime;
use chrono::Utc;
use cron::Schedule;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub struct JobSchedule {
    pub name: String,
    pub schedule: Schedule,
    pub metadata: serde_json::Value,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Job {
    pub id: Uuid,
    pub name: String,
    pub scheduled_at: DateTime<Utc>,
    pub metadata: serde_json::Value,
    pub executed_at: Option<DateTime<Utc>>,
    pub error_message: Option<String>,
}

pub async fn schedule_jobs(connection: &mut dyn Connection, jobs: Vec<JobSchedule>) -> Result<()> {
    for job in jobs {
        let mut upcoming = job.schedule.upcoming(Utc).take(1);

        if let Some(scheduled_at) = upcoming.next() {
            if let Err(_) = connection
                .get_job_by_name_and_scheduled_at(&job.name, &scheduled_at)
                .await
            {
                // mean there's no job already in the db with that name and scheduled_at
                connection
                    .insert_job(&job.name, &scheduled_at, &job.metadata)
                    .await?;
            }
        }
    }

    Ok(())
}

pub async fn run_scheduled_jobs(ctx: &Context, connection: &mut dyn Connection) -> Result<()> {
    let jobs = connection.get_jobs_to_execute().await.unwrap();
    tracing::trace!("jobs to execute: {:#?}", jobs);

    for job in jobs.iter() {
        connection.update_job_executed_at(&job.id).await?;

        match handle_job(&ctx, &job.name, &job.metadata).await {
            Ok(_) => {
                tracing::trace!("job successfully executed (id={})", job.id);
                connection.delete_job(&job.id).await?;
            }
            Err(e) => {
                tracing::error!("job failed on execution (id={:?}, error={:?})", job.id, e);
                connection
                    .update_job_error_message(&job.id, &e.to_string())
                    .await?;
            }
        }
    }

    Ok(())
}
