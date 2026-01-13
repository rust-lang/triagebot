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
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::{fmt::Write, path::PathBuf};

const MENTIONS_KEY: &str = "mentions";

pub(super) struct MentionsInput {
    to_mention: Vec<ToMention>,
}

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

    let modified_paths: Vec<_> = modified_files
        .iter()
        .map(|fd| Path::new(&fd.filename))
        .collect();

    let to_mention: Vec<_> = config
        .entries
        .iter()
        .filter_map(|(entry, MentionsEntryConfig { cc, type_, .. })| {
            let relevant_file_paths: Vec<PathBuf> = match type_ {
                MentionsEntryType::Filename => {
                    // Only mention matching paths.
                    modified_paths_matches(&modified_paths, entry)
                }
                MentionsEntryType::Content => {
                    // Only mentions byte-for-byte matching content inside the patch.
                    modified_files
                        .iter()
                        .filter(|f| patch_adds(&f.patch, entry))
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

            if !relevant_file_paths.is_empty() && !relevant_ccs.is_empty() {
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

fn modified_paths_matches(modified_paths: &[&Path], entry: &str) -> Vec<PathBuf> {
    // Fast-path if entry has no glob components
    if globset::escape(entry) == entry {
        let path = Path::new(entry);

        // Return paths that starts with entry
        return modified_paths
            .iter()
            .filter(|p| p.starts_with(path))
            .map(PathBuf::from)
            .collect();
    }

    // Prepare the glob pattern from the entry.
    //
    // We first trim any excess `/` at the end of the pattern and then add an alternate
    // to match on the path as is or in any sub-directories.
    let pattern = entry.trim_end_matches('/');
    let pattern = format!("{pattern}{{,/*}}");

    // Create the glob pattern and log an error (should have already been reported to
    // the user).
    let pattern = match globset::GlobBuilder::new(&pattern)
        .empty_alternates(true)
        .build()
    {
        Ok(pattern) => pattern,
        Err(err) => {
            tracing::error!("invalid glob pattern for [mentions.\"{entry}\"]: {err}");
            return Vec::new();
        }
    };

    // Compile the glob pattern to a (Regex) matcher
    let matcher = pattern.compile_matcher();

    modified_paths
        .iter()
        .filter(|p| matcher.is_match(p))
        .map(PathBuf::from)
        .collect()
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

    #[test]
    fn entry_not_modified() {
        assert_eq!(
            modified_paths_matches(
                &[
                    Path::new("library/Cargo.lock"),
                    Path::new("library/Cargo.toml")
                ],
                "compiler/rustc_span/",
            ),
            Vec::<PathBuf>::new()
        );
    }

    #[test]
    fn entry_modified() {
        assert_eq!(
            modified_paths_matches(
                &[
                    Path::new("Cargo.lock"),
                    Path::new("compiler/rustc_span/src/lib.rs"),
                    Path::new("compiler/rustc_span/src/symbol.rs"),
                ],
                "compiler/rustc_span/",
            ),
            vec![
                PathBuf::from("compiler/rustc_span/src/lib.rs"),
                PathBuf::from("compiler/rustc_span/src/symbol.rs"),
            ]
        );
    }

    #[test]
    fn entry_filename_modified() {
        assert_eq!(
            modified_paths_matches(
                &[
                    Path::new("compiler/rustc_span/src/lib.rs"),
                    Path::new("compiler/rustc_span/src/symbol.rs"),
                ],
                "compiler/rustc_span/src/lib.rs",
            ),
            vec![PathBuf::from("compiler/rustc_span/src/lib.rs")]
        );
    }

    #[test]
    fn entry_top_level_filename_modified() {
        assert_eq!(
            modified_paths_matches(
                &[
                    Path::new("Cargo.lock"),
                    Path::new("Cargo.toml"),
                    Path::new(".git/submodules"),
                ],
                "Cargo.lock",
            ),
            vec![PathBuf::from("Cargo.lock")]
        );
    }

    #[test]
    fn entry_modified_glob_either() {
        assert_eq!(
            modified_paths_matches(
                &[
                    Path::new("Cargo.lock"),
                    Path::new("Cargo.toml"),
                    Path::new("library/dec2flt/lib.rs"),
                    Path::new("library/flt2dec/lib.rs"),
                ],
                "library/{dec2flt,flt2dec}",
            ),
            vec![
                PathBuf::from("library/dec2flt/lib.rs"),
                PathBuf::from("library/flt2dec/lib.rs"),
            ]
        );
    }

    #[test]
    fn entry_modified_glob_star() {
        assert_eq!(
            modified_paths_matches(
                &[
                    Path::new("Cargo.lock"),
                    Path::new("Cargo.toml"),
                    Path::new("library/dec2flt/lib.rs"),
                    Path::new("library/flt2dec/lib.rs"),
                ],
                "library/dec2*",
            ),
            vec![PathBuf::from("library/dec2flt/lib.rs")]
        );
    }

    #[test]
    fn entry_modified_glob_star_middle() {
        assert_eq!(
            modified_paths_matches(
                &[
                    Path::new("compiler/x86-64-none_eabi.rs"),
                    Path::new("compiler/armv7-none_eabi-something.rs"),
                    Path::new("compiler/armv7-none_eabi-something.txt"),
                    Path::new("compiler/none_eabi.rs"),
                ],
                "compiler/*none_eabi*.rs",
            ),
            vec![
                PathBuf::from("compiler/x86-64-none_eabi.rs"),
                PathBuf::from("compiler/armv7-none_eabi-something.rs"),
                PathBuf::from("compiler/none_eabi.rs"),
            ]
        );
    }

    #[test]
    fn entry_modified_glob_double_star() {
        assert_eq!(
            modified_paths_matches(
                &[
                    Path::new("Cargo.lock"),
                    Path::new("Cargo.toml"),
                    Path::new("library/.empty"),
                    Path::new("library/dec2flt/lib.rs"),
                    Path::new("library/flt2dec/lib.rs"),
                ],
                "library/**",
            ),
            vec![
                PathBuf::from("library/.empty"),
                PathBuf::from("library/dec2flt/lib.rs"),
                PathBuf::from("library/flt2dec/lib.rs"),
            ]
        );
    }

    #[test]
    fn entry_modified_glob_empty_alternates() {
        assert_eq!(
            modified_paths_matches(
                &[Path::new("result.rs"), Path::new("result.rs.stdout")],
                "result.rs{,.stdout}",
            ),
            vec![
                PathBuf::from("result.rs"),
                PathBuf::from("result.rs.stdout"),
            ]
        );
    }

    #[test]
    fn entry_submodule_modified() {
        assert_eq!(
            modified_paths_matches(
                &[
                    Path::new("src/tools/cargo"),
                    Path::new("src/tools/cargotest")
                ],
                "src/tools/cargo"
            ),
            vec![PathBuf::from("src/tools/cargo")]
        );
    }

    #[test]
    fn entry_submodule_and_normal_dir_modified() {
        assert_eq!(
            modified_paths_matches(
                &[
                    Path::new("src/tools/cargo"),
                    Path::new("src/tools/cargotest")
                ],
                "src/tools/cargo{,test}"
            ),
            vec![
                PathBuf::from("src/tools/cargo"),
                PathBuf::from("src/tools/cargotest")
            ]
        );
        assert_eq!(
            modified_paths_matches(
                &[
                    Path::new("src/tools/cargo"),
                    Path::new("src/tools/cargotest")
                ],
                "src/tools/cargo*"
            ),
            vec![
                PathBuf::from("src/tools/cargo"),
                PathBuf::from("src/tools/cargotest")
            ]
        );
    }

    #[test]
    fn entry_submodule_modified_with_trailing_slash() {
        assert_eq!(
            modified_paths_matches(
                &[
                    Path::new("src/tools/cargo"),
                    Path::new("src/tools/cargotest")
                ],
                "src/tools/cargo/"
            ),
            vec![PathBuf::from("src/tools/cargo")]
        );
        assert_eq!(
            modified_paths_matches(
                &[
                    Path::new("src/tools/cargo"),
                    Path::new("src/tools/cargotest")
                ],
                "src/tools/cargo//"
            ),
            vec![PathBuf::from("src/tools/cargo")]
        );
    }

    #[test]
    fn entry_submodule_modified_glob() {
        assert_eq!(
            modified_paths_matches(
                &[
                    Path::new("src/tools/cargo"),
                    Path::new("src/tools/cargotest")
                ],
                "src/*/cargo"
            ),
            vec![PathBuf::from("src/tools/cargo")]
        );
        assert_eq!(
            modified_paths_matches(
                &[
                    Path::new("src/tools/cargo"),
                    Path::new("src/tools/cargotest")
                ],
                "src/*/cargo/"
            ),
            vec![PathBuf::from("src/tools/cargo")]
        );
        assert_eq!(
            modified_paths_matches(
                &[
                    Path::new("src/tools/cargo"),
                    Path::new("src/tools/cargotest")
                ],
                "src/*/cargo//"
            ),
            vec![PathBuf::from("src/tools/cargo")]
        );
    }
}
