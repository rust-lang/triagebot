use reqwest::Client;
use rust_team_data::v1::{BASE_URL, People, Repos, Teams, ZulipMapping};
use serde::de::DeserializeOwned;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct TeamClient {
    base_url: String,
    client: Client,
    teams: CachedTeamItem<Teams>,
    repos: CachedTeamItem<Repos>,
    people: CachedTeamItem<People>,
    zulip_mapping: CachedTeamItem<ZulipMapping>,
}

impl TeamClient {
    pub fn new_from_env() -> Self {
        let base_url = std::env::var("TEAMS_API_URL").unwrap_or(BASE_URL.to_string());
        Self::new(base_url)
    }

    pub fn new(base_url: String) -> Self {
        Self {
            base_url,
            client: Client::new(),
            teams: CachedTeamItem::new("/teams.json"),
            repos: CachedTeamItem::new("/repos.json"),
            people: CachedTeamItem::new("/people.json"),
            zulip_mapping: CachedTeamItem::new("/zulip-map.json"),
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
        self.zulip_mapping.get(&self.client, &self.base_url).await
    }

    pub async fn teams(&self) -> anyhow::Result<Teams> {
        self.teams.get(&self.client, &self.base_url).await
    }

    pub async fn repos(&self) -> anyhow::Result<Repos> {
        self.repos.get(&self.client, &self.base_url).await
    }

    pub async fn people(&self) -> anyhow::Result<People> {
        self.people.get(&self.client, &self.base_url).await
    }
}

/// How long should downloaded team data items be cached in memory.
const CACHE_DURATION: Duration = Duration::from_secs(2 * 60);

#[derive(Clone)]
struct CachedTeamItem<T> {
    value: Arc<RwLock<CachedValue<T>>>,
    url_path: String,
}

impl<T: DeserializeOwned + Clone> CachedTeamItem<T> {
    fn new(url_path: &str) -> Self {
        Self {
            value: Arc::new(RwLock::new(CachedValue::Empty)),
            url_path: url_path.to_string(),
        }
    }

    async fn get(&self, client: &Client, base_url: &str) -> anyhow::Result<T> {
        let now = Instant::now();
        {
            let value = self.value.read().await;
            if let CachedValue::Present {
                value,
                last_download,
            } = &*value
                && *last_download + CACHE_DURATION > now
            {
                return Ok(value.clone());
            }
        }
        match download::<T>(client, base_url, &self.url_path).await {
            Ok(v) => {
                let mut value = self.value.write().await;
                *value = CachedValue::Present {
                    value: v.clone(),
                    last_download: Instant::now(),
                };
                Ok(v)
            }
            Err(e) => Err(e),
        }
    }
}

enum CachedValue<T> {
    Empty,
    Present { value: T, last_download: Instant },
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
            Ok(v) => return Ok(v.json().await?),
            Err(e) if e.is_timeout() => continue,
            Err(e) => return Err(e.into()),
        }
    }

    Err(anyhow::anyhow!("Failed to retrieve {url} in 3 requests"))
}
