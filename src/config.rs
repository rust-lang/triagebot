use crate::changelogs::ChangelogFormat;
use crate::github::GithubClient;
use std::collections::{HashMap, HashSet};
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

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) struct Config {
    pub(crate) relabel: Option<RelabelConfig>,
    pub(crate) assign: Option<AssignConfig>,
    pub(crate) ping: Option<PingConfig>,
    pub(crate) nominate: Option<NominateConfig>,
    pub(crate) prioritize: Option<PrioritizeConfig>,
    pub(crate) major_change: Option<MajorChangeConfig>,
    pub(crate) glacier: Option<GlacierConfig>,
    pub(crate) close: Option<CloseConfig>,
    pub(crate) autolabel: Option<AutolabelConfig>,
    pub(crate) notify_zulip: Option<NotifyZulipConfig>,
    pub(crate) github_releases: Option<GitHubReleasesConfig>,
    pub(crate) review_submitted: Option<ReviewSubmittedConfig>,
    pub(crate) shortcut: Option<ShortcutConfig>,
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
pub(crate) struct NominateConfig {
    // team name -> label
    pub(crate) teams: HashMap<String, String>,
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
pub(crate) struct PingConfig {
    // team name -> message
    // message will have the cc string appended
    #[serde(flatten)]
    teams: HashMap<String, PingTeamConfig>,
}

impl PingConfig {
    pub(crate) fn get_by_name(&self, team: &str) -> Option<(&str, &PingTeamConfig)> {
        if let Some((team, cfg)) = self.teams.get_key_value(team) {
            return Some((team, cfg));
        }

        for (name, cfg) in self.teams.iter() {
            if cfg.alias.contains(team) {
                return Some((name, cfg));
            }
        }

        None
    }
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
pub(crate) struct PingTeamConfig {
    pub(crate) message: String,
    #[serde(default)]
    pub(crate) alias: HashSet<String>,
    pub(crate) label: Option<String>,
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
pub(crate) struct AssignConfig {
    #[serde(default)]
    _empty: (),
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) struct RelabelConfig {
    #[serde(default)]
    pub(crate) allow_unauthenticated: Vec<String>,
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
pub(crate) struct ShortcutConfig {
    #[serde(default)]
    _empty: (),
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
pub(crate) struct PrioritizeConfig {
    pub(crate) label: String,
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
pub(crate) struct AutolabelConfig {
    #[serde(flatten)]
    pub(crate) labels: HashMap<String, AutolabelLabelConfig>,
}

impl AutolabelConfig {
    pub(crate) fn get_by_trigger(&self, trigger: &str) -> Vec<(&str, &AutolabelLabelConfig)> {
        let mut results = Vec::new();
        for (label, cfg) in self.labels.iter() {
            if cfg.trigger_labels.iter().any(|l| l == trigger) {
                results.push((label.as_str(), cfg));
            }
        }
        results
    }
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
pub(crate) struct AutolabelLabelConfig {
    pub(crate) trigger_labels: Vec<String>,
    #[serde(default)]
    pub(crate) exclude_labels: Vec<String>,
    #[serde(default)]
    pub(crate) trigger_files: Vec<String>,
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
pub(crate) struct NotifyZulipConfig {
    #[serde(flatten)]
    pub(crate) labels: HashMap<String, NotifyZulipLabelConfig>,
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
pub(crate) struct NotifyZulipLabelConfig {
    pub(crate) zulip_stream: u64,
    pub(crate) topic: String,
    pub(crate) message_on_add: Option<String>,
    pub(crate) message_on_remove: Option<String>,
    pub(crate) message_on_close: Option<String>,
    pub(crate) message_on_reopen: Option<String>,
    #[serde(default)]
    pub(crate) required_labels: Vec<String>,
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
pub(crate) struct MajorChangeConfig {
    pub(crate) zulip_ping: String,
    pub(crate) second_label: String,
    pub(crate) meeting_label: String,
    pub(crate) zulip_stream: u64,
    pub(crate) open_extra_text: Option<String>,
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
pub(crate) struct GlacierConfig {}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
pub(crate) struct CloseConfig {}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
pub(crate) struct ReviewSubmittedConfig {
    pub(crate) review_labels: Vec<String>,
    pub(crate) reviewed_label: String,
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

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) struct GitHubReleasesConfig {
    pub(crate) format: ChangelogFormat,
    pub(crate) project_name: String,
    pub(crate) changelog_path: String,
    pub(crate) changelog_branch: String,
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
    Http(Arc<anyhow::Error>),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample() {
        let config = r#"
            [relabel]
            allow-unauthenticated = [
                "C-*"
            ]

            [assign]

            [ping.compiler]
            message = """\
            So many people!\
            """
            label = "T-compiler"

            [ping.wg-meta]
            message = """\
            Testing\
            """

            [nominate.teams]
            compiler = "T-compiler"
            release = "T-release"
            core = "T-core"
            infra = "T-infra"

            [shortcut]
        "#;
        let config = toml::from_str::<Config>(&config).unwrap();
        let mut ping_teams = HashMap::new();
        ping_teams.insert(
            "compiler".to_owned(),
            PingTeamConfig {
                message: "So many people!".to_owned(),
                label: Some("T-compiler".to_owned()),
                alias: HashSet::new(),
            },
        );
        ping_teams.insert(
            "wg-meta".to_owned(),
            PingTeamConfig {
                message: "Testing".to_owned(),
                label: None,
                alias: HashSet::new(),
            },
        );
        let mut nominate_teams = HashMap::new();
        nominate_teams.insert("compiler".to_owned(), "T-compiler".to_owned());
        nominate_teams.insert("release".to_owned(), "T-release".to_owned());
        nominate_teams.insert("core".to_owned(), "T-core".to_owned());
        nominate_teams.insert("infra".to_owned(), "T-infra".to_owned());
        assert_eq!(
            config,
            Config {
                relabel: Some(RelabelConfig {
                    allow_unauthenticated: vec!["C-*".into()],
                }),
                assign: Some(AssignConfig { _empty: () }),
                ping: Some(PingConfig { teams: ping_teams }),
                nominate: Some(NominateConfig {
                    teams: nominate_teams
                }),
                shortcut: Some(ShortcutConfig { _empty: () }),
                prioritize: None,
                major_change: None,
                glacier: None,
                close: None,
                autolabel: None,
                notify_zulip: None,
                github_releases: None,
                review_submitted: None,
            }
        );
    }
}
