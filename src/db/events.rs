//! The `events` table provides a way to have scheduled events

use anyhow::{Context as _, Result};
use chrono::{DateTime, FixedOffset};
use tokio_postgres::{Client as DbClient, Transaction};
use uuid::Uuid;

/*
event_id (uuid or equivalent)
event_name (text, one of a set of constants, most likely, indicating which handler to invoke for the event)
expected_event_time (datetime, UTC)
event_metadata (text, JSON-encoded free-form metadata)
executed_at (timestamp)
failed (text, can be null)
*/

#[derive(Debug)]
pub struct Event {
    // pub transaction: Transaction<'db>,
    // uuid or equivalent
    pub event_id: Uuid,
    // One of a set of constants, indicating which handler to invoke
    pub event_name: String,
    // UTC
    pub expected_event_time: DateTime<FixedOffset>,
    // JSON-encoded free-form metadata
    pub event_metadata: String,

    pub executed_at: DateTime<FixedOffset>,

    pub failed: Option<String>,
}

impl Event {
    pub async fn insert_failed(db: &DbClient) -> Result<()> {
        unimplemented!();
    }

    pub async fn delete_event(db: &DbClient) -> Result<()> {
        unimplemented!();
    }
}

pub async fn get_events(db: &DbClient) -> Result<Vec<Event>> {
    let events = db
        .query(
            "
        SELECT * FROM events",
            &[],
        )
        .await
        .unwrap();
    println!("{:#?}", events);
    // events.into_iter().map(|row| Event {}).collect()
    Ok(vec![])
}
