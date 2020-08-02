use anyhow::Context as _;
use chrono::{DateTime, FixedOffset};
use tokio_postgres::Client as DbClient;

/// A bors merge commit.
#[derive(Debug, serde::Serialize)]
pub struct Commit {
    pub sha: String,
    pub parent_sha: String,
    pub time: DateTime<FixedOffset>,
    pub pr: Option<u32>,
}

pub async fn record_commit(db: &DbClient, commit: Commit) -> anyhow::Result<()> {
    let pr = commit.pr.expect("commit has pr");
    db.execute(
        "INSERT INTO rustc_commits (sha, parent_sha, time, pr) VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
        &[&commit.sha, &commit.parent_sha, &commit.time, &(pr as i32)],
    )
    .await
    .context("inserting commit")?;
    Ok(())
}

pub async fn get_commits_with_artifacts(db: &DbClient) -> anyhow::Result<Vec<Commit>> {
    let commits = db
        .query(
            "
        select sha, parent_sha, time, pr
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
        let pr: Option<i32> = commit.get(3);

        data.push(Commit {
            sha,
            parent_sha,
            time,
            pr: pr.map(|n| n as u32),
        });
    }

    Ok(data)
}
