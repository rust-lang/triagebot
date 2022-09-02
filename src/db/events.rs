//! The `events` table provides a way to have scheduled events

use anyhow::{Result};
use chrono::{DateTime, FixedOffset};
use tokio_postgres::{Client as DbClient};
use uuid::Uuid;

#[derive(Debug)]
pub struct Event {
    pub event_id: Uuid,
    pub event_name: String,
    pub expected_event_time: DateTime<FixedOffset>,
    pub event_metadata: String,
    pub executed_at: DateTime<FixedOffset>,
    pub failed: Option<String>,
}

pub async fn insert_failed(db: &DbClient) -> Result<()> {
    unimplemented!();
}

pub async fn delete_event(db: &DbClient) -> Result<()> {
    unimplemented!();
}

pub async fn get_events_to_execute(db: &DbClient) -> Result<Vec<Event>> {
    let events = db
        .query(
            "
        SELECT * FROM events",
            &[],
        )
        .await
        .unwrap();

    Ok(vec![])
}
