use crate::{
    github::{Event, IssuesAction},
    handlers::Context,
};
use anyhow::Context as _;

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
    if repo.organization != "rust-lang" && repo.repository != "rust" {
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

    // Fetch channel.rs from the upstream repository

    let resp = ctx
        .github
        .raw()
        .get(&format!(
            "https://raw.githubusercontent.com/rust-lang/rust/{}/src/bootstrap/channel.rs",
            merge_sha
        ))
        .send()
        .await
        .with_context(|| format!("retrieving channel.rs for {}", merge_sha))?;

    let resp = resp
        .text()
        .await
        .with_context(|| format!("deserializing text channel.rs for {}", merge_sha))?;

    let prefix = r#"const CFG_RELEASE_NUM: &str = ""#;
    let start = if let Some(idx) = resp.find(prefix) {
        idx + prefix.len()
    } else {
        log::error!(
            "No {:?} in contents of channel.rs at {:?}",
            prefix,
            merge_sha
        );
        return Ok(());
    };
    let after = &resp[start..];
    let end = if let Some(idx) = after.find('"') {
        idx
    } else {
        log::error!("No suffix in contents of channel.rs at {:?}", merge_sha);
        return Ok(());
    };
    let version = &after[..end];
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
    e.issue.set_milestone(&ctx.github, version).await?;

    Ok(())
}
