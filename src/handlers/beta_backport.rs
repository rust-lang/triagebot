use crate::config::BetaBackportConfig;
use crate::github::{IssuesAction, IssuesEvent, Label};
use crate::handlers::Context;
use regex::Regex;

lazy_static! {
    // See https://docs.github.com/en/issues/tracking-your-work-with-issues/creating-issues/linking-a-pull-request-to-an-issue
    static ref CLOSES_ISSUE: Regex = Regex::new("(close[sd]|fix(e[sd])?|resolve[sd]) #(\\d+)").unwrap();
}

pub(crate) struct BetaBackportInput {
    ids: Vec<u64>,
}

pub(crate) fn parse_input(
    _ctx: &Context,
    event: &IssuesEvent,
    config: Option<&BetaBackportConfig>,
) -> Result<Option<BetaBackportInput>, String> {
    if config.is_none() {
        return Ok(None);
    }

    if event.action != IssuesAction::Opened {
        return Ok(None);
    }

    if event.issue.pull_request.is_none() {
        return Ok(None);
    }

    let mut ids = vec![];
    for caps in CLOSES_ISSUE.captures_iter(&event.issue.body) {
        let id = caps.get(1).unwrap().as_str();
        let id = match id.parse::<u64>() {
            Ok(id) => id,
            Err(err) => {
                return Err(format!("Failed to parse issue id `{}`, error: {}", id, err));
            }
        };
        ids.push(id);
    }

    return Ok(Some(BetaBackportInput { ids }));
}

pub(super) async fn handle_input(
    ctx: &Context,
    config: &BetaBackportConfig,
    event: &IssuesEvent,
    input: BetaBackportInput,
) -> anyhow::Result<()> {
    let mut issues = input
        .ids
        .iter()
        .copied()
        .map(|id| async move { event.repository.get_issue(&ctx.github, id).await });

    let trigger_labels: Vec<_> = config
        .trigger_labels
        .iter()
        .cloned()
        .map(|name| Label { name })
        .collect();
    while let Some(issue) = issues.next() {
        let issue = issue.await.unwrap();
        if issue
            .labels
            .iter()
            .any(|issue_label| trigger_labels.contains(issue_label))
        {
            let mut new_labels = event.issue.labels().to_owned();
            new_labels.extend(
                config
                    .labels_to_add
                    .iter()
                    .cloned()
                    .map(|name| Label { name }),
            );
            return event.issue.set_labels(&ctx.github, new_labels).await;
        }
    }

    Ok(())
}
