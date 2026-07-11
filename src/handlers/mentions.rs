//! Purpose: When opening a PR, or pushing new changes, check for any paths
//! that are in the `mentions` config, and add a comment that pings the listed
//! interested people.

use crate::{
    config::{MentionsConfig, MentionsEntryConfig, MentionsEntryType},
    db::issue_data::IssueData,
    github::{IssuesAction, IssuesEvent},
    handlers::Context,
    utils::ModifiedPathMatcher,
};
use anyhow::Context as _;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::{fmt::Write, path::PathBuf};

const MENTIONS_KEY: &str = "mentions";

pub(super) struct MentionsInput {
    to_mention: Vec<ToMention>,
}

#[derive(Debug)]
struct ToMention {
    entry: String,
    relevant_file_paths: Vec<PathBuf>,
    relevant_ccs: Vec<String>,
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
    let Some(config) = config else {
        return Ok(None);
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

    // Fetch the PR diff
    let diff = event.issue.diff(&ctx.github).await;

    // Print the error if we got one
    let Ok(Some(modified_files)) = diff else {
        if let Err(err) = diff {
            tracing::error!("failed to fetch diff for mentions handler: {err:?}");
        }
        return Ok(None);
    };

    let mut modified_paths: Vec<_> = modified_files
        .iter()
        .map(|fd| Path::new(&fd.filename))
        .collect();

    // also include the previous filename (in case of renames)
    modified_paths.extend(
        modified_files
            .iter()
            .filter_map(|fd| fd.previous_filename.as_ref())
            .map(|filename| Path::new(filename)),
    );

    let to_mention: Vec<_> = config
        .entries
        .iter()
        .filter_map(|(entry, cfg @ MentionsEntryConfig { cc, type_, .. })| {
            let relevant_file_paths: Vec<PathBuf> = match type_ {
                MentionsEntryType::Filename => {
                    // Only mention matching paths.
                    let matcher = ModifiedPathMatcher::single(entry);
                    modified_paths
                        .iter()
                        .copied()
                        .filter(move |p| matcher.is_match(p))
                        .map(|p| p.to_owned())
                        .collect()
                }
                MentionsEntryType::Content => {
                    let file_matcher = ModifiedPathMatcher::new(&cfg.trigger_files);
                    let file_match = |f| file_matcher.is_match(Path::new(f));
                    // Only mentions byte-for-byte matching content inside the patch.
                    modified_files
                        .iter()
                        .filter(|f| {
                            patch_adds(&f.patch, entry)
                                && (cfg.trigger_files.is_empty()
                                    || file_match(&f.filename)
                                    || f.previous_filename.as_ref().is_some_and(|p| file_match(p)))
                        })
                        .map(|f| PathBuf::from(&f.filename))
                        .collect()
                }
            };

            // Filter author from the cc list
            let relevant_ccs = cc
                .iter()
                .filter(|cc| {
                    cc.trim_start_matches('@').to_lowercase()
                        != event.issue.user.login.to_lowercase()
                })
                .cloned()
                .collect::<Vec<_>>();

            // Has any relevant CCs? (empty cc is allowed)
            let has_relevant_ccs = !relevant_ccs.is_empty() || cc.is_empty();

            if !relevant_file_paths.is_empty() && has_relevant_ccs {
                Some(ToMention {
                    entry: entry.to_string(),
                    relevant_file_paths,
                    relevant_ccs,
                })
            } else {
                None
            }
        })
        .collect();

    if to_mention.is_empty() {
        Ok(None)
    } else {
        Ok(Some(MentionsInput { to_mention }))
    }
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
    for ToMention {
        entry,
        relevant_file_paths,
        relevant_ccs,
    } in input.to_mention
    {
        if state.data.entries.iter().any(|e| e == &entry) {
            // Avoid duplicate mentions.
            continue;
        }
        let MentionsEntryConfig { message, type_, .. } = &config.entries[&entry];
        if !result.is_empty() {
            result.push_str("\n\n");
        }
        match message {
            Some(m) => result.push_str(m),
            None => match type_ {
                MentionsEntryType::Filename => {
                    write!(result, "Some changes occurred in {entry}").unwrap();
                }
                MentionsEntryType::Content => write!(
                    result,
                    "Some changes regarding `{entry}` occurred in {}",
                    relevant_file_paths
                        .iter()
                        .map(|f| f.to_string_lossy())
                        .format(", ")
                )
                .unwrap(),
            },
        }

        if !relevant_ccs.is_empty() {
            write!(result, "\n\ncc {}", relevant_ccs.join(", ")).unwrap();
        }
        state.data.entries.push(entry);
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

fn patch_adds(patch: &str, needle: &str) -> bool {
    patch
        .lines()
        .any(|line| !line.starts_with("+++") && line.starts_with('+') && line.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_added_line() {
        let patch = "\
--- a/file.txt
+++ b/file.txt
+hello world
 context line
";
        assert!(patch_adds(patch, "hello"));
    }

    #[test]
    fn finds_added_line_in_modified() {
        let patch = "\
--- a/file.txt
+++ b/file.txt
-hello
+hello world
";
        assert!(patch_adds(patch, "hello"));
    }

    #[test]
    fn ignore_removed_line() {
        let patch = "\
--- a/file.txt
+++ b/file.txt
-old value
+new value
";
        assert!(!patch_adds(patch, "old value"));
    }

    #[test]
    fn ignores_diff_headers() {
        let patch = "\
--- a/file.txt
+++ b/file.txt
 context line
";
        assert!(!patch_adds(patch, "file.txt")); // should *not* match header
    }

    #[test]
    fn needle_not_present() {
        let patch = "\
--- a/file.txt
+++ b/file.txt
+added line
";
        assert!(!patch_adds(patch, "missing"));
    }
}
