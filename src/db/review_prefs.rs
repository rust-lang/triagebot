use crate::db::users::record_username;
use crate::github::{User, UserId};
use anyhow::Context;
use serde::Serialize;
use std::collections::HashMap;

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

/// Returns a set of review preferences for all passed usernames.
/// Usernames are matched regardless of case.
///
/// Usernames that are not present in the resulting map have no review preferences configured
/// in the database.
pub async fn get_review_prefs_batch<'a>(
    db: &tokio_postgres::Client,
    users: &[&'a str],
) -> anyhow::Result<HashMap<&'a str, ReviewPrefs>> {
    // We need to make sure that we match users regardless of case, but at the
    // same time we need to return the originally-cased usernames in the final hashmap.
    // At the same time, we can't depend on the order of results returned by the DB.
    // So we need to do some additional bookkeeping here.
    let lowercase_map: HashMap<String, &str> = users
        .iter()
        .map(|name| (name.to_lowercase(), *name))
        .collect();
    let lowercase_users: Vec<&str> = lowercase_map.keys().map(|s| s.as_str()).collect();

    // The id/user_id/max_assigned_prs columns have to match the names used in
    // `From<tokio_postgres::row::Row> for ReviewPrefs`.
    let query = "
SELECT lower(u.username) AS username, r.id AS id, r.user_id AS user_id, r.max_assigned_prs AS max_assigned_prs
FROM review_prefs AS r
JOIN users AS u ON u.user_id = r.user_id
WHERE lower(u.username) = ANY($1);";

    Ok(db
        .query(query, &[&lowercase_users])
        .await
        .context("Error retrieving review preferences from usernames")?
        .into_iter()
        .map(|row| {
            // Map back from the lowercase username to the original username.
            let username_lower: &str = row.get("username");
            let username = lowercase_map
                .get(username_lower)
                .expect("Lowercase username not found");
            let prefs: ReviewPrefs = row.into();
            (*username, prefs)
        })
        .collect())
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
    use crate::tests::run_db_test;

    #[tokio::test]
    async fn insert_prefs_create_user() {
        run_db_test(|ctx| async {
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
        run_db_test(|ctx| async {
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
        run_db_test(|ctx| async {
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
