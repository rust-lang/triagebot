use anyhow::Context as _;
use chrono::{DateTime, Utc};

use crate::github::{GithubClient, IssueNumber, PullRequestNumber};

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestWithClosingIssuesReferences {
    pub number: PullRequestNumber,
    pub updated_at: DateTime<Utc>,
    pub closing_issues_references: Vec<ClosingIssueReference>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClosingIssueReference {
    pub number: IssueNumber,
}

impl GithubClient {
    pub async fn closing_issues_references(
        &self,
        owner: &str,
        repo: &str,
    ) -> anyhow::Result<Vec<PullRequestWithClosingIssuesReferences>> {
        fn page_info(data: &serde_json::Value) -> (bool, Option<String>) {
            let has_next = data["pageInfo"]["hasNextPage"].as_bool().unwrap_or(false);
            let end_cursor = data["pageInfo"]["endCursor"]
                .as_str()
                .map(|s| s.to_string());
            (has_next, end_cursor)
        }

        let mut prs_cursor: Option<String> = None;
        let mut prs = Vec::<PullRequestWithClosingIssuesReferences>::new();

        loop {
            let mut data = self
                .graphql_query(
                    r##"
query ($owner: String!, $repo: String!, $after: String) {
  repository(owner: $owner, name: $repo) {
    pullRequests(first: 100, after: $after, states: OPEN) {
      pageInfo {
        hasNextPage
        endCursor
      }
      nodes {
        number
        updatedAt
        closingIssuesReferences(first: 10) {
          nodes {
            number
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
                        "after": prs_cursor.as_deref(),
                    }),
                )
                .await
                .context("failed to fetch opened issues")?;

            let mut value = data["data"]["repository"]["pullRequests"].take();

            for val in value["nodes"].as_array_mut().context("no issues nodes")? {
                prs.push(PullRequestWithClosingIssuesReferences {
                    number: val["number"].as_u64().context("no issue number")?,
                    updated_at: serde_json::from_value(val["updatedAt"].take())?,
                    closing_issues_references: serde_json::from_value(
                        val["closingIssuesReferences"]["nodes"].take(),
                    )?,
                });
            }

            let (has_next, new_end_cursor) = page_info(&value);

            if new_end_cursor.is_some() {
                prs_cursor = new_end_cursor;
            }

            if !has_next {
                break;
            }
        }

        Ok(prs)
    }
}
