//! A scheduled job to post a PR to update the documentation on rust-lang/rust.

use crate::db::jobs::JobSchedule;
use crate::github::{self, GitTreeEntry, GithubClient, Repository};
use anyhow::Context;
use anyhow::Result;
use cron::Schedule;
use std::fmt::Write;
use std::str::FromStr;

/// This is the repository where the commits will be created.
const WORK_REPO: &str = "rustbot/rust";
/// This is the repository where the PR will be created.
const DEST_REPO: &str = "rust-lang/rust";
/// This is the branch in `WORK_REPO` to create the commits.
const BRANCH_NAME: &str = "docs-update";

const SUBMODULES: &[&str] = &[
    "src/doc/book",
    "src/doc/edition-guide",
    "src/doc/embedded-book",
    "src/doc/nomicon",
    "src/doc/reference",
    "src/doc/rust-by-example",
    "src/doc/rustc-dev-guide",
];

const TITLE: &str = "Update books";

pub fn job() -> JobSchedule {
    JobSchedule {
        name: "docs_update".to_string(),
        // Around 9am Pacific time on every Monday.
        schedule: Schedule::from_str("0 00 17 * * Mon *").unwrap(),
        metadata: serde_json::Value::Null,
    }
}

pub async fn handle_job() -> Result<()> {
    // Only run every other week. Doing it every week can be a bit noisy, and
    // (rarely) a PR can take longer than a week to merge (like if there are
    // CI issues). `Schedule` does not allow expressing this, so check it
    // manually.
    //
    // This is set to run the first week after a release, and the week just
    // before a release. That allows getting the latest changes in the next
    // release, accounting for possibly taking a few days for the PR to land.
    let today = chrono::Utc::today().naive_utc();
    let base = chrono::naive::NaiveDate::from_ymd(2015, 12, 10);
    let duration = today.signed_duration_since(base);
    let weeks = duration.num_weeks();
    if weeks % 2 != 0 {
        tracing::trace!("skipping job, this is an odd week");
        return Ok(());
    }

    tracing::trace!("starting docs-update");
    docs_update().await.context("failed to process docs update")
}

async fn docs_update() -> Result<()> {
    let gh = GithubClient::new_from_env();
    let work_repo = gh.repository(WORK_REPO).await?;
    work_repo
        .merge_upstream(&gh, &work_repo.default_branch)
        .await?;

    let updates = get_submodule_updates(&gh, &work_repo).await?;
    if updates.is_empty() {
        tracing::trace!("no updates this week?");
        return Ok(());
    }

    create_commit(&gh, &work_repo, &updates).await?;
    create_pr(&gh, &updates).await?;
    Ok(())
}

struct Update {
    path: String,
    new_hash: String,
    pr_body: String,
}

async fn get_submodule_updates(
    gh: &GithubClient,
    repo: &github::Repository,
) -> Result<Vec<Update>> {
    let mut updates = Vec::new();
    for submodule_path in SUBMODULES {
        tracing::trace!("checking submodule {submodule_path}");
        let submodule = repo.submodule(gh, submodule_path, None).await?;
        let submodule_repo = submodule.repository(gh).await?;
        let latest_commit = submodule_repo
            .get_reference(gh, &format!("heads/{}", submodule_repo.default_branch))
            .await?;
        if submodule.sha == latest_commit.object.sha {
            tracing::trace!(
                "skipping submodule {submodule_path}, no changes sha={}",
                submodule.sha
            );
            continue;
        }
        let current_hash = submodule.sha;
        let new_hash = latest_commit.object.sha;
        let pr_body = generate_pr_body(gh, &submodule_repo, &current_hash, &new_hash).await?;

        let update = Update {
            path: submodule.path,
            new_hash,
            pr_body,
        };
        updates.push(update);
    }
    Ok(updates)
}

async fn generate_pr_body(
    gh: &GithubClient,
    repo: &github::Repository,
    oldest: &str,
    newest: &str,
) -> Result<String> {
    let recent_commits: Vec<_> = repo
        .recent_commits(gh, &repo.default_branch, oldest, newest)
        .await?;
    if recent_commits.is_empty() {
        anyhow::bail!(
            "unexpected empty set of commits for {} oldest={oldest} newest={newest}",
            repo.full_name
        );
    }
    let mut body = format!(
        "## {}\n\
        \n\
        {} commits in {}..{}\n\
        {} to {}\n\
        \n",
        repo.full_name,
        recent_commits.len(),
        oldest,
        newest,
        recent_commits.first().unwrap().committed_date,
        recent_commits.last().unwrap().committed_date,
    );
    for commit in recent_commits {
        write!(body, "- {}", commit.title).unwrap();
        if let Some(num) = commit.pr_num {
            write!(body, " ({}#{})", repo.full_name, num).unwrap();
        }
        body.push('\n');
    }
    Ok(body)
}

async fn create_commit(
    gh: &GithubClient,
    rust_repo: &Repository,
    updates: &[Update],
) -> Result<()> {
    let master_ref = rust_repo
        .get_reference(gh, &format!("heads/{}", rust_repo.default_branch))
        .await?;
    let master_commit = rust_repo.git_commit(gh, &master_ref.object.sha).await?;
    let tree_entries: Vec<_> = updates
        .iter()
        .map(|update| GitTreeEntry {
            path: update.path.clone(),
            mode: "160000".to_string(),
            object_type: "commit".to_string(),
            sha: Some(Some(update.new_hash.clone())),
            content: None,
        })
        .collect();
    let new_tree = rust_repo
        .update_tree(gh, &master_commit.tree.sha, &tree_entries)
        .await?;
    let commit = rust_repo
        .create_commit(gh, TITLE, &[&master_ref.object.sha], &new_tree.sha)
        .await?;
    rust_repo
        .update_reference(gh, &format!("heads/{BRANCH_NAME}"), &commit.sha)
        .await?;
    Ok(())
}

async fn create_pr(gh: &GithubClient, updates: &[Update]) -> Result<()> {
    let dest_repo = gh.repository(DEST_REPO).await?;
    let mut body = String::new();
    for update in updates {
        write!(body, "{}\n", update.pr_body).unwrap();
    }

    let username = WORK_REPO.split('/').next().unwrap();
    let head = format!("{username}:{BRANCH_NAME}");
    let pr = dest_repo
        .new_pr(gh, TITLE, &head, &dest_repo.default_branch, &body)
        .await?;
    tracing::debug!("created PR {}", pr.html_url);
    Ok(())
}
