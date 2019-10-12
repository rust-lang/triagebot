use failure::{Error, ResultExt};

use futures::{
    compat::{Future01CompatExt, Stream01CompatExt},
    stream::{FuturesUnordered, StreamExt},
};
use once_cell::sync::OnceCell;
use reqwest::header::{AUTHORIZATION, USER_AGENT};
use reqwest::{
    r#async::{Client, RequestBuilder, Response},
    StatusCode,
};
use std::fmt;

#[derive(Debug, PartialEq, Eq, serde::Deserialize)]
pub struct User {
    pub login: String,
}

impl GithubClient {
    async fn _send_req(&self, req: RequestBuilder) -> Result<(Response, String), reqwest::Error> {
        log::debug!("_send_req with {:?}", req);
        let req = req.build()?;

        let req_dbg = format!("{:?}", req);

        let resp = self.client.execute(req).compat().await?;

        resp.error_for_status_ref()?;

        Ok((resp, req_dbg))
    }
    async fn send_req(&self, req: RequestBuilder) -> Result<Vec<u8>, Error> {
        let (resp, req_dbg) = self._send_req(req).await?;

        let mut body = Vec::new();
        let mut stream = resp.into_body().compat();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk
                .context("reading stream failed")
                .map_err(Error::from)
                .context(req_dbg.clone())?;
            body.extend_from_slice(&chunk);
        }

        Ok(body)
    }

    async fn json<T>(&self, req: RequestBuilder) -> Result<T, Error>
    where
        T: serde::de::DeserializeOwned,
    {
        let (mut resp, req_dbg) = self._send_req(req).await?;

        Ok(resp.json().compat().await.context(req_dbg)?)
    }
}

impl User {
    pub async fn current(client: &GithubClient) -> Result<Self, Error> {
        client.json(client.get("https://api.github.com/user")).await
    }

    pub async fn is_team_member<'a>(&'a self, client: &'a GithubClient) -> Result<bool, Error> {
        let url = format!("{}/teams.json", rust_team_data::v1::BASE_URL);
        let permission: rust_team_data::v1::Teams = client
            .json(client.raw().get(&url))
            .await
            .context("could not get team data")?;
        let map = permission.teams;
        let is_triager = map
            .get("wg-triage")
            .map_or(false, |w| w.members.iter().any(|g| g.github == self.login));
        Ok(map["all"].members.iter().any(|g| g.github == self.login) || is_triager)
    }
}

pub async fn get_team(
    client: &GithubClient,
    team: &str,
) -> Result<Option<rust_team_data::v1::Team>, Error> {
    let url = format!("{}/teams.json", rust_team_data::v1::BASE_URL);
    let permission: rust_team_data::v1::Teams = client
        .json(client.raw().get(&url))
        .await
        .context("could not get team data")?;
    let mut map = permission.teams;
    Ok(map.swap_remove(team))
}

#[derive(Debug, Clone, serde::Deserialize)]
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
    title: String,
    html_url: String,
    user: User,
    labels: Vec<Label>,
    assignees: Vec<User>,
    pull_request: Option<PullRequestDetails>,
    // API URL
    repository_url: String,
    comments_url: String,
    #[serde(skip)]
    repository: OnceCell<IssueRepository>,
}

#[derive(Debug, serde::Deserialize)]
pub struct Comment {
    pub body: String,
    pub html_url: String,
    pub user: User,
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

impl Issue {
    pub fn repository(&self) -> &IssueRepository {
        self.repository.get_or_init(|| {
            log::trace!("get repository for {}", self.repository_url);
            let url = url::Url::parse(&self.repository_url).unwrap();
            let mut segments = url.path_segments().unwrap();
            let repository = segments.nth_back(0).unwrap();
            let organization = segments.nth_back(1).unwrap();
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

    pub async fn get_comment(&self, client: &GithubClient, id: usize) -> Result<Comment, Error> {
        let comment_url = format!("{}/issues/comments/{}", self.repository_url, id);
        let comment = client.json(client.get(&comment_url)).await?;
        Ok(comment)
    }

    pub async fn edit_body(&self, client: &GithubClient, body: &str) -> Result<(), Error> {
        let edit_url = format!("{}/issues/{}", self.repository_url, self.number);
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
    ) -> Result<(), Error> {
        let comment_url = format!("{}/issues/comments/{}", self.repository_url, id);
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

    pub async fn post_comment(&self, client: &GithubClient, body: &str) -> Result<(), Error> {
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

    pub async fn set_labels(&self, client: &GithubClient, labels: Vec<Label>) -> Result<(), Error> {
        log::info!("set_labels {} to {:?}", self.global_id(), labels);
        // PUT /repos/:owner/:repo/issues/:number/labels
        // repo_url = https://api.github.com/repos/Codertocat/Hello-World
        let url = format!(
            "{repo_url}/issues/{number}/labels",
            repo_url = self.repository_url,
            number = self.number
        );

        let mut stream = labels
            .into_iter()
            .map(|label| async { (label.exists(&self.repository_url, &client).await, label) })
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
            repo_url = self.repository_url,
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
            repo_url = self.repository_url,
            number = self.number
        );

        let check_url = format!(
            "{repo_url}/assignees/{name}",
            repo_url = self.repository_url,
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
}

#[derive(Debug, serde::Deserialize)]
pub struct IssuesEvent {
    pub action: IssuesAction,
    pub issue: Issue,
    pub repository: Repository,
}

#[derive(Debug, serde::Deserialize)]
pub struct Repository {
    pub full_name: String,
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
    ) -> Result<Option<Vec<u8>>, Error> {
        let url = format!(
            "https://raw.githubusercontent.com/{}/{}/{}",
            repo, branch, path
        );
        let req = self.get(&url);
        let req_dbg = format!("{:?}", req);
        let req = req
            .build()
            .with_context(|_| format!("failed to build request {:?}", req_dbg))?;
        let resp = self
            .client
            .execute(req)
            .compat()
            .await
            .context(req_dbg.clone())?;
        let status = resp.status();
        match status {
            StatusCode::OK => {
                let mut buf = Vec::with_capacity(resp.content_length().unwrap_or(4) as usize);
                let mut stream = resp.into_body().compat();
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk
                        .context("reading stream failed")
                        .map_err(Error::from)
                        .context(req_dbg.clone())?;
                    buf.extend_from_slice(&chunk);
                }
                Ok(Some(buf))
            }
            StatusCode::NOT_FOUND => Ok(None),
            status => failure::bail!("failed to GET {}: {}", url, status),
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
}
