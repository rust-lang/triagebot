use anyhow::Context as _;

use crate::config::RangeDiffConfig;
use crate::github::GithubCompare;
use crate::github::IssueRepository;
use crate::github::IssuesEvent;
use crate::handlers::Context;

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
    let org = event.repository.owner();
    let repo = event.repository.name();

    let compare_before = ctx
        .github
        .compare(
            &IssueRepository {
                organization: org.to_string(),
                repository: repo.to_string(),
            },
            &base.sha,
            &before,
        )
        .await
        .context("failed to get the before compare")?;

    // Does the merge_base_commits differs? No, not a force-push with rebase.
    if compare_before.merge_base_commit.sha == compare_after.merge_base_commit.sha {
        return Ok(());
    }

    let protocol = if host.starts_with("localhost:") {
        "http"
    } else {
        "https"
    };

    let branch = &base.git_ref;
    let (oldbase, oldhead) = (&compare_before.merge_base_commit.sha, before);
    let (newbase, newhead) = (&compare_after.merge_base_commit.sha, after);

    // Rebase detected, post a comment to our range-diff.
    event.issue.post_comment(&ctx.github,
        &format!(r#"This PR was rebased onto a different {branch} commit! Check out the changes with our [`range-diff`]({protocol}://{host}/gh-range-diff/{org}/{repo}/{oldbase}..{oldhead}/{newbase}..{newhead})."#)
    ).await.context("failed to post range-diff comment")?;

    Ok(())
}
