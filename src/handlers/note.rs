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
//! <!-- rustbot summary start -->
//!
//! ### Notes
//!
//! - ["summary-title" by @username](link-to-comment)
//!
//! <!-- rustbot summary end -->
//! ```
//!
//! If this is *not* the first summary entry, rustbot will simply append the new entry to the existing notes section:
//!
//! ```md
//! <!-- rustbot summary start -->
//!
//! ### Notes
//!
//! - ["first-note" by @username](link-to-comment)
//! - ["second-note" by @username](link-to-comment)
//! - ["summary-title" by @username](link-to-comment)
//!
//! <!-- rustbot summary end -->
//! ```
//!

use crate::{
    config::NoteConfig,
    github::{self, Event, Selection},
    handlers::Context,
    interactions::EditIssueBody,
};
use anyhow::Context as _;
use parser::command::note::NoteCommand;
use tracing as log;

#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct NoteData {
    title: Option<String>,
    summary: Option<String>,
}

pub(super) async fn handle_command(
    ctx: &Context,
    _config: &NoteConfig,
    event: &Event,
    cmd: NoteCommand,
) -> anyhow::Result<()> {
    log::debug!("Handling Note Command");

    // TODO: edit the original post

    let issue = event.issue().unwrap();
    
    log::debug!("Issue: {:?}", issue);

    if issue.is_pr() {
        let NoteCommand::Summary { title, summary } = &cmd;
        log::debug!("Note: {}, {}", title, summary);
    }
    Ok(())
}
