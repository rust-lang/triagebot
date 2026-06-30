use super::issue_query::Query;
use super::{GithubClient, Issue, IssueRepository, PullRequestDetails};
use anyhow::Context;
use chrono::{DateTime, FixedOffset};
use itertools::Itertools;
use octocrab::models::Author;

type UserId = u64;

// User

#[derive(Debug, PartialEq, Eq, Hash, serde::Deserialize, Clone)]
pub struct GitHubUser {
    pub login: String,
    #[serde(alias = "databaseId")]
    pub id: UserId,
    #[serde(alias = "__typename")]
    pub r#type: GitHubUserType,
}

impl From<&Author> for GitHubUser {
    fn from(author: &Author) -> Self {
        Self {
            id: author.id.0,
            login: author.login.clone(),
            r#type: match author.r#type.as_str() {
                "User" => GitHubUserType::User,
                "Bot" => GitHubUserType::Bot,
                _ => GitHubUserType::Custom(author.r#type.clone()),
            },
        }
    }
}

#[derive(Debug, PartialEq, Eq, Hash, serde::Deserialize, Clone)]
#[serde(rename_all = "PascalCase")]
pub enum GitHubUserType {
    User,
    Bot,
    #[serde(untagged)]
    Custom(String),
}

// https://docs.github.com/en/rest/repos/contents?apiVersion=2022-11-28#get-repository-content
#[derive(serde::Deserialize, serde::Serialize)]
pub struct RepoContent {
    pub name: String,
    pub download_url: String,
}

// Others

impl GithubClient {
    pub async fn issue(&self, repo: &IssueRepository, issue_num: u64) -> anyhow::Result<Issue> {
        let url = format!("{}/issues/{issue_num}", repo.url(self));
        self.json(self.get(&url))
            .await
            .with_context(|| format!("{repo} failed to get issue {issue_num}"))
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

    /// Returns information about a repository.
    ///
    /// The `full_name` should be something like `rust-lang/rust`.
    async fn repository(&self, full_name: &str) -> anyhow::Result<Repository> {
        let req = self.get(&format!("{}/repos/{full_name}", self.api_url));
        self.json(req)
            .await
            .with_context(|| format!("{full_name} failed to get repo"))
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
#[cfg_attr(test, derive(Default))]
pub struct Repository {
    pub full_name: String,
    pub default_branch: String,
    #[serde(default)]
    pub fork: bool,
    pub parent: Option<Box<Repository>>,
}

impl Repository {
    fn url(&self, client: &GithubClient) -> String {
        format!("{}/repos/{}", client.api_url, self.full_name)
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
        #[derive(Debug, serde::Deserialize)]
        struct IssueSearchResult {
            total_count: u64,
            items: Vec<Issue>,
        }

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
                }
                false
            })
            .collect();

        // `is: pull-request` indicates the query to retrieve PRs only
        let is_pr = filters.contains(&("is", "pull-request"));

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
                self.build_search_issues_url(
                    client,
                    &filters,
                    include_labels,
                    exclude_labels,
                    ordering,
                )
            } else if is_pr {
                self.build_pulls_url(client, &filters, include_labels, ordering)
            } else {
                self.build_issues_url(client, &filters, include_labels, ordering)
            };

            let result = client.get(&url);
            if use_search_api {
                let result = client
                    .json::<IssueSearchResult>(result)
                    .await
                    .with_context(|| format!("failed to list issues from {url}"))?;
                issues.extend(result.items);
                if (issues.len() as u64) < result.total_count {
                    ordering.page += 1;
                    continue;
                }
            } else {
                // FIXME: paginate with non-search
                issues = client
                    .json(result)
                    .await
                    .with_context(|| format!("failed to list issues from {url}"))?;
            }

            break;
        }
        Ok(issues)
    }

    fn build_issues_url(
        &self,
        client: &GithubClient,
        filters: &[(&str, &str)],
        include_labels: &[&str],
        ordering: Ordering<'_>,
    ) -> String {
        self.build_endpoint_url(client, "issues", filters, include_labels, ordering)
    }

    fn build_pulls_url(
        &self,
        client: &GithubClient,
        filters: &[(&str, &str)],
        include_labels: &[&str],
        ordering: Ordering<'_>,
    ) -> String {
        self.build_endpoint_url(client, "pulls", filters, include_labels, ordering)
    }

    fn build_endpoint_url(
        &self,
        client: &GithubClient,
        endpoint: &str,
        filters: &[(&str, &str)],
        include_labels: &[&str],
        ordering: Ordering<'_>,
    ) -> String {
        let filters = filters
            .iter()
            .map(|(key, val)| format!("{key}={val}"))
            .chain([format!("labels={}", include_labels.join(","))])
            .chain(["filter=all".to_owned()])
            .chain([format!("sort={}", ordering.sort)])
            .chain([format!("direction={}", ordering.direction)])
            .chain([format!("per_page={}", ordering.per_page)])
            .format("&");
        format!(
            "{}/repos/{}/{}?{}",
            client.api_url, self.full_name, endpoint, filters
        )
    }

    fn build_search_issues_url(
        &self,
        client: &GithubClient,
        filters: &[(&str, &str)],
        include_labels: &[&str],
        exclude_labels: &[&str],
        ordering: Ordering<'_>,
    ) -> String {
        let filters = filters
            .iter()
            .filter(|filter| **filter != ("state", "all"))
            .map(|(key, val)| format!("{key}:{val}"))
            .chain(include_labels.iter().map(|label| format!("label:{label}")))
            .chain(exclude_labels.iter().map(|label| format!("-label:{label}")))
            .chain([format!("repo:{}", self.full_name)])
            .format("+");
        format!(
            "{}/search/issues?q={}&sort={}&order={}&per_page={}&page={}",
            client.api_url,
            filters,
            ordering.sort,
            ordering.direction,
            ordering.per_page,
            ordering.page,
        )
    }

    pub async fn get_issue(&self, client: &GithubClient, issue_num: u64) -> anyhow::Result<Issue> {
        client
            .issue(
                &IssueRepository {
                    organization: self.owner().to_string(),
                    repository: self.name().to_string(),
                },
                issue_num,
            )
            .await
    }

    pub async fn get_pr(&self, client: &GithubClient, pr_num: u64) -> anyhow::Result<Issue> {
        client
            .pull_request(
                &IssueRepository {
                    organization: self.owner().to_string(),
                    repository: self.name().to_string(),
                },
                pr_num,
            )
            .await
    }
}

// Commits

#[derive(Debug, serde::Deserialize)]
pub struct GithubCommit {
    pub sha: String,
    pub commit: GithubCommitCommitField,
    pub parents: Vec<Parent>,
    pub html_url: String,
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

#[derive(Debug, serde::Deserialize)]
pub struct GitUser {
    pub date: DateTime<FixedOffset>,
    pub name: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct Parent {
    pub sha: String,
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

impl Repository {
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
        let mut url = format!("{}/contents/{}", self.url(client), path);
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
}

// Ordering

#[derive(Copy, Clone)]
struct Ordering<'a> {
    sort: &'a str,
    direction: &'a str,
    per_page: &'a str,
    page: u64,
}
