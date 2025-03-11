use crate::github::UserId;
use anyhow::Context;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ReviewPrefs {
    pub id: uuid::Uuid,
    pub user_id: i64,
    pub max_assigned_prs: Option<i32>,
}

impl From<tokio_postgres::row::Row> for ReviewPrefs {
    fn from(row: tokio_postgres::row::Row) -> Self {
        Self {
            id: row.get("id"),
            user_id: row.get("user_id"),
            max_assigned_prs: row.get("max_assigned_prs"),
        }
    }
}

/// Get team member review preferences.
/// If they are missing, returns `Ok(None)`.
pub async fn get_review_prefs(
    db: &tokio_postgres::Client,
    user_id: UserId,
) -> anyhow::Result<Option<ReviewPrefs>> {
    let query = "
SELECT id, user_id, max_assigned_prs
FROM review_prefs
WHERE review_prefs.user_id = $1;";
    let row = db
        .query_opt(query, &[&(user_id as i64)])
        .await
        .context("Error retrieving review preferences")?;
    Ok(row.map(|r| r.into()))
}
