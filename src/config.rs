use crate::changelogs::ChangelogFormat;
use crate::github::{GithubClient, Repository};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, LazyLock, RwLock};
use std::time::{Duration, Instant};
use tracing as log;

pub(crate) static CONFIG_FILE_NAME: &str = "triagebot.toml";
const REFRESH_EVERY: Duration = Duration::from_secs(2 * 60); // Every two minutes

type MaybeConfig = Result<Arc<Config>, ConfigurationError>;

static CONFIG_CACHE: LazyLock<RwLock<HashMap<String, (MaybeConfig, Instant)>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

// This struct maps each possible option of the triagebot.toml.
// See documentation of options at: https://forge.rust-lang.org/triagebot/pr-assignment.html#configuration
// When adding a new config option to the triagebot.toml, it must be also mapped here
// Will be used by the `issue_handlers!()` or `command_handlers!()` macros.
#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    pub(crate) relabel: Option<RelabelConfig>,
    pub(crate) assign: Option<AssignConfig>,
    pub(crate) ping: Option<PingConfig>,
    pub(crate) nominate: Option<NominateConfig>,
    pub(crate) prioritize: Option<PrioritizeConfig>,
    pub(crate) major_change: Option<MajorChangeConfig>,
    pub(crate) close: Option<CloseConfig>,
    pub(crate) autolabel: Option<AutolabelConfig>,
    pub(crate) notify_zulip: Option<NotifyZulipConfig>,
    pub(crate) github_releases: Option<GitHubReleasesConfig>,
    pub(crate) review_submitted: Option<ReviewSubmittedConfig>,
    pub(crate) review_requested: Option<ReviewRequestedConfig>,
    pub(crate) shortcut: Option<ShortcutConfig>,
    pub(crate) note: Option<NoteConfig>,
    pub(crate) concern: Option<ConcernConfig>,
    pub(crate) mentions: Option<MentionsConfig>,
    pub(crate) no_merges: Option<NoMergesConfig>,
    pub(crate) pr_tracking: Option<ReviewPrefsConfig>,
    pub(crate) transfer: Option<TransferConfig>,
    pub(crate) merge_conflicts: Option<MergeConflictConfig>,
    pub(crate) bot_pull_requests: Option<BotPullRequests>,
    pub(crate) rendered_link: Option<RenderedLinkConfig>,
    #[serde(alias = "canonicalize-issue-links")]
    pub(crate) issue_links: Option<IssueLinksConfig>,
    pub(crate) no_mentions: Option<NoMentionsConfig>,
    pub(crate) behind_upstream: Option<BehindUpstreamConfig>,
    pub(crate) backport: Option<BackportConfig>,
    pub(crate) range_diff: Option<RangeDiffConfig>,
    pub(crate) review_changes_since: Option<ReviewChangesSinceConfig>,
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
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

        for (name, cfg) in &self.teams {
            if cfg.alias.contains(team) {
                return Some((name, cfg));
            }
        }

        None
    }
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PingTeamConfig {
    pub(crate) message: String,
    #[serde(default)]
    pub(crate) alias: HashSet<String>,
    pub(crate) label: Option<String>,
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AssignReviewPrefsConfig {}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub(crate) struct AssignCustomMessages {
    /// Message with reviewer automaticaly chosen (`{assignee}`)
    #[serde(alias = "welcome-message")]
    pub(crate) auto_assign_someone: Option<String>,
    /// Message without a reviewer automaticaly chosen
    #[serde(alias = "welcome-message-no-reviewer")]
    pub(crate) auto_assign_no_one: String,
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AssignConfig {
    /// If enabled, then posts a warning comment if the PR is opened against a
    /// different branch than the default (usually master or main).
    #[serde(default)]
    pub(crate) warn_non_default_branch: WarnNonDefaultBranchConfig,
    /// A URL to include in the welcome message.
    pub(crate) contributing_url: Option<String>,
    /// Ad-hoc groups that can be referred to in `owners`.
    #[serde(default)]
    pub(crate) adhoc_groups: HashMap<String, Vec<String>>,
    /// Users to assign when a new PR is opened.
    /// The key is a gitignore-style path, and the value is a list of
    /// usernames, team names, or ad-hoc groups.
    #[serde(default)]
    pub(crate) owners: HashMap<String, Vec<String>>,
    #[serde(default)]
    pub(crate) users_on_vacation: HashSet<String>,
    /// Should review preferences be taken into account when deciding who to assign to a PR?
    #[serde(default)]
    pub(crate) review_prefs: Option<AssignReviewPrefsConfig>,
    /// Custom welcome messages
    #[serde(default)]
    #[serde(alias = "custom_welcome_messages")]
    pub(crate) custom_messages: Option<AssignCustomMessages>,
}

impl AssignConfig {
    pub(crate) fn is_on_vacation(&self, user: &str) -> bool {
        let name_lower = user.to_lowercase();
        self.users_on_vacation
            .iter()
            .any(|vacationer| name_lower == vacationer.to_lowercase())
    }

    /// Return a "fallback" adhoc group, which is used for assigning reviewers if no other
    /// reviewer was found.
    pub(crate) fn fallback_review_group(&self) -> Option<&[String]> {
        self.adhoc_groups.get("fallback").map(Vec::as_slice)
    }
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(untagged)]
pub(crate) enum WarnNonDefaultBranchConfig {
    Simple(bool),
    Extended {
        enable: bool,
        /// List of exceptions that have a different default branch
        #[serde(default)]
        exceptions: Vec<WarnNonDefaultBranchException>,
    },
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WarnNonDefaultBranchException {
    /// Substring in the title that match this exception
    pub(crate) title: String,
    /// The actual branch that should be associated with the issue
    pub(crate) branch: String,
}

impl Default for WarnNonDefaultBranchConfig {
    fn default() -> WarnNonDefaultBranchConfig {
        WarnNonDefaultBranchConfig::Simple(false)
    }
}

impl WarnNonDefaultBranchConfig {
    pub(crate) fn enabled_and_exceptions(&self) -> Option<&[WarnNonDefaultBranchException]> {
        match self {
            WarnNonDefaultBranchConfig::Simple(enable) => enable.then(|| &[] as &[_]),
            WarnNonDefaultBranchConfig::Extended { enable, exceptions } => {
                enable.then(|| exceptions.as_slice())
            }
        }
    }
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NoMergesConfig {
    /// No action will be taken on PRs with these substrings in the title.
    #[serde(default)]
    pub(crate) exclude_titles: Vec<String>,
    /// Set these labels on the PR when merge commits are detected.
    #[serde(default)]
    pub(crate) labels: Vec<String>,
    /// Override the default message to post when merge commits are detected.
    ///
    /// This message will always be followed up with
    /// "The following commits are merge commits:" and then
    /// a list of the merge commits.
    pub(crate) message: Option<String>,
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NoteConfig {
    #[serde(default)]
    _empty: (),
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConcernConfig {
    /// Set the labels on the PR when concerns are active.
    #[serde(default)]
    pub(crate) labels: Vec<String>,
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
pub(crate) struct MentionsConfig {
    #[serde(flatten)]
    pub(crate) entries: HashMap<String, MentionsEntryConfig>,
}

#[derive(PartialEq, Eq, Debug, Default, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum MentionsEntryType {
    #[default]
    Filename,
    Content,
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MentionsEntryConfig {
    #[serde(alias = "type")]
    #[serde(default)]
    pub(crate) type_: MentionsEntryType,
    pub(crate) message: Option<String>,
    #[serde(default)]
    pub(crate) cc: Vec<String>,
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub(crate) struct RelabelConfig {
    #[serde(default)]
    pub(crate) allow_unauthenticated: Vec<String>,
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ShortcutConfig {
    #[serde(default)]
    _empty: (),
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
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
        for (label, cfg) in &self.labels {
            if cfg.trigger_labels.iter().any(|l| l == trigger) {
                results.push((label.as_str(), cfg));
            }
        }
        results
    }
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AutolabelLabelConfig {
    #[serde(default)]
    pub(crate) trigger_labels: Vec<String>,
    #[serde(default)]
    pub(crate) exclude_labels: Vec<String>,
    #[serde(default)]
    pub(crate) trigger_files: Vec<String>,
    #[serde(default)]
    pub(crate) new_pr: bool,
    #[serde(default)]
    pub(crate) new_issue: bool,
    #[serde(default)]
    pub(crate) new_draft: bool,
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
pub(crate) struct NotifyZulipConfig {
    #[serde(flatten)]
    pub(crate) labels: HashMap<String, NotifyZulipTablesConfig>,
}

#[derive(PartialEq, Eq, Debug)]
pub(crate) struct NotifyZulipTablesConfig {
    pub(crate) subtables: HashMap<String, NotifyZulipLabelConfig>,
}

impl<'de> serde::Deserialize<'de> for NotifyZulipTablesConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error;
        use toml::Value;

        // Deserialize into a toml::value::Table for dynamic inspection
        let table = toml::Value::deserialize(deserializer)?
            .as_table()
            .cloned()
            .ok_or_else(|| Error::custom("expected a TOML table"))?;

        let mut subtables = HashMap::new();
        let mut direct_fields = toml::value::Table::new();

        for (k, v) in table {
            if let Some(subtable) = v.as_table() {
                // This is a subtable; deserialize as NotifyZulipLabelConfig
                let sub = NotifyZulipLabelConfig::deserialize(Value::Table(subtable.clone()))
                    .map_err(Error::custom)?;
                subtables.insert(k, sub);
            } else {
                // This is a direct field; collect for the "" entry
                direct_fields.insert(k, v);
            }
        }

        if !direct_fields.is_empty() {
            let direct = NotifyZulipLabelConfig::deserialize(Value::Table(direct_fields))
                .map_err(Error::custom)?;
            subtables.insert(String::new(), direct);
        }

        Ok(NotifyZulipTablesConfig { subtables })
    }
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NotifyZulipLabelConfig {
    pub(crate) zulip_stream: u64,
    pub(crate) topic: String,
    #[serde(rename = "message_on_add", default, deserialize_with = "string_or_seq")]
    pub(crate) messages_on_add: Vec<String>,
    #[serde(
        rename = "message_on_remove",
        default,
        deserialize_with = "string_or_seq"
    )]
    pub(crate) messages_on_remove: Vec<String>,
    #[serde(
        rename = "message_on_close",
        default,
        deserialize_with = "string_or_seq"
    )]
    pub(crate) messages_on_close: Vec<String>,
    #[serde(
        rename = "message_on_reopen",
        default,
        deserialize_with = "string_or_seq"
    )]
    pub(crate) messages_on_reopen: Vec<String>,
    #[serde(default)]
    pub(crate) required_labels: Vec<String>,
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MajorChangeConfig {
    /// A username (typically a group, e.g. T-lang) to ping on Zulip for newly
    /// opened proposals.
    pub(crate) zulip_ping: String,
    /// This label allows an issue to participate in the major change process
    /// (i.e., creates a Zulip thread, tracks seconding, etc.)
    // This has a default primarily for backwards compatibility.
    #[serde(default = "MajorChangeConfig::enabling_label_default")]
    pub(crate) enabling_label: String,
    /// This is the label applied when issuing a `@rustbot second` command, it
    /// indicates that the proposal has moved into the 10 day waiting period.
    pub(crate) second_label: String,
    /// This is the label applied after the waiting period has successfully
    /// elapsed (currently not automatically applied; this must be done
    /// manually).
    // This has a default primarily for backwards compatibility.
    #[serde(default = "MajorChangeConfig::accept_label_default")]
    pub(crate) accept_label: String,
    /// This is the label to be added to newly opened proposals, so they can be
    /// discussed in a meeting.
    pub(crate) meeting_label: String,
    /// This label signals there are concern(s) about the proposal.
    pub(crate) concerns_label: Option<String>,
    /// Waiting period after second for the major change to become accepted.
    #[serde(default = "MajorChangeConfig::waiting_period_default")]
    pub(crate) waiting_period: u16,
    /// Enables automatic closing of the major change when the waiting period is completed.
    #[serde(default)]
    pub(crate) auto_closing: bool,
    /// The Zulip stream ID where the messages about the status of
    /// the major changed should be relayed.
    pub(crate) zulip_stream: u64,
    /// Extra text in the opening major change.
    pub(crate) open_extra_text: Option<String>,
    /// Template for a tracking issue to be created when the major change is accepted
    #[serde(rename = "tracking-issue-template")]
    pub(crate) tracking_issue_template: Option<MajorChangeTrackingIssueTemplateConfig>,
}

impl MajorChangeConfig {
    fn enabling_label_default() -> String {
        String::from("major-change")
    }
    fn accept_label_default() -> String {
        String::from("major-change-accepted")
    }
    fn waiting_period_default() -> u16 {
        10
    }
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub(crate) struct MajorChangeTrackingIssueTemplateConfig {
    /// Template for the title
    pub(crate) title: String,
    /// Repository where to create the tracking issue (otherwise the current repository)
    pub(crate) repository: Option<String>,
    /// List of labels to add to the issue
    pub(crate) labels: Vec<String>,
    /// Template of the body
    pub(crate) body: String,
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CloseConfig {}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReviewSubmittedConfig {
    pub(crate) review_labels: Vec<String>,
    pub(crate) reviewed_label: String,
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReviewRequestedConfig {
    pub(crate) remove_labels: Vec<String>,
    pub(crate) add_labels: Vec<String>,
}

pub(crate) async fn get(gh: &GithubClient, repo: &Repository) -> MaybeConfig {
    if let Some(config) = get_cached_config(&repo.full_name) {
        log::trace!("returning config for {} from cache", repo.full_name);
        config
    } else {
        log::trace!("fetching fresh config for {}", repo.full_name);
        let res = get_fresh_config(gh, repo).await;
        CONFIG_CACHE
            .write()
            .unwrap()
            .insert(repo.full_name.to_string(), (res.clone(), Instant::now()));
        res
    }
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub(crate) struct GitHubReleasesConfig {
    pub(crate) format: ChangelogFormat,
    pub(crate) project_name: String,
    pub(crate) changelog_path: String,
    pub(crate) changelog_branch: String,
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
pub(crate) struct ReviewPrefsConfig {
    #[serde(default)]
    _empty: (),
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub(crate) struct TransferConfig {}

#[derive(Clone, PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub(crate) struct MergeConflictConfig {
    #[serde(default)]
    pub remove: HashSet<String>,
    #[serde(default)]
    pub add: HashSet<String>,
    #[serde(default)]
    pub unless: HashSet<String>,
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub(crate) struct BotPullRequests {}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub(crate) struct RenderedLinkConfig {
    /// List of paths to watch for modifications
    pub(crate) trigger_files: Vec<String>,
    /// List of paths to exclude from watching for modifications
    #[serde(default)]
    pub(crate) exclude_files: Vec<String>,
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub(crate) struct IssueLinksConfig {
    #[serde(default)]
    pub(crate) check_commits: IssueLinksCheckCommitsConfig,
}

#[derive(PartialEq, Eq, Debug, Default)]
pub(crate) enum IssueLinksCheckCommitsConfig {
    /// No checking of commits
    Off,
    /// Only check for uncanonicalized issue links in commits
    Uncanonicalized,
    /// Check for all issue links in commits
    #[default]
    All,
}

impl<'de> serde::Deserialize<'de> for IssueLinksCheckCommitsConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct CheckCommitsVisitor;

        impl serde::de::Visitor<'_> for CheckCommitsVisitor {
            type Value = IssueLinksCheckCommitsConfig;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a bool or the string \"uncanonicalized\"")
            }

            fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E> {
                Ok(if v {
                    IssueLinksCheckCommitsConfig::All
                } else {
                    IssueLinksCheckCommitsConfig::Off
                })
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                if v == "uncanonicalized" {
                    Ok(IssueLinksCheckCommitsConfig::Uncanonicalized)
                } else {
                    Err(E::custom("expected \"uncanonicalized\""))
                }
            }
        }

        deserializer.deserialize_any(CheckCommitsVisitor)
    }
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub(crate) struct NoMentionsConfig {
    /// The check will not be performed on titles that include any of these substrings (case
    /// insensitive)
    #[serde(default)]
    pub(crate) exclude_titles: Vec<String>,
}

/// Configuration for PR behind commits checks
#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub(crate) struct BehindUpstreamConfig {
    /// The threshold of days for parent commit age to trigger a warning.
    /// Default is 7 days if not specified.
    pub(crate) days_threshold: Option<usize>,
}

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
pub(crate) struct BackportConfig {
    // Config identifier -> labels
    #[serde(flatten)]
    pub(crate) configs: HashMap<String, BackportRuleConfig>,
}

#[derive(Default, PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub(crate) struct BackportRuleConfig {
    /// Prerequisite label(s) (one of them) to trigger this handler for a specific team
    pub(crate) required_pr_labels: Vec<String>,
    /// Prerequisite label for an issue to qualify as regression
    pub(crate) required_issue_label: String,
    /// Labels to be added to a pull request closing the regression
    pub(crate) add_labels: Vec<String>,
}

/// Configuration for rebase range-diff comment
#[derive(Default, PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub(crate) struct RangeDiffConfig {}

/// Configuration for the changes since link on review body
#[derive(Default, PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub(crate) struct ReviewChangesSinceConfig {}

fn get_cached_config(repo: &str) -> Option<MaybeConfig> {
    let cache = CONFIG_CACHE.read().unwrap();
    cache.get(repo).and_then(|(config, fetch_time)| {
        if fetch_time.elapsed() < REFRESH_EVERY {
            Some(config.clone())
        } else {
            None
        }
    })
}

async fn get_fresh_config(gh: &GithubClient, repo: &Repository) -> MaybeConfig {
    let contents = gh
        .raw_file(&repo.full_name, &repo.default_branch, CONFIG_FILE_NAME)
        .await
        .map_err(|e| ConfigurationError::Http(Arc::new(e)))?
        .ok_or(ConfigurationError::Missing)?;
    let contents = String::from_utf8_lossy(&contents);
    let config = Arc::new(toml::from_str::<Config>(&contents).map_err(ConfigurationError::Toml)?);
    log::debug!("fresh configuration for {}: {:?}", repo.full_name, config);
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
                 Add a `triagebot.toml` in the root of the default branch to enable it."
            ),
            ConfigurationError::Toml(e) => {
                write!(f, "Malformed `triagebot.toml` in default branch.\n{e}")
            }
            ConfigurationError::Http(e) => {
                write!(
                    f,
                    "Failed to query configuration for this repository.\n{e:?}"
                )
            }
        }
    }
}

fn string_or_seq<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct Visitor;

    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = Vec<String>;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("string or sequence of strings")
        }

        fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
            Ok(Vec::new())
        }

        fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
            Ok(vec![value.to_owned()])
        }

        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, seq: A) -> Result<Self::Value, A::Error> {
            serde::Deserialize::deserialize(serde::de::value::SeqAccessDeserializer::new(seq))
        }
    }

    deserializer.deserialize_any(Visitor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample() {
        let config = r##"
            [relabel]
            allow-unauthenticated = [
                "C-*"
            ]

            [assign]
            users_on_vacation = ["jyn514"]

            [note]

            [concern]
            labels = ["has-concerns"]

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

            [mentions."src/"]
            cc = ["@someone"]
            
            [mentions."target/"]
            message = "This is a message."
            cc = ["@someone"]
            
            [mentions."#[rustc_attr]"]
            type = "content"
            message = "This is a message."
            cc = ["@someone"]

            [shortcut]

            [issue-links]

            [rendered-link]
            trigger-files = ["posts/"]

            [no-mentions]
            exclude-titles = ["subtree update"]

            [behind-upstream]
            days-threshold = 14

            [backport.teamRed]
            required-pr-labels = ["T-libs", "T-libs-api"]
            required-issue-label = "regression-from-stable-to-stable"
            add-labels = ["stable-nominated"]

            [range-diff]

            [review-changes-since]
        "##;
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

        let mut backport_configs = HashMap::new();
        backport_configs.insert(
            "teamRed".into(),
            BackportRuleConfig {
                required_pr_labels: vec!["T-libs".into(), "T-libs-api".into()],
                required_issue_label: "regression-from-stable-to-stable".into(),
                add_labels: vec!["stable-nominated".into()],
            },
        );
        let backport_team_config = BackportConfig {
            configs: backport_configs,
        };

        assert_eq!(
            config,
            Config {
                relabel: Some(RelabelConfig {
                    allow_unauthenticated: vec!["C-*".into()],
                }),
                assign: Some(AssignConfig {
                    warn_non_default_branch: WarnNonDefaultBranchConfig::Simple(false),
                    contributing_url: None,
                    adhoc_groups: HashMap::new(),
                    owners: HashMap::new(),
                    users_on_vacation: HashSet::from(["jyn514".into()]),
                    review_prefs: None,
                    custom_messages: None,
                }),
                note: Some(NoteConfig { _empty: () }),
                ping: Some(PingConfig { teams: ping_teams }),
                nominate: Some(NominateConfig {
                    teams: nominate_teams
                }),
                shortcut: Some(ShortcutConfig { _empty: () }),
                prioritize: None,
                major_change: None,
                close: None,
                autolabel: None,
                notify_zulip: None,
                github_releases: None,
                review_submitted: None,
                review_requested: None,
                mentions: Some(MentionsConfig {
                    entries: HashMap::from([
                        (
                            "src/".to_string(),
                            MentionsEntryConfig {
                                type_: MentionsEntryType::Filename,
                                message: None,
                                cc: vec!["@someone".to_string()]
                            }
                        ),
                        (
                            "target/".to_string(),
                            MentionsEntryConfig {
                                type_: MentionsEntryType::Filename,
                                message: Some("This is a message.".to_string()),
                                cc: vec!["@someone".to_string()]
                            }
                        ),
                        (
                            "#[rustc_attr]".to_string(),
                            MentionsEntryConfig {
                                type_: MentionsEntryType::Content,
                                message: Some("This is a message.".to_string()),
                                cc: vec!["@someone".to_string()]
                            }
                        )
                    ])
                }),
                no_merges: None,
                pr_tracking: None,
                transfer: None,
                merge_conflicts: None,
                bot_pull_requests: None,
                rendered_link: Some(RenderedLinkConfig {
                    trigger_files: vec!["posts/".to_string()],
                    exclude_files: vec![],
                }),
                issue_links: Some(IssueLinksConfig {
                    check_commits: IssueLinksCheckCommitsConfig::All,
                }),
                no_mentions: Some(NoMentionsConfig {
                    exclude_titles: vec!["subtree update".into()],
                }),
                behind_upstream: Some(BehindUpstreamConfig {
                    days_threshold: Some(14),
                }),
                concern: Some(ConcernConfig {
                    labels: vec!["has-concerns".to_string()],
                }),
                backport: Some(backport_team_config),
                range_diff: Some(RangeDiffConfig {}),
                review_changes_since: Some(ReviewChangesSinceConfig {}),
            }
        );
    }

    #[test]
    fn warn_non_default_branch_and_issue_links() {
        let config = r#"
            [assign]
            warn_non_default_branch.enable = true

            [assign.custom_messages]
            auto-assign-someone = "Welcome message, assigning {assignee}!"
            auto-assign-no-one = "Welcome message for when no reviewer could be found!"

            [[assign.warn_non_default_branch.exceptions]]
            title = "[beta"
            branch = "beta"

            [[assign.warn_non_default_branch.exceptions]]
            title = "[stable"
            branch = "stable"

            [issue-links]
            check-commits = false

            [behind-upstream]
            days-threshold = 7
        "#;
        let config = toml::from_str::<Config>(&config).unwrap();
        assert_eq!(
            config,
            Config {
                relabel: None,
                assign: Some(AssignConfig {
                    warn_non_default_branch: WarnNonDefaultBranchConfig::Extended {
                        enable: true,
                        exceptions: vec![
                            WarnNonDefaultBranchException {
                                title: "[beta".to_string(),
                                branch: "beta".to_string()
                            },
                            WarnNonDefaultBranchException {
                                title: "[stable".to_string(),
                                branch: "stable".to_string()
                            },
                        ],
                    },
                    custom_messages: Some(AssignCustomMessages {
                        auto_assign_someone: Some(
                            "Welcome message, assigning {assignee}!".to_string()
                        ),
                        auto_assign_no_one: "Welcome message for when no reviewer could be found!"
                            .to_string()
                    }),
                    contributing_url: None,
                    adhoc_groups: HashMap::new(),
                    owners: HashMap::new(),
                    users_on_vacation: HashSet::new(),
                    review_prefs: None,
                }),
                note: None,
                ping: None,
                concern: None,
                nominate: None,
                shortcut: None,
                prioritize: None,
                major_change: None,
                close: None,
                autolabel: None,
                notify_zulip: None,
                github_releases: None,
                review_submitted: None,
                review_requested: None,
                mentions: None,
                no_merges: None,
                pr_tracking: None,
                transfer: None,
                merge_conflicts: None,
                bot_pull_requests: None,
                rendered_link: None,
                issue_links: Some(IssueLinksConfig {
                    check_commits: IssueLinksCheckCommitsConfig::Off,
                }),
                no_mentions: None,
                behind_upstream: Some(BehindUpstreamConfig {
                    days_threshold: Some(7),
                }),
                backport: None,
                range_diff: None,
                review_changes_since: None,
            }
        );
    }

    #[test]
    fn assign_review_prefs() {
        let config = r#"
            [assign.review_prefs]
        "#;
        let config = toml::from_str::<Config>(&config).unwrap();
        assert!(matches!(
            config.assign.and_then(|c| c.review_prefs),
            Some(AssignReviewPrefsConfig {})
        ));
    }

    #[test]
    fn assign_custom_welcome_message_old() {
        let config = r#"
            [assign.custom_welcome_messages]
            welcome-message-no-reviewer = "welcome message!"
        "#;
        let config = toml::from_str::<Config>(&config).unwrap();
        assert_eq!(
            config.assign.and_then(|c| c.custom_messages),
            Some(AssignCustomMessages {
                auto_assign_someone: None,
                auto_assign_no_one: "welcome message!".to_string(),
            })
        );
    }

    #[test]
    fn issue_links_uncanonicalized() {
        let config = r#"
            [issue-links]
            check-commits = "uncanonicalized"
        "#;
        let config = toml::from_str::<Config>(&config).unwrap();
        assert!(matches!(
            config.issue_links,
            Some(IssueLinksConfig {
                check_commits: IssueLinksCheckCommitsConfig::Uncanonicalized
            })
        ));
    }

    #[test]
    fn major_change() {
        let config = r#"
            [major-change]
            enabling_label = "major-change"
            meeting_label = "to-announce"
            second_label = "final-comment-period"
            concerns_label = "has-concerns"
            accept_label = "major-change-accepted"
            waiting_period = 1
            auto_closing = true
            zulip_stream = 224082
            zulip_ping = "Urgau"

            [major-change.tracking-issue-template]
            repository = "triagebot"
            title = "Tracking issue for MCP#${mcp_number}"
            body = """
Multi text body with ${mcp_issue} and ${mcp_title}
"""
            labels = ["C-tracking-issue", "T-compiler"]
        "#;
        let config = toml::from_str::<Config>(&config).unwrap();
        assert_eq!(
            config.major_change,
            Some(MajorChangeConfig {
                zulip_ping: "Urgau".to_string(),
                enabling_label: "major-change".to_string(),
                second_label: "final-comment-period".to_string(),
                accept_label: "major-change-accepted".to_string(),
                meeting_label: "to-announce".to_string(),
                concerns_label: Some("has-concerns".to_string()),
                waiting_period: 1,
                auto_closing: true,
                zulip_stream: 224082,
                open_extra_text: None,
                tracking_issue_template: Some(MajorChangeTrackingIssueTemplateConfig {
                    title: "Tracking issue for MCP#${mcp_number}".to_string(),
                    repository: Some("triagebot".to_string()),
                    body: "Multi text body with ${mcp_issue} and ${mcp_title}\n".to_string(),
                    labels: vec!["C-tracking-issue".to_string(), "T-compiler".to_string()],
                })
            })
        );
    }
}
