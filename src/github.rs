use anyhow::{anyhow, Context};
use async_trait::async_trait;
use chrono::{DateTime, FixedOffset, Utc};
use futures::{future::BoxFuture, FutureExt};
use hyper::header::HeaderValue;
use once_cell::sync::OnceCell;
use reqwest::header::{AUTHORIZATION, USER_AGENT};
use reqwest::{Client, Request, RequestBuilder, Response, StatusCode};
use std::collections::{HashMap, HashSet};
use std::convert::TryInto;
use std::{
    fmt,
    time::{Duration, SystemTime},
};
use tracing as log;

#[derive(Debug, PartialEq, Eq, serde::Deserialize)]
pub struct User {
    pub login: String,
    pub id: Option<i64>,
}

impl GithubClient {
    async fn _send_req(&self, req: RequestBuilder) -> anyhow::Result<(Response, String)> {
        const MAX_ATTEMPTS: usize = 2;
        log::debug!("_send_req with {:?}", req);
        let req_dbg = format!("{:?}", req);
        let req = req
            .build()
            .with_context(|| format!("building reqwest {}", req_dbg))?;

        let mut resp = self.client.execute(req.try_clone().unwrap()).await?;
        if let Some(sleep) = Self::needs_retry(&resp).await {
            resp = self.retry(req, sleep, MAX_ATTEMPTS).await?;
        }

        resp.error_for_status_ref()?;

        Ok((resp, req_dbg))
    }

    async fn needs_retry(resp: &Response) -> Option<Duration> {
        const REMAINING: &str = "X-RateLimit-Remaining";
        const RESET: &str = "X-RateLimit-Reset";

        if resp.status().is_success() {
            return None;
        }

        let headers = resp.headers();
        if !(headers.contains_key(REMAINING) && headers.contains_key(RESET)) {
            return None;
        }

        // Weird github api behavior. It asks us to retry but also has a remaining count above 1
        // Try again immediately and hope for the best...
        if headers[REMAINING] != "0" {
            return Some(Duration::from_secs(0));
        }

        let reset_time = headers[RESET].to_str().unwrap().parse::<u64>().unwrap();
        Some(Duration::from_secs(Self::calc_sleep(reset_time) + 10))
    }

    fn calc_sleep(reset_time: u64) -> u64 {
        let epoch_time = SystemTime::UNIX_EPOCH.elapsed().unwrap().as_secs();
        reset_time.saturating_sub(epoch_time)
    }

    fn retry(
        &self,
        req: Request,
        sleep: Duration,
        remaining_attempts: usize,
    ) -> BoxFuture<Result<Response, reqwest::Error>> {
        #[derive(Debug, serde::Deserialize)]
        struct RateLimit {
            #[allow(unused)]
            pub limit: u64,
            pub remaining: u64,
            pub reset: u64,
        }

        #[derive(Debug, serde::Deserialize)]
        struct RateLimitResponse {
            pub resources: Resources,
        }

        #[derive(Debug, serde::Deserialize)]
        struct Resources {
            pub core: RateLimit,
            pub search: RateLimit,
            #[allow(unused)]
            pub graphql: RateLimit,
            #[allow(unused)]
            pub source_import: RateLimit,
        }

        log::warn!(
            "Retrying after {} seconds, remaining attepts {}",
            sleep.as_secs(),
            remaining_attempts,
        );

        async move {
            tokio::time::sleep(sleep).await;

            // check rate limit
            let rate_resp = self
                .client
                .execute(
                    self.client
                        .get("https://api.github.com/rate_limit")
                        .configure(self)
                        .build()
                        .unwrap(),
                )
                .await?;
            let rate_limit_response = rate_resp.json::<RateLimitResponse>().await?;

            // Check url for search path because github has different rate limits for the search api
            let rate_limit = if req
                .url()
                .path_segments()
                .map(|mut segments| matches!(segments.next(), Some("search")))
                .unwrap_or(false)
            {
                rate_limit_response.resources.search
            } else {
                rate_limit_response.resources.core
            };

            // If we still don't have any more remaining attempts, try sleeping for the remaining
            // period of time
            if rate_limit.remaining == 0 {
                let sleep = Self::calc_sleep(rate_limit.reset);
                if sleep > 0 {
                    tokio::time::sleep(Duration::from_secs(sleep)).await;
                }
            }

            let resp = self.client.execute(req.try_clone().unwrap()).await?;
            if let Some(sleep) = Self::needs_retry(&resp).await {
                if remaining_attempts > 0 {
                    return self.retry(req, sleep, remaining_attempts - 1).await;
                }
            }

            Ok(resp)
        }
        .boxed()
    }

    async fn send_req(&self, req: RequestBuilder) -> anyhow::Result<Vec<u8>> {
        let (mut resp, req_dbg) = self._send_req(req).await?;

        let mut body = Vec::new();
        while let Some(chunk) = resp.chunk().await.transpose() {
            let chunk = chunk
                .context("reading stream failed")
                .map_err(anyhow::Error::from)
                .context(req_dbg.clone())?;
            body.extend_from_slice(&chunk);
        }

        Ok(body)
    }

    pub async fn json<T>(&self, req: RequestBuilder) -> anyhow::Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        let (resp, req_dbg) = self._send_req(req).await?;
        Ok(resp.json().await.context(req_dbg)?)
    }
}

impl User {
    pub async fn current(client: &GithubClient) -> anyhow::Result<Self> {
        client.json(client.get("https://api.github.com/user")).await
    }

    pub async fn is_team_member<'a>(&'a self, client: &'a GithubClient) -> anyhow::Result<bool> {
        log::trace!("Getting team membership for {:?}", self.login);
        let permission = crate::team_data::teams(client).await?;
        let map = permission.teams;
        let is_triager = map
            .get("wg-triage")
            .map_or(false, |w| w.members.iter().any(|g| g.github == self.login));
        let is_pri_member = map
            .get("wg-prioritization")
            .map_or(false, |w| w.members.iter().any(|g| g.github == self.login));
        let is_async_member = map
            .get("wg-async")
            .map_or(false, |w| w.members.iter().any(|g| g.github == self.login));
        let in_all = map["all"].members.iter().any(|g| g.github == self.login);
        log::trace!(
            "{:?} is all?={:?}, triager?={:?}, prioritizer?={:?}, async?={:?}",
            self.login,
            in_all,
            is_triager,
            is_pri_member,
            is_async_member,
        );
        Ok(in_all || is_triager || is_pri_member || is_async_member)
    }

    // Returns the ID of the given user, if the user is in the `all` team.
    pub async fn get_id<'a>(&'a self, client: &'a GithubClient) -> anyhow::Result<Option<usize>> {
        let permission = crate::team_data::teams(client).await?;
        let map = permission.teams;
        Ok(map["all"]
            .members
            .iter()
            .find(|g| g.github == self.login)
            .map(|u| u.github_id))
    }
}

pub async fn get_team(
    client: &GithubClient,
    team: &str,
) -> anyhow::Result<Option<rust_team_data::v1::Team>> {
    let permission = crate::team_data::teams(client).await?;
    let mut map = permission.teams;
    Ok(map.swap_remove(team))
}

#[derive(PartialEq, Eq, Debug, Clone, serde::Deserialize)]
pub struct Label {
    pub name: String,
}

/// An indicator used to differentiate between an issue and a pull request.
///
/// Some webhook events include a `pull_request` field in the Issue object,
/// and some don't. GitHub does include a few fields here, but they aren't
/// needed at this time (merged_at, diff_url, html_url, patch_url, url).
#[derive(Debug, serde::Deserialize)]
pub struct PullRequestDetails {
    // none for now
}

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
    created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
    /// The SHA for a merge commit.
    ///
    /// This field is complicated, see the [Pull Request
    /// docs](https://docs.github.com/en/rest/pulls/pulls#get-a-pull-request)
    /// for details.
    #[serde(default)]
    pub merge_commit_sha: Option<String>,
    pub title: String,
    /// The common URL for viewing this issue or PR.
    ///
    /// Example: `https://github.com/octocat/Hello-World/pull/1347`
    pub html_url: String,
    pub user: User,
    pub labels: Vec<Label>,
    pub assignees: Vec<User>,
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
    /// The API URL for discussion comments.
    ///
    /// Example: `https://api.github.com/repos/octocat/Hello-World/issues/1347/comments`
    comments_url: String,
    /// The repository for this issue.
    ///
    /// Note that this is constructed via the [`Issue::repository`] method.
    /// It is not deserialized from the GitHub API.
    #[serde(skip)]
    repository: OnceCell<IssueRepository>,

    /// The base commit for a PR (the branch of the destination repo).
    #[serde(default)]
    pub base: Option<CommitBase>,
    /// The head commit for a PR (the branch from the source repo).
    #[serde(default)]
    pub head: Option<CommitBase>,
    /// Whether it is open or closed.
    pub state: IssueState,
}

#[derive(Debug, serde::Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum IssueState {
    Open,
    Closed,
}

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

#[derive(Debug, serde::Deserialize)]
pub struct Comment {
    #[serde(deserialize_with = "opt_string")]
    pub body: String,
    pub html_url: String,
    pub user: User,
    #[serde(alias = "submitted_at")] // for pull request reviews
    pub updated_at: chrono::DateTime<Utc>,
    #[serde(default, rename = "state")]
    pub pr_review_state: Option<PullRequestReviewState>,
}

#[derive(Debug, serde::Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PullRequestReviewState {
    Approved,
    ChangesRequested,
    Commented,
    Dismissed,
    Pending,
}

fn opt_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::de::Deserializer<'de>,
{
    use serde::de::Deserialize;
    match <Option<String>>::deserialize(deserializer) {
        Ok(v) => Ok(v.unwrap_or_default()),
        Err(e) => Err(e),
    }
}

#[derive(Debug)]
pub enum AssignmentError {
    InvalidAssignee,
    Http(anyhow::Error),
}

#[derive(Debug)]
pub enum Selection<'a, T: ?Sized> {
    All,
    One(&'a T),
    Except(&'a T),
}

impl fmt::Display for AssignmentError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            AssignmentError::InvalidAssignee => write!(f, "invalid assignee"),
            AssignmentError::Http(e) => write!(f, "cannot assign: {}", e),
        }
    }
}

impl std::error::Error for AssignmentError {}

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
    pub fn url(&self) -> String {
        format!(
            "https://api.github.com/repos/{}/{}",
            self.organization, self.repository
        )
    }

    async fn has_label(&self, client: &GithubClient, label: &str) -> anyhow::Result<bool> {
        #[allow(clippy::redundant_pattern_matching)]
        let url = format!("{}/labels/{}", self.url(), label);
        match client._send_req(client.get(&url)).await {
            Ok((_, _)) => Ok(true),
            Err(e) => {
                if e.downcast_ref::<reqwest::Error>()
                    .map_or(false, |e| e.status() == Some(StatusCode::NOT_FOUND))
                {
                    Ok(false)
                } else {
                    Err(e)
                }
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct UnknownLabels {
    labels: Vec<String>,
}

// NOTE: This is used to post the Github comment; make sure it's valid markdown.
impl fmt::Display for UnknownLabels {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Unknown labels: {}", &self.labels.join(", "))
    }
}

impl std::error::Error for UnknownLabels {}

impl Issue {
    pub fn to_zulip_github_reference(&self) -> ZulipGitHubReference {
        ZulipGitHubReference {
            number: self.number,
            title: self.title.clone(),
            repository: self.repository().clone(),
        }
    }

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

    pub async fn get_comment(&self, client: &GithubClient, id: usize) -> anyhow::Result<Comment> {
        let comment_url = format!("{}/issues/comments/{}", self.repository().url(), id);
        let comment = client.json(client.get(&comment_url)).await?;
        Ok(comment)
    }

    pub async fn edit_body(&self, client: &GithubClient, body: &str) -> anyhow::Result<()> {
        let edit_url = format!("{}/issues/{}", self.repository().url(), self.number);
        #[derive(serde::Serialize)]
        struct ChangedIssue<'a> {
            body: &'a str,
        }
        client
            ._send_req(client.patch(&edit_url).json(&ChangedIssue { body }))
            .await
            .context("failed to edit issue body")?;
        Ok(())
    }

    pub async fn edit_comment(
        &self,
        client: &GithubClient,
        id: usize,
        new_body: &str,
    ) -> anyhow::Result<()> {
        let comment_url = format!("{}/issues/comments/{}", self.repository().url(), id);
        #[derive(serde::Serialize)]
        struct NewComment<'a> {
            body: &'a str,
        }
        client
            ._send_req(
                client
                    .patch(&comment_url)
                    .json(&NewComment { body: new_body }),
            )
            .await
            .context("failed to edit comment")?;
        Ok(())
    }

    pub async fn post_comment(&self, client: &GithubClient, body: &str) -> anyhow::Result<()> {
        #[derive(serde::Serialize)]
        struct PostComment<'a> {
            body: &'a str,
        }
        client
            ._send_req(client.post(&self.comments_url).json(&PostComment { body }))
            .await
            .context("failed to post comment")?;
        Ok(())
    }

    pub async fn remove_label(&self, client: &GithubClient, label: &str) -> anyhow::Result<()> {
        log::info!("remove_label from {}: {:?}", self.global_id(), label);
        // DELETE /repos/:owner/:repo/issues/:number/labels/{name}
        let url = format!(
            "{repo_url}/issues/{number}/labels/{name}",
            repo_url = self.repository().url(),
            number = self.number,
            name = label,
        );

        if !self.labels().iter().any(|l| l.name == label) {
            log::info!(
                "remove_label from {}: {:?} already not present, skipping",
                self.global_id(),
                label
            );
            return Ok(());
        }

        client
            ._send_req(client.delete(&url))
            .await
            .context("failed to delete label")?;

        Ok(())
    }

    pub async fn add_labels(
        &self,
        client: &GithubClient,
        labels: Vec<Label>,
    ) -> anyhow::Result<()> {
        log::info!("add_labels: {} +{:?}", self.global_id(), labels);
        // POST /repos/:owner/:repo/issues/:number/labels
        // repo_url = https://api.github.com/repos/Codertocat/Hello-World
        let url = format!(
            "{repo_url}/issues/{number}/labels",
            repo_url = self.repository().url(),
            number = self.number
        );

        // Don't try to add labels already present on this issue.
        let labels = labels
            .into_iter()
            .filter(|l| !self.labels().contains(&l))
            .map(|l| l.name)
            .collect::<Vec<_>>();

        log::info!("add_labels: {} filtered to {:?}", self.global_id(), labels);

        if labels.is_empty() {
            return Ok(());
        }

        let mut unknown_labels = vec![];
        let mut known_labels = vec![];
        for label in labels {
            if !self.repository().has_label(client, &label).await? {
                unknown_labels.push(label);
            } else {
                known_labels.push(label);
            }
        }

        if !unknown_labels.is_empty() {
            return Err(UnknownLabels {
                labels: unknown_labels,
            }
            .into());
        }

        #[derive(serde::Serialize)]
        struct LabelsReq {
            labels: Vec<String>,
        }

        client
            ._send_req(client.post(&url).json(&LabelsReq {
                labels: known_labels,
            }))
            .await
            .context("failed to add labels")?;

        Ok(())
    }

    pub fn labels(&self) -> &[Label] {
        &self.labels
    }

    pub fn contain_assignee(&self, user: &str) -> bool {
        self.assignees
            .iter()
            .any(|a| a.login.to_lowercase() == user.to_lowercase())
    }

    pub async fn remove_assignees(
        &self,
        client: &GithubClient,
        selection: Selection<'_, str>,
    ) -> Result<(), AssignmentError> {
        log::info!("remove {:?} assignees for {}", selection, self.global_id());
        let url = format!(
            "{repo_url}/issues/{number}/assignees",
            repo_url = self.repository().url(),
            number = self.number
        );

        let assignees = match selection {
            Selection::All => self
                .assignees
                .iter()
                .map(|u| u.login.as_str())
                .collect::<Vec<_>>(),
            Selection::One(user) => vec![user],
            Selection::Except(user) => self
                .assignees
                .iter()
                .map(|u| u.login.as_str())
                .filter(|&u| u.to_lowercase() != user.to_lowercase())
                .collect::<Vec<_>>(),
        };

        #[derive(serde::Serialize)]
        struct AssigneeReq<'a> {
            assignees: &'a [&'a str],
        }
        client
            ._send_req(client.delete(&url).json(&AssigneeReq {
                assignees: &assignees[..],
            }))
            .await
            .map_err(AssignmentError::Http)?;
        Ok(())
    }

    pub async fn add_assignee(
        &self,
        client: &GithubClient,
        user: &str,
    ) -> Result<(), AssignmentError> {
        log::info!("add_assignee {} for {}", user, self.global_id());
        let url = format!(
            "{repo_url}/issues/{number}/assignees",
            repo_url = self.repository().url(),
            number = self.number
        );

        #[derive(serde::Serialize)]
        struct AssigneeReq<'a> {
            assignees: &'a [&'a str],
        }

        let result: Issue = client
            .json(client.post(&url).json(&AssigneeReq { assignees: &[user] }))
            .await
            .map_err(AssignmentError::Http)?;
        // Invalid assignees are silently ignored. We can just check if the user is now
        // contained in the assignees list.
        let success = result
            .assignees
            .iter()
            .any(|u| u.login.as_str().to_lowercase() == user.to_lowercase());

        if success {
            Ok(())
        } else {
            Err(AssignmentError::InvalidAssignee)
        }
    }

    pub async fn set_assignee(
        &self,
        client: &GithubClient,
        user: &str,
    ) -> Result<(), AssignmentError> {
        log::info!("set_assignee for {} to {}", self.global_id(), user);
        self.add_assignee(client, user).await?;
        self.remove_assignees(client, Selection::Except(user))
            .await?;
        Ok(())
    }

    pub async fn set_milestone(&self, client: &GithubClient, title: &str) -> anyhow::Result<()> {
        log::trace!(
            "Setting milestone for rust-lang/rust#{} to {}",
            self.number,
            title
        );

        let create_url = format!("{}/milestones", self.repository().url());
        let resp = client
            .send_req(
                client
                    .post(&create_url)
                    .body(serde_json::to_vec(&MilestoneCreateBody { title }).unwrap()),
            )
            .await;
        // Explicitly do *not* try to return Err(...) if this fails -- that's
        // fine, it just means the milestone was already created.
        log::trace!("Created milestone: {:?}", resp);

        let list_url = format!("{}/milestones", self.repository().url());
        let milestone_list: Vec<Milestone> = client.json(client.get(&list_url)).await?;
        let milestone_no = if let Some(milestone) = milestone_list.iter().find(|v| v.title == title)
        {
            milestone.number
        } else {
            anyhow::bail!(
                "Despite just creating milestone {} on {}, it does not exist?",
                title,
                self.repository()
            )
        };

        #[derive(serde::Serialize)]
        struct SetMilestone {
            milestone: u64,
        }
        let url = format!("{}/issues/{}", self.repository().url(), self.number);
        client
            ._send_req(client.patch(&url).json(&SetMilestone {
                milestone: milestone_no,
            }))
            .await
            .context("failed to set milestone")?;
        Ok(())
    }

    pub async fn close(&self, client: &GithubClient) -> anyhow::Result<()> {
        let edit_url = format!("{}/issues/{}", self.repository().url(), self.number);
        #[derive(serde::Serialize)]
        struct CloseIssue<'a> {
            state: &'a str,
        }
        client
            ._send_req(
                client
                    .patch(&edit_url)
                    .json(&CloseIssue { state: "closed" }),
            )
            .await
            .context("failed to close issue")?;
        Ok(())
    }

    pub async fn merge(&self, client: &GithubClient) -> anyhow::Result<()> {
        let merge_url = format!("{}/pulls/{}/merge", self.repository().url(), self.number);

        // change defaults by reading from somewhere, maybe in .toml?
        #[derive(serde::Serialize)]
        struct MergeIssue<'a> {
            commit_title: &'a str,
            merge_method: &'a str,
        }

        client
            ._send_req(client.put(&merge_url).json(&MergeIssue {
                commit_title: "Merged by the bot!",
                merge_method: "merge",
            }))
            .await
            .context("failed to merge issue")?;

        Ok(())
    }

    /// Returns the diff in this event, for Open and Synchronize events for now.
    pub async fn diff(&self, client: &GithubClient) -> anyhow::Result<Option<String>> {
        let (before, after) = if let (Some(base), Some(head)) = (&self.base, &self.head) {
            (base.sha.clone(), head.sha.clone())
        } else {
            return Ok(None);
        };

        let mut req = client.get(&format!(
            "{}/compare/{}...{}",
            self.repository().url(),
            before,
            after
        ));
        req = req.header("Accept", "application/vnd.github.v3.diff");
        let diff = client.send_req(req).await?;
        Ok(Some(String::from(String::from_utf8_lossy(&diff))))
    }

    /// Returns the commits from this pull request (no commits are returned if this `Issue` is not
    /// a pull request).
    pub async fn commits(&self, client: &GithubClient) -> anyhow::Result<Vec<GithubCommit>> {
        if !self.is_pr() {
            return Ok(vec![]);
        }

        let mut commits = Vec::new();
        let mut page = 1;
        loop {
            let req = client.get(&format!(
                "{}/pulls/{}/commits?page={page}&per_page=100",
                self.repository().url(),
                self.number
            ));

            let new: Vec<_> = client.json(req).await?;
            if new.is_empty() {
                break;
            }
            commits.extend(new);

            page += 1;
        }
        Ok(commits)
    }

    pub async fn files(&self, client: &GithubClient) -> anyhow::Result<Vec<PullRequestFile>> {
        if !self.is_pr() {
            return Ok(vec![]);
        }

        let req = client.get(&format!(
            "{}/pulls/{}/files",
            self.repository().url(),
            self.number
        ));
        Ok(client.json(req).await?)
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct PullRequestFile {
    pub sha: String,
    pub filename: String,
    pub blob_url: String,
}

#[derive(serde::Serialize)]
struct MilestoneCreateBody<'a> {
    title: &'a str,
}

#[derive(Debug, serde::Deserialize)]
pub struct Milestone {
    number: u64,
    title: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct ChangeInner {
    pub from: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct Changes {
    pub title: Option<ChangeInner>,
    pub body: Option<ChangeInner>,
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PullRequestReviewAction {
    Submitted,
    Edited,
    Dismissed,
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

#[derive(Debug, serde::Deserialize)]
pub struct PullRequestReviewComment {
    pub action: IssueCommentAction,
    pub changes: Option<Changes>,
    #[serde(rename = "pull_request")]
    pub issue: Issue,
    pub comment: Comment,
    pub repository: Repository,
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

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssuesAction {
    Opened,
    Edited,
    Deleted,
    Transferred,
    Pinned,
    Unpinned,
    Closed,
    Reopened,
    Assigned,
    Unassigned,
    Labeled,
    Unlabeled,
    Locked,
    Unlocked,
    Milestoned,
    Demilestoned,
    ReviewRequested,
    ReviewRequestRemoved,
    ReadyForReview,
    Synchronize,
    ConvertedToDraft,
    AutoMergeEnabled,
    AutoMergeDisabled,
}

#[derive(Debug, serde::Deserialize)]
pub struct IssuesEvent {
    pub action: IssuesAction,
    #[serde(alias = "pull_request")]
    pub issue: Issue,
    pub changes: Option<Changes>,
    pub repository: Repository,
    /// Some if action is IssuesAction::Labeled, for example
    pub label: Option<Label>,
}

#[derive(Debug, serde::Deserialize)]
struct PullRequestEventFields {}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct CommitBase {
    sha: String,
    #[serde(rename = "ref")]
    pub git_ref: String,
    pub repo: Repository,
}

pub fn files_changed(diff: &str) -> Vec<&str> {
    let mut files = Vec::new();
    for line in diff.lines() {
        // mostly copied from highfive
        if line.starts_with("diff --git ") {
            files.push(
                line[line.find(" b/").unwrap()..]
                    .strip_prefix(" b/")
                    .unwrap(),
            );
        }
    }
    files
}

#[derive(Debug, serde::Deserialize)]
pub struct IssueSearchResult {
    pub total_count: usize,
    pub incomplete_results: bool,
    pub items: Vec<Issue>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct Repository {
    pub full_name: String,
    pub default_branch: String,
    #[serde(default)]
    pub fork: bool,
}

#[derive(Copy, Clone)]
struct Ordering<'a> {
    pub sort: &'a str,
    pub direction: &'a str,
    pub per_page: &'a str,
    pub page: usize,
}

impl Repository {
    const GITHUB_API_URL: &'static str = "https://api.github.com";
    const GITHUB_GRAPHQL_API_URL: &'static str = "https://api.github.com/graphql";

    fn url(&self) -> String {
        format!("{}/repos/{}", Repository::GITHUB_API_URL, self.full_name)
    }

    pub fn owner(&self) -> &str {
        self.full_name.split_once('/').unwrap().0
    }

    pub fn name(&self) -> &str {
        self.full_name.split_once('/').unwrap().1
    }

    pub async fn get_issues<'a>(
        &self,
        client: &GithubClient,
        query: &Query<'a>,
    ) -> anyhow::Result<Vec<Issue>> {
        let Query {
            filters,
            include_labels,
            exclude_labels,
        } = query;

        let mut ordering = Ordering {
            sort: "created",
            direction: "asc",
            per_page: "100",
            page: 1,
        };
        let filters: Vec<_> = filters
            .clone()
            .into_iter()
            .filter(|(key, val)| {
                match *key {
                    "sort" => ordering.sort = val,
                    "direction" => ordering.direction = val,
                    "per_page" => ordering.per_page = val,
                    _ => return true,
                };
                false
            })
            .collect();

        // `is: pull-request` indicates the query to retrieve PRs only
        let is_pr = filters
            .iter()
            .any(|&(key, value)| key == "is" && value == "pull-request");

        // There are some cases that can only be handled by the search API:
        // 1. When using negating label filters (exclude_labels)
        // 2. When there's a key parameter key=no
        // 3. When the query is to retrieve PRs only and there are label filters
        //
        // Check https://docs.github.com/en/rest/reference/search#search-issues-and-pull-requests
        // for more information
        let use_search_api = !exclude_labels.is_empty()
            || filters.iter().any(|&(key, _)| key == "no")
            || is_pr && !include_labels.is_empty();

        // If there are more than `per_page` of issues, we need to paginate
        let mut issues = vec![];
        loop {
            let url = if use_search_api {
                self.build_search_issues_url(&filters, include_labels, exclude_labels, ordering)
            } else if is_pr {
                self.build_pulls_url(&filters, include_labels, ordering)
            } else {
                self.build_issues_url(&filters, include_labels, ordering)
            };

            let result = client.get(&url);
            if use_search_api {
                let result = client
                    .json::<IssueSearchResult>(result)
                    .await
                    .with_context(|| format!("failed to list issues from {}", url))?;
                issues.extend(result.items);
                if issues.len() < result.total_count {
                    ordering.page += 1;
                    continue;
                }
            } else {
                // FIXME: paginate with non-search
                issues = client
                    .json(result)
                    .await
                    .with_context(|| format!("failed to list issues from {}", url))?
            }

            break;
        }
        Ok(issues)
    }

    fn build_issues_url(
        &self,
        filters: &Vec<(&str, &str)>,
        include_labels: &Vec<&str>,
        ordering: Ordering<'_>,
    ) -> String {
        self.build_endpoint_url("issues", filters, include_labels, ordering)
    }

    fn build_pulls_url(
        &self,
        filters: &Vec<(&str, &str)>,
        include_labels: &Vec<&str>,
        ordering: Ordering<'_>,
    ) -> String {
        self.build_endpoint_url("pulls", filters, include_labels, ordering)
    }

    fn build_endpoint_url(
        &self,
        endpoint: &str,
        filters: &Vec<(&str, &str)>,
        include_labels: &Vec<&str>,
        ordering: Ordering<'_>,
    ) -> String {
        let filters = filters
            .iter()
            .map(|(key, val)| format!("{}={}", key, val))
            .chain(std::iter::once(format!(
                "labels={}",
                include_labels.join(",")
            )))
            .chain(std::iter::once("filter=all".to_owned()))
            .chain(std::iter::once(format!("sort={}", ordering.sort,)))
            .chain(std::iter::once(
                format!("direction={}", ordering.direction,),
            ))
            .chain(std::iter::once(format!("per_page={}", ordering.per_page,)))
            .collect::<Vec<_>>()
            .join("&");
        format!(
            "{}/repos/{}/{}?{}",
            Repository::GITHUB_API_URL,
            self.full_name,
            endpoint,
            filters
        )
    }

    fn build_search_issues_url(
        &self,
        filters: &Vec<(&str, &str)>,
        include_labels: &Vec<&str>,
        exclude_labels: &Vec<&str>,
        ordering: Ordering<'_>,
    ) -> String {
        let filters = filters
            .iter()
            .filter(|&&(key, val)| !(key == "state" && val == "all"))
            .map(|(key, val)| format!("{}:{}", key, val))
            .chain(
                include_labels
                    .iter()
                    .map(|label| format!("label:{}", label)),
            )
            .chain(
                exclude_labels
                    .iter()
                    .map(|label| format!("-label:{}", label)),
            )
            .chain(std::iter::once(format!("repo:{}", self.full_name)))
            .collect::<Vec<_>>()
            .join("+");
        format!(
            "{}/search/issues?q={}&sort={}&order={}&per_page={}&page={}",
            Repository::GITHUB_API_URL,
            filters,
            ordering.sort,
            ordering.direction,
            ordering.per_page,
            ordering.page,
        )
    }

    /// Retrieves a git commit for the given SHA.
    pub async fn git_commit(&self, client: &GithubClient, sha: &str) -> anyhow::Result<GitCommit> {
        let url = format!("{}/git/commits/{sha}", self.url());
        client
            .json(client.get(&url))
            .await
            .with_context(|| format!("{} failed to get git commit {sha}", self.full_name))
    }

    /// Creates a new commit.
    pub async fn create_commit(
        &self,
        client: &GithubClient,
        message: &str,
        parents: &[&str],
        tree: &str,
    ) -> anyhow::Result<GitCommit> {
        let url = format!("{}/git/commits", self.url());
        client
            .json(client.post(&url).json(&serde_json::json!({
                "message": message,
                "parents": parents,
                "tree": tree,
            })))
            .await
            .with_context(|| format!("{} failed to create commit for tree {tree}", self.full_name))
    }

    /// Retrieves a git reference for the given refname.
    pub async fn get_reference(
        &self,
        client: &GithubClient,
        refname: &str,
    ) -> anyhow::Result<GitReference> {
        let url = format!("{}/git/ref/{}", self.url(), refname);
        client
            .json(client.get(&url))
            .await
            .with_context(|| format!("{} failed to get git reference {refname}", self.full_name))
    }

    /// Updates an existing git reference to a new SHA.
    pub async fn update_reference(
        &self,
        client: &GithubClient,
        refname: &str,
        sha: &str,
    ) -> anyhow::Result<()> {
        let url = format!("{}/git/refs/{}", self.url(), refname);
        client
            ._send_req(client.patch(&url).json(&serde_json::json!({
                "sha": sha,
                "force": true,
            })))
            .await
            .with_context(|| {
                format!(
                    "{} failed to update reference {refname} to {sha}",
                    self.full_name
                )
            })?;
        Ok(())
    }

    /// Returns a list of recent commits on the given branch.
    ///
    /// Returns results in the OID range `oldest` (exclusive) to `newest`
    /// (inclusive).
    pub async fn recent_commits(
        &self,
        client: &GithubClient,
        branch: &str,
        oldest: &str,
        newest: &str,
    ) -> anyhow::Result<Vec<RecentCommit>> {
        // This is used to deduplicate the results (so that a PR with multiple
        // commits will only show up once).
        let mut prs_seen = HashSet::new();
        let mut recent_commits = Vec::new(); // This is the final result.
        use cynic::QueryBuilder;
        use github_graphql::docs_update_queries::{
            GitObject, RecentCommits, RecentCommitsArguments,
        };

        let mut args = RecentCommitsArguments {
            branch: branch.to_string(),
            name: self.name().to_string(),
            owner: self.owner().to_string(),
            after: None,
        };
        let mut found_newest = false;
        let mut found_oldest = false;
        // This simulates --first-parent. We only care about top-level commits.
        // Unfortunately the GitHub API doesn't provide anything like that.
        let mut next_first_parent = None;
        // Search for `oldest` within 3 pages (300 commits).
        for _ in 0..3 {
            let query = RecentCommits::build(args.clone());
            let data = client
                .json::<cynic::GraphQlResponse<RecentCommits>>(
                    client.post(Repository::GITHUB_GRAPHQL_API_URL).json(&query),
                )
                .await
                .with_context(|| {
                    format!(
                        "{} failed to get recent commits branch={branch}",
                        self.full_name
                    )
                })?;

            if let Some(errors) = data.errors {
                anyhow::bail!("There were graphql errors. {:?}", errors);
            }
            let target = data
                .data
                .ok_or_else(|| anyhow::anyhow!("No data returned."))?
                .repository
                .ok_or_else(|| anyhow::anyhow!("No repository."))?
                .ref_
                .ok_or_else(|| anyhow::anyhow!("No ref."))?
                .target
                .ok_or_else(|| anyhow::anyhow!("No target."))?;
            let commit = match target {
                GitObject::Commit(commit) => commit,
                _ => anyhow::bail!("unexpected target type {target:?}"),
            };
            let commits = commit
                .history
                .nodes
                .into_iter()
                // Don't include anything newer than `newest`
                .skip_while(|node| {
                    if found_newest || node.oid.0 == newest {
                        found_newest = true;
                        false
                    } else {
                        // This should only happen if there is a commit that arrives
                        // between the time that `update_submodules` fetches the latest
                        // ref, and this runs. This window should be a few seconds, so it
                        // should be unlikely. This warning is here in case my assumptions
                        // about how things work is not correct.
                        tracing::warn!(
                            "unexpected race with submodule history, newest oid={newest} skipping oid={}",
                            node.oid.0
                        );
                        true
                    }
                })
                // Skip nodes that aren't the first parent
                .filter(|node| {
                    let this_first_parent = node.parents.nodes
                        .first()
                        .map(|parent| parent.oid.0.clone());

                    match &next_first_parent {
                        Some(first_parent) => {
                            if first_parent == &node.oid.0 {
                                // Found the next first parent, include it and
                                // set next_first_parent to look for this
                                // commit's first parent.
                                next_first_parent = this_first_parent;
                                true
                            } else {
                                // Still looking for the next first parent.
                                false
                            }
                        }
                        None => {
                            // First commit.
                            next_first_parent = this_first_parent;
                            true
                        }
                    }
                })
                // Stop once reached the `oldest` commit
                .take_while(|node| {
                    if node.oid.0 == oldest {
                        found_oldest = true;
                        false
                    } else {
                        true
                    }
                })
                .filter_map(|node| {
                    // Determine if this is associated with a PR or not.
                    match node.associated_pull_requests
                        // Get the first PR (we only care about one)
                        .and_then(|mut pr| pr.nodes.pop()) {
                        Some(pr) => {
                            // Only include a PR once
                            if prs_seen.insert(pr.number) {
                                Some(RecentCommit {
                                    pr_num: Some(pr.number),
                                    title: pr.title,
                                    oid: node.oid.0.clone(),
                                    committed_date: node.committed_date,
                                })
                            } else {
                                None
                            }
                        }
                        None => {
                            // This is an unassociated commit, possibly
                            // created without a PR.
                            Some(RecentCommit {
                                pr_num: None,
                                title: node.message_headline,
                                oid: node.oid.0,
                                committed_date: node.committed_date,
                            })
                        }
                    }
                });
            recent_commits.extend(commits);
            let page_info = commit.history.page_info;
            if found_oldest || !page_info.has_next_page || page_info.end_cursor.is_none() {
                break;
            }
            args.after = page_info.end_cursor;
        }
        if !found_oldest {
            // This should probably do something more than log a warning, but
            // I don't think it is too important at this time (the log message
            // is only informational, and this should be unlikely to happen).
            tracing::warn!(
                "{} failed to find oldest commit sha={oldest} branch={branch}",
                self.full_name
            );
        }
        Ok(recent_commits)
    }

    /// Creates a new git tree based on another tree.
    pub async fn update_tree(
        &self,
        client: &GithubClient,
        base_tree: &str,
        tree: &[GitTreeEntry],
    ) -> anyhow::Result<GitTreeObject> {
        let url = format!("{}/git/trees", self.url());
        client
            .json(client.post(&url).json(&serde_json::json!({
                "base_tree": base_tree,
                "tree": tree,
            })))
            .await
            .with_context(|| {
                format!(
                    "{} failed to update tree with base {base_tree}",
                    self.full_name
                )
            })
    }

    /// Returns information about the git submodule at the given path.
    ///
    /// `refname` is the ref to use for fetching information. If `None`, will
    /// use the latest version on the default branch.
    pub async fn submodule(
        &self,
        client: &GithubClient,
        path: &str,
        refname: Option<&str>,
    ) -> anyhow::Result<Submodule> {
        let mut url = format!("{}/contents/{}", self.url(), path);
        if let Some(refname) = refname {
            url.push_str("?ref=");
            url.push_str(refname);
        }
        client.json(client.get(&url)).await.with_context(|| {
            format!(
                "{} failed to get submodule path={path} refname={refname:?}",
                self.full_name
            )
        })
    }

    /// Creates a new PR.
    pub async fn new_pr(
        &self,
        client: &GithubClient,
        title: &str,
        head: &str,
        base: &str,
        body: &str,
    ) -> anyhow::Result<Issue> {
        let url = format!("{}/pulls", self.url());
        let mut issue: Issue = client
            .json(client.post(&url).json(&serde_json::json!({
                "title": title,
                "head": head,
                "base": base,
                "body": body,
            })))
            .await
            .with_context(|| {
                format!(
                    "{} failed to create a new PR head={head} base={base} title={title}",
                    self.full_name
                )
            })?;
        issue.pull_request = Some(PullRequestDetails {});
        Ok(issue)
    }

    /// Synchronize a branch (in a forked repository) by pulling in its upstream contents.
    pub async fn merge_upstream(&self, client: &GithubClient, branch: &str) -> anyhow::Result<()> {
        let url = format!("{}/merge-upstream", self.url());
        client
            ._send_req(client.post(&url).json(&serde_json::json!({
                "branch": branch,
            })))
            .await
            .with_context(|| {
                format!(
                    "{} failed to merge upstream branch {branch}",
                    self.full_name
                )
            })?;
        Ok(())
    }
}

pub struct Query<'a> {
    // key/value filter
    pub filters: Vec<(&'a str, &'a str)>,
    pub include_labels: Vec<&'a str>,
    pub exclude_labels: Vec<&'a str>,
}

fn quote_reply(markdown: &str) -> String {
    if markdown.is_empty() {
        String::from("*No content*")
    } else {
        format!("\n\t> {}", markdown.replace("\n", "\n\t> "))
    }
}

#[async_trait]
impl<'q> IssuesQuery for Query<'q> {
    async fn query<'a>(
        &'a self,
        repo: &'a Repository,
        include_fcp_details: bool,
        client: &'a GithubClient,
    ) -> anyhow::Result<Vec<crate::actions::IssueDecorator>> {
        let issues = repo
            .get_issues(&client, self)
            .await
            .with_context(|| "Unable to get issues.")?;

        let fcp_map = if include_fcp_details {
            crate::rfcbot::get_all_fcps().await?
        } else {
            HashMap::new()
        };

        let mut issues_decorator = Vec::new();
        for issue in issues {
            let fcp_details = if include_fcp_details {
                let repository_name = if let Some(repo) = issue.repository.get() {
                    repo.repository.clone()
                } else {
                    let re = regex::Regex::new("https://github.com/rust-lang/|/").unwrap();
                    let split = re.split(&issue.html_url).collect::<Vec<&str>>();
                    split[1].to_string()
                };
                let key = format!(
                    "rust-lang/{}:{}:{}",
                    repository_name, issue.number, issue.title,
                );

                if let Some(fcp) = fcp_map.get(&key) {
                    let bot_tracking_comment_html_url = format!(
                        "{}#issuecomment-{}",
                        issue.html_url, fcp.fcp.fk_bot_tracking_comment
                    );
                    let bot_tracking_comment_content = quote_reply(&fcp.status_comment.body);

                    let fk_initiating_comment = fcp.fcp.fk_initiating_comment;
                    let init_comment = issue
                        .get_comment(&client, fk_initiating_comment.try_into()?)
                        .await?;

                    Some(crate::actions::FCPDetails {
                        bot_tracking_comment_html_url,
                        bot_tracking_comment_content,
                        initiating_comment_html_url: init_comment.html_url.clone(),
                        initiating_comment_content: quote_reply(&init_comment.body),
                    })
                } else {
                    None
                }
            } else {
                None
            };
            issues_decorator.push(crate::actions::IssueDecorator {
                title: issue.title.clone(),
                number: issue.number,
                html_url: issue.html_url.clone(),
                repo_name: repo.name().to_owned(),
                labels: issue
                    .labels
                    .iter()
                    .map(|l| l.name.as_ref())
                    .collect::<Vec<_>>()
                    .join(", "),
                assignees: issue
                    .assignees
                    .iter()
                    .map(|u| u.login.as_ref())
                    .collect::<Vec<_>>()
                    .join(", "),
                updated_at_hts: crate::actions::to_human(issue.updated_at),
                fcp_details,
            });
        }

        Ok(issues_decorator)
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CreateKind {
    Branch,
    Tag,
}

#[derive(Debug, serde::Deserialize)]
pub struct CreateEvent {
    pub ref_type: CreateKind,
    repository: Repository,
    sender: User,
}

#[derive(Debug, serde::Deserialize)]
pub struct PushEvent {
    #[serde(rename = "ref")]
    pub git_ref: String,
    repository: Repository,
    sender: User,
}

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

    /// This will both extract from IssueComment events but also Issue events
    pub fn comment_body(&self) -> Option<&str> {
        match self {
            Event::Create(_) => None,
            Event::Issue(e) => Some(&e.issue.body),
            Event::IssueComment(e) => Some(&e.comment.body),
            Event::Push(_) => None,
        }
    }

    /// This will both extract from IssueComment events but also Issue events
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
            Event::Create(e) => &e.sender,
            Event::Issue(e) => &e.issue.user,
            Event::IssueComment(e) => &e.comment.user,
            Event::Push(e) => &e.sender,
        }
    }

    pub fn time(&self) -> Option<chrono::DateTime<FixedOffset>> {
        match self {
            Event::Create(_) => None,
            Event::Issue(e) => Some(e.issue.created_at.into()),
            Event::IssueComment(e) => Some(e.comment.updated_at.into()),
            Event::Push(_) => None,
        }
    }
}

trait RequestSend: Sized {
    fn configure(self, g: &GithubClient) -> Self;
}

impl RequestSend for RequestBuilder {
    fn configure(self, g: &GithubClient) -> RequestBuilder {
        let mut auth = HeaderValue::from_maybe_shared(format!("token {}", g.token)).unwrap();
        auth.set_sensitive(true);
        self.header(USER_AGENT, "rust-lang-triagebot")
            .header(AUTHORIZATION, &auth)
    }
}

/// Finds the token in the user's environment, panicking if no suitable token
/// can be found.
pub fn default_token_from_env() -> String {
    match std::env::var("GITHUB_API_TOKEN") {
        Ok(v) => return v,
        Err(_) => (),
    }

    match get_token_from_git_config() {
        Ok(v) => return v,
        Err(_) => (),
    }

    panic!("could not find token in GITHUB_API_TOKEN or .gitconfig/github.oath-token")
}

fn get_token_from_git_config() -> anyhow::Result<String> {
    let output = std::process::Command::new("git")
        .arg("config")
        .arg("--get")
        .arg("github.oauth-token")
        .output()?;
    if !output.status.success() {
        anyhow::bail!("error received executing `git`: {:?}", output.status);
    }
    let git_token = String::from_utf8(output.stdout)?.trim().to_string();
    Ok(git_token)
}

#[derive(Clone)]
pub struct GithubClient {
    token: String,
    client: Client,
}

impl GithubClient {
    pub fn new(client: Client, token: String) -> Self {
        GithubClient { client, token }
    }

    pub fn new_with_default_token(client: Client) -> Self {
        Self::new(client, default_token_from_env())
    }

    pub fn raw(&self) -> &Client {
        &self.client
    }

    pub async fn raw_file(
        &self,
        repo: &str,
        branch: &str,
        path: &str,
    ) -> anyhow::Result<Option<Vec<u8>>> {
        let url = format!(
            "https://raw.githubusercontent.com/{}/{}/{}",
            repo, branch, path
        );
        let req = self.get(&url);
        let req_dbg = format!("{:?}", req);
        let req = req
            .build()
            .with_context(|| format!("failed to build request {:?}", req_dbg))?;
        let mut resp = self.client.execute(req).await.context(req_dbg.clone())?;
        let status = resp.status();
        match status {
            StatusCode::OK => {
                let mut buf = Vec::with_capacity(resp.content_length().unwrap_or(4) as usize);
                while let Some(chunk) = resp.chunk().await.transpose() {
                    let chunk = chunk
                        .context("reading stream failed")
                        .map_err(anyhow::Error::from)
                        .context(req_dbg.clone())?;
                    buf.extend_from_slice(&chunk);
                }
                Ok(Some(buf))
            }
            StatusCode::NOT_FOUND => Ok(None),
            status => anyhow::bail!("failed to GET {}: {}", url, status),
        }
    }

    /// Get the raw gist content from the URL of the HTML version of the gist:
    ///
    /// `html_url` looks like `https://gist.github.com/rust-play/7e80ca3b1ec7abe08f60c41aff91f060`.
    ///
    /// `filename` is the name of the file you want the content of.
    pub async fn raw_gist_from_url(
        &self,
        html_url: &str,
        filename: &str,
    ) -> anyhow::Result<String> {
        let url = html_url.replace("github.com", "githubusercontent.com") + "/raw/" + filename;
        let response = self.raw().get(&url).send().await?;
        response.text().await.context("raw gist from url")
    }

    pub fn get(&self, url: &str) -> RequestBuilder {
        log::trace!("get {:?}", url);
        self.client.get(url).configure(self)
    }

    fn patch(&self, url: &str) -> RequestBuilder {
        log::trace!("patch {:?}", url);
        self.client.patch(url).configure(self)
    }

    fn delete(&self, url: &str) -> RequestBuilder {
        log::trace!("delete {:?}", url);
        self.client.delete(url).configure(self)
    }

    fn post(&self, url: &str) -> RequestBuilder {
        log::trace!("post {:?}", url);
        self.client.post(url).configure(self)
    }

    #[allow(unused)]
    fn put(&self, url: &str) -> RequestBuilder {
        log::trace!("put {:?}", url);
        self.client.put(url).configure(self)
    }

    pub async fn rust_commit(&self, sha: &str) -> Option<GithubCommit> {
        let req = self.get(&format!(
            "https://api.github.com/repos/rust-lang/rust/commits/{}",
            sha
        ));
        match self.json(req).await {
            Ok(r) => Some(r),
            Err(e) => {
                log::error!("Failed to query commit {:?}: {:?}", sha, e);
                None
            }
        }
    }

    /// This does not retrieve all of them, only the last several.
    pub async fn bors_commits(&self) -> Vec<GithubCommit> {
        let req = self.get("https://api.github.com/repos/rust-lang/rust/commits?author=bors");
        match self.json(req).await {
            Ok(r) => r,
            Err(e) => {
                log::error!("Failed to query commit list: {:?}", e);
                Vec::new()
            }
        }
    }

    /// Returns whether or not the given GitHub login has made any commits to
    /// the given repo.
    pub async fn is_new_contributor(&self, repo: &Repository, author: &str) -> bool {
        let url = format!(
            "{}/repos/{}/commits?author={}",
            Repository::GITHUB_API_URL,
            repo.full_name,
            author,
        );
        let req = self.get(&url);
        match self.json::<Vec<GithubCommit>>(req).await {
            // Note: This only returns results for the default branch.
            // That should be fine in most cases since I think it is rare for
            // new users to make their first commit to a different branch.
            Ok(res) => res.is_empty(),
            Err(e) => {
                log::warn!(
                    "failed to search for user commits in {} for author {author}: {e}",
                    repo.full_name
                );
                false
            }
        }
    }

    /// Returns information about a repository.
    ///
    /// The `full_name` should be something like `rust-lang/rust`.
    pub async fn repository(&self, full_name: &str) -> anyhow::Result<Repository> {
        let req = self.get(&format!("{}/repos/{full_name}", Repository::GITHUB_API_URL));
        self.json(req)
            .await
            .with_context(|| format!("{} failed to get repo", full_name))
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct GithubCommit {
    pub sha: String,
    pub commit: GithubCommitCommitField,
    pub parents: Vec<Parent>,
}

#[derive(Debug, serde::Deserialize)]
pub struct GithubCommitCommitField {
    pub author: GitUser,
    pub message: String,
    pub tree: GitCommitTree,
}

#[derive(Debug, serde::Deserialize)]
pub struct GitCommit {
    pub sha: String,
    pub author: GitUser,
    pub message: String,
    pub tree: GitCommitTree,
}

#[derive(Debug, serde::Deserialize)]
pub struct GitCommitTree {
    pub sha: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct GitTreeObject {
    pub sha: String,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct GitTreeEntry {
    pub path: String,
    pub mode: String,
    #[serde(rename = "type")]
    pub object_type: String,
    pub sha: String,
}

pub struct RecentCommit {
    pub title: String,
    pub pr_num: Option<i32>,
    pub oid: String,
    pub committed_date: DateTime<Utc>,
}

#[derive(Debug, serde::Deserialize)]
pub struct GitUser {
    pub date: DateTime<FixedOffset>,
}

#[derive(Debug, serde::Deserialize)]
pub struct Parent {
    pub sha: String,
}

#[async_trait]
pub trait IssuesQuery {
    async fn query<'a>(
        &'a self,
        repo: &'a Repository,
        include_fcp_details: bool,
        client: &'a GithubClient,
    ) -> anyhow::Result<Vec<crate::actions::IssueDecorator>>;
}

pub struct LeastRecentlyReviewedPullRequests;
#[async_trait]
impl IssuesQuery for LeastRecentlyReviewedPullRequests {
    async fn query<'a>(
        &'a self,
        repo: &'a Repository,
        _include_fcp_details: bool,
        client: &'a GithubClient,
    ) -> anyhow::Result<Vec<crate::actions::IssueDecorator>> {
        use cynic::QueryBuilder;
        use github_graphql::queries;

        let repository_owner = repo.owner().to_owned();
        let repository_name = repo.name().to_owned();

        let mut prs: Vec<queries::PullRequest> = vec![];

        let mut args = queries::LeastRecentlyReviewedPullRequestsArguments {
            repository_owner,
            repository_name: repository_name.clone(),
            after: None,
        };
        loop {
            let query = queries::LeastRecentlyReviewedPullRequests::build(args.clone());
            let req = client.post(Repository::GITHUB_GRAPHQL_API_URL);
            let req = req.json(&query);

            let (resp, req_dbg) = client._send_req(req).await?;
            let data = resp
                .json::<cynic::GraphQlResponse<queries::LeastRecentlyReviewedPullRequests>>()
                .await
                .context(req_dbg)?;
            if let Some(errors) = data.errors {
                anyhow::bail!("There were graphql errors. {:?}", errors);
            }
            let repository = data
                .data
                .ok_or_else(|| anyhow::anyhow!("No data returned."))?
                .repository
                .ok_or_else(|| anyhow::anyhow!("No repository."))?;
            prs.extend(repository.pull_requests.nodes);
            let page_info = repository.pull_requests.page_info;
            if !page_info.has_next_page || page_info.end_cursor.is_none() {
                break;
            }
            args.after = page_info.end_cursor;
        }

        let mut prs: Vec<_> = prs
            .into_iter()
            .filter_map(|pr| {
                if pr.is_draft {
                    return None;
                }
                let labels = pr
                    .labels
                    .map(|l| l.nodes)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|node| node.name)
                    .collect::<Vec<_>>();
                if !labels.iter().any(|label| label == "T-compiler") {
                    return None;
                }
                let labels = labels.join(", ");

                let assignees: Vec<_> = pr
                    .assignees
                    .nodes
                    .into_iter()
                    .map(|user| user.login)
                    .collect();

                let mut reviews = pr
                    .latest_reviews
                    .map(|connection| connection.nodes)
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|node| {
                        let created_at = node.created_at;
                        node.author.map(|author| (author, created_at))
                    })
                    .map(|(author, created_at)| (author.login, created_at))
                    .collect::<Vec<_>>();

                reviews.sort_by_key(|r| r.1);

                let mut comments = pr
                    .comments
                    .nodes
                    .into_iter()
                    .filter_map(|node| {
                        let created_at = node.created_at;
                        node.author.map(|author| (author, created_at))
                    })
                    .map(|(author, created_at)| (author.login, created_at))
                    .filter(|comment| assignees.contains(&comment.0))
                    .collect::<Vec<_>>();

                comments.sort_by_key(|c| c.1);

                let updated_at = std::cmp::max(
                    reviews.last().map(|t| t.1).unwrap_or(pr.created_at),
                    comments.last().map(|t| t.1).unwrap_or(pr.created_at),
                );
                let assignees = assignees.join(", ");

                Some((
                    updated_at,
                    pr.number as u64,
                    pr.title,
                    pr.url.0,
                    repository_name.clone(),
                    labels,
                    assignees,
                ))
            })
            .collect();
        prs.sort_by_key(|pr| pr.0);

        let prs: Vec<_> = prs
            .into_iter()
            .take(50)
            .map(
                |(updated_at, number, title, html_url, repo_name, labels, assignees)| {
                    let updated_at_hts = crate::actions::to_human(updated_at);

                    crate::actions::IssueDecorator {
                        number,
                        title,
                        html_url,
                        repo_name,
                        labels,
                        assignees,
                        updated_at_hts,
                        fcp_details: None,
                    }
                },
            )
            .collect();

        Ok(prs)
    }
}

async fn project_items_by_status(
    client: &GithubClient,
    status_filter: impl Fn(Option<&str>) -> bool,
) -> anyhow::Result<Vec<github_graphql::project_items_by_status::ProjectV2ItemContent>> {
    use cynic::QueryBuilder;
    use github_graphql::project_items_by_status;

    const DESIGN_MEETING_PROJECT: i32 = 31;
    let mut args = project_items_by_status::Arguments {
        project_number: DESIGN_MEETING_PROJECT,
        after: None,
    };

    let mut all_items = vec![];
    loop {
        let query = project_items_by_status::Query::build(args.clone());
        let req = client.post(Repository::GITHUB_GRAPHQL_API_URL);
        let req = req.json(&query);

        let (resp, req_dbg) = client._send_req(req).await?;
        let data = resp
            .json::<cynic::GraphQlResponse<project_items_by_status::Query>>()
            .await
            .context(req_dbg)?;
        if let Some(errors) = data.errors {
            anyhow::bail!("There were graphql errors. {:?}", errors);
        }
        let items = data
            .data
            .ok_or_else(|| anyhow!("No data returned."))?
            .organization
            .ok_or_else(|| anyhow!("Organization not found."))?
            .project_v2
            .ok_or_else(|| anyhow!("Project not found."))?
            .items;
        let filtered = items
            .nodes
            .ok_or_else(|| anyhow!("Malformed response."))?
            .into_iter()
            .flatten()
            .filter(|item| {
                status_filter(
                    item.field_value_by_name
                        .as_ref()
                        .and_then(|status| status.as_str()),
                )
            })
            .flat_map(|item| item.content);
        all_items.extend(filtered);

        let page_info = items.page_info;
        if !page_info.has_next_page || page_info.end_cursor.is_none() {
            break;
        }
        args.after = page_info.end_cursor;
    }

    Ok(all_items)
}

pub struct ProposedDesignMeetings;
#[async_trait]
impl IssuesQuery for ProposedDesignMeetings {
    async fn query<'a>(
        &'a self,
        _repo: &'a Repository,
        _include_fcp_details: bool,
        client: &'a GithubClient,
    ) -> anyhow::Result<Vec<crate::actions::IssueDecorator>> {
        use github_graphql::project_items_by_status::ProjectV2ItemContent;

        let items =
            project_items_by_status(client, |status| status == Some("Needs triage")).await?;
        Ok(items
            .into_iter()
            .flat_map(|item| match item {
                ProjectV2ItemContent::Issue(issue) => Some(crate::actions::IssueDecorator {
                    assignees: String::new(),
                    number: issue.number.try_into().unwrap(),
                    fcp_details: None,
                    html_url: issue.url.0,
                    title: issue.title,
                    repo_name: String::new(),
                    labels: String::new(),
                    updated_at_hts: String::new(),
                }),
                _ => None,
            })
            .collect())
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct GitReference {
    #[serde(rename = "ref")]
    pub refname: String,
    pub object: GitObject,
}

#[derive(Debug, serde::Deserialize)]
pub struct GitObject {
    #[serde(rename = "type")]
    pub object_type: String,
    pub sha: String,
    pub url: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct Submodule {
    pub name: String,
    pub path: String,
    pub sha: String,
    pub submodule_git_url: String,
}

impl Submodule {
    /// Returns the `Repository` this submodule points to.
    ///
    /// This assumes that the submodule is on GitHub.
    pub async fn repository(&self, client: &GithubClient) -> anyhow::Result<Repository> {
        let fullname = self
            .submodule_git_url
            .strip_prefix("https://github.com/")
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "only github submodules supported, got {}",
                    self.submodule_git_url
                )
            })?
            .strip_suffix(".git")
            .ok_or_else(|| {
                anyhow::anyhow!("expected .git suffix, got {}", self.submodule_git_url)
            })?;
        client.repository(fullname).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_labels() {
        let x = UnknownLabels {
            labels: vec!["A-bootstrap".into(), "xxx".into()],
        };
        assert_eq!(x.to_string(), "Unknown labels: A-bootstrap, xxx");
    }

    #[test]
    fn extract_one_file() {
        let input = r##"\
diff --git a/triagebot.toml b/triagebot.toml
index fb9cee43b2d..b484c25ea51 100644
--- a/triagebot.toml
+++ b/triagebot.toml
@@ -114,6 +114,15 @@ trigger_files = [
        "src/tools/rustdoc-themes",
    ]
+[autolabel."T-compiler"]
+trigger_files = [
+    # Source code
+    "compiler",
+
+    # Tests
+    "src/test/ui",
+]
+
    [notify-zulip."I-prioritize"]
    zulip_stream = 245100 # #t-compiler/wg-prioritization/alerts
    topic = "#{number} {title}"
         "##;
        assert_eq!(files_changed(input), vec!["triagebot.toml".to_string()]);
    }

    #[test]
    fn extract_several_files() {
        let input = r##"\
diff --git a/library/stdarch b/library/stdarch
index b70ae88ef2a..cfba59fccd9 160000
--- a/library/stdarch
+++ b/library/stdarch
@@ -1 +1 @@
-Subproject commit b70ae88ef2a6c83acad0a1e83d5bd78f9655fd05
+Subproject commit cfba59fccd90b3b52a614120834320f764ab08d1
diff --git a/src/librustdoc/clean/types.rs b/src/librustdoc/clean/types.rs
index 1fe4aa9023e..f0330f1e424 100644
--- a/src/librustdoc/clean/types.rs
+++ b/src/librustdoc/clean/types.rs
@@ -2322,3 +2322,4 @@ impl SubstParam {
        if let Self::Lifetime(lt) = self { Some(lt) } else { None }
    }
}
+
diff --git a/src/librustdoc/core.rs b/src/librustdoc/core.rs
index c58310947d2..3b0854d4a9b 100644
--- a/src/librustdoc/core.rs
+++ b/src/librustdoc/core.rs
@@ -591,3 +591,4 @@ fn from(idx: u32) -> Self {
        ImplTraitParam::ParamIndex(idx)
    }
}
+
"##;
        assert_eq!(
            files_changed(input),
            vec![
                "library/stdarch".to_string(),
                "src/librustdoc/clean/types.rs".to_string(),
                "src/librustdoc/core.rs".to_string(),
            ]
        )
    }
}
