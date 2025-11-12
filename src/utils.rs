use crate::handlers::Context;

use anyhow::Context as _;
use std::borrow::Cow;

/// Pluralize (add an 's' sufix) to `text` based on `count`.
pub fn pluralize(text: &str, count: usize) -> Cow<'_, str> {
    if count == 1 {
        text.into()
    } else {
        format!("{text}s").into()
    }
}

pub(crate) async fn is_repo_autorized(
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

    // Verify that the request repo is part of the Rust project
    if !repos.iter().any(|r| r.name == repo) {
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
