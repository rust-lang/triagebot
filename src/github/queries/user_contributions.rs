use anyhow::Context;
use chrono::{DateTime, Utc};

use crate::github::GithubClient;

/// Aggregated contribution counts for a user over a given time window.
pub struct UserContributions {
    pub total_created_commits: u64,
    pub total_created_issues: u64,
    pub total_created_prs: u64,
    pub total_created_repos: u64,
}

impl GithubClient {
    /// Fetches total contribution counts for a user `since` the given date.
    /// The date range must be at most one year.
    pub async fn user_contributions_since(
        &self,
        username: &str,
        since: DateTime<Utc>,
    ) -> anyhow::Result<UserContributions> {
        let now = Utc::now();
        assert!((now - since).num_days() <= 365);

        let data = self
            .graphql_query(
                r#"
query($username: String!, $from: DateTime!, $to: DateTime!) {
  user(login: $username) {
    contributionsCollection(from: $from, to: $to) {
      totalCommitContributions
      totalIssueContributions
      totalPullRequestContributions
      totalRepositoryContributions
    }
  }
}
                "#,
                serde_json::json!({
                    "username": username,
                    "from": since.to_rfc3339(),
                    "to": now.to_rfc3339(),
                }),
            )
            .await
            .context("failed to fetch user contributions")?;

        let collection = &data["data"]["user"]["contributionsCollection"];

        Ok(UserContributions {
            total_created_commits: collection["totalCommitContributions"].as_u64().unwrap_or(0),
            total_created_issues: collection["totalIssueContributions"].as_u64().unwrap_or(0),
            total_created_prs: collection["totalPullRequestContributions"]
                .as_u64()
                .unwrap_or(0),
            total_created_repos: collection["totalRepositoryContributions"]
                .as_u64()
                .unwrap_or(0),
        })
    }
}
