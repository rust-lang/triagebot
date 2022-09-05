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

    let repo = e.issue.repository();
    if !(repo.organization == "rust-lang" && repo.repository == "rfcs") {
        return Ok(());
    }

    if let Err(e) = add_rendered_link(&ctx, &e).await {
        tracing::error!("Error adding rendered link: {:?}", e);
    }

    Ok(())
}

async fn add_rendered_link(ctx: &Context, e: &IssuesEvent) -> anyhow::Result<()> {
    if e.action == IssuesAction::Opened {
        let files = e.issue.files(&ctx.github).await?;

        if let Some(file) = files.iter().find(|f| f.filename.starts_with("text/")) {
            if !e.issue.body.contains("[Rendered]") {
                // This URL should be stable while the PR is open, even if the
                // user pushes new commits.
                //
                // It will go away if the user deletes their branch, or if
                // they reset it (such as if they created a PR from master).
                // That should usually only happen after the PR is closed.
                // During the closing process, the closer should update the
                // Rendered link to the new location (which we should
                // automate!).
                let head = e.issue.head.as_ref().unwrap();
                let url = format!(
                    "https://github.com/{}/blob/{}/{}",
                    head.repo.full_name, head.git_ref, file.filename
                );
                e.issue
                    .edit_body(
                        &ctx.github,
                        &format!("{}\n\n[Rendered]({})", e.issue.body, url),
                    )
                    .await?;
            }
        }
    }

    Ok(())
}
