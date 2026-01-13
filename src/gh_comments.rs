use std::fmt::Write;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Context as _;
use axum::{
    extract::{Path, State},
    http::HeaderValue,
    response::{IntoResponse, Response},
};
use chrono::Utc;
use hyper::{
    HeaderMap, StatusCode,
    header::{CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE},
};

use crate::{
    errors::AppError,
    github::GitHubSimplifiedAuthor,
    handlers::Context,
    utils::{immutable_headers, is_repo_autorized},
};

pub const STYLE_URL: &str = "/gh-comments/style@0.0.1.css";
pub const MARKDOWN_URL: &str = "/gh-comments/github-markdown@5.8.1.css";

pub async fn gh_comments(
    Path((owner, repo, issue_id)): Path<(String, String, u64)>,
    State(ctx): State<Arc<Context>>,
) -> axum::response::Result<Response, AppError> {
    if !is_repo_autorized(&ctx, &owner, &repo).await? {
        return Ok((
            StatusCode::UNAUTHORIZED,
            format!("repository `{owner}/{repo}` is not part of the Rust Project team repos"),
        )
            .into_response());
    }

    let start = Instant::now();

    let issue_with_comments = ctx
        .github
        .issue_with_comments(&owner, &repo, issue_id)
        .await
        .context("unable to fetch the issue and it's comments")?;

    let duration = start.elapsed();
    let duration_secs = duration.as_secs_f64();
    let comment_count = issue_with_comments.comments.nodes.len();

    let mut title = String::new();
    pulldown_cmark_escape::escape_html(&mut title, &issue_with_comments.title)?;

    let title_html = &issue_with_comments.title_html;

    let mut html = String::new();

    writeln!(
        html,
        r###"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>{title} - #{issue_id}</title>
  <link rel="icon" sizes="32x32" type="image/png" href="https://rust-lang.org/static/images/favicon-32x32.png">
  <link rel="stylesheet" href="{MARKDOWN_URL}" />
  <link rel="stylesheet" href="{STYLE_URL}" />
  <script nonce="triagebot-gh-comments">
    document.addEventListener('DOMContentLoaded', function() {{
      document.querySelectorAll('[data-utc-time]').forEach(element => {{
        const utcString = element.getAttribute('data-utc-time');
        const utcDate = new Date(utcString);
        element.textContent = utcDate.toLocaleString();
      }});
    }});
  </script>
</head>
<body>
<div class="comments-container">
<h1 class="markdown-body title">{title_html} #{issue_id}</h1>
<p>{comment_count} comments loaded in {duration_secs:.2}s</p>
"###,
    )
    .unwrap();

    write_comment_as_html(
        &mut html,
        &issue_with_comments.body_html,
        &issue_with_comments.url,
        &issue_with_comments.author,
        &issue_with_comments.created_at,
        &issue_with_comments.updated_at,
        false,
        None,
    )?;

    for comment in &issue_with_comments.comments.nodes {
        write_comment_as_html(
            &mut html,
            &comment.body_html,
            &comment.url,
            &comment.author,
            &comment.created_at,
            &comment.updated_at,
            comment.is_minimized,
            comment.minimized_reason.as_deref(),
        )?;
    }

    writeln!(html, r###"</div></body>"###).unwrap();

    let mut headers = HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    headers.insert(
        CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=30"),
    );
    headers.insert(
        CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'none'; script-src 'nonce-triagebot-gh-comments'; style-src 'self'; img-src *",
        ),
    );

    Ok((StatusCode::OK, headers, html).into_response())
}

pub async fn style_css() -> impl IntoResponse {
    const STYLE_CSS: &str = include_str!("gh_comments/style.css");

    (immutable_headers("text/css; charset=utf-8"), STYLE_CSS)
}

pub async fn markdown_css() -> impl IntoResponse {
    const MARKDOWN_CSS: &str = include_str!("gh_comments/github-markdown@5.8.1.css");

    (immutable_headers("text/css; charset=utf-8"), MARKDOWN_CSS)
}

fn write_comment_as_html(
    buffer: &mut String,
    body_html: &str,
    comment_url: &str,
    author: &GitHubSimplifiedAuthor,
    created_at: &chrono::DateTime<Utc>,
    updated_at: &chrono::DateTime<Utc>,
    minimized: bool,
    minimized_reason: Option<&str>,
) -> anyhow::Result<()> {
    let author_login = &author.login;
    let author_avatar_url = &author.avatar_url;
    let created_at_rfc3339 = created_at.to_rfc3339();

    if minimized && let Some(minimized_reason) = minimized_reason {
        writeln!(
            buffer,
            r###"
    <div class="comment-wrapper">
      <a href="https://github.com/{author_login}" target="_blank" class="desktop">
        <img src="{author_avatar_url}" alt="{author_login} Avatar" class="avatar">
      </a>
      
      <details class="comment">
        <summary class="comment-header">
          <div class="author-info desktop">
            <a href="https://github.com/{author_login}" target="_blank">{author_login}</a>
            <span>on <span data-utc-time="{created_at_rfc3339}">{created_at}</span></span><span> · hidden as {minimized_reason}</span>
          </div>

          <div class="author-mobile">
            <a href="https://github.com/{author_login}" target="_blank">
              <img src="{author_avatar_url}" alt="{author_login} Avatar" class="avatar">
            </a>
            <div class="author-info">
              <a href="https://github.com/{author_login}" target="_blank">{author_login}</a>
              <span>on <span data-utc-time="{created_at_rfc3339}">{created_at}</span></span><span> · hidden as {minimized_reason}</span>
            </div>
          </div>

          <a href="{comment_url}" target="_blank" class="github-link">View on GitHub</a>
        </summary>

        <div class="comment-body markdown-body">
          {body_html}
        </div>
      </details>
    </div>
"###
        )?;
    } else {
        let edited = if created_at != updated_at {
            "<span> · edited</span>"
        } else {
            ""
        };

        writeln!(
            buffer,
            r###"
    <div class="comment-wrapper">
      <a href="https://github.com/{author_login}" target="_blank" class="desktop">
        <img src="{author_avatar_url}" alt="{author_login} Avatar" class="avatar">
      </a>

      <div class="comment">
        <div class="comment-header">
          <div class="author-info desktop">
            <a href="https://github.com/{author_login}" target="_blank">{author_login}</a>
            <span>on <span data-utc-time="{created_at_rfc3339}">{created_at}</span></span>{edited}
          </div>

          <div class="author-mobile">
            <a href="https://github.com/{author_login}" target="_blank">
              <img src="{author_avatar_url}" alt="{author_login} Avatar" class="avatar">
            </a>
            <div class="author-info">
              <a href="https://github.com/{author_login}" target="_blank">{author_login}</a>
              <span>on <span data-utc-time="{created_at_rfc3339}">{created_at}</span></span>{edited}
            </div>
          </div>

          <a href="{comment_url}" target="_blank" class="github-link">View on GitHub</a>
        </div>

        <div class="comment-body markdown-body">
          {body_html}
        </div>
      </div>
    </div>
"###
        )?;
    }

    Ok(())
}
