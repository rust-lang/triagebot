use crate::{handlers::Context, interactions::REPORT_TO};

use anyhow::Context as _;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::borrow::Cow;

/// Pluralize (add an 's' sufix) to `text` based on `count`.
pub fn pluralize(text: &str, count: usize) -> Cow<'_, str> {
    if count == 1 {
        text.into()
    } else {
        format!("{text}s").into()
    }
}

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

pub(crate) async fn is_repo_autorized(
    ctx: &Context,
    owner: &str,
    repo: &str,
) -> anyhow::Result<bool> {
    let repos = ctx
        .team
        .repos()
        .await
        .context("unable to retrieve team repos")?;

    // Verify that the request org is part of the Rust project
    let Some(repos) = repos.repos.get(owner) else {
        return Ok(false);
    };

    // Verify that the request repo is part of the Rust project
    if !repos.iter().any(|r| r.name == repo) {
        return Ok(false);
    }

    Ok(true)
}
