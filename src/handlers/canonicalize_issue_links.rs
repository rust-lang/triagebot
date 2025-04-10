//! This handler is used to canonicalize linked GitHub issues into their long form
//! so that when pulling subtree into the main repository we don't accidentaly
//! close issues in the wrong repository.
//!
//! Example: `Fixes #123` (in rust-lang/clippy) would now become `Fixes rust-lang/clippy#123`

use std::borrow::Cow;
use std::sync::LazyLock;

use regex::Regex;

use crate::{
    config::CanonicalizeIssueLinksConfig,
    github::{IssuesAction, IssuesEvent},
    handlers::Context,
};

// Taken from https://docs.github.com/en/issues/tracking-your-work-with-issues/using-issues/linking-a-pull-request-to-an-issue?quot#linking-a-pull-request-to-an-issue-using-a-keyword
static LINKED_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new("(?i)(?P<action>close|closes|closed|fix|fixes|fixed|resolve|resolves|resolved)(?P<spaces>:? +)(?P<issue>#[0-9]+)")
        .unwrap()
});

pub(super) struct CanonicalizeIssueLinksInput {}

pub(super) async fn parse_input(
    _ctx: &Context,
    event: &IssuesEvent,
    config: Option<&CanonicalizeIssueLinksConfig>,
) -> Result<Option<CanonicalizeIssueLinksInput>, String> {
    if !event.issue.is_pr() {
        return Ok(None);
    }

    if !matches!(
        event.action,
        IssuesAction::Opened | IssuesAction::Reopened | IssuesAction::Edited
    ) {
        return Ok(None);
    }

    // Require a `[canonicalize-issue-links]` configuration block to enable the handler.
    if config.is_none() {
        return Ok(None);
    };

    Ok(Some(CanonicalizeIssueLinksInput {}))
}

pub(super) async fn handle_input(
    ctx: &Context,
    _config: &CanonicalizeIssueLinksConfig,
    e: &IssuesEvent,
    _input: CanonicalizeIssueLinksInput,
) -> anyhow::Result<()> {
    let full_repo_name = e.issue.repository().full_repo_name();

    let new_body = fix_linked_issues(&e.issue.body, full_repo_name.as_str());

    if e.issue.body != new_body {
        e.issue.edit_body(&ctx.github, &new_body).await?;
    }

    Ok(())
}

fn fix_linked_issues<'a>(body: &'a str, full_repo_name: &str) -> Cow<'a, str> {
    let replace_by = format!("${{action}}${{spaces}}{full_repo_name}${{issue}}");
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
    Closes: #987
    resolves:   #655
    Resolves #00000 Closes #888
    "#;

    let fixed_body = r#"
    This is a PR.

    Fix rust-lang/rust#123
    fixed rust-lang/rust#456
    Fixes    rust-lang/rust#7895
    Closes: rust-lang/rust#987
    resolves:   rust-lang/rust#655
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
    Resolves: #abgt
    "#;

    let new_body = fix_linked_issues(body, full_repo_name);
    assert_eq!(new_body, body);
}
