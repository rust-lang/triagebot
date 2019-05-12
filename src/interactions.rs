use crate::github::{GithubClient, Issue};
use failure::Error;
use std::fmt::Write;

pub struct ErrorComment<'a> {
    issue: &'a Issue,
    message: String,
}

impl<'a> ErrorComment<'a> {
    pub fn new<T>(issue: &'a Issue, message: T) -> ErrorComment<'a>
    where
        T: Into<String>,
    {
        ErrorComment {
            issue,
            message: message.into(),
        }
    }

    pub fn post(&self, client: &GithubClient) -> Result<(), Error> {
        let mut body = String::new();
        writeln!(body, "**Error**: {}", self.message)?;
        writeln!(body)?;
        writeln!(
            body,
            "Please let **`@rust-lang/release`** know if you're having trouble with this bot."
        )?;
        self.issue.post_comment(client, &body)
    }
}

pub struct EditIssueBody<'a> {
    issue: &'a Issue,
    id: &'static str,
    text: String,
}

static START_BOT: &str = "<!-- TRIAGEBOT_START -->\n\n----\n";
static END_BOT: &str = "<!-- TRIAGEBOT_END -->";

impl<'a> EditIssueBody<'a> {
    pub fn new(issue: &'a Issue, id: &'static str, text: String) -> EditIssueBody<'a> {
        EditIssueBody { issue, id, text }
    }

    pub fn apply(&self, client: &GithubClient) -> Result<(), Error> {
        let mut current_body = self.issue.body.clone();
        let start_section = format!("<!-- TRIAGEBOT_{}_START -->\n", self.id);
        let end_section = format!("\n<!-- TRIAGEBOT_{}_END -->\n", self.id);

        let bot_section = format!("{}{}{}", start_section, self.text, end_section);

        let all_new = format!("\n\n{}{}{}", START_BOT, bot_section, END_BOT);
        if current_body.contains(START_BOT) {
            if current_body.contains(&start_section) {
                let start_idx = current_body.find(&start_section).unwrap();
                let end_idx = current_body.find(&end_section).unwrap();
                let mut new = current_body.replace(
                    &current_body[start_idx..(end_idx + end_section.len())],
                    &bot_section,
                );
                if new.contains(&all_new) && self.text.is_empty() {
                    let start_idx = new.find(&all_new).unwrap();
                    let end_idx = start_idx + all_new.len();
                    new = new.replace(&new[start_idx..end_idx], "");
                }
                self.issue.edit_body(&client, &new)?;
            } else {
                let end_idx = current_body.find(&END_BOT).unwrap();
                current_body.insert_str(end_idx, &bot_section);
                self.issue.edit_body(&client, &current_body)?;
            }
        } else {
            let new_body = format!("{}{}", current_body, all_new);

            self.issue.edit_body(&client, &new_body)?;
        }
        Ok(())
    }
}
