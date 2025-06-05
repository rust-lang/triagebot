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
