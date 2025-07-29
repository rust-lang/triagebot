use crate::github::{self, WorkflowRunJob};
use crate::handlers::Context;
use anyhow::Context as _;
use hyper::header::{CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE};
use hyper::{Body, Response, StatusCode};
use itertools::Itertools;
use regex::Regex;
use std::collections::VecDeque;
use std::str::FromStr;
use std::sync::{Arc, LazyLock};
use uuid::Uuid;

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
    tree_roots: Vec<String>,
    logs: String,
}

// The `>` at the beginning of the regex is to match the closing `<br>` to ensure we're only
// matching timestamps at the beginning of each line and not some random one present in the logs.
pub static TIMESTAMP_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r">(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d+Z)").unwrap());

impl GitHubActionLogsCache {
    pub fn get(&mut self, key: &String) -> Option<Arc<CachedLog>> {
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
    ctx: Arc<Context>,
    owner: &str,
    repo: &str,
    log_id: &str,
) -> Result<Response<Body>, hyper::Error> {
    let res = process_logs(ctx, owner, repo, log_id).await;
    let res = match res {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("gha_logs: unable to serve logs for {owner}/{repo}#{log_id}: {e:?}");
            return Ok(Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from(format!("{:?}", e)))
                .unwrap());
        }
    };

    Ok(res)
}

async fn process_logs(
    ctx: Arc<Context>,
    owner: &str,
    repo: &str,
    log_id: &str,
) -> anyhow::Result<Response<Body>> {
    let log_id = u128::from_str(log_id).context("log_id is not a number")?;

    let repos = ctx
        .team
        .repos()
        .await
        .context("unable to retrieve team repos")?;

    let Some(repos) = repos.repos.get(owner) else {
        return Ok(bad_request(format!(
            "organization `{owner}` is not part of the Rust Project team repos"
        )));
    };

    if !repos.iter().any(|r| r.name == repo) {
        return Ok(bad_request(format!(
            "repository `{owner}` is not part of the Rust Project team repos"
        )));
    }

    let log_uuid = format!("{owner}/{repo}${log_id}");

    let CachedLog {
        job,
        tree_roots,
        logs,
    } = &*'logs: {
        if let Some(logs) = ctx.gha_logs.write().await.get(&log_uuid) {
            tracing::info!("gha_logs: cache hit for {log_uuid}");
            break 'logs logs;
        }

        tracing::info!("gha_logs: cache miss for {log_uuid}");

        let repo = github::IssueRepository {
            organization: owner.to_string(),
            repository: repo.to_string(),
        };

        let job_and_tree_roots = async {
            let job = ctx
                .github
                .workflow_run_job(&repo, log_id)
                .await
                .context("unable to fetch job details")?;

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
                .map(|(root, childs)| {
                    childs
                        .tree
                        .iter()
                        .filter(|t| t.object_type == "tree" || t.object_type == "blob")
                        .map(|t| format!("{}/{}", root.path, t.path))
                })
                .flatten()
                .collect();

            // We need to sort the tree roots by descending order, otherwise `library/std` will
            // be matched before `library/stdarch`
            tree_roots.sort_by(|a, b| b.cmp(a));

            anyhow::Result::<_>::Ok((job, tree_roots))
        };

        let logs = async {
            ctx.github
                .raw_job_logs(&repo, log_id)
                .await
                .context("unable to get the raw logs")
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
            r#"<link rel="icon" sizes="32x32" type="image/png" href="https://www.rust-lang.org/static/images/favicon-32x32.png">"#.to_string()
        }
    };

    let tree_roots = tree_roots.iter().map(|p| regex::escape(p)).join("|");
    let regex = format!(
        r#"(?<boundary>[^a-zA-Z0-9.\\/])(?<inner>(?:[\\/]?(?:checkout[\\/])?(?<path>(?:{tree_roots})(?:[\\/][a-zA-Z0-9_$\-.\\/]+)?))(?::(?<line>[0-9]+):(?<col>[0-9]+))?)"#,
    );
    let gha_logs_path_regex = Regex::new(&regex).unwrap();

    // 1. Remove UTF-8 useless BOM
    let logs = logs.strip_prefix('\u{FEFF}').unwrap_or(logs.as_str());

    // 2. Tranform the ANSI escape codes to HTML
    let anstyle_svg::HtmlParts { body, style } = anstyle_svg::Term::new()
        .use_html5(true)
        .render_html_parts(logs);

    // 3. Add a self-referencial anchor to all timestamps at the start of the lines
    let html = TIMESTAMP_REGEX.replace_all(&body, |ts: &regex::Captures| {
        let ts = &ts[0][1..];
        format!("><a id='{ts}' href='#{ts}' class='timestamp'>{ts}</a>")
    });

    // 4. Add a anchor around every "##[error]" string
    let mut error_counter = -1;
    let html = html
        .split("##[error]")
        .fold(String::with_capacity(html.len()), |mut acc, part| {
            // We only push the `<a>` tag if it's not the first iteration.
            if error_counter >= 0 {
                acc.push_str(&format!(
                    "<a id=\"error-{error_counter}\" class=\"error-marker\">##[error]</a>"
                ));
            }
            acc.push_str(part);
            error_counter += 1;
            acc
        });

    // 4.b Add a span around every "##[warning]" string
    let html = html.replace(
        "##[warning]",
        r#"<span class="warning-marker">##[warning]</span>"#,
    );

    // 5. Add anchors around some paths
    //  Detailed examples of what the regex does is at https://regex101.com/r/vCnx9Y/2
    //
    //  But simply speaking the regex tries to find absolute (with `/checkout` prefix) and
    //  relative paths, the path must start with one of the repository top-level directory.
    //  We also try to retrieve the lines and cols if given (`<path>:line:col`).
    //
    //  Some examples of paths we want to find:
    //   - src/tools/test-float-parse/src/traits.rs:173:11
    //   - /checkout/compiler/rustc_macros
    //   - /checkout/src/doc/rustdoc/src/advanced-features.md
    //
    //  Any other paths, in particular if prefixed by `./` or `obj/` should not taken.
    let html = gha_logs_path_regex.replace_all(&html, |capture: &regex::Captures| {
        let line = match capture.name("line") {
            Some(line) => format!("#L{}", line.as_str()),
            None => String::new(),
        };
        let boundary = capture.name("boundary").map_or("", |m| m.as_str());
        let inner = capture.name("inner").map_or("", |m| m.as_str());
        let path = &capture["path"];
        format!(r#"{boundary}<a href="https://github.com/{owner}/{repo}/blob/{sha}/{path}{line}" class="path-marker">{inner}</a>"#)
    });

    let nonce = Uuid::new_v4().to_hyphenated().to_string();
    let js = if error_counter >= 0 {
        format!(
            r#"
<script type="module" nonce="{nonce}">
    // 7. If no anchor is given, scroll to the last error
    if (location.hash === "") {{
        const hasSmallViewport = window.innerWidth <= 750;
        document.getElementById(`error-{error_counter}`).scrollIntoView({{
            behavior: 'instant',
            block: 'end',
            inline: hasSmallViewport ? 'start' : 'center'
        }});
    }}
</script>"#,
            error_counter = error_counter - 1,
        )
    } else {
        String::new()
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
body {{
    font: 14px SFMono-Regular, Consolas, Liberation Mono, Menlo, monospace;
    background: #0C0C0C;
    color: #CCC;
    word-break: break-all;
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
{style}
    </style>
</head>
<body>
{html}
{js}
</body>
</html>"###,
    );

    tracing::info!("gha_logs: serving logs for {log_uuid}");

    return Ok(Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "text/html; charset=utf-8")
        .header(
            CONTENT_SECURITY_POLICY,
            format!(
                "default-src 'none'; script-src 'nonce-{nonce}' 'self'; style-src 'unsafe-inline'; img-src 'self' www.rust-lang.org"
            ),
        )
        .body(Body::from(html))?);
}

pub fn success_svg() -> anyhow::Result<Response<Body>, hyper::Error> {
    const SUCCESS_SVG: &str = include_str!("gha_logs/success.svg");

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(CACHE_CONTROL, "public, max-age=15552000, immutable")
        .header(CONTENT_TYPE, "image/svg+xml; charset=utf-8")
        .body(Body::from(SUCCESS_SVG))
        .unwrap())
}

pub fn failure_svg() -> anyhow::Result<Response<Body>, hyper::Error> {
    const FAILURE_SVG: &str = include_str!("gha_logs/failure.svg");

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(CACHE_CONTROL, "public, max-age=15552000, immutable")
        .header(CONTENT_TYPE, "image/svg+xml; charset=utf-8")
        .body(Body::from(FAILURE_SVG))
        .unwrap())
}

fn bad_request(body: String) -> Response<Body> {
    Response::builder()
        .status(StatusCode::BAD_REQUEST)
        .body(Body::from(body))
        .unwrap()
}
