//! The `events` table provides a way to have scheduled events
use anyhow::{Result, Context as _};
use chrono::{DateTime, FixedOffset};
use tokio_postgres::{Client as DbClient};
use uuid::Uuid;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub struct Event {
    pub event_id: Uuid,
    pub event_name: String,
    pub expected_event_time: DateTime<FixedOffset>,
    pub event_metadata: serde_json::Value,
    pub executed_at: DateTime<FixedOffset>,
    pub failed: Option<String>,
}

pub async fn insert_event(db: &DbClient, event: &Event) -> Result<()> {
    tracing::trace!("insert_event(id={})", event.event_id);
    
    db.execute(
        "INSERT INTO events (event_id, event_name, expected_event_time, event_metadata, executed_at, failed) VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT DO NOTHING",
        &[&event.event_id, &event.event_name, &event.expected_event_time, &"", &event.executed_at, &event.failed],
    )
    .await
    .context("inserting event")?;

    Ok(())
}

pub async fn delete_event(db: &DbClient, event_id: &Uuid) -> Result<()> {
    tracing::trace!("delete_event(id={})", event_id);
    
    db.execute(
        "DELETE FROM events WHERE event_id = $1",
        &[&event_id],
    )
    .await
    .context("deleting event")?;

    Ok(())
}

pub async fn update_event_failed_message(db: &DbClient, event_id: &Uuid, message: &String) -> Result<()> {
    tracing::trace!("update_event_failed_message(id={})", event_id);
    
    db.execute(
        "UPDATE events SET failed = $2 WHERE event_id = $1",
        &[&event_id, &message],
    )
    .await
    .context("updating event failed message")?;

    Ok(())
}

pub async fn update_event_executed_at(db: &DbClient, event_id: &Uuid) -> Result<()> {
    tracing::trace!("update_event_executed_at(id={})", event_id);
    
    db.execute(
        "UPDATE events SET executed_at = now() WHERE event_id = $1",
        &[&event_id],
    )
    .await
    .context("updating event executed at")?;

    Ok(())
}

// Selects all events with:
//  - event_time's in the past 
//  - failed is null or executed_at is at least 60 minutes ago (intended to make repeat executions rare enough)
pub async fn get_events_to_execute(db: &DbClient) -> Result<Vec<Event>>  {
    let events = db
        .query(
            "
        SELECT * FROM events WHERE expected_event_time <= now() AND (failed IS NULL OR executed_at <= now() - INTERVAL '60 minutes')",
            &[],
        )
        .await
        .context("Getting events data")?;

    let mut data = Vec::with_capacity(events.len());
    for event in events {
        let event_id: Uuid = event.get(0);
        let event_name: String = event.get(1);
        let expected_event_time: DateTime<FixedOffset> = event.get(2);
        let event_metadata: serde_json::Value = event.get(3);
        let executed_at: DateTime<FixedOffset> = event.get(4);
        let failed: Option<String> = event.get(5);

        data.push(Event {
            event_id,
            event_name,
            expected_event_time,
            event_metadata,
            executed_at,
            failed
        });
    }

    Ok(data)
}
