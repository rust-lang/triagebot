//! This handler is used to "relink" linked GitHub issues into their long form
//! so that when pulling subtree into the main repository we don't accidentaly
//! closes issue in the wrong repository.
//!
//! Example: `Fixes #123` (in rust-lang/clippy) would now become `Fixes rust-lang/clippy#123`

use std::borrow::Cow;
use std::sync::LazyLock;

use regex::Regex;

use crate::{
    config::RelinkConfig,
    github::{Event, IssuesAction, IssuesEvent},
    handlers::Context,
};

// Taken from https://docs.github.com/en/issues/tracking-your-work-with-issues/using-issues/linking-a-pull-request-to-an-issue?quot#linking-a-pull-request-to-an-issue-using-a-keyword
static LINKED_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new("(?i)(close|closes|closed|fix|fixes|fixed|resolve|resolves|resolved) +(#[0-9]+)")
        .unwrap()
});

pub async fn handle(ctx: &Context, event: &Event, config: &RelinkConfig) -> anyhow::Result<()> {
    let Event::Issue(e) = event else {
        return Ok(());
    };

    if !e.issue.is_pr() {
        return Ok(());
    }

    if let Err(e) = relink_pr(&ctx, &e, config).await {
        tracing::error!("Error relinking pr: {:?}", e);
    }

    Ok(())
}

async fn relink_pr(ctx: &Context, e: &IssuesEvent, _config: &RelinkConfig) -> anyhow::Result<()> {
    if e.action == IssuesAction::Opened
        || e.action == IssuesAction::Reopened
        || e.action == IssuesAction::Edited
    {
        let full_repo_name = e.issue.repository().full_repo_name();

        let new_body = fix_linked_issues(&e.issue.body, full_repo_name.as_str());

        if e.issue.body != new_body {
            e.issue.edit_body(&ctx.github, &new_body).await?;
        }
    }

    Ok(())
}

fn fix_linked_issues<'a>(body: &'a str, full_repo_name: &str) -> Cow<'a, str> {
    let replace_by = format!("$1 {full_repo_name}$2");
    LINKED_RE.replace_all(body, replace_by)
}

#[test]
fn fixed_body() {
    let full_repo_name = "rust-lang/rust";

    let body = r#"
    This is a PR.

    Fix #123
    fixed #456
    Fixes    #7895
    Resolves #00000 Closes #888
    "#;

    let fixed_body = r#"
    This is a PR.

    Fix rust-lang/rust#123
    fixed rust-lang/rust#456
    Fixes rust-lang/rust#7895
    Resolves rust-lang/rust#00000 Closes rust-lang/rust#888
    "#;

    let new_body = fix_linked_issues(body, full_repo_name);
    assert_eq!(new_body, fixed_body);
}

#[test]
fn untouched_body() {
    let full_repo_name = "rust-lang/rust";

    let body = r#"
    This is a PR.

    Fix rust-lang#123
    Fixesd #7895
    Resolves #abgt
    "#;

    let new_body = fix_linked_issues(body, full_repo_name);
    assert_eq!(new_body, body);
}
