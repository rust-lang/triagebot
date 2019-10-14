use crate::github::GithubClient;
use failure::Error;
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

static CONFIG_FILE_NAME: &str = "triagebot.toml";
const REFRESH_EVERY: Duration = Duration::from_secs(2 * 60); // Every two minutes

lazy_static::lazy_static! {
    static ref CONFIG_CACHE:
        RwLock<HashMap<String, (Result<Arc<Config>, ConfigurationError>, Instant)>> =
        RwLock::new(HashMap::new());
}

#[derive(Debug, serde::Deserialize)]
pub(crate) struct Config {
    pub(crate) relabel: Option<RelabelConfig>,
    pub(crate) assign: Option<AssignConfig>,
    pub(crate) ping: Option<PingConfig>,
    pub(crate) nominate: Option<NominateConfig>,
}

#[derive(Debug, serde::Deserialize)]
pub(crate) struct NominateConfig {
    // team name -> label
    #[serde(flatten)]
    pub(crate) teams: HashMap<String, String>,
}

#[derive(Debug, serde::Deserialize)]
pub(crate) struct PingConfig {
    // team name -> message
    // message will have the cc string appended
    #[serde(flatten)]
    pub(crate) teams: HashMap<String, PingTeamConfig>,
}

#[derive(Debug, serde::Deserialize)]
pub(crate) struct PingTeamConfig {
    pub(crate) message: String,
    pub(crate) label: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub(crate) struct AssignConfig {
    #[serde(default)]
    _empty: (),
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) struct RelabelConfig {
    #[serde(default)]
    pub(crate) allow_unauthenticated: Vec<String>,
}

pub(crate) async fn get(gh: &GithubClient, repo: &str) -> Result<Arc<Config>, ConfigurationError> {
    if let Some(config) = get_cached_config(repo) {
        log::trace!("returning config for {} from cache", repo);
        config
    } else {
        log::trace!("fetching fresh config for {}", repo);
        let res = get_fresh_config(gh, repo).await;
        CONFIG_CACHE
            .write()
            .unwrap()
            .insert(repo.to_string(), (res.clone(), Instant::now()));
        res
    }
}

fn get_cached_config(repo: &str) -> Option<Result<Arc<Config>, ConfigurationError>> {
    let cache = CONFIG_CACHE.read().unwrap();
    cache.get(repo).and_then(|(config, fetch_time)| {
        if fetch_time.elapsed() < REFRESH_EVERY {
            Some(config.clone())
        } else {
            None
        }
    })
}

async fn get_fresh_config(
    gh: &GithubClient,
    repo: &str,
) -> Result<Arc<Config>, ConfigurationError> {
    let contents = gh
        .raw_file(repo, "master", CONFIG_FILE_NAME)
        .await
        .map_err(|e| ConfigurationError::Http(Arc::new(e)))?
        .ok_or(ConfigurationError::Missing)?;
    let config = Arc::new(toml::from_slice::<Config>(&contents).map_err(ConfigurationError::Toml)?);
    log::debug!("fresh configuration for {}: {:?}", repo, config);
    Ok(config)
}

#[derive(Clone, Debug)]
pub enum ConfigurationError {
    Missing,
    Toml(toml::de::Error),
    Http(Arc<Error>),
}

impl std::error::Error for ConfigurationError {}

impl fmt::Display for ConfigurationError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ConfigurationError::Missing => write!(
                f,
                "This repository is not enabled to use triagebot.\n\
                 Add a `triagebot.toml` in the root of the master branch to enable it."
            ),
            ConfigurationError::Toml(e) => {
                write!(f, "Malformed `triagebot.toml` in master branch.\n{}", e)
            }
            ConfigurationError::Http(_) => {
                write!(f, "Failed to query configuration for this repository.")
            }
        }
    }
}
