use anyhow::Context as _;

use crate::{
    config::ViewAllCommentsLinkConfig,
    github::{Event, Issue},
    handlers::Context,
};

// Based on some crude experiments at around 25 events in timelines GitHub
// starts being lazy and shows it's "Load more" button.
//
// Unfortunately the webhook don't give us the number of timeline events
// but we get the number of comments (without doing any API calls!).
//
// So we approximate to 20 comments (+5 events) the minimum threashold.
const DEFAULT_THRESHOLD: u32 = 20;

pub(super) async fn handle(
    ctx: &Context,
    event: &Event,
    host: &str,
    config: &ViewAllCommentsLinkConfig,
) -> anyhow::Result<()> {
    let Some(issue) = event.issue() else {
        return Ok(());
    };

    if event.user().login == ctx.username {
        // just in case, ignore our own events on issues/PRs
        return Ok(());
    }

    if config.exclude_issues && !issue.is_pr() {
        return Ok(());
    }

    if config.exclude_prs && issue.is_pr() {
        return Ok(());
    }

    if issue.comments.unwrap_or(0) < config.threshold.unwrap_or(DEFAULT_THRESHOLD) {
        return Ok(());
    }

    add_comments_link(ctx, issue, host).await
}

async fn add_comments_link(ctx: &Context, issue: &Issue, host: &str) -> anyhow::Result<()> {
    let repo_name = issue.repository().full_repo_name();
    let type_ = if issue.is_pr() { "pull" } else { "issues" };
    let issue_number = issue.number;

    let mut comments_link = format!(
        "*[View all comments](https://{host}/gh-comments/{repo_name}/{type_}/{issue_number})*"
    );

    // This repository has bors which inlines the PR body by default into merge commits, and we
    // want to hide this from the resulting merge commit. Wrapping it in this section will avoid
    // bors including it in the resulting "Auto merge..." commit.
    //
    // Implementation in bors:
    // https://github.com/rust-lang/bors/blob/b89a1d15c6c68d08964971dab4f4117ae9edb533/src/bors/mod.rs#L307
    if repo_name == "rust-lang/rust" {
        comments_link =
            format!("<!-- homu-ignore:start -->\n{comments_link}\n<!-- homu-ignore:end -->");
    }

    if !issue.body.contains("[View all comments](") {
        // add comments link to the start of the body
        let new_body = format!("{comments_link}\n\n{}", issue.body);

        tracing::info!(
            r#"adding "View all comments" link to {repo_name}#{}"#,
            issue.number
        );
        issue
            .edit_body(&ctx.github, &new_body)
            .await
            .context("failed to edit the body")?;
    }

    Ok(())
}
