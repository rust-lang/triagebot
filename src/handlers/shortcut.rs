//! Purpose: Allow the use of single words shortcut to do specific actions on GitHub via comments.
//!
//! Parsing is done in the `parser::command::shortcut` module.

use crate::{
    config::ShortcutConfig,
    github::{Event, Label},
    handlers::Context,
    interactions::ErrorComment,
};
use parser::command::shortcut::ShortcutCommand;

pub(super) async fn handle_command(
    ctx: &Context,
    _config: &ShortcutConfig,
    event: &Event,
    input: ShortcutCommand,
) -> anyhow::Result<()> {
    let issue = event.issue().unwrap();
    // NOTE: if shortcuts available to issues are created, they need to be allowed here
    if !issue.is_pr() {
        let msg = format!("The \"{:?}\" shortcut only works on pull requests.", input);
        let cmnt = ErrorComment::new(&issue, msg);
        cmnt.post(&ctx.github).await?;
        return Ok(());
    }

    let issue_labels = issue.labels();
    let waiting_on_review = "S-waiting-on-review";
    let waiting_on_author = "S-waiting-on-author";
    let blocked = "S-blocked";
    let status_labels = [waiting_on_review, waiting_on_author, blocked];

    let add = match input {
        ShortcutCommand::Ready => waiting_on_review,
        ShortcutCommand::Author => waiting_on_author,
        ShortcutCommand::Blocked => blocked,
    };

    if !issue_labels.iter().any(|l| l.name == add) {
        for remove in status_labels {
            if remove != add {
                issue.remove_label(&ctx.github, remove).await?;
            }
        }
        issue
            .add_labels(
                &ctx.github,
                vec![Label {
                    name: add.to_owned(),
                }],
            )
            .await?;
    }

    // We add a small reminder for the author to the `@bot ready` when ready
    if matches!(input, ShortcutCommand::Author) {
        if let Event::IssueComment(issue_comment) = event {
            let new_comment_body = format!(
                "{}\n\n*Use `@{bot} ready` when the PR is ready for another round of review.*",
                issue_comment.comment.body,
                bot = &ctx.username
            );
            issue
                .edit_comment(
                    &ctx.github,
                    issue_comment.comment.id,
                    new_comment_body.as_str(),
                )
                .await?;
        }
    }

    Ok(())
}
