use crate::{
    github::{Event, IssuesAction},
    handlers::Context,
};
use anyhow::Context as _;
use reqwest::StatusCode;
use tracing as log;

pub async fn handle(ctx: &Context, event: &Event) -> anyhow::Result<()> {
    let e = if let Event::Issue(e) = event {
        e
    } else {
        return Ok(());
    };

    // Only trigger on closed issues
    if e.action != IssuesAction::Closed {
        return Ok(());
    }

    let repo = e.issue.repository();
    if !(repo.organization == "rust-lang" && repo.repository == "rust") {
        return Ok(());
    }

    if !e.issue.merged {
        log::trace!(
            "Ignoring closing of rust-lang/rust#{}: not merged",
            e.issue.number
        );
        return Ok(());
    }

    let merge_sha = if let Some(s) = &e.issue.merge_commit_sha {
        s
    } else {
        log::error!(
            "rust-lang/rust#{}: no merge_commit_sha in event",
            e.issue.number
        );
        return Ok(());
    };

    // Fetch the version from the upstream repository.
    let version = if let Some(version) = get_version_standalone(ctx, merge_sha).await? {
        version
    } else {
        log::error!("could not find the version of {:?}", merge_sha);
        return Ok(());
    };

    if !version.starts_with("1.") && version.len() < 8 {
        log::error!("Weird version {:?} for {:?}", version, merge_sha);
        return Ok(());
    }

    // Associate this merged PR with the version it merged into.
    //
    // Note that this should work for rollup-merged PRs too. It will *not*
    // auto-update when merging a beta-backport, for example, but that seems
    // fine; we can manually update without too much trouble in that case, and
    // eventually automate it separately.
    e.issue.set_milestone(&ctx.github, &version).await?;

    Ok(())
}

async fn get_version_standalone(ctx: &Context, merge_sha: &str) -> anyhow::Result<Option<String>> {
    let resp = ctx
        .github
        .raw()
        .get(&format!(
            "https://raw.githubusercontent.com/rust-lang/rust/{}/src/version",
            merge_sha
        ))
        .send()
        .await
        .with_context(|| format!("retrieving src/version for {}", merge_sha))?;

    match resp.status() {
        StatusCode::OK => {}
        // Don't treat a 404 as a failure, we'll try another way to retrieve the version.
        StatusCode::NOT_FOUND => return Ok(None),
        status => anyhow::bail!(
            "unexpected status code {} while retrieving src/version for {}",
            status,
            merge_sha
        ),
    }

    Ok(Some(
        resp.text()
            .await
            .with_context(|| format!("deserializing src/version for {}", merge_sha))?
            .trim()
            .to_string(),
    ))
}
