use anyhow::Context as _;

use crate::config::LargePullRequests;
use crate::github::IssuesAction;
use crate::utils::ModifiedPathMatcher;
use crate::{github::Event, handlers::Context};

pub fn large_pull_request_message(threshold: u64) -> String {
    format!(
        "Seems that this pull request is larger than expected (threshold: {threshold} lines of code changed) \
Big pull requests are reviewed much slower than small ones, consider splitting it."
    )
}

pub(crate) async fn handle(
    ctx: &Context,
    event: &Event,
    config: &LargePullRequests,
) -> anyhow::Result<()> {
    let Event::Issue(event) = event else {
        return Ok(());
    };

    // Note that this filters out reopened too, which is what we'd expect when we set the state
    // back to opened after closing.
    if event.action != IssuesAction::Opened && event.action != IssuesAction::Reopened {
        return Ok(());
    }

    if !event.issue.is_pr() {
        return Ok(());
    }

    let files = event.issue.files(&ctx.github).await?;
    let exclude_matcher = ModifiedPathMatcher::new(&config.exclude_files);

    let number_of_changed_lines: u64 = files
        .iter()
        .filter_map(|file| {
            if !exclude_matcher.is_match(&file.filename) {
                Some(file.additions + file.deletions + file.changes)
            } else {
                None
            }
        })
        .sum();

    if number_of_changed_lines >= config.threshold {
        event
            .issue
            .post_comment(&ctx.github, &large_pull_request_message(config.threshold))
            .await
            .context("couldn't post long pull request message")?;
    }

    Ok(())
}
