//! Purpose: Allow the use of single words shortcut to do specific actions on GitHub via comments.
//!
//! Parsing is done in the `parser::command::shortcut` module.

use crate::{
    config::ShortcutConfig,
    errors::user_error,
    github::{Event, Label},
    handlers::Context,
};
use anyhow::Context as _;
use parser::command::shortcut::ShortcutCommand;

pub(super) async fn handle_command(
    ctx: &Context,
    config: &ShortcutConfig,
    event: &Event,
    input: ShortcutCommand,
) -> anyhow::Result<()> {
    let issue = event.issue().unwrap();
    // NOTE: if shortcuts available to issues are created, they need to be allowed here
    if !issue.is_pr() {
        return user_error!(format!(
            "The \"{input:?}\" shortcut only works on pull requests."
        ));
    }

    let issue_labels = issue.labels();
    let waiting_on_review = "S-waiting-on-review";
    let waiting_on_author = "S-waiting-on-author";
    let blocked = "S-blocked";
    let status_labels = [waiting_on_review, waiting_on_author, blocked, "S-inactive"];

    let add = match input {
        ShortcutCommand::Ready => waiting_on_review,
        ShortcutCommand::Author => waiting_on_author,
        ShortcutCommand::Blocked => blocked,
    };

    if !issue_labels.iter().any(|l| l.name == add) {
        issue
            .remove_labels(
                &ctx.github,
                status_labels
                    .iter()
                    .filter(|l| **l != add)
                    .map(|l| Label { name: (*l).into() })
                    .collect(),
            )
            .await?;

        issue
            .add_labels(
                &ctx.github,
                vec![Label {
                    name: add.to_owned(),
                }],
            )
            .await?;
    }

    // We add a small reminder for the author to use `@bot ready` when ready
    //
    // Except if the author is a member (or the owner) of the repository, as
    // the author should already know about the `ready` command and already
    // have the required permissions to update the labels manually anyway.
    if matches!(input, ShortcutCommand::Author) {
        super::review_reminder::remind_author_of_bot_ready(ctx, issue, Some(config))
            .await
            .context("failed to send @bot review reminder")?;
    }

    Ok(())
}
