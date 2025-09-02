//! Purpose: When opening a PR, or pushing new changes, check for any paths
//! that are in the `mentions` config, and add a comment that pings the listed
//! interested people.

use crate::{
    config::{MentionsConfig, MentionsEntryConfig, MentionsEntryType},
    db::issue_data::IssueData,
    github::{IssuesAction, IssuesEvent},
    handlers::Context,
};
use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use std::fmt::Write;
use std::path::Path;
use tracing as log;

const MENTIONS_KEY: &str = "mentions";

pub(super) struct MentionsInput {
    to_mention: Vec<String>,
}

#[derive(Debug, Default, Deserialize, Serialize, Clone, PartialEq)]
struct MentionState {
    #[serde(alias = "paths")]
    entries: Vec<String>,
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
        IssuesAction::Opened | IssuesAction::Synchronize | IssuesAction::ReadyForReview
    ) {
        return Ok(None);
    }

    // Don't ping on rollups or draft PRs.
    if event.issue.title.starts_with("Rollup of")
        || event.issue.draft
        || event.issue.title.contains("[beta] backport")
    {
        return Ok(None);
    }

    if let Some(files) = event
        .issue
        .diff(&ctx.github)
        .await
        .map_err(|e| {
            log::error!("failed to fetch diff: {:?}", e);
        })
        .unwrap_or_default()
    {
        let file_paths: Vec<_> = files.iter().map(|fd| Path::new(&fd.filename)).collect();
        let to_mention: Vec<_> = config
            .entries
            .iter()
            .filter(|(entry, MentionsEntryConfig { cc, type_, .. })| {
                let relevent = match type_ {
                    MentionsEntryType::Filename => {
                        let path = Path::new(entry);
                        // Only mention matching paths.
                        file_paths.iter().any(|p| p.starts_with(path))
                    }
                    MentionsEntryType::Content => {
                        // Only mentions byte-for-byte matching content inside the patch.
                        files.iter().any(|f| f.patch.contains(&**entry))
                    }
                };
                // Don't mention if only the author is in the list.
                let pings_non_author = match &cc[..] {
                    [only_cc] => only_cc.trim_start_matches('@') != &event.issue.user.login,
                    _ => true,
                };
                relevent && pings_non_author
            })
            .map(|(key, _mention)| key.to_string())
            .collect();
        if !to_mention.is_empty() {
            return Ok(Some(MentionsInput { to_mention }));
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
    let mut client = ctx.db.get().await;
    let mut state: IssueData<'_, MentionState> =
        IssueData::load(&mut client, &event.issue, MENTIONS_KEY).await?;
    // Build the message to post to the issue.
    let mut result = String::new();
    for to_mention in &input.to_mention {
        if state.data.entries.iter().any(|p| p == to_mention) {
            // Avoid duplicate mentions.
            continue;
        }
        let MentionsEntryConfig { message, cc, type_ } = &config.entries[to_mention];
        if !result.is_empty() {
            result.push_str("\n\n");
        }
        match message {
            Some(m) => result.push_str(m),
            None => match type_ {
                MentionsEntryType::Filename => {
                    write!(result, "Some changes occurred in {to_mention}").unwrap()
                }
                MentionsEntryType::Content => {
                    write!(result, "Some changes regarding `{to_mention}` occurred").unwrap()
                }
            },
        }
        if !cc.is_empty() {
            write!(result, "\n\ncc {}", cc.join(", ")).unwrap();
        }
        state.data.entries.push(to_mention.to_string());
    }
    if !result.is_empty() {
        event
            .issue
            .post_comment(&ctx.github, &result)
            .await
            .context("failed to post mentions comment")?;
        state.save().await?;
    }
    Ok(())
}
