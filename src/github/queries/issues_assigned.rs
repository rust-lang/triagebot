use anyhow::Context as _;
use chrono::{DateTime, Utc};

use crate::github::{GithubClient, IssueNumber};

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubIssueAssigned {
    pub number: IssueNumber,
    pub updated_at: DateTime<Utc>,
    pub assignees: Vec<GitHubAssignee>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubAssignee {
    pub login: String,
    #[serde(alias = "databaseId")]
    pub id: u64,
}

impl GithubClient {
    pub async fn issues_assigned(
        &self,
        owner: &str,
        repo: &str,
    ) -> anyhow::Result<Vec<GitHubIssueAssigned>> {
        fn page_info(data: &serde_json::Value) -> (bool, Option<String>) {
            let has_next = data["pageInfo"]["hasNextPage"].as_bool().unwrap_or(false);
            let end_cursor = data["pageInfo"]["endCursor"]
                .as_str()
                .map(|s| s.to_string());
            (has_next, end_cursor)
        }

        let mut issues_cursor: Option<String> = None;
        let mut issues = Vec::<GitHubIssueAssigned>::new();

        loop {
            let mut data = self
                .graphql_query(
                    r##"
query(
  $owner: String!, 
  $repo: String!, 
  $after: String
) {
  repository(owner: $owner, name: $repo) {
    issues(
      first: 100,
      after: $after,
      filterBy: { 
        states: [OPEN], 
        assignee: "*" 
      }
    ) {
      pageInfo {
        hasNextPage
        endCursor
      }
      nodes {
        number
        updatedAt
        assignees(first: 5) {
          nodes {
            login
            databaseId
          }
        }
      }
    }
  }
}
            "##,
                    serde_json::json!({
                        "owner": owner,
                        "repo": repo,
                        "after": issues_cursor.as_deref(),
                    }),
                )
                .await
                .context("failed to fetch opened issues")?;

            let mut value = data["data"]["repository"]["issues"].take();

            for val in value["nodes"].as_array_mut().context("no issues nodes")? {
                issues.push(GitHubIssueAssigned {
                    number: val["number"].as_u64().context("no issue number")?,
                    updated_at: serde_json::from_value(val["updatedAt"].take())?,
                    assignees: serde_json::from_value(val["assignees"]["nodes"].take())?,
                });
            }

            let (has_next, new_end_cursor) = page_info(&value);

            if new_end_cursor.is_some() {
                issues_cursor = new_end_cursor;
            }

            if !has_next {
                break;
            }
        }

        Ok(issues)
    }
}
