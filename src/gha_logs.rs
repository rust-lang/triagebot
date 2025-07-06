use crate::github;
use crate::handlers::Context;
use anyhow::Context as _;
use hyper::header::{CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE};
use hyper::{Body, Response, StatusCode};
use std::collections::VecDeque;
use std::str::FromStr;
use std::sync::Arc;
use uuid::Uuid;

pub const ANSI_UP_URL: &str = "/gha_logs/ansi_up@6.0.6.min.js";
const MAX_CACHE_CAPACITY_BYTES: u64 = 50 * 1024 * 1024; // 50 Mb

#[derive(Default)]
pub struct GitHubActionLogsCache {
    capacity: u64,
    entries: VecDeque<(String, Arc<String>)>,
}

impl GitHubActionLogsCache {
    pub fn get(&mut self, key: &String) -> Option<Arc<String>> {
        if let Some(pos) = self.entries.iter().position(|(k, _)| k == key) {
            // Move previously cached entry to the front
            let entry = self.entries.remove(pos).unwrap();
            self.entries.push_front(entry.clone());
            Some(entry.1)
        } else {
            None
        }
    }

    pub fn put(&mut self, key: String, value: Arc<String>) -> Arc<String> {
        if value.len() as u64 > MAX_CACHE_CAPACITY_BYTES {
            // Entry is too large, don't cache, return as is
            return value;
        }

        // Remove duplicate or last entry when necessary
        let removed = if let Some(pos) = self.entries.iter().position(|(k, _)| k == &key) {
            self.entries.remove(pos)
        } else if self.capacity + value.len() as u64 >= MAX_CACHE_CAPACITY_BYTES {
            self.entries.pop_back()
        } else {
            None
        };
        if let Some(removed) = removed {
            self.capacity -= removed.1.len() as u64;
        }

        // Add entry the front of the list ane return it
        self.capacity += value.len() as u64;
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
        anyhow::bail!("Organization `{owner}` is not part of team repos")
    };

    if !repos.iter().any(|r| r.name == repo) {
        anyhow::bail!("Repository `{repo}` is not part of team repos");
    }

    let log_uuid = format!("{owner}/{repo}${log_id}");

    let logs = 'logs: {
        if let Some(logs) = ctx.gha_logs.write().await.get(&log_uuid) {
            tracing::info!("gha_logs: cache hit for {log_uuid}");
            break 'logs logs;
        }

        tracing::info!("gha_logs: cache miss for {log_uuid}");
        let logs = ctx
            .github
            .raw_job_logs(
                &github::IssueRepository {
                    organization: owner.to_string(),
                    repository: repo.to_string(),
                },
                log_id,
            )
            .await
            .context("unable to get the raw logs")?;

        let json_logs = serde_json::to_string(&*logs).context("unable to JSON-ify the raw logs")?;

        ctx.gha_logs
            .write()
            .await
            .put(log_uuid.clone(), json_logs.into())
    };

    let nonce = Uuid::new_v4().to_hyphenated().to_string();

    let html = format!(
        r###"<!DOCTYPE html>
<html>
<head>
    <title>{log_uuid} - triagebot</title>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <link rel="icon" sizes="32x32" type="image/png" href="https://rust-lang.org/static/images/favicon-32x32.png">    
    <style>
        body {{
            font: 14px SFMono-Regular, Consolas, Liberation Mono, Menlo, monospace;
            background: #0C0C0C;
            color: #CCCCCC;
            white-space: pre;
        }}
        .timestamp {{
            color: unset;
            text-decoration: none;
        }}
        .timestamp:hover {{
            text-decoration: underline;
        }}
        .error-marker {{
            scroll-margin-bottom: 15vh;
        }}
    </style>
    <script type="module" nonce="{nonce}">
        import {{ AnsiUp }} from '{ANSI_UP_URL}'

        var logs = {logs};
        var ansi_up = new AnsiUp();

        // 1. Tranform the ANSI escape codes to HTML
        var html = ansi_up.ansi_to_html(logs);

        // 2. Remove UTF-8 useless BOM
        if (html.charCodeAt(0) === 0xFEFF) {{
            html = html.substr(1);
        }}

        // 3. Add a self-referencial anchor to all timestamps at the start of the lines
        const dateRegex = /^(\d{{4}}-\d{{2}}-\d{{2}}T\d{{2}}:\d{{2}}:\d{{2}}\.\d+Z)/gm;
        html = html.replace(dateRegex, (ts) => 
            `<a id="${{ts}}" href="#${{ts}}" class="timestamp">${{ts}}</a>`
        );

        // 4. Add a anchor around every "##[error]" string
        let errorCounter = -1;
        html = html.replace(/##\[error\]/g, () =>
            `<a id="error-${{++errorCounter}}" class="error-marker">##[error]</a>`
        );

        // 5. Add the html to the DOM
        var cdiv = document.getElementById("console");
        cdiv.innerHTML = html;
        
        // 6. If no anchor is given, scroll to the last error
        if (location.hash === "" && errorCounter >= 0) {{
            const hasSmallViewport = window.innerWidth <= 750;
            document.getElementById(`error-${{errorCounter}}`).scrollIntoView({{
                behavior: 'instant',
                block: 'end',
                inline: hasSmallViewport ? 'start' : 'center'
            }});
        }}
    </script>
</head>
<body id="console">
</body>
</html>"###,
    );

    tracing::info!("gha_logs: serving logs for {log_uuid}");

    return Ok(Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "text/html; charset=utf-8")
        .header(
            CONTENT_SECURITY_POLICY,
            format!("script-src 'nonce-{nonce}' {ANSI_UP_URL}"),
        )
        .body(Body::from(html))?);
}

pub fn ansi_up_min_js() -> anyhow::Result<Response<Body>, hyper::Error> {
    const ANSI_UP_MIN_JS: &str = include_str!("gha_logs/ansi_up@6.0.6.min.js");

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(CACHE_CONTROL, "public, max-age=15552000, immutable")
        .header(CONTENT_TYPE, "text/javascript; charset=utf-8")
        .body(Body::from(ANSI_UP_MIN_JS))
        .unwrap())
}
