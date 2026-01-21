//! For pull requests that have changed the triagebot.toml, validate that the
//! changes are a valid configuration file.

use crate::{
    config::{CONFIG_FILE_NAME, MentionsEntryConfig, MentionsEntryType},
    github::FileDiff,
    handlers::{Context, IssuesEvent},
};
use anyhow::{Context as _, bail};

pub(super) async fn validate_config(
    ctx: &Context,
    event: &IssuesEvent,
    diff: &[FileDiff],
) -> anyhow::Result<Option<String>> {
    if !diff.iter().any(|diff| diff.filename == CONFIG_FILE_NAME) {
        return Ok(None);
    }

    let Some(pr_source) = &event.issue.head else {
        bail!("expected head commit");
    };
    let Some(repo) = &pr_source.repo else {
        bail!("repo is not available");
    };

    let triagebot_content = ctx
        .github
        .raw_file(&repo.full_name, &pr_source.sha, CONFIG_FILE_NAME)
        .await
        .context("{CONFIG_FILE_NAME} modified, but failed to get content")?;

    let triagebot_content = triagebot_content.unwrap_or_default();
    let triagebot_content = String::from_utf8_lossy(&triagebot_content);

    match toml::from_str::<crate::handlers::Config>(&triagebot_content) {
        Err(e) => {
            let position = match e.span() {
                // toml sometimes gives bad spans, see https://github.com/toml-rs/toml/issues/589
                Some(span) if span != (0..0) => {
                    let (line, col) = translate_position(&triagebot_content, span.start);
                    let url = format!(
                        "https://github.com/{}/blob/{}/{CONFIG_FILE_NAME}#L{line}",
                        repo.full_name, pr_source.sha
                    );
                    format!(" at position [{line}:{col}]({url})",)
                }
                Some(_) | None => String::new(),
            };

            Ok(Some(format!(
                "Invalid `triagebot.toml`{position}:\n\
                `````\n\
                {e}\n\
                `````",
            )))
        }
        Ok(config) => {
            // Error if `[assign.owners]` is not empty (ie auto-assign) and the custom welcome message for assignee isn't set.
            if let Some(assign) = config.assign
                && !assign.owners.is_empty()
                && let Some(custom_messages) = &assign.custom_messages
                && custom_messages.auto_assign_someone.is_none()
            {
                return Ok(Some(
                    "Invalid `triagebot.toml`:\n\
                    `[assign.owners]` is populated but `[assign.custom_messages.auto-assign-someone]` is not set!".to_string()
                ));
            }

            // Error if one the mentions entry is not a valid glob.
            if let Some(mentions) = config.mentions {
                for (entry, MentionsEntryConfig { type_, .. }) in mentions.entries {
                    if type_ == MentionsEntryType::Filename {
                        if let Err(err) = globset::Glob::new(&entry) {
                            return Ok(Some(format!(
                                "Invalid `triagebot.toml`:\n\
                                `[mentions.\"{entry}\"]` has an invalid glob syntax: {err}"
                            )));
                        }

                        if entry.starts_with('/') {
                            return Ok(Some(format!(
                                "Invalid `triagebot.toml`:\n\
                                `[mentions.\"{entry}\"]` has an invalid pattern: path must be relative (remove the `/` at the start)"
                            )));
                        }
                    }
                }
            }

            Ok(None)
        }
    }
}

/// Helper to translate a toml span to a `(line_no, col_no)` (1-based).
#[expect(
    clippy::sliced_string_as_bytes,
    reason = "don't know if the suggestion applies here, because of the char boundaries thing"
)]
fn translate_position(input: &str, index: usize) -> (usize, usize) {
    if input.is_empty() {
        return (0, index);
    }

    let safe_index = index.min(input.len() - 1);
    let column_offset = index - safe_index;

    let nl = input[0..safe_index]
        .as_bytes()
        .iter()
        .rev()
        .enumerate()
        .find(|(_, b)| **b == b'\n')
        .map(|(nl, _)| safe_index - nl - 1);
    let line_start = match nl {
        Some(nl) => nl + 1,
        None => 0,
    };
    let line = input[0..line_start]
        .as_bytes()
        .iter()
        .filter(|c| **c == b'\n')
        .count();
    let column = input[line_start..=safe_index].chars().count() - 1;
    let column = column + column_offset;

    (line + 1, column + 1)
}
