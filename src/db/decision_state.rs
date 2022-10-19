//! The decision state table provides a way to store the state of each issue

use serde::{Deserialize, Serialize};
use chrono::{DateTime, FixedOffset};
use std::collections::HashMap;
use parser::command::decision::{Resolution, Reversibility};
use anyhow::{Context as _, Result};
use tokio_postgres::Client as DbClient;

#[derive(Debug, Serialize, Deserialize)]
pub struct DecisionState {
    issue_id: String,
    initiator: String,
    team_members: Vec<String>,
    start_date: DateTime<FixedOffset>,
    period_date: DateTime<FixedOffset>,
    current_statuses: HashMap<String, UserStatus>,
    status_history: HashMap<String, Vec<UserStatus>>,
    reversibility: Reversibility,
    resolution: Resolution,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserStatus {
    name: String,
    issue_id: String,
    comment_id: String,
}

pub async fn insert_decision_state(
    db: &DbClient,
    issue_id: &String,
    initiator: &String,
    team_members: &Vec<String>,
    start_date: &DateTime<FixedOffset>,
    period_end_date: &DateTime<FixedOffset>,
    current_statuses: &HashMap<String, UserStatus>,
    status_history: &HashMap<String, Vec<UserStatus>>,
    reversibility: &Reversibility,
    resolution: &Resolution,
) -> Result<()> {
    tracing::trace!("insert_decision_state(issue_id={})", issue_id);

    db.execute(
        "INSERT INTO decision_state (issue_id, initiator, team_members, start_date, period_end_date, current_statuses, status_history, reversibility, resolution) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) 
            ON CONFLICT issue_id DO NOTHING",
        &[&issue_id, &initiator, &team_members, &start_date, &period_end_date, &current_statuses, &status_history, &reversibility, &resolution],
    )
    .await
    .context("Inserting decision state")?;

    Ok(())
}

// pub async fn update_decision_state(
//   db: &DbClient, 
//   issue_id: &String,
//   period_end_date: &DateTime<FixedOffset>,
//   current_statuses: &HashMap<String, UserStatus>,
//   status_history: &HashMap<String, Vec<UserStatus>>,
//   reversibility: &Reversibility,
//   resolution: &Resolution
// ) -> Result<()> {
//     tracing::trace!("update_decision_state(issue_id={})", issue_id);

//     db.execute("UPDATE decision_state SET period_end_date = $2, current_statuses = $3, status_history = $4, reversibility = $5, resolution = $6 WHERE issue_id = $1", 
//       &[&issue_id, &period_end_date, &current_statuses, &status_history, &reversibility, &resolution]
//     )
//     .await
//     .context("Updating decision state")?;

//     Ok(())
// }

// pub async fn get_decision_state_for_issue(db: &DbClient, issue_id: &String) -> Result<DecisionState> {
//     let state = db
//       .query(
//         "SELECT * FROM decision_state WHERE issue_id = $1",
//         &[&issue_id]
//       )
//       .await
//       .context("Getting decision state data")?;

    
//     let issue_id: String = state.get(0);
//     let initiator: String = state.get(1);
//     let team_members: Vec<String> = state.get(2);
//     let start_date: DateTime<FixedOffset> = state.get(3);
//     let period_date: DateTime<FixedOffset> = state.get(4);
//     let current_statuses: HashMap<String, UserStatus> = state.get(5);
//     let status_history: HashMap<String, Vec<UserStatus>> = state.get(6);
//     let reversibility: Reversibility = state.get(7);
//     let resolution: Resolution = state.get(8);

//     Ok(
//       DecisionState {
//         issue_id,
//         initiator,
//         team_members,
//         start_date,
//         period_date,
//         current_statuses,
//         status_history,
//         reversibility,
//         resolution,
//       }
//     )
// }
