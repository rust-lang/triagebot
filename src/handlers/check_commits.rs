use std::collections::HashSet;

use anyhow::Context as _;
use anyhow::bail;
use itertools::Itertools;

use super::Context;
use crate::interactions::ErrorComment;
use crate::{
    config::Config,
    db::issue_data::IssueData,
    github::{Event, IssuesAction, IssuesEvent, Label, ReportedContentClassifiers},
};

#[cfg(test)]
use crate::github::GithubCommit;

mod behind_upstream;
mod force_push_range_diff;
mod issue_links;
mod modified_submodule;
mod no_mentions;
mod no_merges;
mod non_default_branch;
mod validate_config;

/// Key for the state in the database
const CHECK_COMMITS_KEY: &str = "check-commits-warnings";

/// State stored in the database
#[derive(Debug, Default, serde::Deserialize, serde::Serialize, Clone, PartialEq)]
struct CheckCommitsState {
    /// List of the last errors (comment body, comment node-id).
    #[serde(default)]
    last_errors: Vec<(String, String)>,
    /// List of the last warnings in the most recent comment.
    last_warnings: Vec<String>,
    /// ID of the most recent warning comment.
    last_warned_comment: Option<String>,
    /// List of the last labels added.
    last_labels: Vec<String>,
}

fn should_handle_event(event: &IssuesEvent) -> bool {
    // Reject non-PR
    if !event.issue.is_pr() {
        return false;
    }

    // Reject rollups and draft pr
    if event.issue.title.starts_with("Rollup of") || event.issue.draft {
        return false;
    }

    // Check opened, reopened, synchronized and ready-for-review PRs
    if matches!(
        event.action,
        IssuesAction::Opened
            | IssuesAction::Reopened
            | IssuesAction::Synchronize
            | IssuesAction::ReadyForReview
    ) {
        return true;
    }

    // Also check PRs that changed their base branch (master -> beta)
    if event.has_base_changed() {
        return true;
    }

    false
}

pub(super) async fn handle(
    ctx: &Context,
    host: &str,
    event: &Event,
    config: &Config,
) -> anyhow::Result<()> {
    let Event::Issue(event) = event else {
        return Ok(());
    };

    if !should_handle_event(event) {
        return Ok(());
    }

    let Some(compare) = event.issue.compare(&ctx.github).await? else {
        bail!(
            "expected issue {} to be a PR, but the compare could not be determined",
            event.issue.number
        )
    };
    let commits = event.issue.commits(&ctx.github).await?;
    let diff = &compare.files;

    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut labels = Vec::new();

    // Compute the warnings
    if let Some(assign_config) = &config.assign {
        // For legacy reasons the non-default-branch and modifies-submodule warnings
        // are behind the `[assign]` config.

        if let Some(exceptions) = assign_config
            .warn_non_default_branch
            .enabled_and_exceptions()
        {
            warnings.extend(non_default_branch::non_default_branch(exceptions, event));
        }
        warnings.extend(modified_submodule::modifies_submodule(diff));
    }

    if let Some(no_mentions) = &config.no_mentions {
        warnings.extend(no_mentions::mentions_in_commits(
            &event.issue.title,
            no_mentions,
            &commits,
        ));
    }

    if let Some(issue_links) = &config.issue_links {
        warnings.extend(issue_links::issue_links_in_commits(issue_links, &commits));
    }

    if let Some(no_merges) = &config.no_merges {
        if let Some(warn) =
            no_merges::merges_in_commits(&event.issue.title, &event.repository, no_merges, &commits)
        {
            warnings.push(warn.0);
            labels.extend(warn.1);
        }
    }

    // Check if PR is behind upstream branch by a significant number of days
    if let Some(behind_upstream) = &config.behind_upstream {
        let age_threshold = behind_upstream
            .days_threshold
            .unwrap_or(behind_upstream::DEFAULT_DAYS_THRESHOLD);

        if let Some(warning) = behind_upstream::behind_upstream(age_threshold, event, compare).await
        {
            warnings.push(warning);
        }
    }

    // Check if this is a force-push with rebase and if it is emit comment
    // with link to our range-diff viewer.
    if let Some(range_diff) = &config.range_diff {
        force_push_range_diff::handle_event(ctx, host, range_diff, event, compare).await?;
    }

    // Check if the `triagebot.toml` config is valid
    errors.extend(
        validate_config::validate_config(ctx, event, diff)
            .await
            .context("validating the the triagebot config")?,
    );

    handle_new_state(ctx, event, errors, warnings, labels).await
}

// Add, hide or hide&add a comment with the warnings.
async fn handle_new_state(
    ctx: &Context,
    event: &IssuesEvent,
    errors: Vec<String>,
    warnings: Vec<String>,
    labels: Vec<String>,
) -> anyhow::Result<()> {
    // Get the state of the warnings for this PR in the database.
    let mut db = ctx.db.get().await;
    let mut state: IssueData<'_, CheckCommitsState> =
        IssueData::load(&mut db, &event.issue, CHECK_COMMITS_KEY).await?;

    // Handles the errors, post the new ones, hide resolved ones and don't touch the one still active
    if !state.data.last_errors.is_empty() || !errors.is_empty() {
        let (errors_to_remove, errors_to_add) =
            calculate_error_changes(&state.data.last_errors, &errors);

        for error_to_remove in errors_to_remove {
            event
                .issue
                .hide_comment(
                    &ctx.github,
                    &error_to_remove.1,
                    ReportedContentClassifiers::Resolved,
                )
                .await?;
            state.data.last_errors.retain(|e| e != &error_to_remove);
        }

        for error_to_add in errors_to_add {
            let error_comment = ErrorComment::new(&event.issue, &error_to_add);
            let comment = error_comment.post(&ctx.github).await?;
            state.data.last_errors.push((error_to_add, comment.node_id));
        }
    }

    // We only post a new comment when we haven't posted one with the same warnings before.
    if !warnings.is_empty() && state.data.last_warnings != warnings {
        // New set of warnings, let's post them.

        // Hide a previous warnings comment if there was one before printing the new ones.
        if let Some(last_warned_comment_id) = state.data.last_warned_comment {
            event
                .issue
                .hide_comment(
                    &ctx.github,
                    &last_warned_comment_id,
                    ReportedContentClassifiers::Resolved,
                )
                .await?;
        }

        let warning = warning_from_warnings(&warnings);
        let comment = event.issue.post_comment(&ctx.github, &warning).await?;

        state.data.last_warnings = warnings;
        state.data.last_warned_comment = Some(comment.node_id);
    } else if warnings.is_empty() {
        // No warnings to be shown, let's resolve a previous warnings comment, if there was one.
        if let Some(last_warned_comment_id) = state.data.last_warned_comment {
            event
                .issue
                .hide_comment(
                    &ctx.github,
                    &last_warned_comment_id,
                    ReportedContentClassifiers::Resolved,
                )
                .await?;

            state.data.last_warnings = Vec::new();
            state.data.last_warned_comment = None;
        }
    }

    // Handle the labels, add the new ones, remove the one no longer required, or don't do anything
    if !state.data.last_labels.is_empty() || !labels.is_empty() {
        let (labels_to_remove, labels_to_add) =
            calculate_label_changes(&state.data.last_labels, &labels);

        // Remove the labels no longer required
        if !labels_to_remove.is_empty() {
            event
                .issue
                .remove_labels(
                    &ctx.github,
                    labels_to_remove
                        .into_iter()
                        .map(|name| Label { name })
                        .collect(),
                )
                .await
                .context("failed to remove a label in check_commits")?;
        }

        // Add the labels that are now required
        if !labels_to_add.is_empty() {
            event
                .issue
                .add_labels(
                    &ctx.github,
                    labels_to_add
                        .into_iter()
                        .map(|name| Label { name })
                        .collect(),
                )
                .await
                .context("failed to add labels in check_commits")?;
        }

        state.data.last_labels = labels;
    }

    // Save new state in the database
    state.save().await?;

    Ok(())
}

// Format the warnings for user consumption on Github
fn warning_from_warnings(warnings: &[String]) -> String {
    let warnings = warnings
        .iter()
        .map(|warning| warning.trim().replace('\n', "\n    "))
        .format_with("\n", |warning, f| f(&format_args!("* {warning}")));
    format!(":warning: **Warning** :warning:\n\n{warnings}")
}

// Calculate the label changes
fn calculate_label_changes(
    previous: &Vec<String>,
    current: &Vec<String>,
) -> (Vec<String>, Vec<String>) {
    let previous_set: HashSet<String> = previous.iter().cloned().collect();
    let current_set: HashSet<String> = current.iter().cloned().collect();

    let removals = previous_set.difference(&current_set).cloned().collect();
    let additions = current_set.difference(&previous_set).cloned().collect();

    (removals, additions)
}

// Calculate the error changes
fn calculate_error_changes(
    previous: &Vec<(String, String)>,
    current: &Vec<String>,
) -> (Vec<(String, String)>, Vec<String>) {
    let previous_set: HashSet<(String, String)> = previous.iter().cloned().collect();
    let current_set: HashSet<String> = current.iter().cloned().collect();

    let removals = previous_set
        .iter()
        .filter(|(e, _)| !current_set.contains(e))
        .cloned()
        .collect();
    let additions = current_set
        .iter()
        .filter(|e| !previous_set.iter().any(|(e2, _)| e == &e2))
        .cloned()
        .collect();

    (removals, additions)
}

#[cfg(test)]
fn dummy_commit_from_body(sha: &str, body: &str) -> GithubCommit {
    use chrono::{DateTime, FixedOffset};

    GithubCommit {
        sha: sha.to_string(),
        commit: crate::github::GithubCommitCommitField {
            author: crate::github::GitUser {
                date: DateTime::<FixedOffset>::MIN_UTC.into(),
            },
            message: body.to_string(),
            tree: crate::github::GitCommitTree {
                sha: "60ff73dfdd81aa1e6737eb3dacdfd4a141f6e14d".to_string(),
            },
        },
        parents: vec![],
        html_url: "".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[rustfmt::skip]
    fn test_warning_from_warnings() {
        assert_eq!(
            warning_from_warnings(
                &[
r#"This line should NOT be intend with 4 spaces,
but this line should!"#.to_string()
                ]
            ),
r#":warning: **Warning** :warning:

* This line should NOT be intend with 4 spaces,
    but this line should!"#
        );

        assert_eq!(
            warning_from_warnings(&[
r#"This is warning 1.

Look at this list:
 - 12
  - 13"#.to_string(),
r#"This is warning 2.
 - 123456789
"#.to_string()
            ]),
r#":warning: **Warning** :warning:

* This is warning 1.
    
    Look at this list:
     - 12
      - 13
* This is warning 2.
     - 123456789"#
        );
    }

    fn make_opened_pr_event() -> IssuesEvent {
        IssuesEvent {
            action: IssuesAction::Opened,
            issue: crate::github::Issue {
                number: 123,
                body: "My PR body".to_string(),
                created_at: Default::default(),
                updated_at: Default::default(),
                merge_commit_sha: Default::default(),
                title: "Some title".to_string(),
                html_url: Default::default(),
                user: crate::github::User {
                    login: "user".to_string(),
                    id: 654123,
                },
                labels: Default::default(),
                assignees: Default::default(),
                pull_request: Some(Default::default()),
                merged: false,
                draft: false,
                comments: Default::default(),
                comments_url: Default::default(),
                repository: Default::default(),
                base: Some(crate::github::CommitBase {
                    sha: "fake-sha".to_string(),
                    git_ref: "master".to_string(),
                    repo: None,
                }),
                head: Some(crate::github::CommitBase {
                    sha: "fake-sha".to_string(),
                    git_ref: "master".to_string(),
                    repo: None,
                }),
                state: crate::github::IssueState::Open,
                milestone: None,
                mergeable: None,
                author_association: octocrab::models::AuthorAssociation::Contributor,
            },
            changes: None,
            before: None,
            after: None,
            repository: crate::github::Repository {
                full_name: "rust-lang/rust".to_string(),
                default_branch: "master".to_string(),
                fork: false,
                parent: None,
            },
            sender: crate::github::User {
                login: "rustbot".to_string(),
                id: 987654,
            },
        }
    }

    #[test]
    fn test_pr_closed() {
        let mut event = make_opened_pr_event();
        event.action = IssuesAction::Closed;
        assert!(!should_handle_event(&event));
    }

    #[test]
    fn test_pr_opened() {
        let event = make_opened_pr_event();
        assert!(should_handle_event(&event));
    }

    #[test]
    fn test_not_pr() {
        let mut event = make_opened_pr_event();
        event.issue.pull_request = None;
        assert!(!should_handle_event(&event));
    }

    #[test]
    fn test_pr_rollup() {
        let mut event = make_opened_pr_event();
        event.issue.title = "Rollup of 6 pull requests".to_string();
        assert!(!should_handle_event(&event));
    }

    #[test]
    fn test_pr_draft() {
        let mut event = make_opened_pr_event();
        event.issue.draft = true;
        assert!(!should_handle_event(&event));
    }

    #[test]
    fn test_pr_ready() {
        let mut event = make_opened_pr_event();
        event.action = IssuesAction::ReadyForReview;
        assert!(should_handle_event(&event));
    }

    #[test]
    fn test_pr_reopened() {
        let mut event = make_opened_pr_event();
        event.action = IssuesAction::Reopened;
        assert!(should_handle_event(&event));
    }

    #[test]
    fn test_pr_synchronized() {
        let mut event = make_opened_pr_event();
        event.action = IssuesAction::Synchronize;
        assert!(should_handle_event(&event));
    }

    #[test]
    fn test_pr_edited_title() {
        let mut event = make_opened_pr_event();
        event.action = IssuesAction::Edited;
        event.changes = Some(crate::github::Changes {
            base: None,
            body: None,
            title: Some(crate::github::ChangeInner {
                from: "Previous title".to_string(),
            }),
        });
        assert!(!should_handle_event(&event));
    }

    #[test]
    fn test_pr_edited_base() {
        let mut event = make_opened_pr_event();
        event.action = IssuesAction::Edited;
        event.changes = Some(crate::github::Changes {
            title: None,
            body: None,
            base: Some(crate::github::BaseChange {
                r#ref: crate::github::ChangeInner {
                    from: "master".to_string(),
                },
                sha: crate::github::ChangeInner {
                    from: "fake-sha".to_string(),
                },
            }),
        });
        assert!(should_handle_event(&event));
    }
}
