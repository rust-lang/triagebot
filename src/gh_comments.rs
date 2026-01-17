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
    cache,
    github::{
        GitHubGraphQlComment, GitHubGraphQlReviewThreadComment, GitHubIssueWithComments,
        GitHubReviewState,
    },
};
use crate::{
    errors::AppError,
    github::GitHubSimplifiedAuthor,
    handlers::Context,
    utils::{immutable_headers, is_repo_autorized},
};

pub const STYLE_URL: &str = "/gh-comments/style@0.0.2.css";
pub const MARKDOWN_URL: &str = "/gh-comments/github-markdown@20260115.css";

pub const GH_COMMENTS_CACHE_CAPACITY_BYTES: usize = 35 * 1024 * 1024; // 35 Mb

pub type GitHubCommentsCache = cache::LeastRecentlyUsedCache<(String, String, u64), CachedComments>;

pub struct CachedComments {
    estimated_size: usize,
    duration_secs: f64,
    issue_with_comments: GitHubIssueWithComments,
}

impl cache::EstimatedSize for CachedComments {
    fn estimated_size(&self) -> usize {
        self.estimated_size
    }
}

pub async fn gh_comments(
    Path(ref key @ (ref owner, ref repo, issue_id)): Path<(String, String, u64)>,
    State(ctx): State<Arc<Context>>,
) -> axum::response::Result<Response, AppError> {
    if !is_repo_autorized(&ctx, &owner, &repo).await? {
        return Ok((
            StatusCode::UNAUTHORIZED,
            format!("repository `{owner}/{repo}` is not part of the Rust Project team repos"),
        )
            .into_response());
    }

    let CachedComments {
        estimated_size: _,
        duration_secs,
        issue_with_comments,
    } = &*'comments: {
        if let Some(logs) = ctx.gh_comments.write().await.get(&key) {
            tracing::info!("gh_comments: cache hit for issue #{issue_id}");
            break 'comments logs;
        }

        tracing::info!("gh_comments: cache miss for issue #{issue_id}");

        let start = Instant::now();

        let mut issue_with_comments = ctx
            .github
            .issue_with_comments(&owner, &repo, issue_id)
            .await
            .context("unable to fetch the issue/pull-request and it's comments")?;

        let duration = start.elapsed();
        let duration_secs = duration.as_secs_f64();

        // Filter-out reviews that either don't have a body or aren't linked by the first
        // comment of a review comments.
        //
        // We need to do that otherwise we will end up showing "intermediate" review when
        // someone reply in a review thread.
        if let (Some(reviews), Some(review_threads)) = (
            issue_with_comments.reviews.as_mut(),
            issue_with_comments.review_threads.as_ref(),
        ) {
            reviews.nodes.retain(|r| {
                !r.body_html.is_empty()
                    || review_threads
                        .nodes
                        .iter()
                        .any(|rt| rt.comments.nodes[0].pull_request_review.id == r.id)
            });
        }

        // Rough estimation of the byte size of the issue with comments
        let estimated_size: usize = std::mem::size_of::<GitHubIssueWithComments>()
            + issue_with_comments.url.len()
            + issue_with_comments.title.len()
            + issue_with_comments.body_html.len()
            + issue_with_comments.title_html.len()
            + issue_with_comments
                .comments
                .nodes
                .iter()
                .map(|c| {
                    std::mem::size_of::<GitHubGraphQlComment>()
                        + c.url.len()
                        + c.body_html.len()
                        + c.author.login.len()
                        + c.author.avatar_url.len()
                })
                .sum::<usize>();

        ctx.gh_comments.write().await.put(
            key.clone(),
            CachedComments {
                estimated_size,
                duration_secs,
                issue_with_comments,
            }
            .into(),
        )
    };

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

    if let (Some(reviews), Some(review_threads)) = (
        issue_with_comments.reviews.as_ref(),
        issue_with_comments.review_threads.as_ref(),
    ) {
        // A pull-request
        enum Item {
            Comment(usize, chrono::DateTime<Utc>),
            Review(usize, chrono::DateTime<Utc>),
        }

        // Create the timeline
        let mut timeline: Vec<_> = issue_with_comments
            .comments
            .nodes
            .iter()
            .enumerate()
            .map(|(i, c)| Item::Comment(i, c.created_at))
            .collect();
        timeline.extend(
            reviews
                .nodes
                .iter()
                .enumerate()
                .map(|(i, r)| Item::Review(i, r.submitted_at)),
        );
        timeline.sort_unstable_by_key(|i| match i {
            Item::Comment(_i, created_at) => *created_at,
            Item::Review(_i, submitted_at) => *submitted_at,
        });

        // Print the items
        for item in timeline {
            match item {
                Item::Comment(pos, _) => {
                    let comment = &issue_with_comments.comments.nodes[pos];

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
                Item::Review(pos, _) => {
                    let review = &reviews.nodes[pos];

                    write_review_as_html(
                        &mut html,
                        &review.body_html,
                        &review.url,
                        &review.author,
                        review.state,
                        &review.submitted_at,
                        &review.updated_at,
                        review.is_minimized,
                        review.minimized_reason.as_deref(),
                    )?;

                    // Try to print the associated review threads
                    for review_thread in review_threads
                        .nodes
                        .iter()
                        .filter(|rt| rt.comments.nodes[0].pull_request_review.id == review.id)
                    {
                        write_review_thread_as_html(
                            &mut html,
                            &review_thread.path,
                            review_thread.is_collapsed,
                            review_thread.is_resolved,
                            review_thread.is_outdated,
                            &review_thread.comments.nodes,
                        )?;
                    }
                }
            }
        }
    } else {
        // An issue
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
            "default-src 'none'; script-src 'nonce-triagebot-gh-comments'; style-src 'self' 'unsafe-inline'; img-src *",
        ),
    );

    Ok((StatusCode::OK, headers, html).into_response())
}

pub async fn style_css() -> impl IntoResponse {
    const STYLE_CSS: &str = include_str!("gh_comments/style.css");

    (immutable_headers("text/css; charset=utf-8"), STYLE_CSS)
}

pub async fn markdown_css() -> impl IntoResponse {
    const MARKDOWN_CSS: &str = include_str!("gh_comments/github-markdown@20260115.css");

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

fn write_review_as_html(
    buffer: &mut String,
    body_html: &str,
    review_url: &str,
    author: &GitHubSimplifiedAuthor,
    state: GitHubReviewState,
    submitted_at: &chrono::DateTime<Utc>,
    updated_at: &chrono::DateTime<Utc>,
    minimized: bool,
    minimized_reason: Option<&str>,
) -> anyhow::Result<()> {
    let author_login = &author.login;
    let author_avatar_url = &author.avatar_url;
    let submitted_at_rfc3339 = submitted_at.to_rfc3339();

    writeln!(
        buffer,
        r###"
    <div class="review">
      <a href="https://github.com/{author_login}" target="_blank">
        <img src="{author_avatar_url}" alt="{author_login} Avatar" class="avatar">
      </a>
      
      <div class="review-header">
        <div class="author-info">
          <a href="https://github.com/{author_login}" target="_blank">{author_login}</a>
          <span>{state:?} on <span data-utc-time="{submitted_at_rfc3339}">{submitted_at}</span></span>
        </div>
      </div>
    </div>
"###
    )?;

    if !body_html.is_empty() {
        if minimized && let Some(minimized_reason) = minimized_reason {
            writeln!(
                buffer,
                r###"
    <div class="comment-wrapper">
      <div class="avatar desktop"></div>
      <details class="comment">
        <summary class="comment-header">
          <div class="author-info">
            <a href="https://github.com/{author_login}" target="_blank">{author_login}</a>
            <span>left a comment · hidden as {minimized_reason}</span>
          </div>

          <a href="{review_url}" target="_blank" class="github-link">View on GitHub</a>
        </summary>

        <div class="comment-body markdown-body">
          {body_html}
        </div>
      </details>
    </div>
"###
            )?;
        } else {
            let edited = if submitted_at != updated_at {
                "<span> · edited</span>"
            } else {
                ""
            };

            writeln!(
                buffer,
                r###"
    <div class="comment-wrapper">
      <div class="avatar desktop"></div>
      <div class="comment">
        <div class="comment-header">
          <div class="author-info">
            <a href="https://github.com/{author_login}" target="_blank">{author_login}</a>
            <span>left a comment</span>{edited}
          </div>

          <a href="{review_url}" target="_blank" class="github-link">View on GitHub</a>
        </div>

        <div class="comment-body markdown-body">
          {body_html}
        </div>
      </div>
    </div>
"###
            )?;
        }
    }

    Ok(())
}

fn write_review_thread_as_html(
    buffer: &mut String,
    path: &str,
    is_collapsed: bool,
    is_resolved: bool,
    is_outdated: bool,
    comments: &[GitHubGraphQlReviewThreadComment],
) -> anyhow::Result<()> {
    let mut path_html = String::new();
    pulldown_cmark_escape::escape_html(&mut path_html, &path)?;

    let open = if is_collapsed { "" } else { "open" };
    let status = if is_outdated {
        " · outdated"
    } else if is_resolved {
        " · resolved"
    } else {
        ""
    };

    writeln!(
        buffer,
        r###"
      <details class="review-thread" {open}>
        <summary class="review-thread-header">
            <span>{path_html}{status}</span>
        </summary>

        <div class="review-thread-comments">
"###
    )?;

    for comment in comments {
        let author_login = &comment.author.login;
        let author_avatar_url = &comment.author.avatar_url;
        let created_at = &comment.created_at;
        let created_at_rfc3339 = comment.created_at.to_rfc3339();
        let body_html = &comment.body_html;
        let comment_url = &comment.url;

        let edited = if comment.created_at != comment.updated_at {
            "<span> · edited</span>"
        } else {
            ""
        };

        writeln!(
            buffer,
            r###"
      <div class="review-thread-comment">
          <div class="review-thread-comment-header">
            <div class="author-info">
              <a href="https://github.com/{author_login}" target="_blank">
                <img src="{author_avatar_url}" alt="{author_login} Avatar" class="avatar avatar-small">
              </a>
              <a href="https://github.com/{author_login}" target="_blank">{author_login}</a>
              <span>on <span data-utc-time="{created_at_rfc3339}">{created_at}</span></span>{edited}
            </div>
            <a href="{comment_url}" target="_blank" class="github-link">View on GitHub</a>
          </div>
          
          <div class="review-thread-comment-body markdown-body">
            {body_html}
          </div>
      </div>
"###
        )?;
    }

    writeln!(
        buffer,
        r###"
        </div>
      </details>
"###
    )?;

    Ok(())
}
