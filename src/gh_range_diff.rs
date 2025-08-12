use std::collections::HashSet;
use std::fmt::{self, Write};
use std::sync::{Arc, LazyLock};

use anyhow::Context as _;
use axum::{
    extract::{Path, State},
    http::HeaderValue,
    response::IntoResponse,
};
use axum_extra::extract::Host;
use hyper::header::CACHE_CONTROL;
use hyper::{
    HeaderMap, StatusCode,
    header::{CONTENT_SECURITY_POLICY, CONTENT_TYPE},
};
use imara_diff::{
    Algorithm, Diff, InternedInput, Interner, Token, UnifiedDiffConfig, UnifiedDiffPrinter,
};
use pulldown_cmark_escape::FmtWriter;
use regex::Regex;

use crate::{github, handlers::Context, utils::AppError};

static MARKER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"@@ -[\d]+,[\d]+ [+][\d]+,[\d]+ @@").unwrap());

/// Compute and renders an emulated `git range-diff` between two pushes (old and new).
///
/// `basehead` is `OLDHEAD..NEWHEAD`, both `OLDHEAD` and `NEWHEAD` must be SHAs or branch names.
pub async fn gh_range_diff(
    Path((owner, repo, basehead)): Path<(String, String, String)>,
    State(ctx): State<Arc<Context>>,
    Host(host): Host,
) -> axum::response::Result<impl IntoResponse, AppError> {
    let Some((oldhead, newhead)) = basehead.split_once("..") else {
        return Ok((
            StatusCode::BAD_REQUEST,
            HeaderMap::new(),
            format!("`{basehead}` is not in the form `base..head`"),
        ));
    };

    // Configure unified diff
    let config = UnifiedDiffConfig::default();

    let repos = ctx
        .team
        .repos()
        .await
        .context("unable to retrieve team repos")?;

    // Verify that the request org is part of the Rust project
    let Some(repos) = repos.repos.get(&owner) else {
        return Ok((
            StatusCode::BAD_REQUEST,
            HeaderMap::new(),
            format!("organization `{owner}` is not part of the Rust Project team repos"),
        ));
    };

    // Verify that the request repo is part of the Rust project
    if !repos.iter().any(|r| r.name == repo) {
        return Ok((
            StatusCode::BAD_REQUEST,
            HeaderMap::new(),
            format!("repository `{owner}` is not part of the Rust Project team repos"),
        ));
    }

    let issue_repo = github::IssueRepository {
        organization: owner.to_string(),
        repository: repo.to_string(),
    };

    // Determine the oldbase and get the comparison for the old diff
    let old = async {
        // We need to determine the oldbase (ie. the parent sha of all the commits of old).
        // Fortunatly GitHub compare API returns the the merge base commit when comparing
        // two different sha.
        //
        // Unformtunatly for us we don't know in which tree the parent is (could be master, beta, stable, ...)
        // so for now we assume that the parent is in the default branch (that we hardcore for now to "master").
        //
        // We therefore compare those the master and oldhead to get a guess of the oldbase.
        //
        // As an optimization we compare them in reverse to speed up things. The resulting
        // patches won't be correct, but we only care about the merge base commit which
        // is always correct no matter the order.
        let oldbase = ctx
            .github
            .compare(&issue_repo, "master", oldhead)
            .await
            .context("failed to retrive the comparison between newhead and oldhead")?
            .merge_base_commit
            .sha;

        // Get the comparison between the oldbase..oldhead
        let mut old = ctx
            .github
            .compare(&issue_repo, &oldbase, oldhead)
            .await
            .with_context(|| {
                format!("failed to retrive the comparison between {oldbase} and {oldhead}")
            })?;

        // Sort by filename, so it's consistent with GitHub UI
        old.files
            .sort_unstable_by(|f1, f2| f1.filename.cmp(&f2.filename));

        anyhow::Result::<_>::Ok((oldbase, old))
    };

    // Determine the newbase and get the comparison for the new diff
    let new = async {
        // Get the newbase from comparing master and newhead.
        //
        // See the comment above on old for more details.
        let newbase = ctx
            .github
            .compare(&issue_repo, "master", newhead)
            .await
            .context("failed to retrive the comparison between master and newhead")?
            .merge_base_commit
            .sha;

        // Get the comparison between the newbase..newhead
        let mut new = ctx
            .github
            .compare(&issue_repo, &newbase, newhead)
            .await
            .with_context(|| {
                format!("failed to retrive the comparison between {newbase} and {newhead}")
            })?;

        // Sort by filename, so it's consistent with GitHub UI
        new.files
            .sort_unstable_by(|f1, f2| f1.filename.cmp(&f2.filename));

        anyhow::Result::<_>::Ok((newbase, new))
    };

    // Wait for both futures and early exit if there is an error
    let ((oldbase, old), (newbase, new)) = futures::try_join!(old, new)?;

    // Create the HTML buffer with a very rough approximation for the capacity
    let mut html: String = String::with_capacity(800 + old.files.len() * 100);

    // Compute the bookmarklet for the current host
    let bookmarklet = bookmarklet(&host);

    // Write HTML header, style, ...
    writeln!(
        &mut html,
        r#"<!DOCTYPE html>
<html lang="en" translate="no">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <link rel="icon" sizes="32x32" type="image/png" href="https://www.rust-lang.org/static/images/favicon-32x32.png">
    <title>range-diff of {oldbase}...{oldhead} {newbase}...{newhead}</title>
    <style>
    body {{
      font: 14px SFMono-Regular, Consolas, Liberation Mono, Menlo, monospace;
    }}
    details {{
      white-space: pre;
    }}
    summary {{
      font-weight: 800;
    }}
    .removed-block {{
      background-color: rgb(255, 206, 203);
      white-space: pre;
    }}
    .added-block {{
      background-color: rgb(172, 238, 187);
      white-space: pre;
    }}
    .removed-line {{
      color: #DE0000;
    }}
    .added-line {{
      color: #2F6500;
    }}
    @media (prefers-color-scheme: dark) {{
      body {{
        background: #0C0C0C;
        color: #CCC;
      }}
      a {{
        color: #41a6ff;
      }}
      .removed-block {{
        background-color: rgba(248, 81, 73, 0.1);
      }}
      .added-block {{
        background-color: rgba(46, 160, 67, 0.15);
      }}
      .removed-line {{
        color: #F34848;
      }}
      .added-line {{
        color: #86D03C;
      }}
    }}
    </style>
</head>
<body>
<h3>range-diff of {oldbase}...{oldhead} {newbase}...{newhead}</h3>
<p>Bookmarklet: <a href="{bookmarklet}" title="Drag-and-drop me on the bookmarks bar, and use me on GitHub compare page.">range-diff</a> <span title="This javascript bookmark can be used to access this page with the right URL. To use it drag-on-drop the range-diff link to your bookmarks bar and click on it when you are on GitHub's compare page to use range-diff compare.">&#128712;</span> | {ADDED_BLOCK_SIGN} added  {REMOVED_BLOCK_SIGN} removed</p>
"#
    )?;

    let mut process_diffs = |filename, old_patch, new_patch| -> anyhow::Result<()> {
        // Removes diff markers to avoid false-positives
        let new_marker = format!("@@ {filename}:");
        let old_patch = MARKER_RE.replace_all(old_patch, &*new_marker);
        let new_patch = MARKER_RE.replace_all(new_patch, &*new_marker);

        // Prepare input
        let input: InternedInput<&str> = InternedInput::new(&*old_patch, &*new_patch);

        // Compute the diff
        let mut diff = Diff::compute(Algorithm::Histogram, &input);

        // Run postprocessing to improve hunk boundaries
        diff.postprocess_lines(&input);

        // Determine if there are any differences
        let has_hunks = diff.hunks().next().is_some();

        if has_hunks {
            let printer = HtmlDiffPrinter(&input.interner);
            let diff = diff.unified_diff(&printer, config.clone(), &input);

            let before_href =
                format_args!("https://github.com/{owner}/{repo}/blob/{oldhead}/{filename}");
            let after_href =
                format_args!("https://github.com/{owner}/{repo}/blob/{newhead}/{filename}");

            writeln!(
                html,
                r#"<details open=""><summary>{filename} <a href="{before_href}">before</a> <a href="{after_href}">after</a></summary><pre>{diff}</pre></details>"#
            )?;
        }
        Ok(())
    };

    let mut seen_files = HashSet::<&str>::new();

    // Process the old files
    for old_file in &old.files {
        let filename = &*old_file.filename;

        let new_file_patch = new
            .files
            .iter()
            .find(|f| f.filename == filename)
            .map(|f| &*f.patch)
            .unwrap_or_default();

        seen_files.insert(filename);

        process_diffs(filename, &*old_file.patch, new_file_patch)?;
    }

    // Process the not yet seen new files
    for new_file in &new.files {
        let filename = &*new_file.filename;

        if seen_files.contains(filename) {
            continue;
        }

        process_diffs(filename, "", &*new_file.patch)?;
    }

    writeln!(
        html,
        r#"
</body>
</html>
        "#
    )?;

    let mut headers = HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    headers.insert(
        CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=15552000, immutable"),
    );
    headers.insert(
        CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'none'; style-src 'unsafe-inline'; img-src www.rust-lang.org",
        ),
    );

    Ok((StatusCode::OK, headers, html))
}

const REMOVED_BLOCK_SIGN: &str = r#"<span class="removed-block"> - </span>"#;
const ADDED_BLOCK_SIGN: &str = r#"<span class="added-block"> + </span>"#;

struct HtmlDiffPrinter<'a>(pub &'a Interner<&'a str>);

impl HtmlDiffPrinter<'_> {
    fn handle_hunk_token(&self, mut f: impl fmt::Write, class: &str, token: &str) -> fmt::Result {
        write!(f, " ")?;
        // Highlight the whole the line only if it has changes it-self, otherwise
        // only highlight the `+`, `-` to avoid distracting users with context
        // changes.
        if token.starts_with('+') || token.starts_with('-') {
            write!(f, r#"<span class="{class}">"#)?;
            pulldown_cmark_escape::escape_html(FmtWriter(&mut f), token)?;
            write!(f, "</span>")?;
        } else {
            pulldown_cmark_escape::escape_html(FmtWriter(&mut f), token)?;
        }
        Ok(())
    }
}

impl UnifiedDiffPrinter for HtmlDiffPrinter<'_> {
    fn display_header(
        &self,
        _f: impl fmt::Write,
        _start_before: u32,
        _start_after: u32,
        _len_before: u32,
        _len_after: u32,
    ) -> fmt::Result {
        // ignore the header as does not represent anything meaningful for the range-diff
        Ok(())
    }

    fn display_context_token(&self, mut f: impl fmt::Write, token: Token) -> fmt::Result {
        let token = self.0[token];
        write!(f, "    ")?;
        pulldown_cmark_escape::escape_html(FmtWriter(&mut f), token)?;
        if !token.ends_with('\n') {
            writeln!(f)?;
        }
        Ok(())
    }

    fn display_hunk(
        &self,
        mut f: impl fmt::Write,
        before: &[Token],
        after: &[Token],
    ) -> fmt::Result {
        if let Some(&last) = before.last() {
            for &token in before {
                let token = self.0[token];
                write!(f, "{REMOVED_BLOCK_SIGN}")?;
                self.handle_hunk_token(&mut f, "removed-line", token)?;
            }
            if !self.0[last].ends_with('\n') {
                writeln!(f)?;
            }
        }

        if let Some(&last) = after.last() {
            for &token in after {
                let token = self.0[token];
                write!(f, "{ADDED_BLOCK_SIGN}")?;
                self.handle_hunk_token(&mut f, "added-line", token)?;
            }
            if !self.0[last].ends_with('\n') {
                writeln!(f)?;
            }
        }
        Ok(())
    }
}

// Create the javascript bookmarklet based on the host
fn bookmarklet(host: &str) -> String {
    let protocol = if host.starts_with("localhost:") {
        "http"
    } else {
        "https"
    };

    format!(
        r"javascript:(() => {{
    const githubUrlPattern = /^https:\/\/github\.com\/([^\/]+)\/([^\/]+)\/compare\/([^\/]+[.]{{2}}[^\/]+)$/;
    const match = window.location.href.match(githubUrlPattern);
    if (!match) {{alert('Invalid GitHub Compare URL format.\nExpected: https://github.com/ORG_NAME/REPO_NAME/compare/BASESHA..HEADSHA'); return;}}
    const [, orgName, repoName, basehead] = match; window.location = `{protocol}://{host}/gh-range-diff/${{orgName}}/${{repoName}}/${{basehead}}`;
}})();"
    )
}
