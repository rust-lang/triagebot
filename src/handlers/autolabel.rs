use crate::{
    config::AutolabelConfig,
    github::{files_changed, IssuesAction, IssuesEvent, Label},
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
    // On opening a new PR or sync'ing the branch, look at the diff and try to
    // add any appropriate labels.
    //
    // FIXME: This will re-apply labels after a push that the user had tried to
    // remove. Not much can be done about that currently; the before/after on
    // synchronize may be straddling a rebase, which will break diff generation.
    if let Some(config) = config {
        if event.action == IssuesAction::Opened || event.action == IssuesAction::Synchronize {
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
                let mut autolabels = Vec::new();
                for (label, cfg) in config.labels.iter() {
                    if cfg
                        .trigger_files
                        .iter()
                        .any(|f| files.iter().any(|diff_file| diff_file.starts_with(f)))
                    {
                        autolabels.push(Label {
                            name: label.to_owned(),
                        });
                    }
                }
                if !autolabels.is_empty() {
                    return Ok(Some(AutolabelInput {
                        add: autolabels,
                        remove: vec![],
                    }));
                }
            }
        }
    }

    if event.action == IssuesAction::Labeled {
        if let Some(config) = config {
            let mut autolabels = Vec::new();
            let applied_label = &event.label.as_ref().expect("label").name;

            'outer: for (label, config) in config.get_by_trigger(applied_label) {
                let exclude_patterns: Vec<glob::Pattern> = config
                    .exclude_labels
                    .iter()
                    .filter_map(|label| match glob::Pattern::new(label) {
                        Ok(exclude_glob) => Some(exclude_glob),
                        Err(error) => {
                            log::error!("Invalid glob pattern: {}", error);
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
    }
    Ok(None)
}

pub(super) async fn handle_input(
    ctx: &Context,
    _config: &AutolabelConfig,
    event: &IssuesEvent,
    input: AutolabelInput,
) -> anyhow::Result<()> {
    event.issue.add_labels(&ctx.github, input.add).await?;
    for label in input.remove {
        event
            .issue
            .remove_label(&ctx.github, &label.name)
            .await
            .with_context(|| {
                format!(
                    "failed to remove {:?} from {:?}",
                    label,
                    event.issue.global_id()
                )
            })?;
    }
    Ok(())
}
