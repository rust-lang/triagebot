use crate::github::{GithubClient, Issue};
use failure::Error;
use std::fmt::Write;

pub struct ErrorComment<'a> {
    issue: &'a Issue,
    message: String,
}

impl<'a> ErrorComment<'a> {
    pub fn new<T>(issue: &'a Issue, message: T) -> ErrorComment<'a>
    where
        T: Into<String>,
    {
        ErrorComment {
            issue,
            message: message.into(),
        }
    }

    pub fn post(&self, client: &GithubClient) -> Result<(), Error> {
        let mut body = String::new();
        writeln!(body, "**Error**: {}", self.message)?;
        writeln!(body)?;
        writeln!(
            body,
            "Please let **`@rust-lang/release`** know if you're having trouble with this bot."
        )?;
        self.issue.post_comment(client, &body)
    }
}
