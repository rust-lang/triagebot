use failure::{Error, ResultExt};
use reqwest::header::{AUTHORIZATION, USER_AGENT};
use reqwest::{Client, Error as HttpError, RequestBuilder, Response, StatusCode};
use std::io::Read;

#[derive(Debug, serde::Deserialize)]
pub struct User {
    pub login: String,
}

impl User {
    pub fn current(client: &GithubClient) -> Result<Self, Error> {
        Ok(client
            .get("https://api.github.com/user")
            .send_req()?
            .json()?)
    }

    pub fn is_team_member(&self, client: &GithubClient) -> Result<bool, Error> {
        let client = client.raw();
        let url = format!("{}/teams.json", rust_team_data::v1::BASE_URL);
        let permission: rust_team_data::v1::Teams = client
            .get(&url)
            .send()
            .and_then(Response::error_for_status)
            .and_then(|mut r| r.json())
            .context("could not get team data")?;
        let map = permission.teams;
        Ok(map["all"].members.iter().any(|g| g.github == self.login))
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Label {
    pub name: String,
}

impl Label {
    fn exists(&self, repo_api_prefix: &str, client: &GithubClient) -> bool {
        #[allow(clippy::redundant_pattern_matching)]
        match client
            .get(&format!("{}/labels/{}", repo_api_prefix, self.name))
            .send_req()
        {
            Ok(_) => true,
            // XXX: Error handling if the request failed for reasons beyond 'label didn't exist'
            Err(_) => false,
        }
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct Issue {
    pub number: u64,
    title: String,
    user: User,
    labels: Vec<Label>,
    assignees: Vec<User>,
    // API URL
    repository_url: String,
    comments_url: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct Comment {
    pub body: String,
    pub html_url: String,
    pub user: User,
}

impl Issue {
    pub fn post_comment(&self, client: &GithubClient, body: &str) -> Result<(), Error> {
        #[derive(serde::Serialize)]
        struct PostComment<'a> {
            body: &'a str,
        }
        client
            .post(&self.comments_url)
            .json(&PostComment { body })
            .send_req()
            .context("failed to post comment")?;
        Ok(())
    }

    pub fn set_labels(&self, client: &GithubClient, mut labels: Vec<Label>) -> Result<(), Error> {
        // PUT /repos/:owner/:repo/issues/:number/labels
        // repo_url = https://api.github.com/repos/Codertocat/Hello-World
        let url = format!(
            "{repo_url}/issues/{number}/labels",
            repo_url = self.repository_url,
            number = self.number
        );

        labels.retain(|label| label.exists(&self.repository_url, &client));

        #[derive(serde::Serialize)]
        struct LabelsReq {
            labels: Vec<String>,
        }
        client
            .put(&url)
            .json(&LabelsReq {
                labels: labels.iter().map(|l| l.name.clone()).collect(),
            })
            .send_req()
            .context("failed to set labels")?;

        Ok(())
    }

    pub fn labels(&self) -> &[Label] {
        &self.labels
    }

    pub fn add_assignee(&self, client: &GithubClient, user: &str) -> Result<(), Error> {
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

        match client.get(&check_url).send() {
            Ok(resp) => {
                if resp.status() == reqwest::StatusCode::NO_CONTENT {
                    // all okay
                } else if resp.status() == reqwest::StatusCode::NOT_FOUND {
                    failure::bail!("invalid assignee {:?}", user);
                }
            }
            Err(e) => failure::bail!("unable to check assignee validity: {:?}", e),
        }

        #[derive(serde::Serialize)]
        struct AssigneeReq<'a> {
            assignees: &'a [&'a str],
        }
        client
            .post(&url)
            .json(&AssigneeReq { assignees: &[user] })
            .send_req()
            .context("failed to add assignee")?;

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

#[derive(Debug, serde::Deserialize)]
pub struct Repository {
    pub full_name: String,
}

#[derive(Debug)]
pub enum Event {
    IssueComment(IssueCommentEvent),
}

trait RequestSend: Sized {
    fn configure(self, g: &GithubClient) -> Self;
    fn send_req(self) -> Result<Response, HttpError>;
}

impl RequestSend for RequestBuilder {
    fn configure(self, g: &GithubClient) -> RequestBuilder {
        self.header(USER_AGENT, "rust-lang-triagebot")
            .header(AUTHORIZATION, format!("token {}", g.token))
    }

    fn send_req(self) -> Result<Response, HttpError> {
        match self.send() {
            Ok(r) => r.error_for_status(),
            Err(e) => Err(e),
        }
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

    pub fn raw_file(&self, repo: &str, branch: &str, path: &str) -> Result<Option<Vec<u8>>, Error> {
        let url = format!(
            "https://raw.githubusercontent.com/{}/{}/{}",
            repo, branch, path
        );
        let mut resp = self.get(&url).send()?;
        match resp.status() {
            StatusCode::OK => {
                let mut buf = Vec::with_capacity(resp.content_length().unwrap_or(4) as usize);
                resp.read_to_end(&mut buf)?;
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

    fn post(&self, url: &str) -> RequestBuilder {
        log::trace!("post {:?}", url);
        self.client.post(url).configure(self)
    }

    fn put(&self, url: &str) -> RequestBuilder {
        log::trace!("put {:?}", url);
        self.client.put(url).configure(self)
    }
}
