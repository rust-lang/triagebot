use crate::db::users::record_username;
use crate::github::{User, UserId};
use anyhow::Context;
use bytes::BytesMut;
use postgres_types::{FromSql, IsNull, ToSql, Type, to_sql_checked};
use std::collections::HashMap;
use std::error::Error;

#[derive(Debug, Default, Copy, Clone, PartialEq)]
pub enum RotationMode {
    /// The reviewer can be automatically assigned by triagebot,
    /// and they can be assigned through teams and assign groups.
    #[default]
    OnRotation,
    /// The user is off rotation (e.g. on a vacation) and cannot be assigned automatically
    /// nor through teams and assign groups.
    OffRotation,
}

impl<'a> FromSql<'a> for RotationMode {
    fn from_sql(ty: &Type, raw: &'a [u8]) -> Result<Self, Box<dyn Error + Sync + Send>> {
        let value = <&str as FromSql>::from_sql(ty, raw)?;
        match value {
            "on-rotation" => Ok(Self::OnRotation),
            "off-rotation" => Ok(Self::OffRotation),
            _ => Err(format!("Unknown value for RotationMode: {value}").into()),
        }
    }

    fn accepts(ty: &Type) -> bool {
        <&str as FromSql>::accepts(ty)
    }
}

impl ToSql for RotationMode {
    fn to_sql(&self, ty: &Type, out: &mut BytesMut) -> Result<IsNull, Box<dyn Error + Sync + Send>>
    where
        Self: Sized,
    {
        let value = match self {
            RotationMode::OnRotation => "on-rotation",
            RotationMode::OffRotation => "off-rotation",
        };
        <&str as ToSql>::to_sql(&value, ty, out)
    }

    fn accepts(ty: &Type) -> bool
    where
        Self: Sized,
    {
        <&str as FromSql>::accepts(ty)
    }

    to_sql_checked!();
}

#[derive(Debug, PartialEq)]
pub struct UserTeamReviewPreferences {
    pub rotation_mode: RotationMode,
}

#[derive(Debug, PartialEq)]
pub struct UserRepoReviewPreferences {
    pub max_assigned_prs: Option<u32>,
}

/// Review preferences of a single user.
#[derive(Debug)]
pub struct ReviewPreferences {
    pub user_id: UserId,
    pub rotation_mode: RotationMode,
    pub team_review_prefs: HashMap<String, UserTeamReviewPreferences>,
    pub repo_review_prefs: HashMap<String, UserRepoReviewPreferences>,
}

impl ReviewPreferences {
    fn default_for_user(user_id: UserId) -> Self {
        Self {
            user_id,
            rotation_mode: RotationMode::OnRotation,
            team_review_prefs: HashMap::default(),
            repo_review_prefs: HashMap::default(),
        }
    }
}

/// Get team member review preferences.
/// If they are missing, returns default preferences.
pub async fn get_review_prefs(
    db: &tokio_postgres::Client,
    user_id: UserId,
) -> anyhow::Result<ReviewPreferences> {
    // We want to load data from three different tables that have different data shapes.
    // The global review preferences and team review preferences are currently relatively similar,
    // so we load them together. The repository preferences are loaded separately.
    let query = r#"
SELECT prefs.user_id AS user_id,
       prefs.rotation_mode AS rotation_mode,
       team,
       team_prefs.rotation_mode AS team_rotation_mode
FROM review_prefs AS prefs
FULL OUTER JOIN team_review_prefs AS team_prefs ON prefs.user_id = team_prefs.user_id
WHERE prefs.user_id = $1 OR team_prefs.user_id = $1;
"#;
    let rows = db
        .query(query, &[&(user_id as i64)])
        .await
        .context("Error retrieving global and team review preferences")?;
    let mut on_rotation: Option<RotationMode> = None;
    let mut team_prefs: HashMap<String, UserTeamReviewPreferences> = HashMap::default();

    for row in rows {
        // We have global preference data in the row
        if row.get::<_, Option<i64>>("user_id").is_some() {
            on_rotation = Some(row.get("rotation_mode"));
        }
        // We have team preference data in the row
        if let Some(team) = row.get::<_, Option<String>>("team") {
            let rotation_mode: RotationMode = row.get("team_rotation_mode");
            team_prefs.insert(team, UserTeamReviewPreferences { rotation_mode });
        }
    }

    let query = r#"
SELECT repo, max_assigned_prs
FROM repo_review_prefs
WHERE user_id = $1
"#;
    let rows = db
        .query(query, &[&(user_id as i64)])
        .await
        .context("Error retrieving repo review preferences")?;

    let mut repo_prefs: HashMap<String, UserRepoReviewPreferences> = HashMap::default();
    for row in rows {
        let repo: String = row.get("repo");
        let max_assigned_prs: Option<u32> = row
            .get::<_, Option<i32>>("max_assigned_prs")
            .map(|v| v as u32);
        repo_prefs.insert(repo, UserRepoReviewPreferences { max_assigned_prs });
    }

    Ok(ReviewPreferences {
        user_id,
        rotation_mode: on_rotation.unwrap_or_default(),
        team_review_prefs: team_prefs,
        repo_review_prefs: repo_prefs,
    })
}

/// Returns a set of review preferences for all passed usernames.
/// Usernames are matched regardless of case.
///
/// Usernames that are not present in the resulting map have no review preferences configured
/// in the database.
pub async fn get_review_prefs_batch<'a>(
    db: &tokio_postgres::Client,
    users: &[&'a str],
) -> anyhow::Result<HashMap<&'a str, ReviewPreferences>> {
    // We need to make sure that we match users regardless of case, but at the
    // same time we need to return the originally-cased usernames in the final hashmap.
    // At the same time, we can't depend on the order of results returned by the DB.
    // So we need to do some additional bookkeeping here.
    let lowercase_map: HashMap<String, &str> = users
        .iter()
        .map(|name| (name.to_lowercase(), *name))
        .collect();
    let lowercase_users: Vec<&str> = lowercase_map.keys().map(String::as_str).collect();

    // The id/user_id/max_assigned_prs/rotation_mode columns have to match the names used in
    // `From<&tokio_postgres::row::Row> for UserReviewPrefs`.
    let user_query = "
SELECT
    lower(u.username) AS username,
    r.user_id AS user_id,
    r.rotation_mode AS rotation_mode
FROM review_prefs AS r
JOIN users AS u ON u.user_id = r.user_id
WHERE lower(u.username) = ANY($1);";

    let mut user_prefs: HashMap<&str, ReviewPreferences> = HashMap::default();
    let rows = db
        .query(user_query, &[&lowercase_users])
        .await
        .context("Error retrieving user review preferences from usernames")?;
    for row in rows {
        // Map back from the lowercase username to the original username.
        let username_lower: &str = row.get("username");
        let username = lowercase_map
            .get(username_lower)
            .expect("Lowercase username not found");
        let user_id: UserId = row.get::<_, i64>("user_id") as u64;
        let rotation_mode = row.get("rotation_mode");
        let mut review_prefs = ReviewPreferences::default_for_user(user_id);
        review_prefs.rotation_mode = rotation_mode;
        user_prefs.insert(username, review_prefs);
    }

    // We could gather all preferences in a single query, but it would get too
    // complicated. So we split it into multiple queries, batched per table.
    let team_query = "
SELECT
    lower(u.username) AS username,
    r.user_id AS user_id,
    r.team AS team,
    r.rotation_mode AS rotation_mode
FROM team_review_prefs AS r
JOIN users AS u ON u.user_id = r.user_id
WHERE lower(u.username) = ANY($1);";
    let rows = db
        .query(team_query, &[&lowercase_users])
        .await
        .context("Error retrieving team review preferences from usernames")?;
    for row in rows {
        let user_id = row.get::<_, i64>("user_id") as u64;
        // Map back from the lowercase username to the original username.
        let username_lower: &str = row.get("username");
        let username = lowercase_map
            .get(username_lower)
            .expect("Lowercase username not found");

        let team: String = row.get("team");
        let rotation_mode: RotationMode = row.get("rotation_mode");
        let prefs = user_prefs
            .entry(username)
            .or_insert_with(|| ReviewPreferences::default_for_user(user_id));
        prefs
            .team_review_prefs
            .insert(team, UserTeamReviewPreferences { rotation_mode });
    }

    let repo_query = "
SELECT
    lower(u.username) AS username,
    r.user_id AS user_id,
    r.repo AS repo,
    r.max_assigned_prs AS max_assigned_prs
FROM repo_review_prefs AS r
JOIN users AS u ON u.user_id = r.user_id
WHERE lower(u.username) = ANY($1);";
    let rows = db
        .query(repo_query, &[&lowercase_users])
        .await
        .context("Error retrieving repo review preferences from usernames")?;
    for row in rows {
        let user_id = row.get::<_, i64>("user_id") as u64;
        // Map back from the lowercase username to the original username.
        let username_lower: &str = row.get("username");
        let username = lowercase_map
            .get(username_lower)
            .expect("Lowercase username not found");

        let repo: String = row.get("repo");
        let max_assigned_prs = row
            .get::<_, Option<i32>>("max_assigned_prs")
            .map(|v| v as u32);
        let prefs = user_prefs
            .entry(username)
            .or_insert_with(|| ReviewPreferences::default_for_user(user_id));
        prefs
            .repo_review_prefs
            .insert(repo, UserRepoReviewPreferences { max_assigned_prs });
    }

    Ok(user_prefs)
}

/// Updates review preferences of the specified user, or creates them
/// if they do not exist yet.
pub async fn upsert_user_review_prefs(
    db: &tokio_postgres::Client,
    user: User,
    rotation_mode: RotationMode,
) -> anyhow::Result<u64, anyhow::Error> {
    // We need to have the user stored in the DB to have a valid FK link in review_prefs
    record_username(db, user.id, &user.login).await?;

    let query = "
INSERT INTO review_prefs(user_id, rotation_mode)
VALUES ($1, $2)
ON CONFLICT (user_id)
DO UPDATE
SET rotation_mode = excluded.rotation_mode";

    let res = db
        .execute(query, &[&(user.id as i64), &rotation_mode])
        .await
        .context("Error upserting user review preferences")?;
    Ok(res)
}

/// Updates team review preferences of the specified user, or creates them
/// if they do not exist yet.
pub async fn upsert_team_review_prefs(
    db: &tokio_postgres::Client,
    user: User,
    team: &str,
    rotation_mode: RotationMode,
) -> anyhow::Result<u64, anyhow::Error> {
    // We need to have the user stored in the DB to have a valid FK link in [team_]review_prefs
    record_username(db, user.id, &user.login).await?;

    let query = r#"
INSERT INTO team_review_prefs(user_id, team, rotation_mode)
VALUES ($1, $2, $3)
ON CONFLICT (user_id, team)
DO UPDATE
SET rotation_mode = excluded.rotation_mode"#;

    let res = db
        .execute(query, &[&(user.id as i64), &team, &rotation_mode])
        .await
        .context("Error upserting team review preferences")?;
    Ok(res)
}

/// Updates repo review preferences of the specified user, or creates them
/// if they do not exist yet.
pub async fn upsert_repo_review_prefs(
    db: &tokio_postgres::Client,
    user: User,
    repo: &str,
    max_assigned_prs: Option<u32>,
) -> anyhow::Result<u64, anyhow::Error> {
    // We need to have the user stored in the DB to have a valid FK link in [team_]review_prefs
    record_username(db, user.id, &user.login).await?;

    let max_assigned_prs = max_assigned_prs.map(|v| v as i32);
    let query = r#"
INSERT INTO repo_review_prefs(user_id, repo, max_assigned_prs)
VALUES ($1, $2, $3)
ON CONFLICT (user_id, repo)
DO UPDATE
SET max_assigned_prs = excluded.max_assigned_prs"#;

    let res = db
        .execute(query, &[&(user.id as i64), &repo, &max_assigned_prs])
        .await
        .context("Error upserting repo review preferences")?;
    Ok(res)
}

#[cfg(test)]
mod tests {
    use crate::db::review_prefs::{
        RotationMode, UserRepoReviewPreferences, UserTeamReviewPreferences, get_review_prefs,
        get_review_prefs_batch, upsert_repo_review_prefs, upsert_team_review_prefs,
        upsert_user_review_prefs,
    };
    use crate::db::users::get_user;
    use crate::tests::github::user;
    use crate::tests::run_db_test;

    #[tokio::test]
    async fn insert_prefs_create_user() {
        run_db_test(|ctx| async {
            let user = user("Martin", 1);
            upsert_user_review_prefs(
                &ctx.db_client(),
                user.clone(),
                RotationMode::OnRotation,
            )
            .await?;
            assert_eq!(get_user(&ctx.db_client(), user.id).await?.unwrap(), user);

            Ok(ctx)
        })
        .await;
    }

    #[tokio::test]
    async fn set_rotation_mode() {
        run_db_test(|ctx| async {
            let db = ctx.db_client();
            let user = user("Martin", 1);

            upsert_user_review_prefs(&db, user.clone(), RotationMode::OnRotation).await?;
            assert_eq!(
                get_review_prefs(&db, 1).await?.rotation_mode,
                RotationMode::OnRotation
            );
            upsert_user_review_prefs(&db, user.clone(), RotationMode::OffRotation).await?;
            assert_eq!(
                get_review_prefs(&db, 1).await?.rotation_mode,
                RotationMode::OffRotation
            );

            Ok(ctx)
        })
        .await;
    }

    #[tokio::test]
    async fn only_team_prefs() {
        run_db_test(|ctx| async {
            let db = ctx.db_client();
            let user = || user("Martin", 1);

            upsert_team_review_prefs(&db, user(), "compiler", RotationMode::OffRotation).await?;

            let prefs = get_review_prefs(&db, 1).await?;
            assert_eq!(prefs.rotation_mode, RotationMode::OnRotation);
            assert_eq!(
                prefs.team_review_prefs.get("compiler"),
                Some(&UserTeamReviewPreferences {
                    rotation_mode: RotationMode::OffRotation,
                })
            );

            Ok(ctx)
        })
        .await;
    }

    #[tokio::test]
    async fn user_and_team_prefs() {
        run_db_test(|ctx| async {
            let db = ctx.db_client();
            let user = || user("Martin", 1);

            upsert_team_review_prefs(&db, user(), "compiler", RotationMode::OffRotation).await?;
            upsert_team_review_prefs(&db, user(), "libs", RotationMode::OnRotation).await?;
            upsert_user_review_prefs(&db, user(), RotationMode::OffRotation).await?;

            let prefs = get_review_prefs(&db, 1).await?;
            assert_eq!(prefs.rotation_mode, RotationMode::OffRotation);
            assert_eq!(
                prefs.team_review_prefs.get("compiler"),
                Some(&UserTeamReviewPreferences {
                    rotation_mode: RotationMode::OffRotation,
                })
            );
            assert_eq!(
                prefs.team_review_prefs.get("libs"),
                Some(&UserTeamReviewPreferences {
                    rotation_mode: RotationMode::OnRotation,
                })
            );

            Ok(ctx)
        })
        .await;
    }

    #[tokio::test]
    async fn update_team_prefs() {
        run_db_test(|ctx| async {
            let db = ctx.db_client();
            let user = || user("Martin", 1);

            upsert_team_review_prefs(&db, user(), "compiler", RotationMode::OffRotation).await?;
            upsert_team_review_prefs(&db, user(), "compiler", RotationMode::OnRotation).await?;

            let prefs = get_review_prefs(&db, 1).await?;
            assert_eq!(
                prefs.team_review_prefs.get("compiler"),
                Some(&UserTeamReviewPreferences {
                    rotation_mode: RotationMode::OnRotation,
                })
            );

            Ok(ctx)
        })
        .await;
    }

    #[tokio::test]
    async fn insert_repo_prefs() {
        run_db_test(|ctx| async {
            let db = ctx.db_client();

            upsert_repo_review_prefs(&db, user("Martin", 1), "rust-lang/rust", Some(5)).await?;

            let prefs = get_review_prefs(&db, 1).await?;
            assert_eq!(
                prefs.repo_review_prefs.get("rust-lang/rust"),
                Some(&UserRepoReviewPreferences {
                    max_assigned_prs: Some(5),
                })
            );

            Ok(ctx)
        })
        .await;
    }

    #[tokio::test]
    async fn update_repo_prefs() {
        run_db_test(|ctx| async {
            let db = ctx.db_client();

            upsert_repo_review_prefs(&db, user("Martin", 1), "rust-lang/rust", Some(5)).await?;
            upsert_repo_review_prefs(&db, user("Martin", 1), "rust-lang/rust", Some(10)).await?;
            assert_eq!(
                get_review_prefs(&db, 1)
                    .await?
                    .repo_review_prefs
                    .get("rust-lang/rust")
                    .unwrap()
                    .max_assigned_prs,
                Some(10)
            );

            upsert_repo_review_prefs(&db, user("Martin", 1), "rust-lang/rust", None).await?;
            assert_eq!(
                get_review_prefs(&db, 1)
                    .await?
                    .repo_review_prefs
                    .get("rust-lang/rust")
                    .unwrap()
                    .max_assigned_prs,
                None
            );

            Ok(ctx)
        })
        .await;
    }

    #[tokio::test]
    async fn repo_prefs_multiple_repos() {
        run_db_test(|ctx| async {
            let db = ctx.db_client();

            upsert_repo_review_prefs(&db, user("Martin", 1), "rust-lang/rust", Some(5)).await?;
            upsert_repo_review_prefs(&db, user("Martin", 1), "rust-lang/cargo", Some(3)).await?;

            let prefs = get_review_prefs(&db, 1).await?;
            assert_eq!(prefs.repo_review_prefs.len(), 2);
            assert_eq!(
                prefs.repo_review_prefs.get("rust-lang/rust"),
                Some(&UserRepoReviewPreferences {
                    max_assigned_prs: Some(5),
                })
            );
            assert_eq!(
                prefs.repo_review_prefs.get("rust-lang/cargo"),
                Some(&UserRepoReviewPreferences {
                    max_assigned_prs: Some(3),
                })
            );

            Ok(ctx)
        })
        .await;
    }

    #[tokio::test]
    async fn batch_prefs_empty() {
        run_db_test(|ctx| async {
            let db = ctx.db_client();
            let result = get_review_prefs_batch(&db, &[]).await?;
            assert!(result.is_empty());
            Ok(ctx)
        })
        .await;
    }

    #[tokio::test]
    async fn batch_prefs_returns_existing_users() {
        run_db_test(|ctx| async {
            let db = ctx.db_client();

            upsert_user_review_prefs(&db, user("Alice", 1), RotationMode::OnRotation).await?;
            upsert_repo_review_prefs(&db, user("Alice", 1), "rust-lang/rust", Some(3)).await?;
            upsert_user_review_prefs(&db, user("Bob", 2), RotationMode::OffRotation).await?;
            upsert_repo_review_prefs(&db, user("Bob", 2), "rust-lang/rust", Some(5)).await?;

            let result = get_review_prefs_batch(&db, &["Alice", "Bob"]).await?;
            assert_eq!(result.len(), 2);

            let alice = result.get("Alice").expect("Alice should be present");
            assert_eq!(alice.user_id, 1);
            assert_eq!(alice.rotation_mode, RotationMode::OnRotation);
            assert_eq!(
                alice.repo_review_prefs.get("rust-lang/rust"),
                Some(&UserRepoReviewPreferences {
                    max_assigned_prs: Some(3),
                })
            );

            let bob = result.get("Bob").expect("Bob should be present");
            assert_eq!(bob.user_id, 2);
            assert_eq!(bob.rotation_mode, RotationMode::OffRotation);
            assert_eq!(
                bob.repo_review_prefs.get("rust-lang/rust"),
                Some(&UserRepoReviewPreferences {
                    max_assigned_prs: Some(5),
                })
            );

            Ok(ctx)
        })
        .await;
    }

    #[tokio::test]
    async fn batch_prefs_skips_unknown_users() {
        run_db_test(|ctx| async {
            let db = ctx.db_client();

            upsert_user_review_prefs(&db, user("Alice", 1), RotationMode::OnRotation).await?;

            let result = get_review_prefs_batch(&db, &["Alice", "Unknown"]).await?;
            assert_eq!(result.len(), 1);
            assert!(result.contains_key("Alice"));
            assert!(!result.contains_key("Unknown"));

            Ok(ctx)
        })
        .await;
    }

    #[tokio::test]
    async fn batch_prefs_case_insensitive() {
        run_db_test(|ctx| async {
            let db = ctx.db_client();

            upsert_user_review_prefs(&db, user("Alice", 1), RotationMode::OnRotation).await?;

            // Query with different casing than what was inserted
            let result = get_review_prefs_batch(&db, &["ALICE"]).await?;
            assert_eq!(result.len(), 1);
            // The key should be the originally-passed casing, not the DB casing
            let prefs = result.get("ALICE").expect("ALICE should be present");
            assert_eq!(prefs.user_id, 1);

            Ok(ctx)
        })
        .await;
    }

    #[tokio::test]
    async fn batch_prefs_with_team_prefs() {
        run_db_test(|ctx| async {
            let db = ctx.db_client();

            upsert_user_review_prefs(&db, user("Alice", 1), RotationMode::OnRotation).await?;
            upsert_repo_review_prefs(&db, user("Alice", 1), "rust-lang/rust", Some(3)).await?;
            upsert_team_review_prefs(&db, user("Alice", 1), "compiler", RotationMode::OffRotation)
                .await?;
            upsert_team_review_prefs(&db, user("Alice", 1), "libs", RotationMode::OnRotation)
                .await?;

            let result = get_review_prefs_batch(&db, &["Alice"]).await?;
            let alice = result.get("Alice").expect("Alice should be present");
            assert_eq!(alice.rotation_mode, RotationMode::OnRotation);
            assert_eq!(
                alice.repo_review_prefs.get("rust-lang/rust"),
                Some(&UserRepoReviewPreferences {
                    max_assigned_prs: Some(3),
                })
            );
            assert_eq!(
                alice.team_review_prefs.get("compiler"),
                Some(&UserTeamReviewPreferences {
                    rotation_mode: RotationMode::OffRotation,
                })
            );
            assert_eq!(
                alice.team_review_prefs.get("libs"),
                Some(&UserTeamReviewPreferences {
                    rotation_mode: RotationMode::OnRotation,
                })
            );

            Ok(ctx)
        })
        .await;
    }

    #[tokio::test]
    async fn batch_prefs_only_team_prefs() {
        run_db_test(|ctx| async {
            let db = ctx.db_client();

            upsert_team_review_prefs(&db, user("Bob", 2), "compiler", RotationMode::OffRotation)
                .await?;

            let result = get_review_prefs_batch(&db, &["Bob"]).await?;
            let bob = result.get("Bob").expect("Bob should be present");
            assert_eq!(bob.rotation_mode, RotationMode::OnRotation);
            assert_eq!(
                bob.team_review_prefs.get("compiler"),
                Some(&UserTeamReviewPreferences {
                    rotation_mode: RotationMode::OffRotation,
                })
            );

            Ok(ctx)
        })
        .await;
    }

    #[tokio::test]
    async fn batch_prefs_with_repo_prefs() {
        run_db_test(|ctx| async {
            let db = ctx.db_client();

            upsert_repo_review_prefs(&db, user("Alice", 1), "rust-lang/rust", Some(3)).await?;
            upsert_repo_review_prefs(&db, user("Alice", 1), "rust-lang/cargo", Some(7)).await?;

            let result = get_review_prefs_batch(&db, &["Alice"]).await?;
            let alice = result.get("Alice").expect("Alice should be present");
            assert_eq!(alice.repo_review_prefs.len(), 2);
            assert_eq!(
                alice.repo_review_prefs.get("rust-lang/rust"),
                Some(&UserRepoReviewPreferences {
                    max_assigned_prs: Some(3),
                })
            );
            assert_eq!(
                alice.repo_review_prefs.get("rust-lang/cargo"),
                Some(&UserRepoReviewPreferences {
                    max_assigned_prs: Some(7),
                })
            );

            Ok(ctx)
        })
        .await;
    }
}
