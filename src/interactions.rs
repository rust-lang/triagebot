use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio_postgres::Client as DbClient;

use crate::{
    db::issue_data::IssueData,
    github::{Comment, GithubClient, Issue},
};
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

    pub async fn post(&self, client: &GithubClient) -> anyhow::Result<Comment> {
        let mut body = String::new();
        writeln!(body, "**Error**: {}", self.message)?;
        writeln!(body)?;
        writeln!(
            body,
            "Please file an issue on GitHub at [triagebot](https://github.com/rust-lang/triagebot) if there's \
            a problem with this bot, or reach out on [#t-infra](https://rust-lang.zulipchat.com/#narrow/stream/242791-t-infra) on Zulip."
        )?;
        self.issue.post_comment(client, &body).await
    }
}

pub struct EditIssueBody<'a, T>
where
    T: for<'t> Deserialize<'t> + Serialize + Default + std::fmt::Debug + Sync + PartialEq + Clone,
{
    issue_data: IssueData<'a, T>,
    issue: &'a Issue,
    id: &'static str,
}

static START_BOT: &str = "<!-- TRIAGEBOT_START -->\n\n";
static END_BOT: &str = "<!-- TRIAGEBOT_END -->";

fn normalize_body(body: &str) -> String {
    str::replace(body, "\r\n", "\n")
}

impl<'a, T> EditIssueBody<'a, T>
where
    T: for<'t> Deserialize<'t> + Serialize + Default + std::fmt::Debug + Sync + PartialEq + Clone,
{
    pub async fn load(
        db: &'a mut DbClient,
        issue: &'a Issue,
        id: &'static str,
    ) -> Result<EditIssueBody<'a, T>> {
        let issue_data = IssueData::load(db, issue, id).await?;

        let mut edit = EditIssueBody {
            issue_data,
            issue,
            id,
        };

        // Legacy, if we find data inside the issue body for the current
        // id, use that instead of the (hopefully) default value given
        // by IssueData.
        if let Some(d) = edit.current_data_markdown() {
            edit.issue_data.data = d;
        }

        Ok(edit)
    }

    pub fn data_mut(&mut self) -> &mut T {
        &mut self.issue_data.data
    }

    pub async fn apply(self, client: &GithubClient, text: String) -> anyhow::Result<()> {
        let mut current_body = normalize_body(&self.issue.body.clone());
        let start_section = self.start_section();
        let end_section = self.end_section();

        let bot_section = format!("{}{}{}", start_section, text, end_section);
        let empty_bot_section = format!("{}{}", start_section, end_section);
        let all_new = format!("\n\n{}{}{}", START_BOT, bot_section, END_BOT);

        // Edit or add the new text the current triagebot section
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

        // Save the state in the database
        self.issue_data.save().await?;

        Ok(())
    }

    fn start_section(&self) -> String {
        format!("<!-- TRIAGEBOT_{}_START -->\n", self.id)
    }

    fn end_section(&self) -> String {
        format!("\n<!-- TRIAGEBOT_{}_END -->\n", self.id)
    }

    // Legacy, only used for handling data inside the issue body it-self

    fn current_data_markdown(&self) -> Option<T> {
        let all = self.get_current_markdown()?;
        let start = self.data_section_start();
        let end = self.data_section_end();
        let start_idx = all.find(&start)?;
        let end_idx = all.find(&end)?;
        let text = &all[(start_idx + start.len())..end_idx];
        Some(serde_json::from_str(text).unwrap_or_else(|e| {
            panic!("deserializing data {:?} failed: {:?}", text, e);
        }))
    }

    fn get_current_markdown(&self) -> Option<String> {
        let self_issue_body = normalize_body(&self.issue.body);
        let start_section = self.start_section();
        let end_section = self.end_section();
        if self_issue_body.contains(START_BOT) {
            if self_issue_body.contains(&start_section) {
                let start_idx = self_issue_body.find(&start_section).unwrap();
                let end_idx = self_issue_body.find(&end_section).unwrap();
                let current =
                    String::from(&self_issue_body[start_idx..(end_idx + end_section.len())]);
                Some(current)
            } else {
                None
            }
        } else {
            None
        }
    }

    fn data_section_start(&self) -> String {
        format!("\n<!-- TRIAGEBOT_{}_DATA_START$$", self.id)
    }

    fn data_section_end(&self) -> String {
        format!("$$TRIAGEBOT_{}_DATA_END -->\n", self.id)
    }
}
