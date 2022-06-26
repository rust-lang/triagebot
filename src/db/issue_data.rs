//! The `issue_data` table provides a way to track extra metadata about an
//! issue/PR.
//!
//! Each issue has a unique "key" where you can store data under. Typically
//! that key should be the name of the handler. The data can be anything that
//! can be serialized to JSON.

use crate::github::Issue;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio_postgres::types::Json;
use tokio_postgres::Client as DbClient;

pub async fn load<T: for<'a> Deserialize<'a>>(
    db: &DbClient,
    issue: &Issue,
    key: &str,
) -> Result<Option<T>> {
    let repo = issue.repository().to_string();
    let data = db
        .query_opt(
            "SELECT data FROM issue_data WHERE \
            repo = $1 AND issue_number = $2 AND key = $3",
            &[&repo, &(issue.number as i32), &key],
        )
        .await
        .context("selecting issue data")?
        .map(|row| row.get::<usize, Json<T>>(0).0);
    Ok(data)
}

pub async fn save<T: Serialize + std::fmt::Debug + Sync>(
    db: &DbClient,
    issue: &Issue,
    key: &str,
    data: &T,
) -> Result<()> {
    let repo = issue.repository().to_string();
    db.execute(
        "INSERT INTO issue_data (repo, issue_number, key, data) VALUES ($1, $2, $3, $4) \
         ON CONFLICT (repo, issue_number, key) DO UPDATE SET data=EXCLUDED.data",
        &[&repo, &(issue.number as i32), &key, &Json(data)],
    )
    .await
    .context("inserting issue data")?;
    Ok(())
}
