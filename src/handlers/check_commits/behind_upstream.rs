use crate::github::{GithubClient, GithubCommit, IssuesEvent, Repository};
use tracing as log;

/// Default threshold for parent commit age in days to trigger a warning
pub(super) const DEFAULT_DAYS_THRESHOLD: usize = 7;

/// Check if the PR is based on an old parent commit
pub(super) async fn behind_upstream(
    age_threshold: usize,
    event: &IssuesEvent,
    client: &GithubClient,
    commits: &Vec<GithubCommit>,
) -> Option<String> {
    log::debug!("Checking if PR #{} is behind upstream", event.issue.number);

    let Some(head_commit) = commits.first() else {
        return None;
    };

    // First try the parent commit age check as it's more accurate
    match is_parent_commit_too_old(head_commit, &event.repository, client, age_threshold).await {
        Ok(Some(days_old)) => {
            log::info!(
                "PR #{} has a parent commit that is {} days old",
                event.issue.number,
                days_old
            );

            return Some(format!(
                r"This PR is based on an upstream commit that is {} days old.

    *It's recommended to update your branch according to the [rustc_dev_guide](https://rustc-dev-guide.rust-lang.org/contributing.html#keeping-your-branch-up-to-date).*",
                days_old
            ));
        }
        Ok(None) => {
            // Parent commit is not too old, log and do nothing
            log::debug!("PR #{} parent commit is not too old", event.issue.number);
        }
        Err(e) => {
            // Error checking parent commit age, log and do nothing
            log::error!(
                "Error checking parent commit age for PR #{}: {}",
                event.issue.number,
                e
            );
        }
    }

    None
}

/// Checks if the PR's parent commit is too old.
///
/// This determines if a PR needs updating by examining the first parent of the PR's head commit,
/// which typically represents the base branch commit that the PR is based on.
///
/// If this parent commit is older than the specified threshold, it suggests the PR
/// should be updated/rebased to a more recent version of the base branch.
///
/// Returns:
/// - Ok(Some(days_old)) - If parent commit is older than the threshold
/// - Ok(None)
///     - If there is no parent commit
///     - If parent is within threshold
/// - Err(...) - If an error occurred during processing
pub(super) async fn is_parent_commit_too_old(
    commit: &GithubCommit,
    repo: &Repository,
    client: &GithubClient,
    max_days_old: usize,
) -> anyhow::Result<Option<usize>> {
    // Get the first parent (it should be from the base branch)
    let Some(parent_sha) = commit.parents.get(0).map(|c| c.sha.clone()) else {
        return Ok(None);
    };

    let days_old = commit_days_old(&parent_sha, repo, client).await?;

    if days_old > max_days_old {
        Ok(Some(days_old))
    } else {
        Ok(None)
    }
}

/// Returns the number of days old the commit is
pub(super) async fn commit_days_old(
    sha: &str,
    repo: &Repository,
    client: &GithubClient,
) -> anyhow::Result<usize> {
    // Get the commit details to check its date
    let commit: GithubCommit = repo.github_commit(client, &sha).await?;

    // compute the number of days old the commit is
    let commit_date = commit.commit.author.date;
    let now = chrono::Utc::now().with_timezone(&commit_date.timezone());
    let days_old = (now - commit_date).num_days() as usize;

    Ok(days_old)
}
