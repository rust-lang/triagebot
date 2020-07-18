use crate::{
    config::AutolabelConfig,
    github::{self, Event, Label},
    handlers::{Context, Handler},
};
use futures::future::{BoxFuture, FutureExt};
pub(super) struct AutolabelInput {
    labels: Vec<Label>
}

pub(super) struct AutolabelHandler;

impl Handler for AutolabelHandler {
    type Input = AutolabelInput;
    type Config = AutolabelConfig;

    fn parse_input(
        &self,
        _ctx: &Context,
        event: &Event,
        config: Option<&Self::Config>,
    ) -> Result<Option<Self::Input>, String> {
        if let Event::Issue(e) = event {
            if e.action == github::IssuesAction::Labeled {
                if let Some(config) = config {
                    let mut autolabels = Vec::new();
                    let applied_label = &e.label.as_ref().expect("label").name;

                    'outer: for (label, config) in config.get_by_trigger(applied_label) {
                        let exclude_patterns: Vec<glob::Pattern> = config
                            .exclude_labels
                            .iter()
                            .filter_map(|label| {
                                match glob::Pattern::new(label) {
                                    Ok(exclude_glob) => {
                                        Some(exclude_glob)
                                    }
                                    Err(error) => {
                                        log::error!("Invalid glob pattern: {}", error);
                                        None
                                    }
                                }
                            })
                            .collect();

                        for label in event.issue().unwrap().labels() {
                            for pat in &exclude_patterns {
                                if pat.matches(&label.name) {
                                    // If we hit an excluded label, ignore this autolabel and check the next
                                    continue 'outer;
                                }
                            }
                        }

                        // If we reach here, no excluded labels were found, so we should apply the autolabel.
                        autolabels.push(Label { name: label.to_owned() });
                    }
                    if !autolabels.is_empty() {
                        return Ok(Some(AutolabelInput { labels: autolabels }));
                    }
                }
            }
            if e.action == github::IssuesAction::Closed {
                let labels = event.issue().unwrap().labels();
                if let Some(x) = labels.iter().position(|x| x.name == "I-prioritize") {
                    let mut labels_excluded = labels.to_vec();
                    labels_excluded.remove(x);
                    return Ok(Some(AutolabelInput { labels: labels_excluded }));
                }
            }
        }
        Ok(None)
    }

    fn handle_input<'a>(
        &self,
        ctx: &'a Context,
        config: &'a Self::Config,
        event: &'a Event,
        input: Self::Input,
    ) -> BoxFuture<'a, anyhow::Result<()>> {
        handle_input(ctx, config, event, input).boxed()
    }
}

async fn handle_input(
    ctx: &Context,
    _config: &AutolabelConfig,
    event: &Event,
    input: AutolabelInput,
) -> anyhow::Result<()> {
    let issue = event.issue().unwrap();
    let mut labels = issue.labels().to_owned();
    for label in input.labels {
        // Don't add the label if it's already there
        if !labels.contains(&label) {
            labels.push(label);
        }
    }
    issue.set_labels(&ctx.github, labels).await?;
    Ok(())
}
