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

    // TODO: edit the original post to include:
    //
    // ```md
    // <!-- start rustbot summary-->
    // - [title](LINK_TO_SUMMARY_COMMENT)
    // <!-- end rustbot summary-->
    // ```

    let issue = event.issue().unwrap();
    if issue.is_pr() {
        let NoteCommand::Summary { title, summary } = &cmd;
        log::debug!("Note: {}, {}", title, summary);
    }
    Ok(())
}
