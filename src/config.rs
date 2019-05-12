use crate::github::GithubClient;
use failure::Error;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

static CONFIG_FILE_NAME: &str = "triagebot.toml";
const REFRESH_EVERY: Duration = Duration::from_secs(2 * 60); // Every two minutes

lazy_static::lazy_static! {
    static ref CONFIG_CACHE: RwLock<HashMap<String, (Arc<Config>, Instant)>> =
        RwLock::new(HashMap::new());
}

#[derive(serde::Deserialize)]
pub(crate) struct Config {
    pub(crate) relabel: Option<RelabelConfig>,
    pub(crate) assign: Option<AssignConfig>,
    pub(crate) triage: Option<TriageConfig>,
}

#[derive(serde::Deserialize)]
pub(crate) struct AssignConfig {
    #[serde(default)]
    _empty: (),
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) struct RelabelConfig {
    #[serde(default)]
    pub(crate) allow_unauthenticated: Vec<String>,
}

#[derive(serde::Deserialize)]
pub(crate) struct TriageConfig {
    pub(crate) remove: Vec<String>,
    pub(crate) high: String,
    pub(crate) medium: String,
    pub(crate) low: String,
}

pub(crate) fn get(gh: &GithubClient, repo: &str) -> Result<Arc<Config>, Error> {
    if let Some(config) = get_cached_config(repo) {
        Ok(config)
    } else {
        get_fresh_config(gh, repo)
    }
}

fn get_cached_config(repo: &str) -> Option<Arc<Config>> {
    let cache = CONFIG_CACHE.read().unwrap();
    cache.get(repo).and_then(|(config, fetch_time)| {
        if fetch_time.elapsed() < REFRESH_EVERY {
            Some(config.clone())
        } else {
            None
        }
    })
}

fn get_fresh_config(gh: &GithubClient, repo: &str) -> Result<Arc<Config>, Error> {
    let contents = gh
        .raw_file(repo, "master", CONFIG_FILE_NAME)?
        .ok_or_else(|| {
            failure::err_msg(
                "This repository is not enabled to use triagebot.\n\
                 Add a `triagebot.toml` in the root of the master branch to enable it.",
            )
        })?;
    let config = Arc::new(toml::from_slice::<Config>(&contents)?);
    CONFIG_CACHE
        .write()
        .unwrap()
        .insert(repo.to_string(), (config.clone(), Instant::now()));
    Ok(config)
}
