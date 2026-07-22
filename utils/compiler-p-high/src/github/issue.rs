use chrono::Utc;
use std::fmt;
use std::sync::OnceLock;
use tracing as log;

use super::client::GithubClient;
use super::repos::{GitHubUser, Repository};
use super::utils::opt_string;
use crate::github::GithubCommit;

/// An issue or pull request.
///
/// For convenience, since issues and pull requests share most of their
/// fields, this struct is used for both. The `pull_request` field can be used
/// to determine which it is. Some fields are only available on pull requests
/// (but not always, check the GitHub API for details).
#[derive(Debug, serde::Deserialize)]
pub struct Issue {
    pub number: u64,
    #[serde(deserialize_with = "opt_string")]
    pub body: String,
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
    pub title: String,
    /// The common URL for viewing this issue or PR.
    ///
    /// Example: `https://github.com/octocat/Hello-World/pull/1347`
    pub html_url: String,
    // User performing an `action` (or PR/issue author)
    pub user: GitHubUser,
    pub labels: Vec<Label>,
    // Users assigned to the issue/pr after `action` has been performed issue
    // (PR reviewers or issue assignees)
    // These are NOT the same as `IssueEvent.assignee`
    pub assignees: Vec<GitHubUser>,
    /// Indicator if this is a pull request.
    ///
    /// This is `Some` if this is a PR (as opposed to an issue). Note that
    /// this does not always get filled in by GitHub, and must be manually
    /// populated (because some webhook events do not set it).
    pub pull_request: Option<PullRequestDetails>,
    /// Whether or not the pull request was merged.
    #[serde(default)]
    pub merged: bool,
    #[serde(default)]
    pub draft: bool,

    /// Number of comments
    pub comments: Option<u32>,

    /// The API URL for discussion comments.
    ///
    /// Example: `https://api.github.com/repos/octocat/Hello-World/issues/1347/comments`
    comments_url: String,
    /// The repository for this issue.
    ///
    /// Note that this is constructed via the [`Issue::repository`] method.
    /// It is not deserialized from the GitHub API.
    #[serde(skip)]
    pub repository: OnceLock<IssueRepository>,

    /// Whether it is open or closed.
    pub state: IssueState,
}

#[derive(PartialEq, Eq, Debug, Clone, Ord, PartialOrd, serde::Deserialize)]
pub struct Label {
    pub name: String,
}

#[derive(Debug, serde::Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum IssueState {
    Open,
    Closed,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct CommitBase {
    pub sha: String,
    #[serde(rename = "ref")]
    pub git_ref: String,
    pub repo: Option<Repository>,
}

/// An indicator used to differentiate between an issue and a pull request.
///
/// Some webhook events include a `pull_request` field in the Issue object,
/// and some don't. GitHub does include a few fields here, but they aren't
/// needed at this time (merged_at, diff_url, html_url, patch_url, url).
#[derive(Debug, serde::Deserialize)]
#[cfg_attr(test, derive(Default))]
pub struct PullRequestDetails {
    /// This is a slot to hold the diff for a PR.
    ///
    /// This will be filled in only once as an optimization since multiple
    /// handlers want to see PR changes, and getting the diff can be
    /// expensive.
    #[serde(skip)]
    compare: tokio::sync::OnceCell<GithubCompare>,
}

impl PullRequestDetails {
    pub fn new() -> PullRequestDetails {
        PullRequestDetails {
            compare: tokio::sync::OnceCell::new(),
        }
    }
}

/// The return from GitHub compare API
#[derive(Debug, serde::Deserialize)]
pub struct GithubCompare {
    /// The base commit of the PR
    pub base_commit: GithubCommit,
    /// The merge base commit
    ///
    /// See <https://git-scm.com/docs/git-merge-base> for more details
    pub merge_base_commit: GithubCommit,
    /// List of file differences
    pub files: Vec<FileDiff>,
    /// List of commits
    pub commits: Vec<GithubCommit>,
}

/// Representation of a diff to a single file.
#[derive(Debug, serde::Deserialize)]
pub struct FileDiff {
    /// The fullname path of the file.
    pub filename: String,
    /// The previous fullname path of the file.
    #[serde(default)]
    pub previous_filename: Option<String>,
    /// The patch/diff for the file.
    ///
    /// Can be empty when there isn't any changes to the content of the file
    /// (like when a file is renamed without it's content being modified).
    #[serde(default)]
    pub patch: String,
}

impl Issue {
    pub fn repository(&self) -> &IssueRepository {
        self.repository.get_or_init(|| {
            // https://api.github.com/repos/rust-lang/rust/issues/69257/comments
            log::trace!("get repository for {}", self.comments_url);
            let url = url::Url::parse(&self.comments_url).unwrap();
            let mut segments = url.path_segments().unwrap();
            let _comments = segments.next_back().unwrap();
            let _number = segments.next_back().unwrap();
            let _issues_or_prs = segments.next_back().unwrap();
            let repository = segments.next_back().unwrap();
            let organization = segments.next_back().unwrap();
            IssueRepository {
                organization: organization.into(),
                repository: repository.into(),
            }
        })
    }

    pub fn global_id(&self) -> String {
        format!("{}#{}", self.repository(), self.number)
    }

    pub fn is_pr(&self) -> bool {
        self.pull_request.is_some()
    }

    pub fn is_open(&self) -> bool {
        self.state == IssueState::Open
    }

    pub fn labels(&self) -> &[Label] {
        &self.labels
    }

    pub fn contains_label(&self, label: &Label) -> bool {
        self.labels
            .iter()
            .any(|l| l.name.to_lowercase() == label.name.to_lowercase())
    }

    pub fn contain_assignee(&self, user: &str) -> bool {
        self.assignees
            .iter()
            .any(|a| a.login.to_lowercase() == user.to_lowercase())
    }
}

// Comments

#[derive(Debug, serde::Deserialize)]
pub struct Comment {
    pub id: u64,
    pub node_id: String,
    #[serde(default)]
    pub in_reply_to_id: Option<u64>,
    #[serde(default)]
    pub pull_request_review_id: Option<u64>,
    #[serde(deserialize_with = "opt_string")]
    pub body: String,
    pub html_url: String,
    pub user: GitHubUser,
    #[serde(default, alias = "submitted_at")] // for pull-request review comments
    pub created_at: Option<chrono::DateTime<Utc>>,
    #[serde(default)]
    pub updated_at: Option<chrono::DateTime<Utc>>,
}

// Zulip <-> GitHub

/// Contains only the parts of `Issue` that are needed for turning the issue title into a Zulip
/// topic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZulipGitHubReference {
    pub number: u64,
    pub title: String,
    pub repository: IssueRepository,
}

impl ZulipGitHubReference {
    pub fn zulip_topic_reference(&self) -> String {
        let repo = &self.repository;
        if repo.organization == "rust-lang" {
            if repo.repository == "rust" {
                format!("#{}", self.number)
            } else {
                format!("{}#{}", repo.repository, self.number)
            }
        } else {
            format!("{}/{}#{}", repo.organization, repo.repository, self.number)
        }
    }
}

impl Issue {
    pub fn to_zulip_github_reference(&self) -> ZulipGitHubReference {
        ZulipGitHubReference {
            number: self.number,
            title: self.title.clone(),
            repository: self.repository().clone(),
        }
    }
}

// Issue repository

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueRepository {
    pub organization: String,
    pub repository: String,
}

impl fmt::Display for IssueRepository {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}/{}", self.organization, self.repository)
    }
}

impl IssueRepository {
    pub(crate) fn url(&self, client: &GithubClient) -> String {
        format!(
            "{}/repos/{}/{}",
            client.api_url, self.organization, self.repository
        )
    }
}
