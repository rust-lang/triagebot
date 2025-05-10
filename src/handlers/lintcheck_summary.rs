//! Clippy has a tool called [lintcheck] that's used in CI to show how a PR
//! would change the output of Clippy when ran on a large corpus of crates
//!
//! The workflow run uploads a JSON summary as a GitHub artifact which this
//! handler uses to post a summary of the changes as a comment in the pull
//! request
//!
//! [lintcheck]: https://github.com/rust-lang/rust-clippy/tree/master/lintcheck

use std::fmt::Write;
use std::io::{self, Cursor};

use anyhow::Context as _;
use octocrab::models::{CommentId, RunId};
use serde::{Deserialize, Serialize};
use zip::ZipArchive;

use crate::config::LintcheckSummaryConfig;
use crate::db::issue_data::IssueData;
use crate::github::{
    Event, ReportedContentClassifiers, Repository, WorkflowRunAction, WorkflowRunConclusion,
};
use crate::handlers::Context;

/// An arbitrary limit of 32KiB, avoids downloading large files and/or
/// decompressing a ZIP bomb
const MAX_SIZE: u64 = 32768;

const LINTCHECK_SUMMARY_KEY: &str = "lintcheck-summary";

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
struct CommentIds {
    comment_id: CommentId,
    /// The GraphQL id used for [un]hiding the comment
    node_id: String,
}

#[derive(Debug, Default, Deserialize, Serialize, Clone, PartialEq)]
struct LintcheckSummaryState {
    /// The IDs of the comment if we've already commented on a particular PR
    comment_ids: Option<CommentIds>,
}

/// The summary JSON can come from PRs from forks and so may contain arbitrary
/// content. Before using their contents in a comment we must validate that they
/// contain only the expected characters
///
/// This is to avoid e.g. @user mass ping spams or triggering any other bots
/// that may be a privileged operation
#[derive(Debug, Deserialize)]
struct UntrustedString(String);

impl UntrustedString {
    fn validate(&self, f: impl Fn(char) -> bool) -> anyhow::Result<&str> {
        for ch in self.0.chars() {
            anyhow::ensure!(f(ch), "string contains invalid character: {ch:?}");
        }
        Ok(&self.0)
    }
}

#[derive(Debug, Deserialize)]
struct SummaryRow {
    name: UntrustedString,
    url: UntrustedString,
    added: u64,
    removed: u64,
    changed: u64,
}

#[derive(Debug, Deserialize)]
struct Summary {
    commit: UntrustedString,
    rows: Vec<SummaryRow>,
}

pub(super) async fn handle(
    ctx: &Context,
    event: &Event,
    config: &LintcheckSummaryConfig,
) -> anyhow::Result<()> {
    let Event::WorkflowRun(event) = event else {
        return Ok(());
    };

    if event.action != WorkflowRunAction::Completed
        || event.workflow_run.name != config.workflow
        || event.workflow_run.conclusion != Some(WorkflowRunConclusion::Success)
    {
        return Ok(());
    }

    let [pr] = event.workflow_run.pull_requests.as_slice() else {
        return Ok(());
    };

    let summary = download_summary(
        ctx,
        &event.repository,
        event.workflow_run.id,
        &config.artifact,
    )
    .await?;

    let mut db = ctx.db.get().await;
    let mut state: IssueData<'_, LintcheckSummaryState> = IssueData::load(
        &mut db,
        event.repository.full_name.clone(),
        pr.number as i32,
        LINTCHECK_SUMMARY_KEY,
    )
    .await?;

    if state.data.comment_ids.is_none() && summary.is_none() {
        return Ok(());
    }

    if let Some(ids) = &state.data.comment_ids {
        // There is a previous comment, if there's a summary unhide it, hide it
        // if not
        //
        // It's not an error to [un]hide an already [un]hidden comment
        if summary.is_some() {
            ctx.github.unhide_comment(&ids.node_id).await?;
        } else {
            ctx.github
                .hide_comment(&ids.node_id, ReportedContentClassifiers::Outdated)
                .await?;
        }
    }

    let Some(summary) = summary else {
        return Ok(());
    };

    let markdown = summary_to_markdown(&summary)?;

    // Post the comment, or update the previous one if it already exists
    if let Some(ids) = &state.data.comment_ids {
        ctx.octocrab
            .issues(event.repository.owner(), event.repository.name())
            .update_comment(ids.comment_id, markdown)
            .await?;
    } else {
        let comment = ctx
            .octocrab
            .issues(event.repository.owner(), event.repository.name())
            .create_comment(pr.number, markdown)
            .await?;
        state.data.comment_ids = Some(CommentIds {
            comment_id: comment.id,
            node_id: comment.node_id,
        });
        state.save().await?;
    }

    Ok(())
}

async fn download_summary(
    ctx: &Context,
    repo: &Repository,
    run_id: RunId,
    artifact_name: &str,
) -> anyhow::Result<Option<Summary>> {
    let artifacts = ctx
        .octocrab
        .actions()
        .list_workflow_run_artifacts(repo.owner(), repo.name(), run_id)
        .send()
        .await?
        .value
        .context("missing value")?
        .items;

    let Some(artifact) = artifacts
        .into_iter()
        .find(|artifact| artifact.name == artifact_name)
    else {
        return Ok(None);
    };

    anyhow::ensure!(
        artifact.size_in_bytes < MAX_SIZE as usize,
        "artifact archive is too large"
    );

    let bytes = ctx
        .octocrab
        // This is a ZIP file but the API wants `application/json`
        .download(artifact.archive_download_url.as_str(), "application/json")
        .await?;
    let mut zip = ZipArchive::new(Cursor::new(bytes))?;
    let file = zip.by_index(0)?;
    anyhow::ensure!(file.size() < MAX_SIZE, "artifact file is too large");

    let file_contents = io::read_to_string(file)?;
    let summary: Summary = serde_json::from_str(&file_contents)?;

    Ok(Some(summary))
}

fn summary_to_markdown(summary: &Summary) -> anyhow::Result<String> {
    let commit = summary.commit.validate(|ch| ch.is_ascii_alphanumeric())?;

    let mut md = format!(
        "Lintcheck changes for {commit}

| Lint | Added | Removed | Changed |
| ---- | ----: | ------: | ------: |
"
    );

    for SummaryRow {
        name,
        url,
        added,
        removed,
        changed,
    } in &summary.rows
    {
        let name = name.validate(|ch| matches!(ch, 'a'..='z' | ':' | '_' | '-'))?;
        let url = url.validate(|ch| {
            ch.is_ascii_alphanumeric() || matches!(ch, ':' | '/' | '.' | '#' | '-')
        })?;
        writeln!(
            &mut md,
            "| [`{name}`]({url}) | {added} | {removed} | {changed} |"
        )?;
    }

    md.push_str("\nThis comment will be updated if you push new changes");

    Ok(md)
}
