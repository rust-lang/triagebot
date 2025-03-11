use crate::db::users::record_username;
use crate::github::{User, UserId};
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

/// Updates review preferences of the specified user, or creates them
/// if they do not exist yet.
pub async fn upsert_review_prefs(
    db: &tokio_postgres::Client,
    user: User,
    max_assigned_prs: Option<u32>,
) -> anyhow::Result<u64, anyhow::Error> {
    // We need to have the user stored in the DB to have a valid FK link in review_prefs
    record_username(db, user.id, &user.login).await?;

    let max_assigned_prs = max_assigned_prs.map(|v| v as i32);
    let query = "
INSERT INTO review_prefs(user_id, max_assigned_prs)
VALUES ($1, $2)
ON CONFLICT (user_id)
DO UPDATE
SET max_assigned_prs = excluded.max_assigned_prs";

    let res = db
        .execute(query, &[&(user.id as i64), &max_assigned_prs])
        .await
        .context("Error upserting review preferences")?;
    Ok(res)
}

#[cfg(test)]
mod tests {
    use crate::db::review_prefs::{get_review_prefs, upsert_review_prefs};
    use crate::db::users::get_user;
    use crate::tests::github::user;
    use crate::tests::run_test;

    #[tokio::test]
    async fn insert_prefs_create_user() {
        run_test(|ctx| async {
            let db = ctx.db_client().await;

            let user = user("Martin", 1);
            upsert_review_prefs(&db, user.clone(), Some(1)).await?;

            assert_eq!(get_user(&db, user.id).await?.unwrap(), user);

            Ok(ctx)
        })
        .await;
    }

    #[tokio::test]
    async fn insert_max_assigned_prs() {
        run_test(|ctx| async {
            let db = ctx.db_client().await;

            upsert_review_prefs(&db, user("Martin", 1), Some(5)).await?;
            assert_eq!(
                get_review_prefs(&db, 1).await?.unwrap().max_assigned_prs,
                Some(5)
            );

            Ok(ctx)
        })
        .await;
    }

    #[tokio::test]
    async fn update_max_assigned_prs() {
        run_test(|ctx| async {
            let db = ctx.db_client().await;

            upsert_review_prefs(&db, user("Martin", 1), Some(5)).await?;
            assert_eq!(
                get_review_prefs(&db, 1).await?.unwrap().max_assigned_prs,
                Some(5)
            );
            upsert_review_prefs(&db, user("Martin", 1), Some(10)).await?;
            assert_eq!(
                get_review_prefs(&db, 1).await?.unwrap().max_assigned_prs,
                Some(10)
            );

            upsert_review_prefs(&db, user("Martin", 1), None).await?;
            assert_eq!(
                get_review_prefs(&db, 1).await?.unwrap().max_assigned_prs,
                None
            );

            Ok(ctx)
        })
        .await;
    }
}
