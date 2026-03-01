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

impl GithubClient {
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
        merged
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

        let mut prs: Vec<UserPullRequest> = Vec::new();

        if let Some(nodes) = data["data"]["search"]["nodes"].as_array() {
            for node in nodes {
                let Some(title) = node["title"].as_str() else {
                    continue;
                };
                let url = node["url"].as_str().unwrap_or("");
                let number = node["number"].as_u64().unwrap_or(0);
                let repository_owner = node["repository"]["owner"]["login"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                let repository_name = node["repository"]["name"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                let body = node["body"].as_str().unwrap_or("").to_string();
                let created_at = node["createdAt"]
                    .as_str()
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&Utc));

                let state = if node["merged"].as_bool().unwrap_or(false) {
                    PullRequestState::Merged
                } else if node["state"].as_str() == Some("CLOSED") {
                    PullRequestState::Closed
                } else {
                    PullRequestState::Open
                };

                prs.push(UserPullRequest {
                    title: title.to_string(),
                    url: url.to_string(),
                    number,
                    repo_owner: repository_owner,
                    repo_name: repository_name,
                    body,
                    created_at,
                    state,
                });
            }
        }

        Ok(prs)
    }
}
