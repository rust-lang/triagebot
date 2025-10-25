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
    Message(String),
}

impl std::error::Error for UserError {}

impl fmt::Display for UserError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            UserError::Message(msg) => f.write_str(msg),
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
        tracing::error!("{:?}", &self.0);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Something went wrong: {}\n\n{REPORT_TO}", self.0),
        )
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
