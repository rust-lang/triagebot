use crate::config::ContributionGuidelinesConfig;
use crate::github::{Event, IssueCommentAction, IssuesAction};
use crate::handlers::Context;
use tracing as log;

/// What triggered the handler.
enum Trigger<'a> {
    /// PR was newly opened (non-draft or draft — we respond either way).
    Opened,
    /// PR was moved from draft to ready-for-review.
    Undrafted,
    /// A comment was posted on the PR. Carries the comment body.
    Comment(&'a str),
}

pub(crate) async fn handle(
    ctx: &Context,
    event: &Event,
    config: &ContributionGuidelinesConfig,
) -> anyhow::Result<()> {
    // Determine the trigger and get the issue reference.
    let (trigger, issue, sender_id) = match event {
        Event::Issue(e) if e.issue.is_pr() => {
            // Skip events triggered by the bot itself.
            if e.sender.login == ctx.username {
                return Ok(());
            }
            match &e.action {
                IssuesAction::Opened => (Trigger::Opened, &e.issue, e.sender.id),
                IssuesAction::ReadyForReview => (Trigger::Undrafted, &e.issue, e.sender.id),
                _ => return Ok(()),
            }
        }
        Event::IssueComment(e) if e.issue.is_pr() => {
            if e.comment.user.login == ctx.username {
                return Ok(());
            }
            match e.action {
                IssueCommentAction::Created => (
                    Trigger::Comment(&e.comment.body),
                    &e.issue,
                    e.comment.user.id,
                ),
                _ => return Ok(()),
            }
        }
        _ => return Ok(()),
    };

    let author_id = issue.user.id;
    let author_login = &issue.user.login;
    let repo = issue.repository();

    // Only act on comments from the PR author.
    if matches!(trigger, Trigger::Comment(_)) {
        if sender_id != author_id {
            return Ok(());
        }
    }

    // Check if the author already has a merged PR in this repo.
    let query = format!(
        "author:{} repo:{}/{} type:pr is:merged",
        author_login, repo.organization, repo.repository
    );
    let merged_count = ctx.github.issue_search_count(&query).await?;
    if merged_count > 0 {
        log::debug!(
            "contribution_guidelines: {} has {} merged PRs, skipping",
            author_login,
            merged_count
        );
        return Ok(());
    }

    // Notice-only mode: when `expect` is empty, just post an informational
    // message on newly opened PRs without requiring acknowledgement.
    if !config.requires_acknowledgement() {
        if matches!(trigger, Trigger::Opened) {
            let prompt = config.prompt();
            let body = config.substitute(&prompt, author_login, &ctx.username);
            issue.post_comment(&ctx.github, &body).await?;
        }
        return Ok(());
    }

    // --- Gated mode: acknowledgement is required ---

    // Fetch existing comments to check for prior acknowledgement.
    // FIXME: only fetches the first 100 comments — add pagination if this
    // ever becomes a problem in practice.
    let comments = issue.get_first100_comments(&ctx.github).await?;

    // Check if the author has already posted the expected acknowledgement.
    let expect_lower = config.expect.trim().to_lowercase();
    let bot_prefix = format!("@{}", ctx.username);
    let acknowledged = comments.iter().any(|c| {
        if c.user.id != author_id {
            return false;
        }
        let body = c.body.trim();
        // Strip optional leading @bot mention.
        let body = body
            .strip_prefix(&bot_prefix)
            .map(|s| s.trim())
            .unwrap_or(body);
        body.eq_ignore_ascii_case(&expect_lower)
    });

    if acknowledged {
        log::debug!(
            "contribution_guidelines: {} already acknowledged on PR #{}",
            author_login,
            issue.number
        );
        return Ok(());
    }

    // Determine what message to post.
    let prompt = config.prompt();
    let template = match &trigger {
        Trigger::Opened => &prompt,
        Trigger::Undrafted => &config.undrafted,
        Trigger::Comment(body) => {
            // Only respond if the comment is more recent than the bot's last comment
            // and looks like an attempt (mentions the bot or partially matches expect).
            let bot_last_comment_time = comments
                .iter()
                .rev()
                .find(|c| c.user.login.eq_ignore_ascii_case(&ctx.username))
                .and_then(|c| c.created_at);

            let comment_time = if let Event::IssueComment(e) = event {
                e.comment.created_at
            } else {
                None
            };

            let is_more_recent = match (comment_time, bot_last_comment_time) {
                (Some(ct), Some(bt)) => ct > bt,
                (Some(_), None) => {
                    // Bot has never commented — feature may have been enabled after
                    // the PR was opened. Treat the author's comment as recent so we
                    // respond rather than silently ignoring.
                    true
                }
                (None, _) => {
                    // Can't determine comment timing; be conservative and don't respond.
                    false
                }
            };

            if !is_more_recent {
                return Ok(());
            }

            // Check if the comment looks like an attempt to acknowledge.
            let body_lower = body.trim().to_lowercase();
            let mentions_bot = body_lower.contains(&ctx.username.to_lowercase());
            let partial_match = expect_lower
                .split_whitespace()
                .any(|word| word.len() > 3 && body_lower.contains(word));

            if mentions_bot || partial_match {
                &config.wrong_response
            } else {
                return Ok(());
            }
        }
    };

    let body = config.substitute(template, author_login, &ctx.username);
    issue.post_comment(&ctx.github, &body).await?;

    // Convert the PR to draft.
    if let Some(node_id) = &issue.node_id {
        if !issue.draft {
            ctx.github.convert_to_draft(node_id).await?;
            log::info!(
                "contribution_guidelines: converted PR #{} to draft",
                issue.number
            );
        }
    } else {
        log::warn!(
            "contribution_guidelines: no node_id on PR #{}, cannot convert to draft",
            issue.number
        );
    }

    Ok(())
}
