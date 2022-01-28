//! Allow users to add summary comments in Issues & Pull Requests.
//!
//! Users can make a new summary entry by commenting the following:
//!
//! ```md
//! @rustbot note summary-title
//!
//! ...details details details...
//! ```
//!
//! If this is the first summary entry, rustbot will amend the original post (the top-level comment) to add a "Notes" section:
//!
//! ```md
//! <!-- TRIAGEBOT_SUMMARY_START -->
//!
//! ### Summary Notes
//!
//! - ["summary-title" by @username](link-to-comment)
//!
//! <!-- TRIAGEBOT_SUMMARY_END -->
//! ```
//!
//! If this is *not* the first summary entry, rustbot will simply append the new entry to the existing notes section:
//!
//! ```md
//! <!-- TRIAGEBOT_SUMMARY_START -->
//!
//! ### Summary Notes
//!
//! - ["first-note" by @username](link-to-comment)
//! - ["second-note" by @username](link-to-comment)
//! - ["summary-title" by @username](link-to-comment)
//!
//! <!-- TRIAGEBOT_SUMMARY_END -->
//! ```
//!

use crate::{config::NoteConfig, github::Event, handlers::Context, interactions::EditIssueBody};
use parser::command::note::NoteCommand;
use tracing as log;

#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct NoteDataEntry {
    title: String,
    comment_url: String,
    author: String,
}

impl NoteDataEntry {
    pub fn to_markdown(&self) -> String {
        format!(
            "\n- [\"{title}\" by @{author}]({comment_url})",
            title = self.title.to_owned(),
            author = self.author.to_owned(),
            comment_url = self.comment_url.to_owned()
        )
    }
}

#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
struct NoteData {
    entries: Vec<NoteDataEntry>,
}

impl NoteData {
    pub fn to_markdown(&self) -> String {
        if self.entries.is_empty() {
            return String::new();
        }
        let mut text = String::from("\n### Summary Notes\n");
        for entry in &self.entries {
            text.push_str(&entry.to_markdown());
        }
        text
    }
}

pub(super) async fn handle_command(
    ctx: &Context,
    _config: &NoteConfig,
    event: &Event,
    cmd: NoteCommand,
) -> anyhow::Result<()> {
    let issue = event.issue().unwrap();
    let e = EditIssueBody::new(&issue, "SUMMARY");

    let mut current: NoteData = e.current_data().unwrap_or_default();

    let comment_url = String::from(event.html_url().unwrap());
    let author = event.user().login.to_owned();
    let NoteCommand::Summary { title } = &cmd;

    let new_entry = NoteDataEntry {
        title: title.to_owned(),
        comment_url,
        author,
    };

    log::debug!("New Note Entry: {:#?}", new_entry);
    current.entries.push(new_entry);
    let new_markdown = current.to_markdown();
    log::debug!("New MD: {:#?}", new_markdown);

    e.apply(&ctx.github, new_markdown, current).await?;

    Ok(())
}
