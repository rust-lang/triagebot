//! Database support for the notifications feature for tracking `@` mentions.
//!
//! See <https://github.com/rust-lang/triagebot/wiki/Notifications>

use chrono::{DateTime, FixedOffset};

/// Tracking `@` mentions for users in issues/PRs.
pub struct Notification {
    pub user_id: i64,
    pub origin_url: String,
    pub origin_html: String,
    pub short_description: Option<String>,
    pub time: DateTime<FixedOffset>,

    /// If this is Some, then the notification originated in a team-wide ping
    /// (e.g., @rust-lang/libs). The String is the team name (e.g., libs).
    pub team_name: Option<String>,
}

/// Metadata associated with an `@` notification that the user can set via Zulip.
#[derive(Debug)]
pub struct NotificationData {
    pub origin_url: String,
    pub origin_text: String,
    pub short_description: Option<String>,
    pub time: DateTime<FixedOffset>,
    pub metadata: Option<String>,
}

/// Selector for deleting `@` notifications.
#[derive(Copy, Clone)]
pub enum Identifier<'a> {
    Url(&'a str),
    Index(std::num::NonZeroUsize),
    /// Glob identifier (`all` or `*`).
    All,
}
