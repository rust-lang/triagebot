use anyhow::Context as _;
use reqwest::Client;
use rust_team_data::v1::{People, Teams, ZulipMapping, BASE_URL};
use serde::de::DeserializeOwned;

#[derive(Clone)]
pub struct TeamApiClient {
    base_url: String,
    client: Client,
}

impl TeamApiClient {
    pub fn new_from_env() -> Self {
        let base_url = std::env::var("TEAMS_API_URL").unwrap_or(BASE_URL.to_string());
        Self::new(base_url)
    }

    pub fn new(base_url: String) -> Self {
        Self {
            base_url,
            client: Client::new(),
        }
    }

    pub async fn zulip_to_github_id(&self, zulip_id: u64) -> anyhow::Result<Option<u64>> {
        let map = self.zulip_map().await?;
        Ok(map.users.get(&zulip_id).copied())
    }

    pub async fn username_from_gh_id(&self, github_id: u64) -> anyhow::Result<Option<String>> {
        let people_map = self.people().await?;
        Ok(people_map
            .people
            .into_iter()
            .filter(|(_, p)| p.github_id == github_id)
            .map(|p| p.0)
            .next())
    }

    // Returns the ID of the given user, if the user is in the `all` team.
    pub async fn get_gh_id_from_username(&self, login: &str) -> anyhow::Result<Option<u64>> {
        let permission = self.teams().await?;
        let map = permission.teams;
        let login = login.to_lowercase();
        Ok(map["all"]
            .members
            .iter()
            .find(|g| g.github.to_lowercase() == login)
            .map(|u| u.github_id))
    }

    pub async fn github_to_zulip_id(&self, github_id: u64) -> anyhow::Result<Option<u64>> {
        let map = self.zulip_map().await?;
        Ok(map
            .users
            .iter()
            .find(|&(_, &github)| github == github_id)
            .map(|v| *v.0))
    }

    pub async fn get_team(&self, team: &str) -> anyhow::Result<Option<rust_team_data::v1::Team>> {
        let permission = self.teams().await?;
        let mut map = permission.teams;
        Ok(map.swap_remove(team))
    }

    /// Fetches a Rust team via its GitHub team name.
    pub async fn get_team_by_github_name(
        &self,
        org: &str,
        team: &str,
    ) -> anyhow::Result<Option<rust_team_data::v1::Team>> {
        let teams = self.teams().await?;
        for rust_team in teams.teams.into_values() {
            if let Some(github) = &rust_team.github {
                for gh_team in &github.teams {
                    if gh_team.org == org && gh_team.name == team {
                        return Ok(Some(rust_team));
                    }
                }
            }
        }
        Ok(None)
    }

    pub async fn zulip_map(&self) -> anyhow::Result<ZulipMapping> {
        download(&self.client, &self.base_url, "/zulip-map.json")
            .await
            .context("team-api: zulip-map.json")
    }

    pub async fn teams(&self) -> anyhow::Result<Teams> {
        download(&self.client, &self.base_url, "/teams.json")
            .await
            .context("team-api: teams.json")
    }

    pub async fn people(&self) -> anyhow::Result<People> {
        download(&self.client, &self.base_url, "/people.json")
            .await
            .context("team-api: people.json")
    }
}

async fn download<T: DeserializeOwned>(
    client: &Client,
    base_url: &str,
    path: &str,
) -> anyhow::Result<T> {
    let url = format!("{base_url}{path}");
    for _ in 0i32..3 {
        let response = client.get(&url).send().await;
        match response {
            Ok(v) => {
                return Ok(v.json().await?);
            }
            Err(e) => {
                if e.is_timeout() {
                    continue;
                } else {
                    return Err(e.into());
                }
            }
        }
    }

    Err(anyhow::anyhow!("Failed to retrieve {url} in 3 requests"))
}
