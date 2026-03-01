use anyhow::Context;
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::github::GithubClient;

#[derive(Debug, Clone)]
pub struct UserRepository {
    pub name: String,
    pub owner: String,
    pub created_at: DateTime<Utc>,
    pub fork: bool,
}

#[derive(Deserialize)]
struct RepoResponse {
    name: String,
    owner: OwnerResponse,
    created_at: DateTime<Utc>,
    fork: bool,
}

#[derive(Deserialize)]
struct OwnerResponse {
    login: String,
}

impl GithubClient {
    /// Fetches recently created repositories for a GitHub user.
    ///
    /// Returns up to `limit` repositories, sorted by creation date (most recent first).
    pub async fn recent_user_repositories(
        &self,
        username: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<UserRepository>> {
        // GitHub allows at most 100 items per page.
        const MAX_PAGE_SIZE: usize = 100;

        let mut repos: Vec<UserRepository> = Vec::new();
        let mut page: u32 = 1;

        loop {
            let per_page = limit.saturating_sub(repos.len()).min(MAX_PAGE_SIZE);
            let per_page_str = per_page.to_string();
            let page_str = page.to_string();
            let url = format!("{}/users/{username}/repos", self.api_url);
            let page_repos: Vec<RepoResponse> = self
                .json(self.get(&url).query(&[
                    ("sort", "created"),
                    ("direction", "desc"),
                    ("per_page", per_page_str.as_str()),
                    ("page", page_str.as_str()),
                ]))
                .await
                .with_context(|| format!("failed to fetch repositories for {username}"))?;

            let is_last_page = page_repos.len() < per_page;

            repos.extend(page_repos.into_iter().map(|r| UserRepository {
                name: r.name,
                owner: r.owner.login,
                created_at: r.created_at,
                fork: r.fork,
            }));

            if is_last_page || repos.len() >= limit {
                break;
            }

            page += 1;
        }

        repos.truncate(limit);
        Ok(repos)
    }
}
