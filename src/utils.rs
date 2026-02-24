use crate::handlers::Context;

use anyhow::Context as _;
use axum::http::HeaderValue;
use hyper::{
    HeaderMap,
    header::{CACHE_CONTROL, CONTENT_TYPE},
};
use std::borrow::Cow;

/// Pluralize (add an 's' sufix) to `text` based on `count`.
pub fn pluralize(text: &str, count: usize) -> Cow<'_, str> {
    if count == 1 {
        text.into()
    } else {
        format!("{text}s").into()
    }
}

/// Can triagebot provide extended GitHub features (such as comments, logs, etc.)
/// for this repository for unauthorized users?
pub(crate) async fn is_known_and_public_repo(
    ctx: &Context,
    owner: &str,
    repo: &str,
) -> anyhow::Result<bool> {
    let repos = ctx
        .team
        .repos()
        .await
        .context("unable to retrieve team repos")?;

    // Verify that the request org is part of the Rust project
    let Some(repos) = repos.repos.get(owner) else {
        return Ok(false);
    };

    let repo = repos.iter().find(|r| r.name == repo);
    // Verify that the request repo is part of the Rust project
    let Some(repo) = repo else {
        return Ok(false);
    };

    // Only allow public repositories
    if repo.private {
        return Ok(false);
    }

    Ok(true)
}

pub(crate) async fn is_issue_under_rfcbot_fcp(
    issue_full_repo_name: &str,
    issue_number: u64,
) -> bool {
    match crate::rfcbot::get_all_fcps().await {
        Ok(fcps) => {
            if fcps.iter().any(|(_, fcp)| {
                u64::from(fcp.issue.number) == issue_number
                    && fcp.issue.repository == issue_full_repo_name
            }) {
                return true;
            }
        }
        Err(err) => {
            tracing::warn!("unable to fetch rfcbot active FCPs: {err:?}, skipping check");
        }
    }

    false
}

pub(crate) fn immutable_headers(content_type: &'static str) -> HeaderMap {
    let mut headers = HeaderMap::new();

    headers.insert(
        CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=15552000, immutable"),
    );
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(content_type));

    headers
}
