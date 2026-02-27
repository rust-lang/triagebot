use chrono::FixedOffset;

use std::ops::Deref;

use crate::github::{Comment, Issue, Label, Repository, User};

/// An event triggered by a webhook.
#[derive(Debug)]
pub enum Event {
    /// A Git branch or tag is created.
    Create(CreateEvent),
    /// A comment on an issue or PR.
    ///
    /// Can be:
    /// - Regular comment on an issue or PR.
    /// - A PR review.
    /// - A comment on a PR review.
    ///
    /// These different scenarios are unified into the `IssueComment` variant
    /// when triagebot receives the corresponding webhook event.
    IssueComment(IssueCommentEvent),
    /// Activity on an issue or PR.
    Issue(IssuesEvent),
    /// One or more commits are pushed to a repository branch or tag.
    Push(PushEvent),
}

impl Event {
    pub fn repo(&self) -> &Repository {
        match self {
            Event::Create(event) => &event.repository,
            Event::IssueComment(event) => &event.repository,
            Event::Issue(event) => &event.repository,
            Event::Push(event) => &event.repository,
        }
    }

    pub fn issue(&self) -> Option<&Issue> {
        match self {
            Event::Create(_) => None,
            Event::IssueComment(event) => Some(&event.issue),
            Event::Issue(event) => Some(&event.issue),
            Event::Push(_) => None,
        }
    }

    /// This will both extract from `IssueComment` events but also `Issue` events
    pub fn comment_body(&self) -> Option<&str> {
        match self {
            Event::Create(_) => None,
            Event::Issue(e) => Some(&e.issue.body),
            Event::IssueComment(e) => Some(&e.comment.body),
            Event::Push(_) => None,
        }
    }

    /// This will both extract from `IssueComment` events but also `Issue` events
    pub fn comment_from(&self) -> Option<&str> {
        match self {
            Event::Create(_) => None,
            Event::Issue(e) => Some(&e.changes.as_ref()?.body.as_ref()?.from),
            Event::IssueComment(e) => Some(&e.changes.as_ref()?.body.as_ref()?.from),
            Event::Push(_) => None,
        }
    }

    pub fn html_url(&self) -> Option<&str> {
        match self {
            Event::Create(_) => None,
            Event::Issue(e) => Some(&e.issue.html_url),
            Event::IssueComment(e) => Some(&e.comment.html_url),
            Event::Push(_) => None,
        }
    }

    pub fn user(&self) -> &User {
        match self {
            Event::Create(e) => &e.sender.user,
            Event::Issue(e) => &e.issue.user,
            Event::IssueComment(e) => &e.comment.user,
            Event::Push(e) => &e.sender.user,
        }
    }

    pub fn time(&self) -> Option<chrono::DateTime<FixedOffset>> {
        match self {
            Event::Create(_) => None,
            Event::Issue(e) => Some(e.issue.created_at.into()),
            Event::IssueComment(e) => e
                .comment
                .updated_at
                .or(e.comment.created_at)
                .map(Into::into),
            Event::Push(_) => None,
        }
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct CreateEvent {
    pub ref_type: CreateKind,
    repository: Repository,
    sender: Sender,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CreateKind {
    Branch,
    Tag,
}

#[derive(Debug, serde::Deserialize)]
pub struct PushEvent {
    /// The SHA of the most recent commit on `ref` after the push.
    pub after: String,
    /// The full git ref that was pushed.
    ///
    /// Example: `refs/heads/main` or `refs/tags/v3.14.1`.
    #[serde(rename = "ref")]
    pub git_ref: String,
    pub repository: Repository,
    sender: Sender,
}

/// The action that occurred in an org_block event.
#[derive(Debug, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OrgBlockAction {
    /// User was banned
    Blocked,
    /// User was unbannded
    Unblocked,
}

/// Organization information from an org_block event.
#[derive(Debug, serde::Deserialize)]
pub struct Sender {
    #[serde(flatten)]
    pub user: User,
    pub r#type: String,
}

impl Deref for Sender {
    type Target = User;

    fn deref(&self) -> &User {
        &self.user
    }
}

/// Organization information from an org_block event.
#[derive(Debug, serde::Deserialize)]
pub struct Organization {
    pub login: String,
    pub id: u64,
}

/// Event triggered when a user is blocked or unblocked from an organization.
#[derive(Debug, serde::Deserialize)]
pub struct OrgBlockEvent {
    pub action: OrgBlockAction,
    pub blocked_user: User,
    pub organization: Organization,
    pub sender: Sender,
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IssueCommentAction {
    Created,
    Edited,
    Deleted,
}

#[derive(Debug, serde::Deserialize)]
pub struct IssueCommentEvent {
    pub action: IssueCommentAction,
    pub changes: Option<Changes>,
    pub issue: Issue,
    pub comment: Comment,
    pub repository: Repository,
}

#[derive(Debug, serde::Deserialize)]
pub struct IssuesEvent {
    #[serde(flatten)]
    pub action: IssuesAction,
    #[serde(alias = "pull_request")]
    pub issue: Issue,
    pub changes: Option<Changes>,
    pub before: Option<String>,
    pub after: Option<String>,
    pub repository: Repository,
    /// The GitHub user that triggered the event.
    pub sender: Sender,
}

impl IssuesEvent {
    pub fn has_base_changed(&self) -> bool {
        matches!(self.action, IssuesAction::Edited)
            && matches!(&self.changes, Some(changes) if changes.base.is_some())
    }
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "action")]
pub enum IssuesAction {
    Opened,
    Edited,
    Deleted,
    Transferred,
    Pinned,
    Unpinned,
    Closed,
    Reopened,
    Assigned {
        /// Github users assigned to the issue / pull request
        assignee: User,
    },
    Unassigned {
        /// Github users removed from the issue / pull request
        assignee: User,
    },
    Labeled {
        /// The label added from the issue
        label: Label,
    },
    Unlabeled {
        /// The label removed from the issue
        ///
        /// The `label` is `None` when a label is deleted from the repository.
        label: Option<Label>,
    },
    Locked,
    Unlocked,
    Milestoned,
    Demilestoned,
    ReviewRequested {
        /// The person requested to review the pull request
        ///
        /// This can be `None` when a review is requested for a team.
        requested_reviewer: Option<User>,
    },
    ReviewRequestRemoved,
    ReadyForReview,
    Synchronize,
    ConvertedToDraft,
    AutoMergeEnabled,
    AutoMergeDisabled,
    Enqueued,
    Dequeued,
    Typed,
    Untyped,
}

#[derive(Debug, serde::Deserialize)]
pub struct Changes {
    pub title: Option<ChangeInner>,
    pub body: Option<ChangeInner>,
    pub base: Option<BaseChange>,
}

#[derive(Debug, serde::Deserialize)]
pub struct ChangeInner {
    pub from: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct BaseChange {
    pub r#ref: ChangeInner,
    pub sha: ChangeInner,
}

/// A pull request review event.
///
/// <https://docs.github.com/en/developers/webhooks-and-events/webhooks/webhook-events-and-payloads#pull_request_review>
#[derive(Debug, serde::Deserialize)]
pub struct PullRequestReviewEvent {
    pub action: PullRequestReviewAction,
    pub pull_request: Issue,
    pub review: Comment,
    pub changes: Option<Changes>,
    pub repository: Repository,
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PullRequestReviewAction {
    Submitted,
    Edited,
    Dismissed,
}

#[derive(Debug, serde::Deserialize)]
pub struct PullRequestReviewComment {
    pub action: IssueCommentAction,
    pub changes: Option<Changes>,
    #[serde(rename = "pull_request")]
    pub issue: Issue,
    pub comment: Comment,
    pub repository: Repository,
}
