//! Triage command
//!
//! Removes I-nominated tag and adds priority label.

use crate::{
    config::TriageConfig,
    github::{self, Event},
    handlers::Context,
};
use parser::command::triage::{Priority, TriageCommand};

pub(super) async fn handle_command(
    ctx: &Context,
    config: &TriageConfig,
    event: &Event,
    cmd: TriageCommand,
) -> anyhow::Result<()> {
    let event = if let Event::IssueComment(e) = event {
        e
    } else {
        // not interested in other events
        return Ok(());
    };

    let is_team_member =
        if let Err(_) | Ok(false) = event.comment.user.is_team_member(&ctx.github).await {
            false
        } else {
            true
        };
    if !is_team_member {
        anyhow::bail!("Cannot use triage command as non-team-member");
    }

    let mut labels = event.issue.labels().to_owned();
    let add = match cmd.priority {
        Priority::High => &config.high,
        Priority::Medium => &config.medium,
        Priority::Low => &config.low,
    };
    labels.push(github::Label { name: add.clone() });
    labels.retain(|label| !config.remove.contains(&label.name));
    if &labels[..] != event.issue.labels() {
        event.issue.set_labels(&ctx.github, labels).await?;
    }

    Ok(())
}
