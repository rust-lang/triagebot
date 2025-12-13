use crate::errors::AppError;
use crate::github::{self, WorkflowRunJob};
use crate::handlers::Context;
use crate::interactions::REPORT_TO;
use crate::utils::is_repo_autorized;
use anyhow::Context as _;
use axum::extract::{Path, State};
use axum::http::HeaderValue;
use axum::response::IntoResponse;
use hyper::header::{CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE};
use hyper::{HeaderMap, StatusCode};
use std::collections::VecDeque;
use std::sync::Arc;
use uuid::Uuid;

pub const GHA_LOGS_JS: &str = include_str!("gha_logs/gha_logs.js");
pub const ANSI_UP_URL: &str = "/gha_logs/ansi_up@0.0.1-custom.js";
pub const SUCCESS_URL: &str = "/gha_logs/success@1.svg";
pub const FAILURE_URL: &str = "/gha_logs/failure@1.svg";

const MAX_CACHE_CAPACITY_BYTES: u64 = 50 * 1024 * 1024; // 50 Mb

#[derive(Default)]
pub struct GitHubActionLogsCache {
    capacity: u64,
    entries: VecDeque<(String, Arc<CachedLog>)>,
}

pub struct CachedLog {
    job: WorkflowRunJob,
    tree_roots: String,
    logs: String,
}

impl GitHubActionLogsCache {
    pub fn get(&mut self, key: &str) -> Option<Arc<CachedLog>> {
        if let Some(pos) = self.entries.iter().position(|(k, _)| k == key) {
            // Move previously cached entry to the front
            let entry = self.entries.remove(pos).unwrap();
            self.entries.push_front(entry.clone());
            Some(entry.1)
        } else {
            None
        }
    }

    pub fn put(&mut self, key: String, value: Arc<CachedLog>) -> Arc<CachedLog> {
        if value.logs.len() as u64 > MAX_CACHE_CAPACITY_BYTES {
            // Entry is too large, don't cache, return as is
            return value;
        }

        // Remove duplicate or last entry when necessary
        let removed = if let Some(pos) = self.entries.iter().position(|(k, _)| k == &key) {
            self.entries.remove(pos)
        } else if self.capacity + value.logs.len() as u64 >= MAX_CACHE_CAPACITY_BYTES {
            self.entries.pop_back()
        } else {
            None
        };
        if let Some(removed) = removed {
            self.capacity -= removed.1.logs.len() as u64;
        }

        // Add entry the front of the list ane return it
        self.capacity += value.logs.len() as u64;
        self.entries.push_front((key, value.clone()));
        value
    }
}

pub async fn gha_logs(
    Path((owner, repo, log_id)): Path<(String, String, u128)>,
    State(ctx): State<Arc<Context>>,
) -> axum::response::Result<impl IntoResponse, AppError> {
    if !is_repo_autorized(&ctx, &owner, &repo).await? {
        return Ok((
            StatusCode::UNAUTHORIZED,
            HeaderMap::new(),
            format!("repository `{owner}/{repo}` is not part of the Rust Project team repos"),
        ));
    }

    let log_uuid = format!("{owner}/{repo}${log_id}");

    let CachedLog {
        job,
        tree_roots,
        logs,
    } = &*'logs: {
        if let Some(logs) = ctx.gha_logs.write().await.get(&log_uuid) {
            tracing::info!("gha_logs: cache hit for log {log_uuid}");
            break 'logs logs;
        }

        tracing::info!("gha_logs: cache miss for log {log_uuid}");

        let repo = github::IssueRepository {
            organization: owner.to_string(),
            repository: repo.to_string(),
        };

        let job_and_tree_roots = async {
            let job = ctx
                .github
                .workflow_run_job(&repo, log_id)
                .await
                .with_context(|| format!("unable to fetch the job details for log {log_id}"))?;

            // To minimize false positives in paths linked to the GitHub repositories, we
            // restrict matching to only the second-level directories of the repository.
            //
            // We achieve this by retrieving the contents of the root repository and then
            // retrive the content of the top-level directory which we then serialize for
            // the JS so they can be escaped and concatenated into a regex OR pattern
            // (e.g., `compiler/rustc_ast|tests/ui|src/version`) which is used in the JS regex.
            let mut root_trees = ctx
                .github
                .repo_git_trees(&repo, &job.head_sha)
                .await
                .context("unable to fetch git tree for the repository")?;

            // Prune every entry that isn't a tree (aka directory)
            root_trees.tree.retain(|t| t.object_type == "tree");

            // Retrive all the sub-directories trees (for rust-lang/rust it's 6 API calls)
            let roots_trees: Vec<_> = root_trees
                .tree
                .iter()
                .map(|t| async { ctx.github.repo_git_trees(&repo, &t.sha).await })
                .collect();

            // Join all futures and fail fast if one of them returns an error
            let roots_trees = futures::future::try_join_all(roots_trees)
                .await
                .context("unable to fetch content details")?;

            // Collect and fix-up all the paths to directories and files (avoid submodules)
            let mut tree_roots: Vec<_> = root_trees
                .tree
                .iter()
                .zip(&roots_trees)
                .flat_map(|(root, childs)| {
                    childs
                        .tree
                        .iter()
                        .filter(|t| t.object_type == "tree" || t.object_type == "blob")
                        .map(|t| format!("{}/{}", root.path, t.path))
                })
                .collect();

            // We need to sort the tree roots by descending order, otherwise `library/std` will
            // be matched before `library/stdarch`
            tree_roots.sort_by(|a, b| b.cmp(a));

            // Serialize to a JS(ON) array so we can escape them in the browser
            let tree_roots =
                serde_json::to_string(&tree_roots).context("unable to serialize the tree roots")?;

            anyhow::Result::<_>::Ok((job, tree_roots))
        };

        let logs = async {
            let logs = ctx
                .github
                .raw_job_logs(&repo, log_id)
                .await
                .with_context(|| format!("unable to get the raw logs for log {log_id}"))?;

            let json_logs =
                serde_json::to_string(&*logs).context("unable to JSON-ify the raw logs")?;

            anyhow::Result::<_>::Ok(json_logs)
        };

        let (job_and_tree_roots, logs) = futures::join!(job_and_tree_roots, logs);
        let ((job, tree_roots), logs) = (job_and_tree_roots?, logs?);

        ctx.gha_logs.write().await.put(
            log_uuid.clone(),
            CachedLog {
                job,
                tree_roots,
                logs,
            }
            .into(),
        )
    };

    let nonce = Uuid::new_v4().to_hyphenated().to_string();
    let job_name = &*job.name;
    let sha = &*job.head_sha;
    let short_sha = &job.head_sha[..7];

    let icon_status = match job.conclusion {
        Some(github::JobConclusion::Failure | github::JobConclusion::TimedOut) => {
            format!(r#"<link rel="icon" sizes="any" type="image/svg+xml" href="{FAILURE_URL}">"#)
        }
        Some(github::JobConclusion::Success) => {
            format!(r#"<link rel="icon" sizes="any" type="image/svg+xml" href="{SUCCESS_URL}">"#)
        }
        _ => {
            r#"<link rel="icon" sizes="32x32" type="image/png" href="https://rust-lang.org/static/images/favicon-32x32.png">"#.to_string()
        }
    };

    let html = format!(
        r###"<!DOCTYPE html>
<html lang="en" translate="no">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{job_name} - {owner}/{repo}@{short_sha}</title>
    {icon_status}
    <style>
[data-pseudo-content]::before {{
  content: attr(data-pseudo-content);
}}
body {{
  font: 14px SFMono-Regular, Consolas, Liberation Mono, Menlo, monospace;
  background: #0C0C0C;
  color: #CCC;
}}
table {{
  white-space: pre;
  table-layout: fixed;
  width: 100%;
}}
.timestamp {{
  color: #848484;
  text-decoration: none;
}}
.timestamp:hover {{
  text-decoration: underline;
}}
.error-marker {{
  scroll-margin-bottom: 15vh;
  color: #e5534b;
}}
.warning-marker {{
  color: #c69026;
}}
.path-marker {{
  color: #26c6a8;
}}
.ansi-black-fg {{ color: rgb(0, 0, 0); }}
.ansi-red-fg {{ color: rgb(187, 0, 0); }}
.ansi-green-fg {{ color: rgb(0, 187, 0); }}
.ansi-yellow-fg {{ color: rgb(187, 187, 0); }}
.ansi-blue-fg {{ color: rgb(0, 0, 187); }}
.ansi-magenta-fg {{ color: rgb(187, 0, 187); }}
.ansi-cyan-fg {{ color: rgb(0, 187, 187); }}
.ansi-white-fg {{ color: rgb(255, 255, 255); }}
.ansi-bright-black-fg {{ color: rgb(85, 85, 85); }}
.ansi-bright-red-fg {{ color: rgb(255, 85, 85); }}
.ansi-bright-green-fg {{ color: rgb(0, 255, 0); }}
.ansi-bright-yellow-fg {{ color: rgb(255, 255, 85); }}
.ansi-bright-blue-fg {{ color: rgb(85, 85, 255); }}
.ansi-bright-magenta-fg {{ color: rgb(255, 85, 255); }}
.ansi-bright-cyan-fg {{ color: rgb(85, 255, 255); }}
.ansi-bright-white-fg {{ color: rgb(255, 255, 255); }}

.bold {{ font-weight: bold; }}
.faint {{ opacity: 0.7; }}
.italic {{ font-style: italic; }}
.underline {{ text-decoration: underline; }}
    </style>
    <script type="module" nonce="{nonce}">
        import {{ AnsiUp }} from '{ANSI_UP_URL}'
        
        try {{

        const logs = {logs};
        const tree_roots = {tree_roots};
        const owner = "{owner}";
        const repo = "{repo}";
        const sha = "{sha}";

        {GHA_LOGS_JS}

        }} catch (e) {{
           console.error(e);
           document.body.innerText = `Something went wrong: ${{e}}\n\n{REPORT_TO}`;
        }}
    </script>
</head>
<body>
<table>
    <colgroup>
        <col style="width: 29ch">
        <col style="width: 100%">
    </colgroup>
    <tbody id="logs">
    </tbody>
</table>
</body>
</html>"###,
    );

    tracing::info!("gha_logs: serving logs for {log_uuid}");

    let mut headers = HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    headers.insert(
        CONTENT_SECURITY_POLICY,
        HeaderValue::from_str(&format!(
            "default-src 'none'; script-src 'nonce-{nonce}' 'self'; style-src 'unsafe-inline'; img-src 'self' rust-lang.org"
        )).unwrap(),
    );

    Ok((StatusCode::OK, headers, html))
}

pub async fn ansi_up_min_js() -> impl IntoResponse {
    const ANSI_UP_MIN_JS: &str = include_str!("gha_logs/ansi_up@0.0.1-custom.js");

    (
        immutable_headers("text/javascript; charset=utf-8"),
        ANSI_UP_MIN_JS,
    )
}

pub async fn success_svg() -> impl IntoResponse {
    const SUCCESS_SVG: &str = include_str!("gha_logs/success.svg");

    (
        immutable_headers("image/svg+xml; charset=utf-8"),
        SUCCESS_SVG,
    )
}

pub async fn failure_svg() -> impl IntoResponse {
    const FAILURE_SVG: &str = include_str!("gha_logs/failure.svg");

    (
        immutable_headers("image/svg+xml; charset=utf-8"),
        FAILURE_SVG,
    )
}

fn immutable_headers(content_type: &'static str) -> HeaderMap {
    let mut headers = HeaderMap::new();

    headers.insert(
        CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=15552000, immutable"),
    );
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(content_type));

    headers
}
