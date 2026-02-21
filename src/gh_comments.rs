use std::sync::Arc;
use std::time::Instant;
use std::{fmt::Write, sync::LazyLock};

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
    github::issue_with_comments::{
        GitHubGraphQlComment, GitHubGraphQlReactionGroup, GitHubGraphQlReviewThreadComment,
        GitHubIssueState, GitHubIssueStateReason, GitHubIssueWithComments, GitHubReviewState,
        GitHubSimplifiedAuthor,
    },
};
use crate::{
    errors::AppError,
    handlers::Context,
    utils::{immutable_headers, is_repo_autorized},
};

pub const STYLE_URL: &str = "/gh-comments/style@0.0.4.css";
pub const MARKDOWN_URL: &str = "/gh-comments/github-markdown@20260117.css";
pub const SELF_CONTAINED_URL: &str = "/gh-comments/self_contained@0.0.1.js";

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

static GHOST_ACCOUNT: LazyLock<GitHubSimplifiedAuthor> =
    LazyLock::new(GitHubSimplifiedAuthor::default);

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
                        + c.author
                            .as_ref()
                            .map(|a| a.login.len() + a.avatar_url.len())
                            .unwrap_or(0)
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
  <script src="{SELF_CONTAINED_URL}" data-to-remove-on-export></script>
  <script nonce="triagebot-gh-comments">
    const ISSUE_ID = {issue_id};
    document.addEventListener('DOMContentLoaded', function() {{
      document.querySelectorAll('[data-utc-time]').forEach(element => {{
        const utcString = element.getAttribute('data-utc-time');
        const utcDate = new Date(utcString);
        element.textContent = utcDate.toLocaleString();
      }});
      document.querySelectorAll('.markdown-body a').forEach(link => {{
        const linkUrl = new URL(link.href, window.location.origin);
        const anchor = linkUrl.hash.slice(1);
        if (link.host === "github.com" && anchor) {{
          const target = document.getElementById(anchor);
          if (target) {{
            link.href = `#${{anchor}}`;
          }}
        }}
      }});
    }});
  </script>
</head>
<body>
<div class="comments-container">
<h1 class="title"><bdi class="markdown-body">{title_html}</bdi> <a class="github-link" href="https://github.com/{owner}/{repo}/issues/{issue_id}">{owner}/{repo}#{issue_id}</a></h1>
"###,
    )?;

    // Print the state
    writeln!(html, r##"<div class="meta-header">"##)?;

    let (state_class, state_text) = match issue_with_comments.state {
        GitHubIssueState::Open => ("badge-success", "Open"),
        GitHubIssueState::Closed => match issue_with_comments.state_reason {
            Some(GitHubIssueStateReason::Completed) => ("badge-done", "Closed"),
            Some(GitHubIssueStateReason::Duplicate) => ("badge-neutral", "Closed as duplicate"),
            Some(GitHubIssueStateReason::NotPlanned) => ("badge-neutral", "Closed as not planned"),
            Some(GitHubIssueStateReason::Reopened) | None => ("badge-danger", "Closed"),
        },
        GitHubIssueState::Merged => ("badge-done", "Merged"),
    };
    writeln!(
        html,
        r##"<div class="state-badge {state_class}">{state_text}</div>"##
    )?;

    // Print time and number of comments (+ reviews) loaded
    if let Some(reviews) = issue_with_comments.reviews.as_ref() {
        let count = issue_with_comments.comments.nodes.len() + reviews.nodes.len();
        writeln!(
            html,
            r###"<p>{count} comments and reviews loaded in {duration_secs:.2}s</p>"###,
        )?;
    } else {
        let comment_count = issue_with_comments.comments.nodes.len();

        writeln!(
            html,
            r###"<p>{comment_count} comments loaded in {duration_secs:.2}s</p>"###,
        )?;
    }
    writeln!(
        html,
        r#"<button id="gh-comments-export-btn" data-to-remove-on-export>Export</button>"#
    )?;
    writeln!(html, "</div>")?;

    // Print shortcut links for PRs
    if issue_with_comments.reviews.is_some() {
        let base = format!("https://github.com/{owner}/{repo}/pull/{issue_id}");
        writeln!(html, r##"<div class="meta-links">"##)?;
        write!(
            html,
            r##"<a class="github-link selected" href="{base}">Conversations</a>"##
        )?;
        write!(
            html,
            r##" · <a class="github-link" href="{base}/commits">Commits</a>"##
        )?;
        write!(
            html,
            r##" · <a class="github-link" href="{base}/checks">Checks</a>"##
        )?;
        write!(
            html,
            r##" · <a class="github-link" href="{base}/changes">Files changes</a>"##
        )?;
        writeln!(html, "</div>")?;
    }

    // Print issue/PR body
    write_comment_as_html(
        &mut html,
        &issue_with_comments.body_html,
        &issue_with_comments.url,
        issue_with_comments
            .author
            .as_ref()
            .unwrap_or(&GHOST_ACCOUNT),
        &issue_with_comments.created_at,
        &issue_with_comments.updated_at,
        &issue_with_comments.reactions,
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
                        comment.author.as_ref().unwrap_or(&GHOST_ACCOUNT),
                        &comment.created_at,
                        &comment.updated_at,
                        &comment.reactions,
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
                        review.author.as_ref().unwrap_or(&GHOST_ACCOUNT),
                        review.state,
                        &review.submitted_at,
                        &review.updated_at,
                        &review.reactions,
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
                comment.author.as_ref().unwrap_or(&GHOST_ACCOUNT),
                &comment.created_at,
                &comment.updated_at,
                &comment.reactions,
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
            "default-src 'none'; script-src 'nonce-triagebot-gh-comments' 'self'; style-src 'self' 'unsafe-inline'; img-src *",
        ),
    );

    Ok((StatusCode::OK, headers, html).into_response())
}

pub async fn style_css() -> impl IntoResponse {
    const STYLE_CSS: &str = include_str!("gh_comments/style.css");

    (immutable_headers("text/css; charset=utf-8"), STYLE_CSS)
}

pub async fn markdown_css() -> impl IntoResponse {
    const MARKDOWN_CSS: &str = include_str!("gh_comments/github-markdown@20260117.css");

    (immutable_headers("text/css; charset=utf-8"), MARKDOWN_CSS)
}

pub async fn self_contained_js() -> impl IntoResponse {
    const SELF_CONTAINED_JS: &str = include_str!("gh_comments/self_contained.js");

    (
        immutable_headers("text/javascript; charset=utf-8"),
        SELF_CONTAINED_JS,
    )
}

fn write_comment_as_html(
    buffer: &mut String,
    body_html: &str,
    comment_url: &str,
    author: &GitHubSimplifiedAuthor,
    created_at: &chrono::DateTime<Utc>,
    updated_at: &chrono::DateTime<Utc>,
    reaction_groups: &[GitHubGraphQlReactionGroup],
    minimized: bool,
    minimized_reason: Option<&str>,
) -> anyhow::Result<()> {
    let author_login = &author.login;
    let author_avatar_url = &author.avatar_url;
    let created_at_rfc3339 = created_at.to_rfc3339();
    let id = extract_id_from_github_link(comment_url);

    if minimized && let Some(minimized_reason) = minimized_reason {
        writeln!(
            buffer,
            r###"
    <div class="comment-wrapper">
      <a href="https://github.com/{author_login}" target="_blank" class="desktop">
        <img src="{author_avatar_url}" alt="{author_login} Avatar" class="avatar">
      </a>
      
      <details id="{id}" class="comment">
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
"###
        )?;
        write_reaction_groups_as_html(buffer, reaction_groups)?;
        writeln!(buffer, "</details></div>")?;
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

      <div id="{id}" class="comment">
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
"###
        )?;
        write_reaction_groups_as_html(buffer, reaction_groups)?;
        writeln!(buffer, "</div></div>")?;
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
    reaction_groups: &[GitHubGraphQlReactionGroup],
    minimized: bool,
    minimized_reason: Option<&str>,
) -> anyhow::Result<()> {
    let author_login = &author.login;
    let author_avatar_url = &author.avatar_url;
    let submitted_at_rfc3339 = submitted_at.to_rfc3339();
    let id = extract_id_from_github_link(review_url);

    let (badge_color, badge_svg) = match state {
        GitHubReviewState::Approved => {
            // https://primer.github.io/octicons/check-16
            (
                "badge-success",
                r##"<svg class="octicon" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" width="16" height="16"><path d="M13.78 4.22a.75.75 0 0 1 0 1.06l-7.25 7.25a.75.75 0 0 1-1.06 0L2.22 9.28a.751.751 0 0 1 .018-1.042.751.751 0 0 1 1.042-.018L6 10.94l6.72-6.72a.75.75 0 0 1 1.06 0Z"></path></svg>"##,
            )
        }
        GitHubReviewState::ChangesRequested => {
            // https://primer.github.io/octicons/file-diff-16
            (
                "badge-danger",
                r##"<svg class="octicon" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" width="16" height="16"><path d="M1 1.75C1 .784 1.784 0 2.75 0h7.586c.464 0 .909.184 1.237.513l2.914 2.914c.329.328.513.773.513 1.237v9.586A1.75 1.75 0 0 1 13.25 16H2.75A1.75 1.75 0 0 1 1 14.25Zm1.75-.25a.25.25 0 0 0-.25.25v12.5c0 .138.112.25.25.25h10.5a.25.25 0 0 0 .25-.25V4.664a.25.25 0 0 0-.073-.177l-2.914-2.914a.25.25 0 0 0-.177-.073ZM8 3.25a.75.75 0 0 1 .75.75v1.5h1.5a.75.75 0 0 1 0 1.5h-1.5v1.5a.75.75 0 0 1-1.5 0V7h-1.5a.75.75 0 0 1 0-1.5h1.5V4A.75.75 0 0 1 8 3.25Zm-3 8a.75.75 0 0 1 .75-.75h4.5a.75.75 0 0 1 0 1.5h-4.5a.75.75 0 0 1-.75-.75Z"></path></svg>"##,
            )
        }
        GitHubReviewState::Dismissed | GitHubReviewState::Commented => {
            // https://primer.github.io/octicons/eye-16
            (
                "",
                r##"<svg class="octicon" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" width="16" height="16"><path d="M8 2c1.981 0 3.671.992 4.933 2.078 1.27 1.091 2.187 2.345 2.637 3.023a1.62 1.62 0 0 1 0 1.798c-.45.678-1.367 1.932-2.637 3.023C11.67 13.008 9.981 14 8 14c-1.981 0-3.671-.992-4.933-2.078C1.797 10.83.88 9.576.43 8.898a1.62 1.62 0 0 1 0-1.798c.45-.677 1.367-1.931 2.637-3.022C4.33 2.992 6.019 2 8 2ZM1.679 7.932a.12.12 0 0 0 0 .136c.411.622 1.241 1.75 2.366 2.717C5.176 11.758 6.527 12.5 8 12.5c1.473 0 2.825-.742 3.955-1.715 1.124-.967 1.954-2.096 2.366-2.717a.12.12 0 0 0 0-.136c-.412-.621-1.242-1.75-2.366-2.717C10.824 4.242 9.473 3.5 8 3.5c-1.473 0-2.825.742-3.955 1.715-1.124.967-1.954 2.096-2.366 2.717ZM8 10a2 2 0 1 1-.001-3.999A2 2 0 0 1 8 10Z"></path></svg>"##,
            )
        }
    };

    let state_message = match state {
        GitHubReviewState::Commented => "commented",
        GitHubReviewState::Approved => "approved",
        GitHubReviewState::ChangesRequested => "requested changes",
        GitHubReviewState::Dismissed => "dismissed review",
    };

    writeln!(
        buffer,
        r###"
    <div id="{id}" class="review">
      <a href="https://github.com/{author_login}" target="_blank">
        <img src="{author_avatar_url}" alt="{author_login} Avatar" class="avatar">
      </a>
      
      <div class="review-header">
        <div class="review-badge {badge_color}">{badge_svg}</div>
        <div class="author-info">
          <a href="https://github.com/{author_login}" target="_blank">{author_login}</a>
          <span>{state_message} on <span data-utc-time="{submitted_at_rfc3339}">{submitted_at}</span></span>
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
"###
            )?;
            write_reaction_groups_as_html(buffer, reaction_groups)?;
            writeln!(buffer, "</details></div>")?;
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
"###
            )?;
            write_reaction_groups_as_html(buffer, reaction_groups)?;
            writeln!(buffer, "</div></div>")?;
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
        let author = comment.author.as_ref().unwrap_or(&GHOST_ACCOUNT);
        let author_login = &author.login;
        let author_avatar_url = &author.avatar_url;
        let created_at = &comment.created_at;
        let created_at_rfc3339 = comment.created_at.to_rfc3339();
        let body_html = &comment.body_html;
        let comment_url = &comment.url;
        let reaction_groups = &*comment.reactions;
        let id = extract_id_from_github_link(comment_url);

        let edited = if comment.created_at != comment.updated_at {
            "<span> · edited</span>"
        } else {
            ""
        };

        writeln!(
            buffer,
            r###"
      <div id="{id}" class="review-thread-comment">
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
"###
        )?;
        write_reaction_groups_as_html(buffer, reaction_groups)?;
        writeln!(buffer, "</div>")?;
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

fn write_reaction_groups_as_html(
    buffer: &mut String,
    reaction_groups: &[GitHubGraphQlReactionGroup],
) -> anyhow::Result<()> {
    let any_reactions = reaction_groups.iter().any(|rg| rg.users.total_count > 0);

    if any_reactions {
        writeln!(buffer, r##"<div class="reactions">"##)?;

        for reaction_group in reaction_groups {
            let total_count = reaction_group.users.total_count;

            if total_count == 0 {
                continue;
            }

            use crate::github::issue_with_comments::GitHubGraphQlReactionContent::*;
            let emoji = match reaction_group.content {
                ThumbsUp => "👍",
                ThumbsDown => "👎",
                Laugh => "😄",
                Hooray => "🎉",
                Confused => "😕",
                Heart => "❤️",
                Rocket => "🚀",
                Eyes => "👀",
            };

            write!(
                buffer,
                r##"<div class="reaction">{emoji}<span class="reaction-number">{total_count}</span></div>"##
            )?;
        }

        writeln!(buffer, r##"</div>"##)?;
    }

    Ok(())
}

fn extract_id_from_github_link(url: &str) -> &str {
    url.rfind('#').map(|pos| &url[pos + 1..]).unwrap_or("")
}
