use indexmap::IndexSet;
use std::sync::atomic::AtomicBool;
use std::sync::{LazyLock, Mutex};

use crate::github::{IssueRepository, IssuesAction, PrStatus};
use crate::{github::Event, handlers::Context};

pub(crate) async fn handle(ctx: &Context, event: &Event) -> anyhow::Result<()> {
    let Event::Issue(event) = event else {
        return Ok(());
    };
    if event.action != IssuesAction::Opened {
        return Ok(());
    }
    if !event.issue.is_pr() {
        return Ok(());
    }

    // avoid acting on our own open events, otherwise we'll infinitely loop
    if event.sender.login == ctx.username {
        return Ok(());
    }

    // If it's not the github-actions bot, we don't expect this handler to be needed. Skip the
    // event.
    if event.sender.login != "github-actions" {
        return Ok(());
    }

    if DISABLE.load(std::sync::atomic::Ordering::Relaxed) {
        tracing::warn!("skipping bot_pull_requests handler due to previous disable",);
        return Ok(());
    }

    // Sanity check that our logic above doesn't cause us to act on PRs in a loop, by
    // tracking a window of PRs we've acted on. We can probably drop this if we don't see problems
    // in the first few days/weeks of deployment.
    {
        let mut touched = TOUCHED_PRS.lock().unwrap();
        if !touched.insert((event.issue.repository().clone(), event.issue.number)) {
            tracing::warn!("touching same PR twice despite username check: {:?}", event);
            DISABLE.store(true, std::sync::atomic::Ordering::Relaxed);
            return Ok(());
        }
        if touched.len() > 300 {
            touched.drain(..150);
        }
    }

    ctx.github
        .set_pr_status(
            event.issue.repository(),
            event.issue.number,
            PrStatus::Closed,
        )
        .await?;
    ctx.github
        .set_pr_status(event.issue.repository(), event.issue.number, PrStatus::Open)
        .await?;

    Ok(())
}

static TOUCHED_PRS: LazyLock<Mutex<IndexSet<(IssueRepository, u64)>>> =
    LazyLock::new(|| std::sync::Mutex::new(IndexSet::new()));
static DISABLE: AtomicBool = AtomicBool::new(false);
