use anyhow::Context;
use chrono::{DateTime, Utc};

use crate::github::GithubClient;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PullRequestState {
    Open,
    Closed,
    Merged,
}

#[derive(Debug, Clone)]
pub struct UserPullRequest {
    pub title: String,
    pub url: String,
    pub number: u64,
    pub repo_owner: String,
    pub repo_name: String,
    pub body: String,
    pub created_at: Option<DateTime<Utc>>,
    pub state: PullRequestState,
}

fn parse_pr_node(node: &serde_json::Value) -> Option<UserPullRequest> {
    let title = node["title"].as_str()?;
    let url = node["url"].as_str().unwrap_or("");
    let number = node["number"].as_u64().unwrap_or(0);
    let repo_owner = node["repository"]["owner"]["login"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let repo_name = node["repository"]["name"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let body = node["body"].as_str().unwrap_or("").to_string();
    let created_at = node["createdAt"]
        .as_str()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc));
    let state = match node["state"].as_str() {
        Some("MERGED") => PullRequestState::Merged,
        Some("CLOSED") => PullRequestState::Closed,
        _ => PullRequestState::Open,
    };

    Some(UserPullRequest {
        title: title.to_string(),
        url: url.to_string(),
        number,
        repo_owner,
        repo_name,
        body,
        created_at,
        state,
    })
}

impl GithubClient {
    /// Fetches recent pull requests created by a user across all repositories.
    ///
    /// Returns up to `limit` PRs, sorted by creation date (most recent first).
    /// Uses cursor-based pagination to retrieve multiple pages if needed.
    pub async fn user_prs(
        &self,
        username: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<UserPullRequest>> {
        // GitHub allows at most 100 items per page.
        const MAX_PAGE_SIZE: usize = 100;

        // Here we don't need to scope anything to a given organization, so we don't use the
        // search endpoint to conserve rate limit.
        let mut prs: Vec<UserPullRequest> = Vec::new();
        let mut cursor: Option<String> = None;

        loop {
            let page_size = (limit - prs.len()).min(MAX_PAGE_SIZE);
            let data = self
                .graphql_query(
                    r#"
query($username: String!, $pageSize: Int!, $cursor: String) {
  user(login: $username) {
    pullRequests(first: $pageSize, after: $cursor, orderBy: {field: CREATED_AT, direction: DESC}) {
      pageInfo {
        hasNextPage
        endCursor
      }
      nodes {
        title
        url
        number
        body
        createdAt
        state
        repository {
          name
          owner {
            login
          }
        }
      }
    }
  }
}
                    "#,
                    serde_json::json!({
                        "username": username,
                        "pageSize": page_size,
                        "cursor": cursor,
                    }),
                )
                .await
                .context("failed to fetch user PRs")?;

            let connection = &data["data"]["user"]["pullRequests"];

            if let Some(nodes) = connection["nodes"].as_array() {
                prs.extend(nodes.iter().filter_map(parse_pr_node));
            }

            let has_next_page = connection["pageInfo"]["hasNextPage"]
                .as_bool()
                .unwrap_or(false);

            if !has_next_page || prs.len() >= limit {
                break;
            }

            cursor = connection["pageInfo"]["endCursor"]
                .as_str()
                .map(|s| s.to_string());
        }

        prs.truncate(limit);
        Ok(prs)
    }

    /// Fetches recent pull requests created by a user in a GitHub organization.
    ///
    /// Returns up to `limit` PRs, sorted by creation date (most recent first).
    pub async fn user_prs_in_org(
        &self,
        username: &str,
        org: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<UserPullRequest>> {
        // We could avoid the search API by searching for user's PRs directly. However,
        // if the user makes a lot of PRs in various organizations, we might have to load a bunch
        // of pages before we get to PRs from the given org. So instead we use the search API.
        let search_query = format!("author:{username} org:{org} type:pr sort:created-desc");

        let data = self
            .graphql_query(
                r#"
query($query: String!, $limit: Int!) {
  search(query: $query, type: ISSUE, first: $limit) {
    nodes {
      ... on PullRequest {
        title
        url
        number
        body
        createdAt
        state
        repository {
          name
          owner {
            login
          }
        }
      }
    }
  }
}
                "#,
                serde_json::json!({
                    "query": search_query,
                    "limit": limit,
                }),
            )
            .await
            .context("failed to search for user PRs")?;

        let prs = data["data"]["search"]["nodes"]
            .as_array()
            .map(|nodes| nodes.iter().filter_map(parse_pr_node).collect())
            .unwrap_or_default();

        Ok(prs)
    }
}
