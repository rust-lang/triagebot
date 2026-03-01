use anyhow::Context;
use chrono::{DateTime, Utc};

use crate::github::GithubClient;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct UserInfo {
    /// When was the user account created?
    pub created_at: DateTime<Utc>,
    pub public_repos: u32,
}

impl GithubClient {
    /// Fetches basic public information about a GitHub user.
    pub async fn user_info(&self, username: &str) -> anyhow::Result<UserInfo> {
        let url = format!("{}/users/{username}", self.api_url);
        let info: UserInfo = self
            .json(self.get(&url))
            .await
            .with_context(|| format!("failed to fetch user info for {username}"))?;

        Ok(info)
    }
}
