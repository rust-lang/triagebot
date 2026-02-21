use crate::github::GithubClient;
use crate::github::{
    GithubCommit, GithubCompare, Issue, IssueRepository, PullRequestDetails, Repository,
};
use crate::team_data::TeamClient;

use super::UserId;

use anyhow::Context;
use bytes::Bytes;
use itertools::Itertools;
use octocrab::models::Author;
use reqwest::StatusCode;
use tracing as log;

pub(crate) mod issue_with_comments;
pub(crate) mod user_comments_in_org;

// User

#[derive(Debug, PartialEq, Eq, Hash, serde::Deserialize, Clone)]
pub struct User {
    pub login: String,
    pub id: UserId,
}

impl From<&Author> for User {
    fn from(author: &Author) -> Self {
        Self {
            id: author.id.0,
            login: author.login.clone(),
        }
    }
}

impl User {
    pub async fn current(client: &GithubClient) -> anyhow::Result<Self> {
        client
            .json(client.get(&format!("{}/user", client.api_url)))
            .await
    }

    pub async fn is_team_member<'a>(&'a self, client: &'a TeamClient) -> anyhow::Result<bool> {
        log::trace!("Getting team membership for {:?}", self.login);
        let permission = client.teams().await?;
        let map = permission.teams;
        let is_triager = map
            .get("wg-triage")
            .is_some_and(|w| w.members.iter().any(|g| g.github == self.login));
        let is_async_member = map
            .get("wg-async")
            .is_some_and(|w| w.members.iter().any(|g| g.github == self.login));
        let in_all = map["all"].members.iter().any(|g| g.github == self.login);
        log::trace!(
            "{:?} is all?={:?}, triager?={:?}, async?={:?}",
            self.login,
            in_all,
            is_triager,
            is_async_member,
        );
        Ok(in_all || is_triager || is_async_member)
    }
}

// New issue

#[derive(Debug, serde::Deserialize)]
pub struct NewIssueResponse {
    pub number: u64,
}

impl GithubClient {
    pub(crate) async fn new_issue(
        &self,
        repo: &IssueRepository,
        title: &str,
        body: &str,
        labels: Vec<String>,
    ) -> anyhow::Result<NewIssueResponse> {
        #[derive(serde::Serialize)]
        struct NewIssue<'a> {
            title: &'a str,
            body: &'a str,
            labels: Vec<String>,
        }
        let url = format!("{}/issues", repo.url(self));
        self.json(self.post(&url).json(&NewIssue {
            title,
            body,
            labels,
        }))
        .await
        .context("failed to create issue")
    }
}

// Set pull-request state

#[derive(Debug, serde::Serialize)]
pub(crate) enum PrState {
    #[serde(rename = "open")]
    Open,
    #[serde(rename = "closed")]
    Closed,
}

impl GithubClient {
    pub(crate) async fn set_pr_state(
        &self,
        repo: &IssueRepository,
        number: u64,
        state: PrState,
    ) -> anyhow::Result<()> {
        #[derive(serde::Serialize)]
        struct Update {
            state: PrState,
        }
        let url = format!("{}/pulls/{number}", repo.url(self));
        self.send_req(self.patch(&url).json(&Update { state }))
            .await
            .context("failed to update pr state")?;
        Ok(())
    }
}

// Workflow

#[derive(Debug, serde::Deserialize)]
pub struct WorkflowRunJob {
    pub name: String,
    pub head_sha: String,
    pub conclusion: Option<JobConclusion>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobConclusion {
    ActionRequired,
    Cancelled,
    Failure,
    Neutral,
    Skipped,
    Success,
    TimedOut,
}

impl GithubClient {
    pub async fn workflow_run_job(
        &self,
        repo: &IssueRepository,
        job_id: u128,
    ) -> anyhow::Result<WorkflowRunJob> {
        let url = format!("{}/actions/jobs/{job_id}", repo.url(self));
        self.json(self.get(&url))
            .await
            .context("failed to retrive workflow job run details")
    }
}

// Git Trees

#[derive(Debug, serde::Deserialize)]
pub struct GitTrees {
    pub sha: String,
    pub tree: Vec<GitTreeEntry>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct GitTreeEntry {
    pub path: String,
    pub mode: String,
    #[serde(rename = "type")]
    pub object_type: String,
    pub sha: String,
}

impl GithubClient {
    pub async fn repo_git_trees(
        &self,
        repo: &IssueRepository,
        sha: &str,
    ) -> anyhow::Result<GitTrees> {
        let url = format!("{}/git/trees/{sha}", repo.url(self));
        self.json(self.get(&url))
            .await
            .context("failed to retrive git trees")
    }
}

// Others

impl GithubClient {
    pub async fn raw_job_logs(
        &self,
        repo: &IssueRepository,
        job_id: u128,
    ) -> anyhow::Result<String> {
        let url = format!("{}/actions/jobs/{job_id}/logs", repo.url(self));
        let (body, _req_dbg) = self
            .send_req(self.get(&url))
            .await
            .context("failed to retrieve job logs")?;
        Ok(String::from_utf8_lossy(&body).to_string())
    }

    pub async fn compare(
        &self,
        repo: &IssueRepository,
        before: &str,
        after: &str,
    ) -> anyhow::Result<GithubCompare> {
        let url = format!("{}/compare/{before}...{after}", repo.url(self));
        self.json(self.get(&url))
            .await
            .context("failed to retrive the compare")
    }

    pub async fn pull_request(&self, repo: &IssueRepository, pr_num: u64) -> anyhow::Result<Issue> {
        let url = format!("{}/pulls/{pr_num}", repo.url(self));
        let mut pr: Issue = self
            .json(self.get(&url))
            .await
            .with_context(|| format!("{repo} failed to get pr {pr_num}"))?;
        pr.pull_request = Some(PullRequestDetails::new());
        Ok(pr)
    }

    pub async fn raw_file(
        &self,
        repo: &str,
        branch: &str,
        path: &str,
    ) -> anyhow::Result<Option<Bytes>> {
        let url = format!("{}/{repo}/{branch}/{path}", self.raw_url);
        let req = self.get(&url);
        let req_dbg = format!("{req:?}");
        let req = req
            .build()
            .with_context(|| format!("failed to build request {req_dbg:?}"))?;
        let resp = self.raw().execute(req).await.context(req_dbg.clone())?;
        let status = resp.status();
        let body = resp
            .bytes()
            .await
            .with_context(|| format!("failed to read response body {req_dbg}"))?;
        match status {
            StatusCode::OK => Ok(Some(body)),
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

    pub async fn rust_commit(&self, sha: &str) -> Option<GithubCommit> {
        let req = self.get(&format!(
            "{}/repos/rust-lang/rust/commits/{sha}",
            self.api_url
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
        let req = self.get(&format!(
            "{}/repos/rust-lang/rust/commits?author=bors",
            self.api_url
        ));
        match self.json(req).await {
            Ok(r) => r,
            Err(e) => {
                log::error!("Failed to query commit list: {:?}", e);
                Vec::new()
            }
        }
    }

    /// Returns the object ID of the given user.
    ///
    /// Returns `None` if the user doesn't exist.
    pub async fn user_object_id(&self, user: &str) -> anyhow::Result<Option<String>> {
        let user_info: serde_json::Value = self
            .graphql_query_with_errors(
                "query($user:String!) {
                    user(login:$user) {
                        id
                    }
                }",
                serde_json::json!({
                    "user": user,
                }),
            )
            .await?;
        if let Some(id) = user_info["data"]["user"]["id"].as_str() {
            return Ok(Some(id.to_string()));
        }
        if let Some(errors) = user_info["errors"].as_array() {
            if errors
                .iter()
                .any(|err| err["type"].as_str().unwrap_or_default() == "NOT_FOUND")
            {
                return Ok(None);
            }
            let messages = errors
                .iter()
                .map(|err| err["message"].as_str().unwrap_or_default())
                .format("\n");
            anyhow::bail!("failed to query user: {messages}");
        }
        anyhow::bail!("query for user {user} failed, no error message? {user_info:?}");
    }

    /// Returns whether or not the given GitHub login has made any commits to
    /// the given repo.
    pub async fn is_new_contributor(&self, repo: &Repository, author: &str) -> bool {
        let user_id = match self.user_object_id(author).await {
            Ok(None) => return true,
            Ok(Some(id)) => id,
            Err(e) => {
                log::warn!("failed to query user: {e:?}");
                return true;
            }
        };
        // Note: This only returns results for the default branch. That should
        // be fine in most cases since I think it is rare for new users to
        // make their first commit to a different branch.
        //
        // Note: This is using GraphQL because the
        // `/repos/ORG/REPO/commits?author=AUTHOR` API was having problems not
        // finding users (https://github.com/rust-lang/triagebot/issues/1689).
        // The other possibility is the `/search/commits?q=repo:{}+author:{}`
        // API, but that endpoint has a very limited rate limit, and doesn't
        // work on forks. This GraphQL query seems to work fairly reliably,
        // and seems to cost only 1 point.
        match self
            .graphql_query_with_errors(
                "query($repository_owner:String!, $repository_name:String!, $user_id:ID!) {
                        repository(owner: $repository_owner, name: $repository_name) {
                            defaultBranchRef {
                                target {
                                    ... on Commit {
                                        history(author: {id: $user_id}) {
                                            totalCount
                                        }
                                    }
                                }
                            }
                        }
                    }",
                serde_json::json!({
                        "repository_owner": repo.owner(),
                        "repository_name": repo.name(),
                        "user_id": user_id
                }),
            )
            .await
        {
            Ok(c) => {
                if let Some(c) =
                    c["data"]["repository"]["defaultBranchRef"]["target"]["history"]["totalCount"]
                        .as_i64()
                {
                    return c == 0;
                }
                log::warn!("new user query failed: {c:?}");
                false
            }
            Err(e) => {
                log::warn!(
                    "failed to search for user commits in {} for author {author}: {e:?}",
                    repo.full_name
                );
                // Using `false` since if there is some underlying problem, we
                // don't need to spam everyone with the "new user" welcome
                // message.
                false
            }
        }
    }

    /// Returns information about a repository.
    ///
    /// The `full_name` should be something like `rust-lang/rust`.
    pub async fn repository(&self, full_name: &str) -> anyhow::Result<Repository> {
        let req = self.get(&format!("{}/repos/{full_name}", self.api_url));
        self.json(req)
            .await
            .with_context(|| format!("{full_name} failed to get repo"))
    }

    /// Returns the GraphQL ID of the given repository.
    pub async fn graphql_repo_id(&self, owner: &str, repo: &str) -> anyhow::Result<String> {
        let mut repo_id = self
            .graphql_query(
                "query($owner:String!, $repo:String!) {
                    repository(owner: $owner, name: $repo) {
                        id
                    }
                }",
                serde_json::json!({
                    "owner": owner,
                    "repo": repo,
                }),
            )
            .await?;
        let serde_json::Value::String(repo_id) = repo_id["data"]["repository"]["id"].take() else {
            anyhow::bail!("expected repo id, got {repo_id}");
        };
        Ok(repo_id)
    }

    /// Returns the number of issues or PRs that match the given query.
    ///
    /// See
    /// <https://docs.github.com/en/search-github/searching-on-github/searching-issues-and-pull-requests>
    /// and
    /// <https://docs.github.com/en/search-github/getting-started-with-searching-on-github/understanding-the-search-syntax>
    /// for the query syntax.
    pub async fn issue_search_count(&self, query: &str) -> anyhow::Result<u64> {
        let data = self
            .graphql_query(
                "query($query: String!) {
                  search(query: $query, type: ISSUE, first: 0) {
                    issueCount
                  }
                }",
                serde_json::json!({
                    "query": query,
                }),
            )
            .await?;
        if let serde_json::Value::Number(count) = &data["data"]["search"]["issueCount"]
            && let Some(count) = count.as_u64()
        {
            Ok(count)
        } else {
            anyhow::bail!("expected issue count, got {data}");
        }
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct Milestone {
    number: u64,
    title: String,
}

impl GithubClient {
    /// Get or create a [`Milestone`].
    ///
    /// This will not change the state if it already exists.
    pub(crate) async fn get_or_create_milestone(
        &self,
        full_repo_name: &str,
        title: &str,
        state: &str,
    ) -> anyhow::Result<Milestone> {
        let url = format!("{}/repos/{full_repo_name}/milestones", self.api_url);
        let resp = self
            .send_req(self.post(&url).json(&serde_json::json!({
                "title": title,
                "state": state,
            })))
            .await;
        match resp {
            Ok((body, _dbg)) => {
                let milestone = serde_json::from_slice(&body)?;
                log::trace!("Created milestone: {milestone:?}");
                return Ok(milestone);
            }
            Err(e) => {
                if e.downcast_ref::<reqwest::Error>()
                    .is_some_and(|e| e.status() == Some(StatusCode::UNPROCESSABLE_ENTITY))
                {
                    // fall-through, it already exists
                } else {
                    return Err(e.context(format!(
                        "failed to create milestone {url} with title {title}"
                    )));
                }
            }
        }
        // In the case where it already exists, we need to search for its number.
        let mut page = 1;
        loop {
            let url = format!(
                "{}/repos/{full_repo_name}/milestones?page={page}&state=all",
                self.api_url
            );
            let milestones: Vec<Milestone> = self
                .json(self.get(&url))
                .await
                .with_context(|| format!("failed to get milestones {url} searching for {title}"))?;
            if milestones.is_empty() {
                anyhow::bail!("expected to find milestone with title {title}");
            }
            if let Some(milestone) = milestones.into_iter().find(|m| m.title == title) {
                return Ok(milestone);
            }
            page += 1;
        }
    }

    /// Set the milestone of an issue or PR.
    pub(crate) async fn set_milestone(
        &self,
        full_repo_name: &str,
        milestone: &Milestone,
        issue_num: u64,
    ) -> anyhow::Result<()> {
        let url = format!("{}/repos/{full_repo_name}/issues/{issue_num}", self.api_url);
        self.send_req(self.patch(&url).json(&serde_json::json!({
            "milestone": milestone.number
        })))
        .await
        .with_context(|| format!("failed to set milestone for {url} to milestone {milestone:?}"))?;
        Ok(())
    }
}
