use anyhow::bail;

use crate::{
    github::{Event, IssuesAction, IssuesEvent},
    handlers::Context,
};

pub async fn handle(ctx: &Context, event: &Event) -> anyhow::Result<()> {
    let e = if let Event::Issue(e) = event {
        e
    } else {
        return Ok(());
    };

    if !e.issue.is_pr() {
        return Ok(());
    }

    let repo = e.issue.repository();
    let prefix = match (&*repo.organization, &*repo.repository) {
        ("rust-lang", "rfcs") => "text/",
        ("rust-lang", "blog.rust-lang.org") => "posts/",
        _ => return Ok(()),
    };

    if let Err(e) = add_rendered_link(&ctx, &e, prefix).await {
        tracing::error!("Error adding rendered link: {:?}", e);
    }

    Ok(())
}

async fn add_rendered_link(ctx: &Context, e: &IssuesEvent, prefix: &str) -> anyhow::Result<()> {
    if e.action == IssuesAction::Opened
        || e.action == IssuesAction::Closed
        || e.action == IssuesAction::Reopened
    {
        let files = e.issue.files(&ctx.github).await?;

        if let Some(file) = files.iter().find(|f| f.filename.starts_with(prefix)) {
            let head = e.issue.head.as_ref().unwrap();
            let base = e.issue.base.as_ref().unwrap();

            // This URL should be stable while the PR is open, even if the
            // user pushes new commits.
            //
            // It will go away if the user deletes their branch, or if
            // they reset it (such as if they created a PR from master).
            // That should usually only happen after the PR is closed
            // a which point we switch to a SHA-based url.
            //
            // If the PR is merged we use a URL that points to the actual
            // repository, as to be resilient to branch deletion, as well
            // be in sync with current "master" branch.
            //
            // For a PR "octocat:master" <- "Bob:patch-1", we generate,
            //  - if merged: `https://github.com/octocat/REPO/blob/master/FILEPATH`
            //  - if open: `https://github.com/Bob/REPO/blob/patch-1/FILEPATH`
            //  - if closed: `https://github.com/octocat/REPO/blob/SHA/FILEPATH`
            let rendered_link = format!(
                "[Rendered](https://github.com/{}/blob/{}/{})",
                if e.issue.merged || e.action == IssuesAction::Closed {
                    &e.repository.full_name
                } else {
                    &head.repo.full_name
                },
                if e.issue.merged {
                    &base.git_ref
                } else if e.action == IssuesAction::Closed {
                    &head.sha
                } else {
                    &head.git_ref
                },
                file.filename
            );

            let new_body = if !e.issue.body.contains("[Rendered]") {
                // add rendered link to the end of the body
                format!("{}\n\n{rendered_link}", e.issue.body)
            } else if let Some(start_pos) = e.issue.body.find("[Rendered](") {
                let Some(end_offset) = &e.issue.body[start_pos..].find(')') else {
                    bail!("no `)` after `[Rendered]` found")
                };

                // replace the current rendered link with the new one
                e.issue.body.replace(
                    &e.issue.body[start_pos..=(start_pos + end_offset)],
                    &rendered_link,
                )
            } else {
                bail!(
                    "found `[Rendered]` but not it's associated link, can't replace it, bailing out"
                )
            };

            e.issue.edit_body(&ctx.github, &new_body).await?;
        }
    }

    Ok(())
}
