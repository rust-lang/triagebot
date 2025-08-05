use std::sync::Arc;

use axum::{Json, extract::State};

use crate::{db, handlers::Context, utils::AppError};

pub async fn bors_commit_list(
    State(ctx): State<Arc<Context>>,
) -> axum::response::Result<Json<Vec<db::rustc_commits::Commit>>, AppError> {
    Ok(Json(
        db::rustc_commits::get_commits_with_artifacts(&*ctx.db.get().await).await?,
    ))
}
