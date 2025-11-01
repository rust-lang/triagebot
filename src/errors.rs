//! Errors handling

use std::fmt;

use crate::interactions::REPORT_TO;

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};

/// Represent a user error.
///
/// The message will be shown to the user via comment posted by this bot.
#[derive(Debug)]
pub enum UserError {
    /// Simple message
    Message(String),
    /// Unknown labels
    UnknownLabels { labels: Vec<String> },
    /// Invalid assignee
    InvalidAssignee,
}

impl std::error::Error for UserError {}

// NOTE: This is used to post the Github comment; make sure it's valid markdown.
impl fmt::Display for UserError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            UserError::Message(msg) => f.write_str(msg),
            UserError::UnknownLabels { labels } => {
                write!(f, "Unknown labels: {}", labels.join(", "))
            }
            UserError::InvalidAssignee => write!(f, "invalid assignee"),
        }
    }
}

/// Creates a [`UserError`] with message.
///
/// Should be used when an handler is in error due to the user action's (not a PR,
/// not a issue, not authorized, ...).
///
/// Should be used like this `return user_error!("My error message.");`.
macro_rules! user_error {
    ($err:expr $(,)?) => {
        anyhow::Result::Err(anyhow::anyhow!(crate::errors::UserError::Message(
            $err.into()
        )))
    };
}

// export the macro
pub(crate) use user_error;

/// Represent a application error.
///
/// Useful for returning a error via the API
pub struct AppError(anyhow::Error);

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        tracing::error!("app error: {:?}", &self.0);

        // Let's avoid returning 500 when it's a network error and instead return the status
        // code of the network request (useful for GHA logs which can get 404, 410 from GitHub).
        if let Some(err) = self.0.downcast_ref::<reqwest::Error>()
            && let Some(status) = err.status()
        {
            (
                status,
                format!(
                    "Something went wrong: {}\n\nNetwork error: {err}\n\n{REPORT_TO}",
                    self.0
                ),
            )
        } else {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Something went wrong: {}\n\n{REPORT_TO}", self.0),
            )
        }
        .into_response()
    }
}

impl<E> From<E> for AppError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        AppError(err.into())
    }
}

/// Represent an error when trying to assign someone
#[derive(Debug)]
pub enum AssignmentError {
    InvalidAssignee,
    Other(anyhow::Error),
}

// NOTE: This is used to post the Github comment; make sure it's valid markdown.
impl fmt::Display for AssignmentError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            AssignmentError::InvalidAssignee => write!(f, "invalid assignee"),
            AssignmentError::Other(err) => write!(f, "{err}"),
        }
    }
}

impl From<AssignmentError> for anyhow::Error {
    fn from(a: AssignmentError) -> Self {
        match a {
            AssignmentError::InvalidAssignee => UserError::InvalidAssignee.into(),
            AssignmentError::Other(err) => err.context("assignment error"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_labels() {
        let x = UserError::UnknownLabels {
            labels: vec!["A-bootstrap".into(), "xxx".into()],
        };
        assert_eq!(x.to_string(), "Unknown labels: A-bootstrap, xxx");
    }
}
