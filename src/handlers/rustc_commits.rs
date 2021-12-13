use crate::db::rustc_commits;
use crate::{
    github::{self, Event},
    handlers::Context,
};
use std::convert::TryInto;
use tracing as log;

const BORS_GH_ID: i64 = 3372342;

pub async fn handle(ctx: &Context, event: &Event) -> anyhow::Result<()> {
    let body = match event.comment_body() {
        Some(v) => v,
        // Skip events that don't have comment bodies associated
        None => return Ok(()),
    };

    let event = if let Event::IssueComment(e) = event {
        if e.action != github::IssueCommentAction::Created {
            return Ok(());
        }

        e
    } else {
        return Ok(());
    };

    if !body.contains("Test successful") {
        return Ok(());
    }

    if event.comment.user.id != Some(BORS_GH_ID) {
        log::trace!("Ignoring non-bors comment, user: {:?}", event.comment.user);
        return Ok(());
    }

    let repo = event.issue.repository();
    if !(repo.organization == "rust-lang" && repo.repository == "rust") {
        return Ok(());
    }

    let start = "<!-- homu: ";
    let start = body.find(start).map(|s| s + start.len());
    let end = body.find(" -->");
    let (start, end) = if let (Some(start), Some(end)) = (start, end) {
        (start, end)
    } else {
        log::warn!("Unable to extract build completion from comment {:?}", body);
        return Ok(());
    };

    let bors: BorsMessage = match serde_json::from_str(&body[start..end]) {
        Ok(bors) => bors,
        Err(e) => {
            log::error!(
                "failed to parse build completion from {:?}: {:?}",
                &body[start..end],
                e
            );
            return Ok(());
        }
    };

    if bors.type_ != "BuildCompleted" {
        log::trace!("Not build completion? {:?}", bors);
    }

    if bors.base_ref != "master" {
        log::trace!("Ignoring bors merge, not on master");
        return Ok(());
    }

    let mut sha = bors.merge_sha;
    let mut pr = Some(event.issue.number.try_into().unwrap());

    let db = ctx.db.get().await;
    loop {
        // FIXME: ideally we would pull in all the commits here, but unfortunately
        // in rust-lang/rust's case there's bors-authored commits that aren't
        // actually from rust-lang/rust as they were merged into the clippy repo.
        let mut gc = match ctx.github.rust_commit(&sha).await {
            Some(c) => c,
            None => {
                log::error!("Could not find bors-reported sha: {:?}", sha);
                return Ok(());
            }
        };
        let parent_sha = gc.parents.remove(0).sha;

        if pr.is_none() {
            if let Some(tail) = gc.message.strip_prefix("Auto merge of #") {
                if let Some(end) = tail.find(' ') {
                    if let Ok(number) = tail[..end].parse::<u32>() {
                        pr = Some(number);
                    }
                }
            }
        }

        let pr = match pr.take() {
            Some(number) => number,
            None => break,
        };

        let res = rustc_commits::record_commit(
            &db,
            rustc_commits::Commit {
                sha: gc.sha,
                parent_sha: parent_sha.clone(),
                time: gc.commit.author.date,
                pr: Some(pr),
            },
        )
        .await;
        match res {
            Ok(()) => {
                // proceed to the next commit once we've recorded this one.
                // We'll stop when we hit a duplicate commit, but this allows us
                // to backfill commits.
                sha = parent_sha;
            }
            Err(e) => {
                log::error!("Failed to record commit {:?}", e);
                break;
            }
        }
    }

    Ok(())
}

#[derive(Debug, serde::Deserialize)]
struct BorsMessage {
    #[serde(rename = "type")]
    type_: String,
    base_ref: String,
    merge_sha: String,
}
