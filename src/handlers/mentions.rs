//! Purpose: When opening a PR, or pushing new changes, check for any paths
//! that are in the `mentions` config, and add a comment that pings the listed
//! interested people.

use crate::{
    config::{MentionsConfig, MentionsPathConfig},
    db::issue_data,
    github::{files_changed, IssuesAction, IssuesEvent},
    handlers::Context,
};
use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use std::fmt::Write;
use std::path::Path;
use tracing as log;

const MENTIONS_KEY: &str = "mentions";

pub(super) struct MentionsInput {
    paths: Vec<String>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct MentionState {
    paths: Vec<String>,
}

pub(super) async fn parse_input(
    ctx: &Context,
    event: &IssuesEvent,
    config: Option<&MentionsConfig>,
) -> Result<Option<MentionsInput>, String> {
    let config = match config {
        Some(config) => config,
        None => return Ok(None),
    };

    if !matches!(
        event.action,
        IssuesAction::Opened | IssuesAction::Synchronize
    ) {
        return Ok(None);
    }

    if let Some(diff) = event
        .issue
        .diff(&ctx.github)
        .await
        .map_err(|e| {
            log::error!("failed to fetch diff: {:?}", e);
        })
        .unwrap_or_default()
    {
        let files = files_changed(&diff);
        let file_paths: Vec<_> = files.iter().map(|p| Path::new(p)).collect();
        let to_mention: Vec<_> = config
            .paths
            .iter()
            // Only mention matching paths.
            // Don't mention if the author is in the list.
            .filter(|(path, MentionsPathConfig { reviewers, .. })| {
                let path = Path::new(path);
                file_paths.iter().any(|p| p.starts_with(path))
                    && !reviewers.iter().any(|r| r == &event.issue.user.login)
            })
            .map(|(key, _mention)| key.to_string())
            .collect();
        if !to_mention.is_empty() {
            return Ok(Some(MentionsInput { paths: to_mention }));
        }
    }
    Ok(None)
}

pub(super) async fn handle_input(
    ctx: &Context,
    config: &MentionsConfig,
    event: &IssuesEvent,
    input: MentionsInput,
) -> anyhow::Result<()> {
    let client = ctx.db.get().await;
    let mut state: MentionState = issue_data::load(&client, &event.issue, MENTIONS_KEY)
        .await?
        .unwrap_or_default();
    // Build the message to post to the issue.
    let mut result = String::new();
    for to_mention in &input.paths {
        if state.paths.iter().any(|p| p == to_mention) {
            // Avoid duplicate mentions.
            continue;
        }
        let MentionsPathConfig { message, reviewers } = &config.paths[to_mention];
        if !result.is_empty() {
            result.push_str("\n\n");
        }
        match message {
            Some(m) => result.push_str(m),
            None => write!(result, "Some changes occurred in {to_mention}").unwrap(),
        }
        if !reviewers.is_empty() {
            write!(result, "\n\ncc {}", reviewers.join(",")).unwrap();
        }
        state.paths.push(to_mention.to_string());
    }
    if !result.is_empty() {
        event
            .issue
            .post_comment(&ctx.github, &result)
            .await
            .context("failed to post mentions comment")?;
        issue_data::save(&client, &event.issue, MENTIONS_KEY, &state).await?;
    }
    Ok(())
}
