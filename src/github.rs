use anyhow::Context;

use chrono::{DateTime, FixedOffset, Utc};
use futures::stream::{FuturesUnordered, StreamExt};
use futures::{future::BoxFuture, FutureExt};
use once_cell::sync::OnceCell;
use reqwest::header::{AUTHORIZATION, USER_AGENT};
use reqwest::{Client, Request, RequestBuilder, Response, StatusCode};
use std::{
    fmt,
    time::{Duration, SystemTime},
};

#[derive(Debug, PartialEq, Eq, serde::Deserialize)]
pub struct User {
    pub login: String,
    pub id: Option<i64>,
}

impl GithubClient {
    async fn _send_req(&self, req: RequestBuilder) -> Result<(Response, String), reqwest::Error> {
        const MAX_ATTEMPTS: usize = 2;
        log::debug!("_send_req with {:?}", req);
        let req = req.build()?;
        let req_dbg = format!("{:?}", req);

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
            pub graphql: RateLimit,
            pub source_import: RateLimit,
        }

        async move {
            tokio::time::delay_for(sleep).await;

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
                    tokio::time::delay_for(Duration::from_secs(sleep)).await;
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
        let permission = crate::team_data::teams(client).await?;
        let map = permission.teams;
        let is_triager = map
            .get("wg-triage")
            .map_or(false, |w| w.members.iter().any(|g| g.github == self.login));
        let is_pri_member = map
            .get("wg-prioritization")
            .map_or(false, |w| w.members.iter().any(|g| g.github == self.login));
        Ok(
            map["all"].members.iter().any(|g| g.github == self.login)
                || is_triager
                || is_pri_member,
        )
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

impl Label {
    async fn exists<'a>(&'a self, repo_api_prefix: &'a str, client: &'a GithubClient) -> bool {
        #[allow(clippy::redundant_pattern_matching)]
        let url = format!("{}/labels/{}", repo_api_prefix, self.name);
        match client.send_req(client.get(&url)).await {
            Ok(_) => true,
            // XXX: Error handling if the request failed for reasons beyond 'label didn't exist'
            Err(_) => false,
        }
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct PullRequestDetails {
    // none for now
}

#[derive(Debug, serde::Deserialize)]
pub struct Issue {
    pub number: u64,
    pub body: String,
    created_at: chrono::DateTime<Utc>,
    pub title: String,
    pub html_url: String,
    pub user: User,
    pub labels: Vec<Label>,
    pub assignees: Vec<User>,
    pull_request: Option<PullRequestDetails>,
    // API URL
    comments_url: String,
    #[serde(skip)]
    repository: OnceCell<IssueRepository>,
}

#[derive(Debug, serde::Deserialize)]
pub struct Comment {
    #[serde(deserialize_with = "opt_string")]
    pub body: String,
    pub html_url: String,
    pub user: User,
    #[serde(alias = "submitted_at")] // for pull request reviews
    pub updated_at: chrono::DateTime<Utc>,
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
    Http(reqwest::Error),
}

#[derive(Debug)]
pub enum Selection<'a, T> {
    All,
    One(&'a T),
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

#[derive(Debug)]
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
    fn url(&self) -> String {
        format!(
            "https://api.github.com/repos/{}/{}",
            self.organization, self.repository
        )
    }
}

impl Issue {
    pub fn zulip_topic_reference(&self) -> String {
        let repo = self.repository();
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

    pub async fn set_labels(
        &self,
        client: &GithubClient,
        labels: Vec<Label>,
    ) -> anyhow::Result<()> {
        log::info!("set_labels {} to {:?}", self.global_id(), labels);
        // PUT /repos/:owner/:repo/issues/:number/labels
        // repo_url = https://api.github.com/repos/Codertocat/Hello-World
        let url = format!(
            "{repo_url}/issues/{number}/labels",
            repo_url = self.repository().url(),
            number = self.number
        );

        let mut stream = labels
            .into_iter()
            .map(|label| async { (label.exists(&self.repository().url(), &client).await, label) })
            .collect::<FuturesUnordered<_>>();
        let mut labels = Vec::new();
        while let Some((true, label)) = stream.next().await {
            labels.push(label);
        }

        #[derive(serde::Serialize)]
        struct LabelsReq {
            labels: Vec<String>,
        }
        client
            ._send_req(client.put(&url).json(&LabelsReq {
                labels: labels.iter().map(|l| l.name.clone()).collect(),
            }))
            .await
            .context("failed to set labels")?;

        Ok(())
    }

    pub fn labels(&self) -> &[Label] {
        &self.labels
    }

    pub fn contain_assignee(&self, user: &User) -> bool {
        self.assignees.contains(user)
    }

    pub async fn remove_assignees(
        &self,
        client: &GithubClient,
        selection: Selection<'_, User>,
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
            Selection::One(user) => vec![user.login.as_str()],
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

    pub async fn set_assignee(
        &self,
        client: &GithubClient,
        user: &str,
    ) -> Result<(), AssignmentError> {
        log::info!("set_assignee for {} to {}", self.global_id(), user);
        let url = format!(
            "{repo_url}/issues/{number}/assignees",
            repo_url = self.repository().url(),
            number = self.number
        );

        let check_url = format!(
            "{repo_url}/assignees/{name}",
            repo_url = self.repository().url(),
            name = user,
        );

        match client._send_req(client.get(&check_url)).await {
            Ok((resp, _)) => {
                if resp.status() == reqwest::StatusCode::NO_CONTENT {
                    // all okay
                    log::debug!("set_assignee: assignee is valid");
                } else {
                    log::error!(
                        "unknown status for assignee check, assuming all okay: {:?}",
                        resp
                    );
                }
            }
            Err(e) => {
                if e.status() == Some(reqwest::StatusCode::NOT_FOUND) {
                    log::debug!("set_assignee: assignee is invalid, returning");
                    return Err(AssignmentError::InvalidAssignee);
                }
                log::debug!("set_assignee: get {} failed, {:?}", check_url, e);
                return Err(AssignmentError::Http(e));
            }
        }

        self.remove_assignees(client, Selection::All).await?;

        #[derive(serde::Serialize)]
        struct AssigneeReq<'a> {
            assignees: &'a [&'a str],
        }

        client
            ._send_req(client.post(&url).json(&AssigneeReq { assignees: &[user] }))
            .await
            .map_err(AssignmentError::Http)?;

        Ok(())
    }
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PullRequestReviewAction {
    Submitted,
    Edited,
    Dismissed,
}

#[derive(Debug, serde::Deserialize)]
pub struct PullRequestReviewEvent {
    pub action: PullRequestReviewAction,
    pub pull_request: Issue,
    pub review: Comment,
    pub repository: Repository,
}

#[derive(Debug, serde::Deserialize)]
pub struct PullRequestReviewComment {
    pub action: IssueCommentAction,
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
    pub issue: Issue,
    pub comment: Comment,
    pub repository: Repository,
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
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
}

#[derive(Debug, serde::Deserialize)]
pub struct IssuesEvent {
    pub action: IssuesAction,
    #[serde(alias = "pull_request")]
    pub issue: Issue,
    pub repository: Repository,
    /// Some if action is IssuesAction::Labeled, for example
    pub label: Option<Label>,
}

#[derive(Debug, serde::Deserialize)]
pub struct IssueSearchResult {
    pub total_count: usize,
    pub incomplete_results: bool,
    pub items: Vec<Issue>,
}

#[derive(Debug, serde::Deserialize)]
pub struct Repository {
    pub full_name: String,
}

impl Repository {
    fn base_url(&self) -> &str {
        "https://api.github.com"
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
            ..
        } = query;

        let use_issues = exclude_labels.is_empty() || filters.iter().any(|&(key, _)| key == "no");
        // negating filters can only be handled by the search api
        let url = if use_issues {
            self.build_issues_url(filters, include_labels)
        } else {
            self.build_search_issues_url(filters, include_labels, exclude_labels)
        };

        let result = client.get(&url);
        if use_issues {
            client
                .json(result)
                .await
                .with_context(|| format!("failed to list issues from {}", url))
        } else {
            let result = client
                .json::<IssueSearchResult>(result)
                .await
                .with_context(|| format!("failed to list issues from {}", url))?;
            Ok(result.items)
        }
    }

    pub async fn get_issues_count<'a>(
        &self,
        client: &GithubClient,
        query: &Query<'a>,
    ) -> anyhow::Result<usize> {
        Ok(self.get_issues(client, query).await?.len())
    }

    fn build_issues_url(&self, filters: &Vec<(&str, &str)>, include_labels: &Vec<&str>) -> String {
        let filters = filters
            .iter()
            .map(|(key, val)| format!("{}={}", key, val))
            .chain(std::iter::once(format!(
                "labels={}",
                include_labels.join(",")
            )))
            .chain(std::iter::once("filter=all".to_owned()))
            .chain(std::iter::once(format!("sort=created")))
            .chain(std::iter::once(format!("direction=asc")))
            .collect::<Vec<_>>()
            .join("&");
        format!(
            "{}/repos/{}/issues?{}",
            self.base_url(),
            self.full_name,
            filters
        )
    }

    fn build_search_issues_url(
        &self,
        filters: &Vec<(&str, &str)>,
        include_labels: &Vec<&str>,
        exclude_labels: &Vec<&str>,
    ) -> String {
        let filters = filters
            .iter()
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
            "{}/search/issues?q={}&sort=created&order=asc",
            self.base_url(),
            filters
        )
    }
}

pub struct Query<'a> {
    pub kind: QueryKind,
    // key/value filter
    pub filters: Vec<(&'a str, &'a str)>,
    pub include_labels: Vec<&'a str>,
    pub exclude_labels: Vec<&'a str>,
}

pub enum QueryKind {
    List,
    Count,
}

#[derive(Debug)]
pub enum Event {
    IssueComment(IssueCommentEvent),
    Issue(IssuesEvent),
}

impl Event {
    pub fn repo_name(&self) -> &str {
        match self {
            Event::IssueComment(event) => &event.repository.full_name,
            Event::Issue(event) => &event.repository.full_name,
        }
    }

    pub fn issue(&self) -> Option<&Issue> {
        match self {
            Event::IssueComment(event) => Some(&event.issue),
            Event::Issue(event) => Some(&event.issue),
        }
    }

    /// This will both extract from IssueComment events but also Issue events
    pub fn comment_body(&self) -> Option<&str> {
        match self {
            Event::Issue(e) => Some(&e.issue.body),
            Event::IssueComment(e) => Some(&e.comment.body),
        }
    }

    pub fn html_url(&self) -> Option<&str> {
        match self {
            Event::Issue(e) => Some(&e.issue.html_url),
            Event::IssueComment(e) => Some(&e.comment.html_url),
        }
    }

    pub fn user(&self) -> &User {
        match self {
            Event::Issue(e) => &e.issue.user,
            Event::IssueComment(e) => &e.comment.user,
        }
    }

    pub fn time(&self) -> chrono::DateTime<FixedOffset> {
        match self {
            Event::Issue(e) => e.issue.created_at.into(),
            Event::IssueComment(e) => e.comment.updated_at.into(),
        }
    }
}

trait RequestSend: Sized {
    fn configure(self, g: &GithubClient) -> Self;
}

impl RequestSend for RequestBuilder {
    fn configure(self, g: &GithubClient) -> RequestBuilder {
        self.header(USER_AGENT, "rust-lang-triagebot")
            .header(AUTHORIZATION, format!("token {}", g.token))
    }
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

    fn get(&self, url: &str) -> RequestBuilder {
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
}

#[derive(Debug, serde::Deserialize)]
pub struct GithubCommit {
    pub sha: String,
    pub commit: GitCommit,
    pub parents: Vec<Parent>,
}

#[derive(Debug, serde::Deserialize)]
pub struct GitCommit {
    pub author: GitUser,
}

#[derive(Debug, serde::Deserialize)]
pub struct GitUser {
    pub date: DateTime<FixedOffset>,
}

#[derive(Debug, serde::Deserialize)]
pub struct Parent {
    pub sha: String,
}
