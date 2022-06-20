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
    if e.action == IssuesAction::Opened || e.action == IssuesAction::Synchronize {
        let files = e.issue.files(&ctx.github).await?;

        if let Some(file) = files.iter().find(|f| f.filename.starts_with("text/")) {
            if !e.issue.body.contains("[Rendered]") {
                e.issue
                    .edit_body(
                        &ctx.github,
                        &format!("{}\n\n[Rendered]({})", e.issue.body, file.blob_url),
                    )
                    .await?;
            }
        }
    }

    Ok(())
}
