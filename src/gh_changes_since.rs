use std::sync::Arc;

use anyhow::Context as _;
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Redirect, Response},
};
use hyper::StatusCode;

use crate::{errors::AppError, github, handlers::Context, utils::is_repo_autorized};

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

    if !is_repo_autorized(&ctx, &owner, &repo).await? {
        return Ok((
            StatusCode::UNAUTHORIZED,
            format!("repository `{owner}/{repo}` is not part of the Rust Project team repos"),
        )
            .into_response());
    }

    let issue_repo = github::IssueRepository {
        organization: owner.to_string(),
        repository: repo.to_string(),
    };

    let pr = ctx
        .github
        .pull_request(&issue_repo, pr_num)
        .await
        .context("could not get the pull request details")?;

    let newbase = &pr.base.as_ref().context("no base")?.sha;
    let newhead = &pr.head.as_ref().context("no head")?.sha;

    // Has the base changed?
    if oldbase == newbase {
        // No, try finding if only new commits have been added by trying to finding the oldhead between oldbase..newhead
        let cmp = ctx
            .github
            .compare(&issue_repo, oldbase, newhead)
            .await
            .with_context(|| format!("unable to fetch the comparaison between the oldhead ({oldhead}) and newhead ({newhead})"))?;

        // Have only new commits been added?
        if cmp.commits.iter().find(|c| c.sha == oldhead).is_some() {
            // Yes, redirect to GitHub PR changes page
            return Ok(Redirect::to(&format!(
                "https://github.com/{owner}/{repo}/pull/{pr_num}/changes/{oldhead}..{newhead}"
            ))
            .into_response());
        }

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
