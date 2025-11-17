use octocrab::models::AuthorAssociation;

use crate::{config::ShortcutConfig, db::issue_data::IssueData, github::Issue, handlers::Context};

/// Key for the state in the database
const AUTHOR_REMINDER_KEY: &str = "author-reminder";

/// State stored in the database for a PR.
#[derive(Debug, Default, serde::Deserialize, serde::Serialize, Clone, PartialEq)]
struct AuthorReminderState {
    /// ID of the reminder comment.
    reminder_comment: Option<String>,
}

pub(crate) async fn remind_author_of_bot_ready(
    ctx: &Context,
    issue: &Issue,
    config: Option<&ShortcutConfig>,
) -> anyhow::Result<()> {
    let Some(_config) = config else {
        // Ignore repository that don't have `[shortcut]` enabled
        return Ok(());
    };

    if matches!(
        issue.author_association,
        AuthorAssociation::Member | AuthorAssociation::Owner
    ) {
        // Don't post the reminder for org members (public-only unfortunately) and owner of the org.
        return Ok(());
    }

    // Get the state of the author reminder for this PR
    let mut db = ctx.db.get().await;
    let mut state: IssueData<'_, AuthorReminderState> =
        IssueData::load(&mut db, issue, AUTHOR_REMINDER_KEY).await?;

    // If no comment already posted, let's post it
    if state.data.reminder_comment.is_none() {
        let comment_body = format!(
            "Reminder, once the PR becomes ready for a review, use `@{bot} ready`.",
            bot = &ctx.username,
        );
        let comment = issue
            .post_comment(&ctx.github, comment_body.as_str())
            .await?;

        state.data.reminder_comment = Some(comment.node_id);
        state.save().await?;
    }

    Ok(())
}
