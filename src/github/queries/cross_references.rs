use anyhow::Context;
use chrono::{DateTime, Utc};

use crate::github::{GithubClient, Issue};

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CrossReference {
    pub created_at: DateTime<Utc>,
    pub will_close_target: bool,
    pub source: CrossReferenceSource,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CrossReferenceSource {
    pub updated_at: DateTime<Utc>,
}

impl Issue {
    /// Fetches basic public information about a GitHub user.
    pub async fn cross_references(
        &self,
        client: &GithubClient,
    ) -> anyhow::Result<Vec<CrossReference>> {
        let mut data = client
            .graphql_query(
                r#"
query ($owner: String!, $repo: String!, $issue: Int!) {
  repository(owner: $owner, name: $repo) {
    issue(number: $issue) {
      timelineItems(first: 100, itemTypes: [CROSS_REFERENCED_EVENT]) {
        nodes {
          ... on CrossReferencedEvent {
            createdAt
            source {
              ... on Issue {
                updatedAt
              }
              ... on PullRequest {
                updatedAt
              }
            }
            willCloseTarget
          }
        }
      }
    }
  }
}
            "#,
                serde_json::json!({
                    "owner": self.repository().organization,
                    "repo": self.repository().repository,
                    "issue": self.number,
                }),
            )
            .await
            .context("failed to fetch cross references for an issue")?;

        let nodes = data["data"]["repository"]["issue"]["timelineItems"]["nodes"].take();
        let cross_references: Vec<_> =
            serde_json::from_value(nodes).context("unable to deserialize cross references")?;

        Ok(cross_references)
    }
}
