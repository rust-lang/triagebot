pub(crate) mod client;
pub(crate) mod issue;
pub(crate) mod issue_query;
pub(crate) mod queries;
pub(crate) mod repos;
pub(crate) mod utils;
mod webhook;

pub use client::GithubClient;
pub use issue::*;
pub use repos::*;
pub use webhook::event::*;
pub use webhook::webhook;

pub type UserId = u64;
pub type PullRequestNumber = u64;

/// GitHub repositories that opt into using triagebot with GitHub App authentication.
const REPOS_WITH_GITHUB_APP_AUTH: &[&str] = &["rust-lang/triagebot", "rust-lang/bors-kindergarten"];
