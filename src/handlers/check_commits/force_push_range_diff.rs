use anyhow::Context as _;

use crate::config::RangeDiffConfig;
use crate::db::issue_data::IssueData;
use crate::github::CommitBase;
use crate::github::GithubCompare;
use crate::github::IssueRepository;
use crate::github::IssuesEvent;
use crate::github::ReportedContentClassifiers;
use crate::handlers::Context;

/// Key for the state in the database
const RANGE_DIFF_LINK_KEY: &str = "range-diff-link";

/// State stored in the database
#[derive(Debug, Default, serde::Deserialize, serde::Serialize, Clone, PartialEq)]
struct RangeDiffLinkState {
    /// ID of the most recent range-diff comment.
    last_comment: Option<String>,
}

pub(super) async fn handle_event(
    ctx: &Context,
    host: &str,
    _config: &RangeDiffConfig,
    event: &IssuesEvent,
    compare_after: &GithubCompare,
) -> anyhow::Result<()> {
    if !matches!(event.action, crate::github::IssuesAction::Synchronize) {
        return Ok(());
    }

    let (Some(before), Some(after)) = (event.before.as_ref(), event.after.as_ref()) else {
        tracing::warn!("synchronize event but no before or after field");
        return Ok(());
    };

    let base = event.issue.base.as_ref().context("no base ref")?;
    let issue_repo = IssueRepository {
        organization: event.repository.owner().to_string(),
        repository: event.repository.name().to_string(),
    };

    let compare_before = ctx
        .github
        .compare(&issue_repo, &base.sha, before)
        .await
        .context("failed to get the before compare")?;

    if let Some(message) = changed_base_commit(
        host,
        &issue_repo,
        base,
        &compare_before,
        compare_after,
        before,
        after,
    ) {
        // Rebase detected, post a comment linking to our range-diff.
        post_new_comment(ctx, event, message).await?;
    }

    Ok(())
}

fn changed_base_commit(
    host: &str,
    issue_repo: &IssueRepository,
    base: &CommitBase,
    compare_before: &GithubCompare,
    compare_after: &GithubCompare,
    oldhead: &str,
    newhead: &str,
) -> Option<String> {
    // Does the merge_base_commits differs? No, not a force-push with rebase.
    if compare_before.merge_base_commit.sha == compare_after.merge_base_commit.sha {
        return None;
    }

    let protocol = if host.starts_with("localhost:") {
        "http"
    } else {
        "https"
    };

    let branch = &base.git_ref;
    let oldbase = &compare_before.merge_base_commit.sha;
    let newbase = &compare_after.merge_base_commit.sha;

    let message = format!(
        r"This PR was rebased onto a different {branch} commit. Here's a [range-diff]({protocol}://{host}/gh-range-diff/{issue_repo}/{oldbase}..{oldhead}/{newbase}..{newhead}) highlighting what actually changed.

*Rebasing is a normal part of keeping PRs up to date, so no action is needed—this note is just to help reviewers.*"
    );

    Some(message)
}

async fn post_new_comment(
    ctx: &Context,
    event: &IssuesEvent,
    message: String,
) -> anyhow::Result<()> {
    let mut db = ctx.db.get().await;
    let mut state: IssueData<'_, RangeDiffLinkState> =
        IssueData::load(&mut db, &event.issue, RANGE_DIFF_LINK_KEY).await?;

    // Hide previous range-diff comment.
    if let Some(last_comment) = state.data.last_comment {
        event
            .issue
            .hide_comment(
                &ctx.github,
                &last_comment,
                ReportedContentClassifiers::Outdated,
            )
            .await
            .context("failed to hide previous range-diff comment")?;
    }

    // Post new range-diff comment and remember it.
    state.data.last_comment = Some(
        event
            .issue
            .post_comment(&ctx.github, &message)
            .await
            .context("failed to post range-diff comment")?
            .node_id,
    );

    state.save().await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::handlers::check_commits::dummy_commit_from_body;

    use super::changed_base_commit;

    #[test]
    fn unchanged_base_commit() {
        assert_eq!(
            changed_base_commit(
                "mytriagebot.com",
                &crate::github::IssueRepository {
                    organization: "rust-lang".to_string(),
                    repository: "rust".to_string()
                },
                &crate::github::CommitBase {
                    sha: "sha-base-commit".to_string(),
                    git_ref: "master".to_string(),
                    repo: None
                },
                &crate::github::GithubCompare {
                    base_commit: dummy_commit_from_body("base-commit-sha", "base-commit-body"),
                    merge_base_commit: dummy_commit_from_body(
                        "same-merge-commit",
                        "merge-commit-body"
                    ),
                    files: vec![]
                },
                &crate::github::GithubCompare {
                    base_commit: dummy_commit_from_body("base-commit-sha", "base-commit-body"),
                    merge_base_commit: dummy_commit_from_body(
                        "same-merge-commit",
                        "merge-commit-body"
                    ),
                    files: vec![]
                },
                "oldhead",
                "newhead"
            ),
            None
        );
    }

    #[test]
    fn changed_base_commit_() {
        assert_eq!(
            changed_base_commit(
                "mytriagebot.com",
                &crate::github::IssueRepository {
                    organization: "rust-lang".to_string(),
                    repository: "rust".to_string()
                },
                &crate::github::CommitBase {
                    sha: "sha-base-commit".to_string(),
                    git_ref: "master".to_string(),
                    repo: None
                },
                &crate::github::GithubCompare {
                    base_commit: dummy_commit_from_body("base-commit-sha", "base-commit-body"),
                    merge_base_commit: dummy_commit_from_body(
                        "before-merge-commit",
                        "merge-commit-body"
                    ),
                    files: vec![]
                },
                &crate::github::GithubCompare {
                    base_commit: dummy_commit_from_body("base-commit-sha", "base-commit-body"),
                    merge_base_commit: dummy_commit_from_body(
                        "after-merge-commit",
                        "merge-commit-body"
                    ),
                    files: vec![]
                },
                "oldhead",
                "newhead"
            ),
            Some(
                r#"This PR was rebased onto a different master commit. Here's a [range-diff](https://mytriagebot.com/gh-range-diff/rust-lang/rust/before-merge-commit..oldhead/after-merge-commit..newhead) highlighting what actually changed.

*Rebasing is a normal part of keeping PRs up to date, so no action is needed—this note is just to help reviewers.*"#
                .to_string()
            )
        );
    }
}
