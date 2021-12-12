use crate::{
    config::AutolabelConfig,
    github::{IssuesAction, IssuesEvent, Label},
    handlers::Context,
};
pub(super) struct AutolabelInput {
    labels: Vec<Label>,
}

pub(super) async fn parse_input(
    ctx: &Context,
    event: &IssuesEvent,
    config: Option<&AutolabelConfig>,
) -> Result<Option<AutolabelInput>, String> {
    if let Some(diff) = event
        .diff_between(&ctx.github)
        .await
        .map_err(|e| {
            log::error!("failed to fetch diff: {:?}", e);
        })
        .unwrap_or_default()
    {
        if let Some(config) = config {
            let mut files = Vec::new();
            for line in diff.lines() {
                // mostly copied from highfive
                if line.starts_with("diff --git ") {
                    let parts = line[line.find(" b/").unwrap() + " b/".len()..].split("/");
                    let path = parts.collect::<Vec<_>>().join("/");
                    if !path.is_empty() {
                        files.push(path);
                    }
                }
            }
            let mut autolabels = Vec::new();
            for trigger_file in files {
                if trigger_file.is_empty() {
                    // TODO: when would this be true?
                    continue;
                }
                for (label, cfg) in config.labels.iter() {
                    if cfg
                        .trigger_files
                        .iter()
                        .any(|f| trigger_file.starts_with(f))
                    {
                        autolabels.push(Label {
                            name: label.to_owned(),
                        });
                    }
                }
                if !autolabels.is_empty() {
                    return Ok(Some(AutolabelInput { labels: autolabels }));
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
                return Ok(Some(AutolabelInput { labels: autolabels }));
            }
        }
    }
    if event.action == IssuesAction::Closed {
        let labels = event.issue.labels();
        if let Some(x) = labels.iter().position(|x| x.name == "I-prioritize") {
            let mut labels_excluded = labels.to_vec();
            labels_excluded.remove(x);
            return Ok(Some(AutolabelInput {
                labels: labels_excluded,
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
    let mut labels = event.issue.labels().to_owned();
    for label in input.labels {
        // Don't add the label if it's already there
        if !labels.contains(&label) {
            labels.push(label);
        }
    }
    event.issue.set_labels(&ctx.github, labels).await?;
    Ok(())
}
