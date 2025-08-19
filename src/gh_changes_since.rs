use std::sync::Arc;

use anyhow::Context as _;
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Redirect, Response},
};
use hyper::StatusCode;

use crate::{github, handlers::Context, utils::AppError};

/// Redirects to either `/gh-range-diff` (when the base changed) or to GitHub's compare
/// page (when the base is the same).
///
/// Takes an PR number and an `oldbase..oldhead` representing the range we are starting from.
pub async fn gh_changes_since(
    Path((owner, repo, pr_num, oldbasehead)): Path<(String, String, u64, String)>,
    State(ctx): State<Arc<Context>>,
) -> axum::response::Result<Response, AppError> {
    let Some((oldbase, oldhead)) = oldbasehead.split_once("..") else {
        return Ok((
            StatusCode::BAD_REQUEST,
            format!("`{oldbasehead}` is not in the form `base..head`"),
        )
            .into_response());
    };

    let repos = ctx
        .team
        .repos()
        .await
        .context("unable to retrieve team repos")?;

    // Verify that the request org is part of the Rust project
    let Some(repos) = repos.repos.get(&owner) else {
        return Ok((
            StatusCode::BAD_REQUEST,
            format!("organization `{owner}` is not part of the Rust Project team repos"),
        )
            .into_response());
    };

    // Verify that the request repo is part of the Rust project
    if !repos.iter().any(|r| r.name == repo) {
        return Ok((
            StatusCode::BAD_REQUEST,
            format!("repository `{owner}` is not part of the Rust Project team repos"),
        )
            .into_response());
    }

    let issue_repo = github::IssueRepository {
        organization: owner.to_string(),
        repository: repo.to_string(),
    };

    let pr = ctx.github.pull_request(&issue_repo, pr_num).await?;

    let newbase = &pr.base.as_ref().context("no base")?.sha;
    let newhead = &pr.head.as_ref().context("no head")?.sha;

    // Has the base changed?
    if oldbase == newbase {
        // No, redirect to GitHub native compare page
        return Ok(Redirect::to(&format!(
            "https://github.com/{owner}/{repo}/compare/{oldhead}..{newhead}"
        ))
        .into_response());
    }

    // Yes, use our Github range-diff instead
    Ok(Redirect::to(&format!(
        "/gh-range-diff/{owner}/{repo}/{oldbase}..{oldhead}/{newbase}..{newhead}"
    ))
    .into_response())
}
