//! Purpose: Allow the use of single words shortcut to do specific actions on GitHub via comments.
//!
//! Parsing is done in the `parser::command::shortcut` module.

use crate::{
    config::ShortcutConfig,
    github::{Event, Label},
    handlers::Context,
    interactions::{ErrorComment, PingComment},
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

    let mut issue_labels = issue.labels().to_owned();
    let waiting_on_review = "S-waiting-on-review";
    let waiting_on_author = "S-waiting-on-author";

    match input {
        ShortcutCommand::Ready => {
            if assign_and_remove_label(&mut issue_labels, waiting_on_review, waiting_on_author)
                .is_some()
            {
                return Ok(());
            }
            issue.set_labels(&ctx.github, issue_labels).await?;

            let to_ping: Vec<_> = issue
                .assignees
                .iter()
                .map(|user| user.login.as_str())
                .collect();
            let cmnt = PingComment::new(&issue, &to_ping);
            cmnt.post(&ctx.github).await?;
        }
        ShortcutCommand::Author => {
            if assign_and_remove_label(&mut issue_labels, waiting_on_author, waiting_on_review)
                .is_some()
            {
                return Ok(());
            }
            issue.set_labels(&ctx.github, issue_labels).await?;

            let to_ping = vec![issue.user.login.as_str()];
            let cmnt = PingComment::new(&issue, &to_ping);
            cmnt.post(&ctx.github).await?;
        }
    }

    Ok(())
}

fn assign_and_remove_label(
    issue_labels: &mut Vec<Label>,
    assign: &str,
    remove: &str,
) -> Option<()> {
    if issue_labels.iter().any(|label| label.name == assign) {
        return Some(());
    }

    if let Some(index) = issue_labels.iter().position(|label| label.name == remove) {
        issue_labels.swap_remove(index);
    }

    issue_labels.push(Label {
        name: assign.into(),
    });

    None
}

#[cfg(test)]
mod tests {

    use super::{assign_and_remove_label, Label};
    fn create_labels(names: Vec<&str>) -> Vec<Label> {
        names
            .into_iter()
            .map(|name| Label { name: name.into() })
            .collect()
    }

    #[test]
    fn test_adds_without_labels() {
        let expected = create_labels(vec!["assign"]);
        let mut labels = vec![];
        assert!(assign_and_remove_label(&mut labels, "assign", "remove").is_none());
        assert_eq!(labels, expected);
    }

    #[test]
    fn test_do_nothing_with_label_already_set() {
        let expected = create_labels(vec!["assign"]);
        let mut labels = create_labels(vec!["assign"]);
        assert!(assign_and_remove_label(&mut labels, "assign", "remove").is_some());
        assert_eq!(labels, expected);
    }

    #[test]
    fn test_other_labels_untouched() {
        let expected = create_labels(vec!["bug", "documentation", "assign"]);
        let mut labels = create_labels(vec!["bug", "documentation"]);
        assert!(assign_and_remove_label(&mut labels, "assign", "remove").is_none());
        assert_eq!(labels, expected);
    }

    #[test]
    fn test_correctly_remove_label() {
        let expected = create_labels(vec!["bug", "documentation", "assign"]);
        let mut labels = create_labels(vec!["bug", "documentation", "remove"]);
        assert!(assign_and_remove_label(&mut labels, "assign", "remove").is_none());
        assert_eq!(labels, expected);
    }
}
