use anyhow::Context as _;
use chrono::{DateTime, FixedOffset};
use crate::db::DbClient;

/// A bors merge commit.
#[derive(Debug, serde::Serialize)]
pub struct Commit {
    pub sha: String,
    pub parent_sha: String,
    pub time: DateTime<FixedOffset>,
}

pub async fn record_commit(db: &DbClient, commit: Commit) -> anyhow::Result<()> {
    db.execute(
        "INSERT INTO rustc_commits (sha, parent_sha, time) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
        &[&commit.sha, &commit.parent_sha, &commit.time],
    )
    .await
    .context("inserting commit")?;
    Ok(())
}

pub async fn get_commits_with_artifacts(db: &DbClient) -> anyhow::Result<Vec<Commit>> {
    let commits = db
        .query(
            "
        select sha, parent_sha, time
        from rustc_commits
        where time >= current_date - interval '168 days'
        order by time desc;",
            &[],
        )
        .await
        .context("Getting commit data")?;

    let mut data = Vec::with_capacity(commits.len());
    for commit in commits {
        let sha: String = commit.get(0);
        let parent_sha: String = commit.get(1);
        let time: DateTime<FixedOffset> = commit.get(2);

        data.push(Commit {
            sha,
            parent_sha,
            time,
        });
    }

    Ok(data)
}
