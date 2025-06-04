use crate::db::notifications::Identifier;
use crate::db::review_prefs::RotationMode;
use clap::{ColorChoice, Parser};
use std::num::NonZeroU32;
use std::str::FromStr;

/// Command sent in a DM with triagebot on Zulip.
#[derive(clap::Parser, Debug)]
#[clap(override_usage("<command>"), disable_colored_help(true))]
pub enum ChatCommand {
    /// Acknowledge a notification
    #[clap(alias = "ack")]
    Acknowledge {
        /// Notification identifier. `*`, `all`, non-zero index or a URL.
        identifier: IdentifierCli,
    },
    /// Add a notification
    Add {
        url: String,
        #[clap(trailing_var_arg(true))]
        description: Vec<String>,
    },
    /// Move a notification
    Move { from: u32, to: u32 },
    /// Add meta notification
    Meta {
        index: u32,
        #[clap(trailing_var_arg(true))]
        description: Vec<String>,
    },
    /// Output your membership in Rust teams.
    Whoami,
    /// Perform lookup of GitHub or Zulip username.
    #[clap(subcommand)]
    Lookup(LookupCmd),
    /// Inspect or modify your reviewer workqueue.
    #[clap(subcommand)]
    Work(WorkqueueCmd),
}

#[derive(clap::Parser, Debug)]
pub enum LookupCmd {
    /// Try to find the Zulip name of a user with the provided GitHub username.
    Zulip {
        /// GitHub username to lookup the Zulip user from.
        github_username: String,
    },
    ///  Try to find the GitHub username of a user with the provided Zulip name.
    GitHub {
        /// Zulip name to lookup the GitHub username from.
        // Zulip usernames could contain spaces, so take everything to the end of the input
        #[clap(trailing_var_arg(true))]
        zulip_username: Vec<String>,
    },
}

#[derive(clap::Parser, Debug)]
pub enum WorkqueueCmd {
    /// Show the current state of your workqueue
    Show,
    /// Set the maximum capacity limit of your workqueue.
    SetPrLimit {
        /// Workqueue capacity
        limit: WorkqueueLimit,
    },
    /// Set your rotation mode (`on` rotation or `off` rotation).
    SetRotationMode {
        /// Rotation mode
        rotation_mode: RotationModeCli,
    },
}

#[derive(Debug, Clone)]
pub enum WorkqueueLimit {
    Unlimited,
    Limit(u32),
}

impl FromStr for WorkqueueLimit {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "unlimited" => Ok(Self::Unlimited),
            v => v.parse::<u32>().map_err(|_|
                                              "Wrong parameter format. Must be a positive integer or `unlimited` to unset the limit.".to_string(),
            ).map(WorkqueueLimit::Limit)
        }
    }
}

#[derive(Debug, Clone)]
pub struct RotationModeCli(pub RotationMode);

impl FromStr for RotationModeCli {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "on" => Ok(Self(RotationMode::OnRotation)),
            "off" => Ok(Self(RotationMode::OffRotation)),
            _ => Err("Invalid value for rotation mode. Must be `on` or `off`.".to_string()),
        }
    }
}

#[derive(Debug, Clone)]
pub enum IdentifierCli {
    Url(String),
    Index(NonZeroU32),
    All,
}

impl<'a> From<&'a IdentifierCli> for Identifier<'a> {
    fn from(value: &'a IdentifierCli) -> Self {
        match value {
            IdentifierCli::Url(url) => Self::Url(&url),
            IdentifierCli::Index(index) => Self::Index(*index),
            IdentifierCli::All => Self::All,
        }
    }
}

impl FromStr for IdentifierCli {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "all" | "*" => Ok(Self::All),
            v => match v.parse::<u32>() {
                Ok(v) => NonZeroU32::new(v)
                    .ok_or_else(|| "index must be at least 1".to_string())
                    .map(Self::Index),
                Err(_) => Ok(Self::Url(v.to_string())),
            },
        }
    }
}

/// Command sent in a Zulip stream after `@**triagebot**`.
#[derive(clap::Parser, Debug)]
#[clap(override_usage("`@triagebot <command>`"), disable_colored_help(true))]
pub enum StreamCommand {
    /// End the current topic.
    #[clap(alias = "await")]
    EndTopic,
    /// End the current meeting.
    EndMeeting,
    /// Read a document.
    Read,
    /// Ping project goal owners.
    PingGoals {
        /// Number of days before an update is considered stale
        threshold: u64,
        /// Date of next update
        next_update: String,
    },
    /// Update docs
    DocsUpdate,
}

/// Helper function to parse CLI arguments without any colored help or error output.
pub fn parse_no_color<'a, T: Parser, I: Iterator<Item = &'a str>>(input: I) -> anyhow::Result<T> {
    let matches = T::command()
        .color(ColorChoice::Never)
        .try_get_matches_from(input)?;
    let value = T::from_arg_matches(&matches)?;
    Ok(value)
}
