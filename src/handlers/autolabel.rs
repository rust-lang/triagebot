use crate::{
    config::AutolabelConfig,
    github::{IssuesAction, IssuesEvent, Label},
    handlers::Context,
};
use anyhow::Context as _;
use tracing as log;

pub(super) struct AutolabelInput {
    add: Vec<Label>,
    remove: Vec<Label>,
}

pub(super) async fn parse_input(
    ctx: &Context,
    event: &IssuesEvent,
    config: Option<&AutolabelConfig>,
) -> Result<Option<AutolabelInput>, String> {
    let Some(config) = config else {
        return Ok(None);
    };

    // On opening a new PR or sync'ing the branch, look at the diff and try to
    // add any appropriate labels.
    //
    // FIXME: This will re-apply labels after a push that the user had tried to
    // remove. Not much can be done about that currently; the before/after on
    // synchronize may be straddling a rebase, which will break diff generation.
    let can_trigger_files = matches!(
        event.action,
        IssuesAction::Opened | IssuesAction::Synchronize
    );

    if can_trigger_files
        || matches!(
            event.action,
            IssuesAction::Closed
                | IssuesAction::Reopened
                | IssuesAction::ReadyForReview
                | IssuesAction::ConvertedToDraft
        )
        || event.has_base_changed()
    {
        let files = if can_trigger_files {
            event
                .issue
                .diff(&ctx.github)
                .await
                .map_err(|e| log::error!("failed to fetch diff: {e:?}"))
                .unwrap_or_default()
        } else {
            Default::default()
        };

        let mut autolabels = Vec::new();
        let mut to_remove = Vec::new();

        'outer: for (label, cfg) in &config.labels {
            let exclude_patterns: Vec<glob::Pattern> = cfg
                .exclude_labels
                .iter()
                .filter_map(|label| match glob::Pattern::new(label) {
                    Ok(exclude_glob) => Some(exclude_glob),
                    Err(error) => {
                        log::error!("Invalid glob pattern: {error}");
                        None
                    }
                })
                .collect();

            for label in event.issue.labels() {
                for pat in &exclude_patterns {
                    if pat.matches(&label.name) {
                        // If we hit an excluded label, ignore this autolabel and check the next
                        continue 'outer;
                    }
                }
            }

            if event.issue.is_pr() {
                if let Some(files) = &files {
                    // This a PR with modified files.

                    // Add the matching labels for the modified files paths
                    if cfg.trigger_files.iter().any(|f| {
                        files
                            .iter()
                            .any(|file_diff| file_diff.filename.starts_with(f))
                    }) {
                        autolabels.push(Label {
                            name: label.to_owned(),
                        });
                    }
                }

                let is_opened =
                    matches!(event.action, IssuesAction::Opened | IssuesAction::Reopened);

                // Treat the following situations as a "new PR":
                // 1) PRs that were (re)opened and are not draft
                // 2) PRs that have been converted from a draft to being "ready for review"
                let is_opened_non_draft = is_opened && !event.issue.draft;
                let is_ready_for_review = event.action == IssuesAction::ReadyForReview;

                // Treat the following situations as a "new draft":
                // 1) PRs that were (re)opened and are draft
                // 2) PRs that have been converted to a draft
                let is_opened_as_draft = is_opened && event.issue.draft;
                let is_converted_to_draft = event.action == IssuesAction::ConvertedToDraft;

                #[expect(clippy::if_same_then_else, reason = "suggested code looks ugly")]
                if cfg.new_pr && (is_opened_non_draft || is_ready_for_review) {
                    autolabels.push(Label {
                        name: label.to_owned(),
                    });
                } else if cfg.new_draft && (is_opened_as_draft || is_converted_to_draft) {
                    autolabels.push(Label {
                        name: label.to_owned(),
                    });
                }

                // If a PR is converted to draft or closed, remove all the "new PR" labels.
                // Same for "new draft" labels when the PR is ready for review or closed.
                #[expect(clippy::if_same_then_else, reason = "suggested code looks ugly")]
                if cfg.new_pr
                    && matches!(
                        event.action,
                        IssuesAction::ConvertedToDraft | IssuesAction::Closed
                    )
                {
                    to_remove.push(Label {
                        name: label.to_owned(),
                    });
                } else if cfg.new_draft
                    && matches!(
                        event.action,
                        IssuesAction::ReadyForReview | IssuesAction::Closed
                    )
                {
                    to_remove.push(Label {
                        name: label.to_owned(),
                    });
                }
            } else if cfg.new_issue && event.action == IssuesAction::Opened {
                autolabels.push(Label {
                    name: label.to_owned(),
                });
            }
        }

        if !autolabels.is_empty() || !to_remove.is_empty() {
            return Ok(Some(AutolabelInput {
                add: autolabels,
                remove: to_remove,
            }));
        }
    }

    if let IssuesAction::Labeled { label } = &event.action {
        let mut autolabels = Vec::new();
        let applied_label = &label.name;

        'outer: for (label, config) in config.get_by_trigger(applied_label) {
            let exclude_patterns: Vec<glob::Pattern> = config
                .exclude_labels
                .iter()
                .filter_map(|label| match glob::Pattern::new(label) {
                    Ok(exclude_glob) => Some(exclude_glob),
                    Err(error) => {
                        log::error!("Invalid glob pattern: {error}");
                        None
                    }
                })
                .collect();

            for label in event.issue.labels() {
                for pat in &exclude_patterns {
                    if pat.matches(&label.name) {
                        // If we hit an excluded label, ignore this autolabel and check the next
                        continue 'outer;
                    }
                }
            }

            // If we reach here, no excluded labels were found, so we should apply the autolabel.
            autolabels.push(Label {
                name: label.to_owned(),
            });
        }
        if !autolabels.is_empty() {
            return Ok(Some(AutolabelInput {
                add: autolabels,
                remove: vec![],
            }));
        }
    }
    Ok(None)
}

pub(super) async fn handle_input(
    ctx: &Context,
    _config: &AutolabelConfig,
    event: &IssuesEvent,
    input: AutolabelInput,
) -> anyhow::Result<()> {
    event
        .issue
        .add_labels(&ctx.github, input.add)
        .await
        .context("failed to add the labels to the issue")?;

    event
        .issue
        .remove_labels(&ctx.github, input.remove)
        .await
        .context("failed to remove labels from the issue")?;

    Ok(())
}
