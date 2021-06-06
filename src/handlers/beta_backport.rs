use crate::github::{Event, IssuesAction, Label};
use crate::handlers::Context;
use regex::Regex;

lazy_static! {
    // See https://docs.github.com/en/issues/tracking-your-work-with-issues/creating-issues/linking-a-pull-request-to-an-issue
    // Max 19 digits long to prevent u64 overflow
    static ref CLOSES_ISSUE: Regex = Regex::new("(close[sd]|fix(e[sd])?|resolve[sd]) #(\\d{1,19})").unwrap();
}

pub(crate) async fn handle(
    ctx: &Context,
    event: &Event,
) -> anyhow::Result<()> {
    let issue_event = if let Event::Issue(event) = event {
        event
    } else {
        return Ok(());
    };

    if issue_event.action != IssuesAction::Opened {
        return Ok(());
    }

    if issue_event.issue.pull_request.is_none() {
        return Ok(());
    }

    for caps in CLOSES_ISSUE.captures_iter(&issue_event.issue.body) {
        // Should never fail due to the regex
        let issue_id = caps.get(1).unwrap().as_str().parse::<u64>().unwrap();
        let issue = issue_event
            .repository
            .get_issue(&ctx.github, issue_id)
            .await?;
        if issue.labels.contains(&Label {
            name: "regression-from-stable-to-beta".to_string(),
        }) {
            let mut labels = issue_event.issue.labels().to_owned();
            labels.push(Label {
                name: "beta-nominated".to_string(),
            });
            issue_event
                .issue
                .set_labels(&ctx.github, labels)
                .await?;
            break;
        }
    }

    Ok(())
}
