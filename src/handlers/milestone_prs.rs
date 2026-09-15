use crate::{
    github::{Event, GithubClient, IssuesAction, IssuesEvent},
    handlers::Context,
};
use anyhow::Context as _;
use futures::stream::{self, StreamExt};
use regex::Regex;
use reqwest::StatusCode;
use std::sync::LazyLock;
use tracing as log;

static MERGE_QUEUE_COMMIT_TITLE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^Merge pull request #([0-9]+)").unwrap());

pub(super) async fn handle(ctx: &Context, event: &Event) -> anyhow::Result<()> {
    let Event::Issue(e) = event else {
        return Ok(());
    };

    // Only trigger on closed issues
    if e.action != IssuesAction::Closed {
        return Ok(());
    }

    let repo = e.issue.repository();
    if !(repo.organization == "rust-lang" && repo.repository == "rust") {
        return Ok(());
    }

    if e.issue.merged_at.is_none() {
        log::trace!(
            "Ignoring closing of rust-lang/rust#{}: not merged",
            e.issue.number
        );
        return Ok(());
    }

    let Some(merge_sha) = &e.issue.merge_commit_sha else {
        log::error!(
            "rust-lang/rust#{}: no merge_commit_sha in event",
            e.issue.number
        );
        return Ok(());
    };

    // Fetch the version from the upstream repository.
    let Some(version) = get_version_standalone(&ctx.github, merge_sha).await? else {
        log::error!("could not find the version of {merge_sha:?}");
        return Ok(());
    };

    if !version.starts_with("1.") && version.len() < 8 {
        log::error!("Weird version {version:?} for {merge_sha:?}");
        return Ok(());
    }

    // Associate this merged PR with the version it merged into.
    //
    // Note that this should work for rollup-merged PRs too. It will *not*
    // auto-update when merging a beta-backport, for example, but that seems
    // fine; we can manually update without too much trouble in that case, and
    // eventually automate it separately.
    e.issue.set_milestone(&ctx.github, &version).await?;

    milestone_submodules(&ctx.github, e, &version).await?;
    milestone_git_subtrees(&ctx.github, e, &version).await?;

    Ok(())
}

async fn get_version_standalone(
    gh: &GithubClient,
    merge_sha: &str,
) -> anyhow::Result<Option<String>> {
    let resp = gh
        .raw()
        .get(format!(
            "https://raw.githubusercontent.com/rust-lang/rust/{merge_sha}/src/version"
        ))
        .send()
        .await
        .with_context(|| format!("retrieving src/version for {merge_sha}"))?;

    match resp.status() {
        StatusCode::OK => {}
        // Don't treat a 404 as a failure, we'll try another way to retrieve the version.
        StatusCode::NOT_FOUND => return Ok(None),
        status => anyhow::bail!(
            "unexpected status code {status} while retrieving src/version for {merge_sha}"
        ),
    }

    Ok(Some(
        resp.text()
            .await
            .with_context(|| format!("deserializing src/version for {merge_sha}"))?
            .trim()
            .to_string(),
    ))
}

async fn milestone_submodules(
    gh: &GithubClient,
    event: &IssuesEvent,
    version: &str,
) -> anyhow::Result<()> {
    let Some(files) = event.issue.diff(gh).await? else {
        return Ok(());
    };
    for (repo, submodule) in [
        ("rust-lang/cargo", "src/tools/cargo"),
        ("rust-lang/reference", "src/doc/reference"),
    ] {
        if let Some(fd) = files.iter().find(|fd| fd.filename == submodule) {
            // The webhook timeout of 10 seconds can be too short, so process in
            // the background.
            let diff = fd.patch.clone();
            let ver = version.to_string();
            tokio::task::spawn(async move {
                let gh = GithubClient::new_from_env();
                if let Err(e) = milestone_submodule(&gh, repo, submodule, &ver, &diff).await {
                    log::error!("failed to milestone {submodule}: {e:?}");
                }
            });
        }
    }

    Ok(())
}

/// Milestones all PRs in the submodule when the submodule is synced in
/// rust-lang/rust.
async fn milestone_submodule(
    gh: &GithubClient,
    repo_name: &str,
    submodule: &str,
    release_version: &str,
    submodule_diff: &str,
) -> anyhow::Result<()> {
    // Determine the start/end range of commits in this submodule update by
    // looking at the diff content which indicates the old and new hash.
    let subproject_re = Regex::new("Subproject commit ([0-9a-f]+)").unwrap();
    let mut caps = subproject_re.captures_iter(submodule_diff);
    let submodule_start_hash = &caps.next().unwrap()[1];
    let submodule_end_hash = &caps.next().unwrap()[1];
    if let Some(next) = caps.next() {
        anyhow::bail!("unexpected submodule capture {}", &next[1]);
    }

    // Get all of the git commits in the submodule repo.
    let submodule_repo = gh.repository(repo_name).await?;
    log::info!(
        "loading submodule {repo_name} changes {submodule_start_hash}...{submodule_end_hash}"
    );
    let commits = submodule_repo
        .github_commits_in_range(gh, submodule_start_hash, submodule_end_hash)
        .await?;

    // For each commit, look for a message that indicates which PR was merged.
    //
    // GitHub has a specific API for this at
    // /repos/{owner}/{repo}/commits/{commit_sha}/pulls
    // <https://docs.github.com/en/rest/commits/commits?apiVersion=2022-11-28#list-pull-requests-associated-with-a-commit>,
    // but it is a little awkward to use, only works on the default branch,
    // and this is a bit simpler/faster. However, it is sensitive to the
    // specific messages generated by bors or GitHub merge queue, and won't
    // catch things merged beyond them.
    let merge_re =
        Regex::new(r"(?:Auto merge of|Merge pull request) #([0-9]+)|\(#([0-9]+)\)$").unwrap();

    let pr_nums = commits
        .iter()
        .filter(|commit|
            // Assumptions:
            // * A merge commit always has two parent commits.
            // * Submodule PRs never got merged as fast-forward / rebase / squash merge.
            commit.parents.len() == 2)
        .filter_map(|commit| {
            let first = commit.commit.message.lines().next().unwrap_or_default();
            merge_re.captures(first).map(|cap| {
                cap.get(1)
                    .or_else(|| cap.get(2))
                    .unwrap()
                    .as_str()
                    .parse::<u64>()
                    .expect("digits only")
            })
        });
    let milestone = submodule_repo
        .get_or_create_milestone(gh, release_version, "closed")
        .await?;
    for pr_num in pr_nums {
        log::info!("setting submodule {submodule} milestone {milestone:?} for {pr_num}");
        submodule_repo.set_milestone(gh, &milestone, pr_num).await?;
    }

    Ok(())
}

async fn milestone_git_subtrees(
    gh: &GithubClient,
    event: &IssuesEvent,
    milestone_version: &str,
) -> anyhow::Result<()> {
    let Ok(Some(event_diff)) = event.issue.compare(&gh).await else {
        log::error!(
            "failed to fetch the event comparision for git subtee milestones {:#?}",
            event.issue
        );
        return Ok(());
    };

    for (repo, subtree) in [("rust-lang/rustfmt", "src/tools/rustfmt")] {
        if event_diff
            .files
            .iter()
            .any(|fd| fd.filename.starts_with(subtree))
        {
            // This PR touches the subtree so we'll try to assign milestones
            let commits = event_diff.commits.clone();
            let mv = milestone_version.to_owned();
            let gh = gh.clone();
            tokio::task::spawn(async move {
                if let Err(e) = milestone_git_subtree(&gh, repo, commits, mv).await {
                    log::error!("failed to milestone {subtree}: {e:?}");
                }
            });
        }
    }

    Ok(())
}

/// Try to sync milestones from rust-lang/rust to one of it's git subtrees.
async fn milestone_git_subtree(
    gh: &GithubClient,
    repo_name: &str,
    commits: Vec<crate::github::GithubCommit>,
    milestone_version: String,
) -> anyhow::Result<()> {
    let Ok(subtree) = gh.repository(repo_name).await else {
        log::error!("failed to fetch repository details for {repo_name}");
        return Ok(());
    };

    let Ok(milestone) = subtree
        .get_or_create_milestone(&gh, &milestone_version, "open")
        .await
    else {
        log::error!(
            "failed to get or create milestone {} for {}",
            &milestone_version,
            repo_name
        );
        return Ok(());
    };

    let merge_commits = stream::iter(
        commits
            .iter()
            .filter(|commit| {
                // A merge commit will have two parents
                commit.parents.len() == 2
            })
            .filter_map(|commit| {
                let commit_title = commit.commit.message.lines().next().unwrap_or_default();
                MERGE_QUEUE_COMMIT_TITLE_RE
                    .captures(commit_title)
                    .map(|cap| {
                        let pr_number = cap
                            .get(1)
                            .unwrap()
                            .as_str()
                            .parse::<u64>()
                            .expect("digits only");
                        (pr_number, commit)
                    })
            }),
    );

    let pr_numbers = merge_commits
        .filter_map(async |(pr_number, merge_commit)| {
            // Check that the merge commit exists on the git subtree
            subtree
                .github_commit(&gh, &merge_commit.sha)
                .await
                .ok()
                .map(|_| pr_number)
        })
        .collect::<Vec<_>>()
        .await;

    log::info!("milestoning {repo_name} PRs: {pr_numbers:?}");

    for pr_number in pr_numbers {
        if let Err(e) = subtree.set_milestone(&gh, &milestone, pr_number).await {
            log::error!(
                "failed to set {} subtree milestone {:?} for {}: {e:?}",
                repo_name,
                milestone,
                pr_number
            );
        };
    }

    Ok(())
}

#[test]
fn test_merge_queue_commit_title_re() {
    let merge_commit_message = "Merge pull request #9999 from user/branch";
    assert!(MERGE_QUEUE_COMMIT_TITLE_RE.is_match(merge_commit_message));
    assert!(
        MERGE_QUEUE_COMMIT_TITLE_RE
            .captures(merge_commit_message)
            .is_some_and(|c| c.get(1).unwrap().as_str() == "9999")
    );
}
