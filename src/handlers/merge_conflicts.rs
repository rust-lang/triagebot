//! Merge conflict notifications.
//!
//! This posts comments on GitHub PRs when the PR has a merge conflict that
//! would prevent it from merging.
//!
//! ## Locking
//!
//! This implementation currently does not implement locking to prevent
//! racing scans. My intention is that it can be added later if it is
//! demonstrably a problem.
//!
//! In general, multiple pushes happening quickly should be rare. And when it
//! does happen, hopefully the state in the database will prevent duplicate
//! messages.

use crate::{
    config::MergeConflictConfig,
    db::{PooledClient, issue_data::IssueData},
    github::{
        Event, GitHubUserType, GithubClient, Issue, IssuesAction, IssuesEvent, Label,
        MergeConflictInfo, MergeableState, PushEvent, ReportedContentClassifiers, Repository,
    },
    handlers::Context,
};
use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, LazyLock},
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, OnceCell};
use tokio_postgres::Client as DbClient;
use tracing as log;

/// Key for the database.
const MERGE_CONFLICTS_KEY: &str = "merge-conflicts";

/// The amount of time to wait before scanning an unknown mergeable status.
///
/// GitHub has a background job which updates the mergeable status. We have to
/// wait for it to be finished. Unfortunately there is no notification when it
/// is done. It seems to usually run pretty quickly, but this timeout is set
/// to be a little conservative just in case it takes a while to compute. I
/// don't particularly want to loop to avoid hitting GitHub too hard, and this
/// conflict notification is not that important to be perfect. If it is too
/// unreliable, then we could add a loop that will try one or two more times.
const UNKNOWN_RESCAN_DELAY: Duration = Duration::from_secs(60);

/// The minimum amount of time before scans for the same branch or pr can
/// be started.
///
/// Should be at minimum `UNKNOWN_RESCAN_DELAY`.
const DELAY_BETWEEN_SCANS: Duration = Duration::from_secs(61);

/// List of scans for a given branch or PR with the last attempt
static SCANS: LazyLock<Arc<Mutex<HashMap<String, Instant>>>> =
    LazyLock::new(|| Arc::new(Mutex::new(HashMap::new())));

/// State stored in the database for a PR.
#[derive(Debug, Default, Deserialize, Serialize, Clone, PartialEq)]
struct MergeConflictState {
    /// The GraphQL ID of the most recent warning comment.
    ///
    /// After the conflict is resolved, this will be set to `None`.
    last_warned_comment: Option<String>,
}

pub(super) async fn handle(
    ctx: &Context,
    event: &Event,
    config: &MergeConflictConfig,
) -> anyhow::Result<()> {
    match event {
        Event::Push(push) => handle_branch_push(ctx, config, push).await,
        Event::Issue(IssuesEvent {
            action: IssuesAction::Opened | IssuesAction::Reopened | IssuesAction::Synchronize,
            repository,
            issue,
            ..
        }) if issue.pull_request.is_some() => {
            handle_pr(ctx, config, repository.clone(), issue).await
        }
        _ => Ok(()),
    }
}

/// Handles a push to a branch in the repository.
///
/// This will scan open PRs to see if any of them are now unmergeable.
async fn handle_branch_push(
    ctx: &Context,
    config: &MergeConflictConfig,
    push: &PushEvent,
) -> anyhow::Result<()> {
    let git_ref = push.git_ref.clone();

    let Some(branch_name) = push.git_ref.strip_prefix("refs/heads/") else {
        log::trace!("ignoring push to {git_ref}");
        return Ok(());
    };
    if branch_name.starts_with("gh-readonly-queue/") {
        log::trace!("ignoring push to {git_ref}");
        return Ok(());
    }

    let branch_name = branch_name.to_string();
    let push_sha = push.after.to_string();
    let config = config.clone();
    let repo = push.repository.clone();
    let db = ctx.db.get().await;
    let gh = ctx.github.clone();

    // Spawn since this can trigger a lot of work.
    spawn_scan_for(
        format!("{full_name}/{branch_name}", full_name = &repo.full_name),
        async move {
            if let Err(e) = scan_prs(&gh, db, &config, repo, &branch_name, &push_sha).await {
                log::error!("failed to scan PRs for merge conflicts: {e:?}");
            }
        },
    )
    .await;

    Ok(())
}

/// Handles a new PR or a push to a PR.
async fn handle_pr(
    ctx: &Context,
    config: &MergeConflictConfig,
    repo: Repository,
    issue: &Issue,
) -> anyhow::Result<()> {
    if issue.user.r#type == GitHubUserType::Bot && !config.consider_prs_from_bots {
        log::trace!("ignoring issue {}", issue.number);
        return Ok(());
    }

    let mut db = ctx.db.get().await;
    match issue.mergeable {
        Some(true) => maybe_hide_comment(&ctx.github, &mut db, issue).await?,
        Some(false) => maybe_add_comment(&ctx.github, &mut db, config, issue, None).await?,
        None => {
            // Status is unknown, spawn a task to try again later.
            let pr_number = issue.number;
            let db = ctx.db.get().await;
            let config = config.clone();
            let gh = ctx.github.clone();
            spawn_scan_for(
                format!("{full_name}/{pr_number}", full_name = &repo.full_name),
                async move {
                    // See module note about locking.
                    tokio::time::sleep(UNKNOWN_RESCAN_DELAY).await;
                    if let Err(e) = rescan_pr(&gh, db, &config, repo, pr_number).await {
                        log::error!("failed to rescan PR for merge conflicts: {e:?}");
                    }
                },
            )
            .await;
        }
    }
    Ok(())
}

/// Re-scans a PR to check its mergeable status after waiting for GitHub to
/// update the status.
async fn rescan_pr(
    gh: &GithubClient,
    mut db: PooledClient,
    config: &MergeConflictConfig,
    repo: Repository,
    pr_number: u64,
) -> anyhow::Result<()> {
    let pr = repo.get_pr(gh, pr_number).await?;
    log::debug!(
        "re-scanning unknown PR {} for merge conflict after delay",
        pr.global_id()
    );
    match pr.mergeable {
        Some(true) => maybe_hide_comment(gh, &mut db, &pr).await?,
        Some(false) => maybe_add_comment(gh, &mut db, config, &pr, None).await?,
        None => log::info!(
            "re-scan of mergeable status still unknown for {}",
            pr.global_id()
        ),
    }
    Ok(())
}

/// Scans all open PRs for anything that is no longer mergeable after a push
/// to the repository.
async fn scan_prs(
    gh: &GithubClient,
    mut db: PooledClient,
    config: &MergeConflictConfig,
    repo: Repository,
    branch_name: &str,
    push_sha: &str,
) -> anyhow::Result<()> {
    // Prepare the retrieval of the reason for the conflicts (ie the PR number if
    // it's a PR that's the cause). We only load it eagerly in case there are no
    // conflicts but also bc GitHub doesn't always updates the field in the first
    // few milliseconds.
    let reason = ReasonForConflict::new(push_sha.to_string());

    // There is a small risk of a race condition here. Consider the following
    // sequence of events:
    //
    // 1. Clicking "Merge" on a PR
    // 2. GitHub pushing that PR to the branch
    // 3. GitHub sending a webhook notification about the push
    // 4. GitHub closing the PR
    //
    // I don't actually know how GitHub handles steps 2 and 4 (are they
    // synchronized? does step 3 actually happen after step 4). This gets
    // complicated with merge commits (like rust-lang/rust rollups) which
    // close multiple PRs at once. If there are problems with "merge conflict"
    // notifications happening on closed PRs, then we'll need to add something
    // to prevent that race (like a delay or some other verification).
    let mut prs = repo.get_merge_conflict_prs(gh).await?;
    if !config.consider_prs_from_bots {
        prs.retain(|pr| pr.author.r#type != GitHubUserType::Bot);
    }
    let (conflicting, unknowns): (Vec<_>, Vec<_>) = prs
        .into_iter()
        .filter(|pr| pr.mergeable != MergeableState::Mergeable)
        // Assume that pushes to other branches won't affect this PR (maybe
        // not the greatest assumption, but might help with some noise). In
        // practice, this shouldn't matter much since simultaneous pushes to
        // multiple branches is rare.
        .filter(|pr| pr.base_ref_name == branch_name)
        .partition(|pr| pr.mergeable == MergeableState::Conflicting);

    // Report conflicts for conflicting PRs
    for conflict in conflicting {
        let pr = repo.get_pr(gh, conflict.number).await?;
        let reason = reason.get_reason(gh, &repo).await;
        maybe_add_comment(gh, &mut db, config, &pr, Some(reason)).await?;
    }

    // Wait and fetch the new status for unknowns PRs
    if !unknowns.is_empty() {
        // See module note about locking.
        tokio::time::sleep(UNKNOWN_RESCAN_DELAY).await;
        if let Err(e) = scan_unknowns(&gh, db, &config, &repo, &unknowns, reason).await {
            log::error!("failed to scan unknown PRs for merge conflicts: {e:?}");
        }
    }

    Ok(())
}

/// Scans open PRs with an unknown mergeable status to see if the mergeability
/// has been updated.
async fn scan_unknowns(
    gh: &GithubClient,
    mut db: PooledClient,
    config: &MergeConflictConfig,
    repo: &Repository,
    unknowns: &[MergeConflictInfo],
    reason: ReasonForConflict,
) -> anyhow::Result<()> {
    log::debug!(
        "re-scanning {} unknown PRs for merge conflicts for {}",
        unknowns.len(),
        repo.full_name
    );
    for unknown in unknowns {
        let pr = repo.get_pr(gh, unknown.number).await?;
        match pr.mergeable {
            Some(true) => maybe_hide_comment(gh, &mut db, &pr).await?,
            Some(false) => {
                let reason = reason.get_reason(gh, repo).await;
                maybe_add_comment(gh, &mut db, config, &pr, Some(reason)).await?
            }
            // Ignore None, we don't want to repeatedly hammer GitHub asking for the answer.
            None => log::info!("unable to determine mergeable after delay for {unknown:?}"),
        }
    }
    Ok(())
}

async fn maybe_add_comment(
    gh: &GithubClient,
    db: &mut DbClient,
    config: &MergeConflictConfig,
    issue: &Issue,
    reason: Option<&str>,
) -> anyhow::Result<()> {
    let mut state: IssueData<'_, MergeConflictState> =
        IssueData::load(db, issue, MERGE_CONFLICTS_KEY).await?;
    if state.data.last_warned_comment.is_some() {
        // There was already an unresolved notification, don't warn again.
        return Ok(());
    }

    let possibly = reason
        .as_ref()
        .map(|s| format!(" (possibly {s})"))
        .unwrap_or_default();
    let message = format!(
        ":umbrella: \
        The latest upstream changes{possibly} made this pull request unmergeable. \
        Please [resolve the merge conflicts]\
        (https://rustc-dev-guide.rust-lang.org/git.html#rebasing-and-conflicts)."
    );
    let comment = issue
        .post_comment(gh, &message)
        .await
        .context("failed to post no_merges comment")?;

    state.data.last_warned_comment = Some(comment.node_id);
    state.save().await?;

    let current_labels: HashSet<_> = issue.labels.iter().map(|l| l.name.clone()).collect();
    if current_labels.is_disjoint(&config.unless) {
        let to_add = config
            .add
            .iter()
            .map(|l| Label { name: l.clone() })
            .collect();
        let to_remove = config
            .remove
            .iter()
            .map(|l| Label { name: l.clone() })
            .collect();

        issue.add_labels(gh, to_add).await?;
        issue.remove_labels(gh, to_remove).await?;
    }

    Ok(())
}

async fn maybe_hide_comment(
    gh: &GithubClient,
    db: &mut DbClient,
    issue: &Issue,
) -> anyhow::Result<()> {
    let mut state: IssueData<'_, MergeConflictState> =
        IssueData::load(db, issue, MERGE_CONFLICTS_KEY).await?;
    let Some(comment_id) = &state.data.last_warned_comment else {
        return Ok(());
    };

    issue
        .hide_comment(gh, comment_id, ReportedContentClassifiers::Resolved)
        .await?;

    state.data.last_warned_comment = None;
    state.save().await?;

    Ok(())
}

/// Start the `scan_fut` future if enough time has passed.
///
/// Checks for each `key` if the minimum delay between scans is respected.
/// Otherwise `scan_fut` is dropped without ever being polled.
async fn spawn_scan_for(
    key: String,
    scan_fut: impl std::future::Future<Output = ()> + Send + 'static,
) {
    let should_spawn = {
        let mut scans = SCANS.lock().await;
        let now = Instant::now();

        if let Some(&last_spawn) = scans.get(&key) {
            if now.duration_since(last_spawn) < DELAY_BETWEEN_SCANS {
                // Don't spawn, threshold not met
                tracing::debug!("threshold not met to scan for {key}");
                false
            } else {
                // Last scan was sometime ago, fine to start a new one
                scans.insert(key, now);
                true
            }
        } else {
            // No scan record, fine to start one
            scans.insert(key, now);
            true
        }
    };

    if should_spawn {
        tokio::spawn(scan_fut);
    }
}

struct ReasonForConflict {
    cached_pr_number: OnceCell<Option<String>>,
    push_sha: String,
}

impl ReasonForConflict {
    fn new(push_sha: String) -> ReasonForConflict {
        ReasonForConflict {
            cached_pr_number: OnceCell::new(),
            push_sha,
        }
    }

    async fn get_reason(&self, gh: &GithubClient, repo: &Repository) -> &str {
        self.cached_pr_number
            .get_or_init(|| async {
                // Make a guess as to what is responsible for the conflict. This is only a
                // guess, it can be inaccurate due to many factors (races, rebases, force
                // pushes, etc.).
                match repo.pulls_for_commit(gh, &self.push_sha).await {
                    Ok(prs) if prs.len() == 1 => Some(format!("#{}", prs[0].number)),
                    Err(e) => {
                        log::warn!("could not determine PRs for {}: {e:?}", self.push_sha);
                        None
                    }
                    _ => None,
                }
            })
            .await
            .as_deref()
            .unwrap_or(&self.push_sha)
    }
}
