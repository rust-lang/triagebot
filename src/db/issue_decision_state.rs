//! The issue decision state table provides a way to store
//! the decision process state of each issue

use anyhow::{Context as _, Result};
use chrono::{DateTime, Utc};
use parser::command::decision::{Resolution, Reversibility};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tokio_postgres::Client as DbClient;

#[derive(Debug, Serialize, Deserialize)]
pub struct IssueDecisionState {
    pub issue_id: i64,
    pub initiator: String,
    pub start_date: DateTime<Utc>,
    pub end_date: DateTime<Utc>,
    pub current: BTreeMap<String, Option<UserStatus>>,
    pub history: BTreeMap<String, Vec<UserStatus>>,
    pub reversibility: Reversibility,
    pub resolution: Resolution,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserStatus {
    pub comment_id: String,
    pub text: String,
    pub reversibility: Reversibility,
    pub resolution: Resolution,
}

pub async fn insert_issue_decision_state(
    db: &DbClient,
    issue_number: &u64,
    initiator: &String,
    start_date: &DateTime<Utc>,
    end_date: &DateTime<Utc>,
    current: &BTreeMap<String, Option<UserStatus>>,
    history: &BTreeMap<String, Vec<UserStatus>>,
    reversibility: &Reversibility,
    resolution: &Resolution,
) -> Result<()> {
    tracing::trace!("insert_issue_decision_state(issue_id={})", issue_number);
    let issue_id = *issue_number as i64;

    db.execute(
        "INSERT INTO issue_decision_state (issue_id, initiator, start_date, end_date, current, history, reversibility, resolution) VALUES ($1, $2, $3, $4, $5, $6, $7, $8) 
            ON CONFLICT DO NOTHING",
        &[&issue_id, &initiator, &start_date, &end_date, &serde_json::to_value(current).unwrap(), &serde_json::to_value(history).unwrap(), &reversibility, &resolution],
    )
    .await
    .context("Inserting decision state")?;

    Ok(())
}

pub async fn update_issue_decision_state(
    db: &DbClient,
    issue_number: &u64,
    end_date: &DateTime<Utc>,
    current: &BTreeMap<String, UserStatus>,
    history: &BTreeMap<String, Vec<UserStatus>>,
    reversibility: &Reversibility,
    resolution: &Resolution,
) -> Result<()> {
    tracing::trace!("update_issue_decision_state(issue_id={})", issue_number);
    let issue_id = *issue_number as i64;

    db.execute("UPDATE issue_decision_state SET end_date = $2, current = $3, history = $4, reversibility = $5, resolution = $6 WHERE issue_id = $1", 
        &[&issue_id, &end_date, &serde_json::to_value(current).unwrap(), &serde_json::to_value(history).unwrap(), &reversibility, &resolution]
    )
    .await
    .context("Updating decision state")?;

    Ok(())
}

pub async fn get_issue_decision_state(
    db: &DbClient,
    issue_number: &u64,
) -> Result<IssueDecisionState> {
    tracing::trace!("get_issue_decision_state(issue_id={})", issue_number);
    let issue_id = *issue_number as i64;

    let state = db
        .query_one(
            "SELECT * FROM issue_decision_state WHERE issue_id = $1",
            &[&issue_id],
        )
        .await
        .context("Getting decision state data")?;

    deserialize_issue_decision_state(&state)
}

fn deserialize_issue_decision_state(row: &tokio_postgres::row::Row) -> Result<IssueDecisionState> {
    let issue_id: i64 = row.try_get(0)?;
    let initiator: String = row.try_get(1)?;
    let start_date: DateTime<Utc> = row.try_get(2)?;
    let end_date: DateTime<Utc> = row.try_get(3)?;
    let current: BTreeMap<String, Option<UserStatus>> =
        serde_json::from_value(row.try_get(4).unwrap())?;
    let history: BTreeMap<String, Vec<UserStatus>> =
        serde_json::from_value(row.try_get(5).unwrap())?;
    let reversibility: Reversibility = row.try_get(6)?;
    let resolution: Resolution = row.try_get(7)?;

    Ok(IssueDecisionState {
        issue_id,
        initiator,
        start_date,
        end_date,
        current,
        history,
        reversibility,
        resolution,
    })
}
