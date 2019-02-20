//! Purpose: Allow any user to modify issue labels on GitHub via comments.
//!
//! Labels are checked against the labels in the project; the bot does not support creating new
//! labels.
//!
//! Parsing is done in the `parser::command::label` module.
//!
//! If the command was successful, there will be no feedback beyond the label change to reduce
//! notification noise.

use crate::{
    github::{self, GithubClient},
    registry::{Event, Handler},
};
use failure::Error;
use parser::command::label::{LabelCommand, LabelDelta};
use parser::command::{Command, Input};

pub struct LabelHandler {
    pub client: GithubClient,
}

impl Handler for LabelHandler {
    fn handle_event(&self, event: &Event) -> Result<(), Error> {
        #[allow(irrefutable_let_patterns)]
        let event = if let Event::IssueComment(e) = event {
            e
        } else {
            // not interested in other events
            return Ok(());
        };

        let mut issue_labels = event.issue.labels().to_owned();

        let mut input = Input::new(&event.comment.body, self.client.username());
        let deltas = match input.parse_command() {
            Command::Label(Ok(LabelCommand(deltas))) => deltas,
            Command::Label(Err(_)) => {
                // XXX: inform user of error
                return Ok(());
            }
            _ => return Ok(()),
        };

        let mut changed = false;
        for delta in &deltas {
            match delta {
                LabelDelta::Add(label) => {
                    if !issue_labels.iter().any(|l| l.name == label.as_str()) {
                        changed = true;
                        issue_labels.push(github::Label {
                            name: label.to_string(),
                        });
                    }
                }
                LabelDelta::Remove(label) => {
                    if let Some(pos) = issue_labels.iter().position(|l| l.name == label.as_str()) {
                        changed = true;
                        issue_labels.remove(pos);
                    }
                }
            }
        }

        if changed {
            event.issue.set_labels(&self.client, issue_labels)?;
        }

        Ok(())
    }
}
