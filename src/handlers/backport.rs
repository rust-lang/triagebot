use std::collections::HashMap;
use std::sync::LazyLock;

use crate::config::BackportConfig;
use crate::github::{IssuesAction, IssuesEvent, Label};
use crate::handlers::Context;
use anyhow::Context as AnyhowContext;
use futures::future::join_all;
use regex::Regex;
use tracing as log;

// See https://docs.github.com/en/issues/tracking-your-work-with-issues/creating-issues/linking-a-pull-request-to-an-issue
// See tests to see what matches
static CLOSES_ISSUE_REGEXP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new("(?i)(?P<action>close[sd]*|fix([e]*[sd]*)?|resolve[sd]*)(?P<spaces>:? +)(?P<org_repo>[a-zA-Z0-9_-]*/[a-zA-Z0-9_-]*)?#(?P<issue_num>[0-9]+)").unwrap()
});

const BACKPORT_LABELS: [&str; 4] = [
    "beta-nominated",
    "beta-accepted",
    "stable-nominated",
    "stable-accepted",
];

const REGRESSION_LABELS: [&str; 3] = [
    "regression-from-stable-to-nightly",
    "regression-from-stable-to-beta",
    "regression-from-stable-to-stable",
];

// auto-nominate for backport only patches fixing high/critical regressions
// For `P-{medium,low}` regressions, let the author decide
const PRIORITY_LABELS: [&str; 2] = ["P-high", "P-critical"];

#[derive(Default)]
pub(crate) struct BackportInput {
    // Issue(s) fixed by this PR
    ids: Vec<u64>,
    // Handler configuration, it's a compound value of (required_issue_label -> add_labels)
    labels: HashMap<String, Vec<String>>,
}

pub(super) async fn parse_input(
    _ctx: &Context,
    event: &IssuesEvent,
    config: Option<&BackportConfig>,
) -> Result<Option<BackportInput>, String> {
    let config = match config {
        Some(config) => config,
        None => return Ok(None),
    };

    // Only handle events when the PR is opened or the first comment is edited
    let should_check = matches!(event.action, IssuesAction::Opened | IssuesAction::Edited);
    if !should_check || !event.issue.is_pr() {
        log::debug!(
            "Skipping backport event because: IssuesAction = {:?} issue.is_pr() {}",
            event.action,
            event.issue.is_pr()
        );
        return Ok(None);
    }
    let pr = &event.issue;

    let pr_labels: Vec<&str> = pr.labels.iter().map(|l| l.name.as_str()).collect();
    if contains_any(&pr_labels, &BACKPORT_LABELS) {
        log::debug!("PR #{} already has a backport label", pr.number);
        return Ok(None);
    }

    // Retrieve backport config for this PR, based on its team label(s)
    // If the PR has no team label matching any [backport.*.required-pr-labels] config, the backport labelling will be skipped
    let mut input = BackportInput::default();
    let valid_configs: Vec<_> = config
        .configs
        .iter()
        .clone()
        .filter(|(_cfg_name, cfg)| {
            let required_pr_labels: Vec<&str> =
                cfg.required_pr_labels.iter().map(|l| l.as_str()).collect();
            if !contains_any(&pr_labels, &required_pr_labels) {
                log::warn!(
                    "Skipping backport nomination: PR is missing one required label: {:?}",
                    pr_labels
                );
                return false;
            }
            input
                .labels
                .insert(cfg.required_issue_label.clone(), cfg.add_labels.clone());
            true
        })
        .collect();
    if valid_configs.is_empty() {
        log::warn!(
            "Skipping backport nomination: could not find a suitable backport config. Please ensure the triagebot.toml has a `[backport.*.required-pr-labels]` section matching the team label(s) for PR #{}.",
            pr.number
        );
        return Ok(None);
    }

    // Check marker text in the opening comment of the PR to retrieve the issue(s) being fixed
    for caps in CLOSES_ISSUE_REGEXP.captures_iter(&event.issue.body) {
        let id = caps
            .name("issue_num")
            .ok_or_else(|| format!("failed to get issue_num from {caps:?}"))?
            .as_str();

        let id = match id.parse::<u64>() {
            Ok(id) => id,
            Err(err) => {
                return Err(format!("Failed to parse issue id `{id}`, error: {err}"));
            }
        };
        if let Some(org_repo) = caps.name("org_repo")
            && org_repo.as_str() != event.repository.full_name
        {
            log::info!(
                "Skipping backport nomination: Ignoring issue#{id} pointing to a different git repository: Expected {0}, found {org_repo:?}",
                event.repository.full_name
            );
            continue;
        }
        input.ids.push(id);
    }

    if input.ids.is_empty() || input.labels.is_empty() {
        return Ok(None);
    }

    log::debug!(
        "Will handle event action {:?} in backport. Regression IDs found {:?}",
        event.action,
        input.ids
    );

    Ok(Some(input))
}

pub(super) async fn handle_input(
    ctx: &Context,
    _config: &BackportConfig,
    event: &IssuesEvent,
    input: BackportInput,
) -> anyhow::Result<()> {
    let pr = &event.issue;

    // Retrieve the issue(s) this pull request closes
    let issues = input
        .ids
        .iter()
        .copied()
        .map(|id| async move { event.repository.get_issue(&ctx.github, id).await });
    let issues = join_all(issues).await;

    // Add backport nomination label to the pull request
    for issue in issues {
        if let Err(ref err) = issue {
            log::warn!("Failed to get issue: {err:?}");
            continue;
        }
        let issue = issue.context("failed to get issue")?;
        let issue_labels: Vec<&str> = issue.labels.iter().map(|l| l.name.as_str()).collect();

        // Check issue for a prerequisite priority label
        // If none, skip this issue
        if !contains_any(&issue_labels, &PRIORITY_LABELS) {
            continue;
        }

        // Get the labels to be added the PR according to the matching (required) regression label
        // that is found in the configuration that this handler has received
        // If no regression label is found, skip this issue
        let add_labels = issue_labels.iter().find_map(|l| input.labels.get(*l));
        if add_labels.is_none() {
            log::warn!(
                "Skipping backport nomination: nothing to do for issue #{}. No config found for regression label ({:?})",
                issue.number,
                REGRESSION_LABELS
            );
            continue;
        }

        // Add backport nomination label(s) to PR
        let mut new_labels = pr.labels().to_owned();
        new_labels.extend(
            add_labels
                .expect("failed to unwrap add_labels")
                .iter()
                .cloned()
                .map(|name| Label { name }),
        );
        log::debug!(
            "PR#{} adding labels for backport {:?}",
            pr.number,
            add_labels
        );
        let _ = pr
            .add_labels(&ctx.github, new_labels)
            .await
            .context("failed to add backport labels to the PR");
    }

    Ok(())
}

fn contains_any(haystack: &[&str], needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

#[cfg(test)]
mod tests {
    use crate::handlers::backport::CLOSES_ISSUE_REGEXP;

    #[tokio::test]
    async fn backport_match_comment() {
        let test_strings = vec![
            ("close #10", vec![10]),
            ("closes #10", vec![10]),
            ("closed #10", vec![10]),
            ("Closes #10", vec![10]),
            ("close  #10", vec![10]),
            ("close rust-lang/rust#10", vec![10]),
            ("cLose: rust-lang/rust#10", vec![10]),
            ("fix #10", vec![10]),
            ("fixes #10", vec![10]),
            ("fixed #10", vec![10]),
            ("resolve #10", vec![10]),
            ("resolves #10", vec![10]),
            ("resolved #10", vec![10]),
            (
                "Fixes #20, Resolves #21, closed #22, LOL #23",
                vec![20, 21, 22],
            ),
            ("Resolved #10", vec![10]),
            ("Fixes #10", vec![10]),
            ("Closes #10", vec![10]),
        ];
        for test_case in test_strings {
            let mut ids: Vec<u64> = vec![];
            let test_str = test_case.0;
            let expected = test_case.1;
            for caps in CLOSES_ISSUE_REGEXP.captures_iter(test_str) {
                // eprintln!("caps {:?}", caps);
                let id = &caps["issue_num"];
                ids.push(id.parse::<u64>().unwrap());
            }
            // eprintln!("ids={:?}", ids);
            assert_eq!(ids, expected);
        }
    }
}
