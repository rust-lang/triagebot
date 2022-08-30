//! The decision process command parser.
//!
//! This can parse arbitrary input, giving the user to be assigned.
//!
//! The grammar is as follows:
//!
//! ```text
//! Command: `@bot merge`, `@bot hold`, `@bot restart`, `@bot dissent`, `@bot stabilize` or `@bot close`.
//! ```

use crate::error::Error;
use crate::token::{Token, Tokenizer};
use std::fmt;

/// A command as parsed and received from calling the bot with some arguments,
/// like `@rustbot merge`
#[derive(PartialEq, Eq, Debug)]
pub struct DecisionCommand {
    user: String,
    disposition: Resolution,
    reversibility: Reversibility,
    issue_id: String,
    comment_id: String,
}


#[derive(Debug)]
pub enum Error {
    /// The first command that was given to this bot is not a valid one.
    /// Decision process must start with a resolution.
    InvalidFirstCommand,
}

use Error::*;

#[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
enum Reversibility {
    Reversible,
    Irreversible,
}

use Reversibility::*;

impl fmt::Display for Reversibility {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Reversible => write!(formatter, "a **reversible**"),
            Irreversible => writeln!(formatter, "an **irreversible**"),
        }
    }
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
enum Resolution {
    Hold,
    Custom(String),
}

use Resolution::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserStatus {
    name: String,
    issue_id: String,
    comment_id: String,
}

impl UserStatus {
    fn new(name: String, issue_id: String, comment_id: String) -> UserStatus {
        UserStatus {
            name,
            issue_id,
            comment_id,
        }
    }
}

impl fmt::Display for UserStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        write!(formatter, "{}", self.name)
    }
}
pub trait LinkRenderer {
    fn render_link(&self, data: &UserStatus) -> String;
}

#[derive(Debug, Serialize, Deserialize)]
pub struct State {
    initiator: String,
    team_members: Vec<String>,
    period_start: DateTime<Utc>,
    original_period_start: DateTime<Utc>,
    current_statuses: HashMap<String, UserStatus>,
    status_history: HashMap<String, Vec<UserStatus>>,
    reversibility: Reversibility,
    resolution: Resolution,
}

impl State {
    /// Renders the current state to the form it will have when seen as a
    /// comment in the github issue/PR
    pub fn render(&self, renderer: &impl LinkRenderer) -> String {
        let initiator = &self.initiator;
        let reversibility = self.reversibility.to_string();
        let comment = format!("Hello! {initiator} has proposed to merge this. This is {reversibility} decision, which means that it will be affirmed once the \"final comment period\" of 10 days have passed, unless a team member places a \"hold\" on the decision (or cancels it).\n\n");

        let mut table = String::from(if self.status_history.is_empty() {
            "| Team member | State |\n\
            |-------------|-------|\n"
        } else {
            "| Team member | History | State |\n\
            |-------------|---------|-------|\n"
        });

        for member in self.team_members.iter() {
            let current_status = self
                .current_statuses
                .get(member)
                .map(|status| {
                    let link = renderer.render_link(status);

                    format!("[{status}]({link})")
                })
                .unwrap_or_else(|| "".into());

            if self.status_history.is_empty() {
                table.push_str(&format!("| {member} | {current_status} |\n"));
            } else {
                let status_history = self
                    .status_history
                    .get(member)
                    .map(|statuses| {
                        statuses
                            .iter()
                            .map(|status| {
                                let link = renderer.render_link(status);

                                format!("[{status}]({link})")
                            })
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_else(|| "".into());

                table.push_str(&format!(
                    "| {member} | {status_history} | {current_status} |\n"
                ));
            }
        }

        comment + &table
    }
}

impl Command {
    #[cfg(test)]
    fn merge(user: String, issue_id: String, comment_id: String) -> Self {
        Self {
            user,
            issue_id,
            comment_id,
            disposition: Custom("merge".to_owned()),
            reversibility: Reversibility::Reversible,
        }
    }

    #[cfg(test)]
    fn hold(user: String, issue_id: String, comment_id: String) -> Self {
        Self {
            user,
            issue_id,
            comment_id,
            disposition: Hold,
            reversibility: Reversibility::Reversible,
        }
    }
}


pub struct Context {
    team_members: Vec<String>,
    now: DateTime<Utc>,
}

impl Context {
    pub fn new(team_members: Vec<String>, now: DateTime<Utc>) -> Context {
        Context { team_members, now }
    }
}
