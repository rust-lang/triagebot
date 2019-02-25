use failure::{Error, ResultExt};
use reqwest::header::USER_AGENT;
use reqwest::Error as HttpError;
use reqwest::{Client, RequestBuilder, Response};

#[derive(Debug, serde::Deserialize)]
pub struct User {
    pub login: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Label {
    pub name: String,
}

impl Label {
    fn exists(&self, repo_api_prefix: &str, client: &GithubClient) -> bool {
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
            .json(&PostComment { body: body })
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

trait RequestSend: Sized {
    fn configure(self, g: &GithubClient) -> Self;
    fn send_req(self) -> Result<Response, HttpError>;
}

impl RequestSend for RequestBuilder {
    fn configure(self, g: &GithubClient) -> RequestBuilder {
        self.header(USER_AGENT, "rust-lang-triagebot")
            .basic_auth(&g.username, Some(&g.token))
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
    username: String,
    token: String,
    client: Client,
}

impl GithubClient {
    pub fn new(c: Client, token: String, username: String) -> Self {
        GithubClient {
            client: c,
            token,
            username,
        }
    }

    pub fn username(&self) -> &str {
        self.username.as_str()
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
