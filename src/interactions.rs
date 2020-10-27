use crate::github::{GithubClient, Issue};
use std::fmt::Write;

pub struct Comment<'a> {
    issue: &'a Issue,
    message: String,
}

impl<'a> Comment<'a> {
    pub fn new<T>(issue: &'a Issue, message: T) -> Comment<'a>
    where
        T: Into<String>,
    {
        Comment {
            issue,
            message: message.into(),
        }
    }

    pub async fn post(&self, client: &GithubClient) -> anyhow::Result<()> {
        self.issue.post_comment(client, &self.message).await
    }
}

pub struct ErrorComment<'a> {
    comment: Comment<'a>,
}

impl<'a> ErrorComment<'a> {
    pub fn new<T>(issue: &'a Issue, message: T) -> ErrorComment<'a>
    where
        T: Into<String>,
    {
        ErrorComment {
            comment: Comment::new(issue, message),
        }
    }

    pub async fn post(&self, client: &GithubClient) -> anyhow::Result<()> {
        let mut body = String::new();
        writeln!(body, "**Error**: {}", self.comment.message)?;
        writeln!(body)?;
        writeln!(
            body,
            "Please let **`@rust-lang/release`** know if you're having trouble with this bot."
        )?;
        self.comment.issue.post_comment(client, &body).await
    }
}

pub struct EditIssueBody<'a> {
    issue: &'a Issue,
    id: &'static str,
}

static START_BOT: &str = "<!-- TRIAGEBOT_START -->\n\n";
static END_BOT: &str = "<!-- TRIAGEBOT_END -->";

impl<'a> EditIssueBody<'a> {
    pub fn new(issue: &'a Issue, id: &'static str) -> EditIssueBody<'a> {
        EditIssueBody { issue, id }
    }

    fn get_current(&self) -> Option<&str> {
        let start_section = self.start_section();
        let end_section = self.end_section();
        if self.issue.body.contains(START_BOT) {
            if self.issue.body.contains(&start_section) {
                let start_idx = self.issue.body.find(&start_section).unwrap();
                let end_idx = self.issue.body.find(&end_section).unwrap();
                Some(&self.issue.body[start_idx..(end_idx + end_section.len())])
            } else {
                None
            }
        } else {
            None
        }
    }

    pub fn current_data<T: serde::de::DeserializeOwned>(&self) -> Option<T> {
        let all = self.get_current()?;
        let start = self.data_section_start();
        let end = self.data_section_end();
        let start_idx = all.find(&start).unwrap();
        let end_idx = all.find(&end).unwrap();
        let text = &all[(start_idx + start.len())..end_idx];
        Some(serde_json::from_str(text).unwrap_or_else(|e| {
            panic!("deserializing data {:?} failed: {:?}", text, e);
        }))
    }

    fn start_section(&self) -> String {
        format!("<!-- TRIAGEBOT_{}_START -->\n", self.id)
    }

    fn end_section(&self) -> String {
        format!("\n<!-- TRIAGEBOT_{}_END -->\n", self.id)
    }

    fn data_section_start(&self) -> String {
        format!("\n<!-- TRIAGEBOT_{}_DATA_START$$", self.id)
    }

    fn data_section_end(&self) -> String {
        format!("$$TRIAGEBOT_{}_DATA_END -->\n", self.id)
    }

    fn data_section<T>(&self, data: T) -> String
    where
        T: serde::Serialize,
    {
        format!(
            "{}{}{}",
            self.data_section_start(),
            serde_json::to_string(&data).unwrap(),
            self.data_section_end()
        )
    }

    pub async fn apply<T>(&self, client: &GithubClient, text: String, data: T) -> anyhow::Result<()>
    where
        T: serde::Serialize,
    {
        let mut current_body = self.issue.body.clone();
        let start_section = self.start_section();
        let end_section = self.end_section();

        let bot_section = format!(
            "{}{}{}{}",
            start_section,
            text,
            self.data_section(data),
            end_section
        );
        let empty_bot_section = format!("{}{}", start_section, end_section);

        let all_new = format!("\n\n{}{}{}", START_BOT, bot_section, END_BOT);
        if current_body.contains(START_BOT) {
            if current_body.contains(&start_section) {
                let start_idx = current_body.find(&start_section).unwrap();
                let end_idx = current_body.find(&end_section).unwrap();
                current_body.replace_range(start_idx..(end_idx + end_section.len()), &bot_section);
                if current_body.contains(&all_new) && bot_section == empty_bot_section {
                    let start_idx = current_body.find(&all_new).unwrap();
                    let end_idx = start_idx + all_new.len();
                    current_body.replace_range(start_idx..end_idx, "");
                }
                self.issue.edit_body(&client, &current_body).await?;
            } else {
                let end_idx = current_body.find(&END_BOT).unwrap();
                current_body.insert_str(end_idx, &bot_section);
                self.issue.edit_body(&client, &current_body).await?;
            }
        } else {
            let new_body = format!("{}{}", current_body, all_new);

            self.issue.edit_body(&client, &new_body).await?;
        }
        Ok(())
    }
}
