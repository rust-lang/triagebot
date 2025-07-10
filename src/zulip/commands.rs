use crate::db::notifications::Identifier;
use crate::db::review_prefs::RotationMode;
use clap::{ColorChoice, Parser};
use std::num::NonZeroU32;
use std::str::FromStr;

/// Command sent in a DM with triagebot on Zulip.
#[derive(clap::Parser, Debug, PartialEq)]
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
    /// Ping project goal owners.
    PingGoals(PingGoalsArgs),
    /// Update docs
    DocsUpdate,
    /// Shows review queue statistics of members of the given Rust team.
    TeamStats {
        /// Name of the team to query.
        name: String,
    },
}

#[derive(clap::Parser, Debug, PartialEq)]
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

#[derive(clap::Parser, Debug, PartialEq)]
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

#[derive(Debug, Clone, PartialEq)]
pub enum WorkqueueLimit {
    Unlimited,
    Limit(u32),
}

impl FromStr for WorkqueueLimit {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "unlimited" => Ok(Self::Unlimited),
            v => {
                v.parse::<u32>()
                    .map_err(|_| "Wrong parameter format. Must be a positive integer or `unlimited` to unset the limit.".to_string())
                    .map(WorkqueueLimit::Limit)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
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

#[derive(Debug, Clone, PartialEq)]
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
#[derive(clap::Parser, Debug, PartialEq)]
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
    PingGoals(PingGoalsArgs),
    /// Update docs
    DocsUpdate,
}

#[derive(clap::Parser, Debug, PartialEq, Clone)]
pub struct PingGoalsArgs {
    /// Number of days before an update is considered stale
    pub threshold: u64,
    /// Date of next update
    pub next_update: String,
}

/// Helper function to parse CLI arguments without any colored help or error output.
pub fn parse_cli<'a, T: Parser, I: Iterator<Item = &'a str>>(input: I) -> anyhow::Result<T> {
    // Add a fake first argument, which is expected by clap
    let input = std::iter::once("triagebot").chain(input);

    let matches = T::command()
        .color(ColorChoice::Never)
        .try_get_matches_from(input)?;
    let value = T::from_arg_matches(&matches)?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acknowledge_command() {
        assert_eq!(
            parse_chat(&["acknowledge", "1"]),
            ChatCommand::Acknowledge {
                identifier: IdentifierCli::Index(NonZeroU32::new(1).unwrap())
            }
        );
    }

    #[test]
    fn add_command() {
        assert_eq!(
            parse_chat(&["add", "https://example.com", "test", "description"]),
            ChatCommand::Add {
                url: "https://example.com".to_string(),
                description: vec!["test".to_string(), "description".to_string()]
            }
        );
    }

    #[test]
    fn move_command() {
        assert_eq!(
            parse_chat(&["move", "1", "2"]),
            ChatCommand::Move { from: 1, to: 2 }
        );
    }

    #[test]
    fn meta_command() {
        assert_eq!(
            parse_chat(&["meta", "1", "test", "meta"]),
            ChatCommand::Meta {
                index: 1,
                description: vec!["test".to_string(), "meta".to_string()]
            }
        );
    }

    #[test]
    fn whoami_command() {
        assert_eq!(parse_chat(&["whoami"]), ChatCommand::Whoami);
    }

    #[test]
    fn lookup_command() {
        assert_eq!(
            parse_chat(&["lookup", "zulip", "username"]),
            ChatCommand::Lookup(LookupCmd::Zulip {
                github_username: "username".to_string()
            })
        );
    }

    #[test]
    fn work_command() {
        assert_eq!(
            parse_chat(&["work", "show"]),
            ChatCommand::Work(WorkqueueCmd::Show)
        );

        assert_eq!(
            parse_chat(&["work", "set-pr-limit", "unlimited"]),
            ChatCommand::Work(WorkqueueCmd::SetPrLimit {
                limit: WorkqueueLimit::Unlimited
            })
        );
    }

    #[test]
    fn end_meeting_command() {
        assert_eq!(parse_stream(&["end-meeting"]), StreamCommand::EndMeeting);
        assert_eq!(parse_stream(&["await"]), StreamCommand::EndTopic);
    }

    fn parse_chat(input: &[&str]) -> ChatCommand {
        parse_cli::<ChatCommand, _>(input.into_iter().copied()).unwrap()
    }

    fn parse_stream(input: &[&str]) -> StreamCommand {
        parse_cli::<StreamCommand, _>(input.into_iter().copied()).unwrap()
    }
}
