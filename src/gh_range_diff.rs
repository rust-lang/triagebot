use std::collections::HashSet;
use std::fmt::{self, Write};
use std::iter;
use std::sync::{Arc, LazyLock};

use anyhow::Context as _;
use axum::{
    extract::{Path, State},
    http::HeaderValue,
    response::IntoResponse,
};
use gix_imara_diff::{Algorithm, Diff, Hunk, InternedInput, Interner, Token, UnifiedDiffPrinter};
use hyper::header::CACHE_CONTROL;
use hyper::{
    HeaderMap, StatusCode,
    header::{CONTENT_SECURITY_POLICY, CONTENT_TYPE},
};
use pulldown_cmark_escape::FmtWriter;
use regex::Regex;
use unicode_segmentation::UnicodeSegmentation;

use crate::github::GithubCompare;
use crate::utils::is_known_and_public_repo;
use crate::{errors::AppError, github, handlers::Context};

static MARKER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"@@ -[\d]+,[\d]+ [+][\d]+,[\d]+ @@").unwrap());

/// Compute and renders an emulated `git range-diff` between two pushes (old and new).
///
/// `basehead` is `OLDHEAD..NEWHEAD`, both `OLDHEAD` and `NEWHEAD` must be SHAs or branch names.
pub async fn gh_range_diff(
    Path((owner, repo, basehead)): Path<(String, String, String)>,
    State(ctx): State<Arc<Context>>,
) -> axum::response::Result<impl IntoResponse, AppError> {
    let Some((oldhead, newhead)) = basehead.split_once("..") else {
        return Ok((
            StatusCode::BAD_REQUEST,
            HeaderMap::new(),
            format!("`{basehead}` is not in the form `base..head`"),
        ));
    };

    if !is_known_and_public_repo(&ctx, &owner, &repo).await? {
        return Ok((
            StatusCode::UNAUTHORIZED,
            HeaderMap::new(),
            format!("repository `{owner}/{repo}` is not part of the Rust Project team repos"),
        ));
    }

    let issue_repo = github::IssueRepository {
        organization: owner.to_string(),
        repository: repo.to_string(),
    };

    let gh_repo = ctx.github.repository(&format!("{owner}/{repo}")).await?;

    // Determine the oldbase and get the comparison for the old diff
    let old = async {
        // We need to determine the oldbase (ie. the parent sha of all the commits of old).
        // Fortunatly GitHub compare API returns the the merge base commit when comparing
        // two different sha.
        //
        // Unfortunately for us we don't know in which tree the parent is (could be master, beta, stable, ...)
        // so for now we assume that the parent is in the default branch (that we hardcore for now to "master").
        //
        // We therefore compare those the master and oldhead to get a guess of the oldbase.
        //
        // As an optimization we compare them in reverse to speed up things. The resulting
        // patches won't be correct, but we only care about the merge base commit which
        // is always correct no matter the order.
        let oldbase = ctx
            .github
            .compare(&issue_repo, &gh_repo.default_branch, oldhead)
            .await
            .context("failed to retrive the comparison between newhead and oldhead")?
            .merge_base_commit
            .sha;

        // Get the comparison between the oldbase..oldhead
        let old = ctx
            .github
            .compare(&issue_repo, &oldbase, oldhead)
            .await
            .with_context(|| {
                format!("failed to retrive the comparison between {oldbase} and {oldhead}")
            })?;

        anyhow::Result::<_>::Ok((oldbase, old))
    };

    // Determine the newbase and get the comparison for the new diff
    let new = async {
        // Get the newbase from comparing master and newhead.
        //
        // See the comment above on old for more details.
        let newbase = ctx
            .github
            .compare(&issue_repo, &gh_repo.default_branch, newhead)
            .await
            .context("failed to retrive the comparison between the default branch and newhead")?
            .merge_base_commit
            .sha;

        // Get the comparison between the newbase..newhead
        let new = ctx
            .github
            .compare(&issue_repo, &newbase, newhead)
            .await
            .with_context(|| {
                format!("failed to retrive the comparison between {newbase} and {newhead}")
            })?;

        anyhow::Result::<_>::Ok((newbase, new))
    };

    // Wait for both futures and early exit if there is an error
    let ((oldbase, old), (newbase, new)) = futures::try_join!(old, new)?;

    process_old_new(
        (&owner, &repo),
        (&oldbase, oldhead, old),
        (&newbase, newhead, new),
    )
}

/// Compute and renders an emulated `git range-diff` between two pushes (old and new).
///
/// - `oldbasehead` is `OLDBASE..OLDHEAD`
/// - `newbasehead` is `NEWBASE..NEWHEAD`
pub async fn gh_ranges_diff(
    Path((owner, repo, oldbasehead, newbasehead)): Path<(String, String, String, String)>,
    State(ctx): State<Arc<Context>>,
) -> axum::response::Result<impl IntoResponse, AppError> {
    let Some((oldbase, oldhead)) = oldbasehead.split_once("..") else {
        return Ok((
            StatusCode::BAD_REQUEST,
            HeaderMap::new(),
            format!("`{oldbasehead}` is not in the form `base..head`"),
        ));
    };

    let Some((newbase, newhead)) = newbasehead.split_once("..") else {
        return Ok((
            StatusCode::BAD_REQUEST,
            HeaderMap::new(),
            format!("`{newbasehead}` is not in the form `base..head`"),
        ));
    };

    if !is_known_and_public_repo(&ctx, &owner, &repo).await? {
        return Ok((
            StatusCode::UNAUTHORIZED,
            HeaderMap::new(),
            format!("repository `{owner}/{repo}` is not part of the Rust Project team repos"),
        ));
    }

    let issue_repo = github::IssueRepository {
        organization: owner.to_string(),
        repository: repo.to_string(),
    };

    // Get the comparison between the oldbase..oldhead
    let old = async {
        ctx.github
            .compare(&issue_repo, oldbase, oldhead)
            .await
            .with_context(|| {
                format!("failed to retrive the comparison between {oldbase} and {oldhead}")
            })
    };

    // Get the comparison between the newbase..newhead
    let new = async {
        ctx.github
            .compare(&issue_repo, newbase, newhead)
            .await
            .with_context(|| {
                format!("failed to retrive the comparison between {newbase} and {newhead}")
            })
    };

    // Wait for both futures and early exit if there is an error
    let (old, new) = futures::try_join!(old, new)?;

    process_old_new(
        (&owner, &repo),
        (oldbase, oldhead, old),
        (newbase, newhead, new),
    )
}

fn process_old_new(
    (owner, repo): (&str, &str),
    (oldbase, oldhead, mut old): (&str, &str, GithubCompare),
    (newbase, newhead, mut new): (&str, &str, GithubCompare),
) -> axum::response::Result<(StatusCode, HeaderMap, String), AppError> {
    // Configure unified diff
    let config = CustomUnifiedDiffConfig { context_len: 3 };

    // Sort by filename, so it's consistent with GitHub UI
    old.files
        .sort_unstable_by(|f1, f2| f1.filename.cmp(&f2.filename));
    new.files
        .sort_unstable_by(|f1, f2| f1.filename.cmp(&f2.filename));

    // Create the HTML buffer with a very rough approximation for the capacity
    let mut html: String = String::with_capacity(800 + old.files.len() * 100);

    let a_compare_before = a_github_compare("compare-before", owner, repo, oldbase, oldhead);
    let a_compare_after = a_github_compare("compare-after", owner, repo, newbase, newhead);

    // Write HTML header, style, ...
    writeln!(
        &mut html,
        r#"<!DOCTYPE html>
<html lang="en" translate="no">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <link rel="icon" sizes="32x32" type="image/png" href="https://rust-lang.org/static/images/favicon-32x32.png">
    <title>range-diff of {oldbase}..{oldhead} {newbase}..{newhead}</title>
    <style>
    body {{
      font: 14px SFMono-Regular, Consolas, Liberation Mono, Menlo, monospace;
    }}
    details {{
      white-space: pre;
    }}
    summary {{
      font-weight: 800;
      overflow-wrap: break-word;
      white-space: normal;
    }}
    .compare {{
      text-decoration: none;
      color: unset;
    }}
    .compare-before {{
      color: rgb(255, 93, 93);
    }}
    .compare-after {{
      color: rgb(55, 227, 55);
    }}
    .diff-content {{
      overflow-x: auto;
    }}
    .diff-content > .filename-line:first-child {{
      display: none;
    }}
    .filename-block {{
      background-color: #89a1cd;  
      color: black;
    }}
    .removed-block {{
      background-color: rgba(255, 150, 150, 1);
      white-space: pre;
    }}
    .added-block {{
      background-color: rgba(150, 255, 150, 1);
      white-space: pre;
    }}
    .line-removed-after {{
      color: rgb(220, 0, 0)
    }}
    .line-removed-after .word-added {{
      color: white;
      background-color: rgb(63, 128, 94);
    }}
    .line-added-after {{
      color: rgb(0, 221, 0)
    }}
    .line-added-after .word-added {{
      color: white;
      background-color: rgb(0, 73, 0);
    }}
    .line-removed-before {{
      color: rgb(192, 78, 76)
    }}
    .line-removed-before .word-removed {{
      color: white;
      background-color: rgb(192, 78, 76);
    }}
    .line-added-before {{
      color: rgb(63, 128, 94)
    }}
    .line-added-before .word-removed {{
      color: white;
      background-color: rgb(220, 0, 0);
    }}
    .spacer {{
      margin-bottom: 1rem;
    }}
    @media (prefers-color-scheme: dark) {{
      body {{
        background: #0C0C0C;
        color: #CCC;
      }}
      a {{
        color: #41a6ff;
      }}
      .compare-before {{
        color: rgb(255, 93, 93);
      }}
      .compare-after {{
        color: rgb(88, 177, 88);
      }}
      .filename-block {{
        background-color: #5f8fe5;
      }}
      .removed-block {{
        background-color: rgba(80, 45, 45, 1);
        white-space: pre;
      }}
      .added-block {{
        background-color: rgba(70, 120, 70, 1);
        white-space: pre;
      }}
      .line-removed-after {{
        color: rgba(255, 0, 0, 1);
      }}
      .line-removed-after .word-added {{
        color: black;
        background-color: rgb(0, 100, 0);
      }}
      .line-added-after {{
        color: rgba(0, 255, 0, 1);
      }}
      .line-added-after .word-added {{
        color: black;
        background-color: rgb(0, 255, 0);
      }}
      .line-removed-before {{
        color: rgb(255, 159, 131);
      }}
      .line-removed-before .word-removed {{
        color: black;
        background-color: rgb(100, 0, 0);
      }}
      .line-added-before {{
        color: rgba(11, 142, 0, 1);
      }}
      .line-added-before .word-removed {{
        color: black;
        background-color: rgb(255, 0, 0);
      }}
    }}
    </style>
</head>
<body>
<h3>range-diff of {a_compare_before} {a_compare_after} in {owner}/{repo}</h3>
<span>Legend: {REMOVED_BLOCK_SIGN}&nbsp;Removed from previous diff | {ADDED_BLOCK_SIGN}&nbsp;Added in new diff</span>
<div class="spacer"></div>
"#
    )?;

    let mut diff_displayed = 0;

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

        // Collect and filter-out hunks don't contain any diff marker (-, +)
        // as those are context-only changes, which are not interesting in
        // a range-diff.
        //
        // See <https://github.com/rust-lang/triagebot/issues/2394>
        let hunks = diff
            .hunks()
            .filter(|hunk| contains_diff_marker(&input, hunk.clone()))
            .collect::<Vec<_>>();

        // Show the changes if there are any hunks to be shown
        if !hunks.is_empty() {
            let printer = HtmlDiffPrinter(&input.interner, filename);
            let unified_diff = CustomUnifiedDiff {
                printer: &printer,
                hunks: &hunks,
                config: config.clone(),
                before: &input.before,
                after: &input.after,
            };

            let before_href =
                format_args!("https://github.com/{owner}/{repo}/blob/{oldhead}/{filename}");
            let after_href =
                format_args!("https://github.com/{owner}/{repo}/blob/{newhead}/{filename}");

            write!(html, r#"<details open=""><summary>{filename}"#)?;
            write!(
                html,
                r#" <a href="{before_href}">before</a> <a href="{after_href}">after</a></summary><pre class="diff-content">{unified_diff}</pre>"#
            )?;
            writeln!(html, "</details>")?;

            diff_displayed += 1;
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

    // Print message when there aren't any differences
    if diff_displayed == 0 {
        writeln!(html, "<p>No differences</p>")?;
    }

    writeln!(
        html,
        r"
</body>
</html>
        "
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
            "default-src 'none'; style-src 'unsafe-inline'; img-src rust-lang.org",
        ),
    );

    Ok((StatusCode::OK, headers, html))
}

const REMOVED_BLOCK_SIGN: &str = r#"<span class="removed-block"> - </span>"#;
const ADDED_BLOCK_SIGN: &str = r#"<span class="added-block"> + </span>"#;

#[derive(Copy, Clone)]
enum HunkTokenStatus {
    Added,
    Removed,
}

struct HtmlDiffPrinter<'a>(pub &'a Interner<&'a str>, pub &'a str);

impl HtmlDiffPrinter<'_> {
    #[expect(clippy::unused_self, reason = "might use it later")]
    fn handle_hunk_line<'a>(
        &self,
        mut f: impl fmt::Write,
        hunk_token_status: HunkTokenStatus,
        words: impl Iterator<Item = (&'a str, bool)>,
    ) -> fmt::Result {
        // Show the hunk status
        match hunk_token_status {
            HunkTokenStatus::Added => write!(f, "{ADDED_BLOCK_SIGN} ")?,
            HunkTokenStatus::Removed => write!(f, "{REMOVED_BLOCK_SIGN} ")?,
        }

        let mut words = words.peekable();

        let first_word = words.peek();
        let is_add = first_word.is_some_and(|w| w.0.starts_with('+'));
        let is_remove = first_word.is_some_and(|w| w.0.starts_with('-'));

        // Highlight in the same was as `git range-diff` does for diff-lines
        // that changed. In addition we also do word highlighting.
        //
        // (Contrary to `git range-diff` we don't color unchanged
        // diff lines though, since then the coloring distracts from what is
        // relevant.)
        if is_add || is_remove {
            let line_class = match (is_add, hunk_token_status) {
                (true, HunkTokenStatus::Removed) => "line-added-before",
                (false, HunkTokenStatus::Removed) => "line-removed-before",
                (true, HunkTokenStatus::Added) => "line-added-after",
                (false, HunkTokenStatus::Added) => "line-removed-after",
            };
            write!(f, r#"<span class="{line_class}">"#)?;

            for (word, changed) in words {
                if changed {
                    let word_class = match hunk_token_status {
                        HunkTokenStatus::Removed => "word-removed",
                        HunkTokenStatus::Added => "word-added",
                    };

                    write!(f, r#"<span class="{word_class}">"#)?;
                    pulldown_cmark_escape::escape_html(FmtWriter(&mut f), word)?;
                    write!(f, "</span>")?;
                } else {
                    pulldown_cmark_escape::escape_html(FmtWriter(&mut f), word)?;
                }
            }

            write!(f, "</span>")?;
        } else {
            for (word, _status) in words {
                pulldown_cmark_escape::escape_html(FmtWriter(&mut f), word)?;
            }
        }

        Ok(())
    }
}

impl UnifiedDiffPrinter for HtmlDiffPrinter<'_> {
    fn display_header(
        &self,
        mut f: impl fmt::Write,
        _start_before: u32,
        _start_after: u32,
        _len_before: u32,
        _len_after: u32,
    ) -> fmt::Result {
        const NEW_LINE: &str = "\n";

        write!(
            f,
            r#"<span class="filename-line"> <span class="filename-block">@@</span> <b>{}</b>{NEW_LINE}</span>"#,
            self.1
        )?;
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
        // To improve on the line-by-line diff we also want to do a sort of `git --words-diff`
        // (aka word highlighting). To achieve word highlighting, we only consider hunk that
        // have the same number of lines removed and added, otherwise it's much more complex
        // to link the changes together.

        if before.len() == after.len() {
            // Same number of lines before and after, can do word-hightling.

            // Diff the individual lines together.
            let diffs_and_inputs: Vec<_> = iter::zip(before, after)
                .map(|(b_token, a_token)| {
                    // Split both lines by words and intern them.
                    let input: InternedInput<&str> = InternedInput::new(
                        SplitWordBoundaries(self.0[*b_token]),
                        SplitWordBoundaries(self.0[*a_token]),
                    );

                    // Compute the (word) diff
                    let diff = Diff::compute(Algorithm::Histogram, &input);

                    (diff, input)
                })
                .collect();

            // Process all before lines first
            for (diff, input) in &diffs_and_inputs {
                self.handle_hunk_line(
                    &mut f,
                    HunkTokenStatus::Removed,
                    input.before.iter().enumerate().map(|(b_pos, b_token)| {
                        (input.interner[*b_token], diff.is_removed(b_pos as u32))
                    }),
                )?;
            }

            // Add potentially missing new-line after the last before diff line
            if let Some(&last) = before.last() {
                if !self.0[last].ends_with('\n') {
                    writeln!(f)?;
                }
            }

            // Then process all after lines
            for (diff, input) in &diffs_and_inputs {
                self.handle_hunk_line(
                    &mut f,
                    HunkTokenStatus::Added,
                    input.after.iter().enumerate().map(|(a_pos, a_token)| {
                        (input.interner[*a_token], diff.is_added(a_pos as u32))
                    }),
                )?;
            }

            // Add potentially missing new-line after the last after diff line
            if let Some(&last) = after.last() {
                if !self.0[last].ends_with('\n') {
                    writeln!(f)?;
                }
            }
        } else {
            // Can't do word-highlighting, simply print each line.

            if let Some(&last) = before.last() {
                for &token in before {
                    let token = self.0[token];
                    self.handle_hunk_line(
                        &mut f,
                        HunkTokenStatus::Removed,
                        std::iter::once((token, false)),
                    )?;
                }
                if !self.0[last].ends_with('\n') {
                    writeln!(f)?;
                }
            }

            if let Some(&last) = after.last() {
                for &token in after {
                    let token = self.0[token];
                    self.handle_hunk_line(
                        &mut f,
                        HunkTokenStatus::Added,
                        std::iter::once((token, false)),
                    )?;
                }
                if !self.0[last].ends_with('\n') {
                    writeln!(f)?;
                }
            }
        }
        Ok(())
    }
}

/// Custom imara-diff UnifiedDiff
struct CustomUnifiedDiff<'a, P: UnifiedDiffPrinter> {
    printer: &'a P,
    hunks: &'a [Hunk],
    config: CustomUnifiedDiffConfig,
    before: &'a [Token],
    after: &'a [Token],
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CustomUnifiedDiffConfig {
    context_len: u32,
}

// Customized version of <https://github.com/GitoxideLabs/gitoxide/blob/8af2691270a72c711bbec8100ce07273de29f52a/gix-imara-diff/src/unified_diff.rs#L218>
impl<P: UnifiedDiffPrinter> fmt::Display for CustomUnifiedDiff<'_, P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let first_hunk = self.hunks.first().cloned().unwrap_or_default();
        let context_len = self.config.context_len.min(1024 * 1024);
        let mut pos = first_hunk.before.start.saturating_sub(context_len);
        let mut before_context_start = pos;
        let mut after_context_start = first_hunk.after.start.saturating_sub(context_len);
        let mut before_context_len = 0;
        let mut after_context_len = 0;
        let mut buffer = String::new();
        for hunk in self.hunks {
            if hunk.before.start - pos > 2 * context_len {
                if !buffer.is_empty() {
                    let end = (pos + context_len).min(self.before.len() as u32);
                    self.printer.display_header(
                        &mut *f,
                        before_context_start,
                        after_context_start,
                        before_context_len + end - pos,
                        after_context_len + end - pos,
                    )?;
                    write!(f, "{buffer}")?;
                    for &token in &self.before[pos as usize..end as usize] {
                        self.printer.display_context_token(&mut *f, token)?;
                    }
                    buffer.clear();
                }
                pos = hunk.before.start - context_len;
                before_context_start = pos;
                after_context_start = hunk.after.start - context_len;
                before_context_len = 0;
                after_context_len = 0;
            }
            for &token in &self.before[pos as usize..hunk.before.start as usize] {
                self.printer.display_context_token(&mut buffer, token)?;
            }
            let context_len = hunk.before.start - pos;
            before_context_len += hunk.before.len() as u32 + context_len;
            after_context_len += hunk.after.len() as u32 + context_len;
            self.printer.display_hunk(
                &mut buffer,
                &self.before[hunk.before.start as usize..hunk.before.end as usize],
                &self.after[hunk.after.start as usize..hunk.after.end as usize],
            )?;
            pos = hunk.before.end;
        }
        if !buffer.is_empty() {
            let end = (pos + context_len).min(self.before.len() as u32);
            self.printer.display_header(
                &mut *f,
                before_context_start,
                after_context_start,
                before_context_len + end - pos,
                after_context_len + end - pos,
            )?;
            write!(f, "{buffer}")?;
            for &token in &self.before[pos as usize..end as usize] {
                self.printer.display_context_token(&mut *f, token)?;
            }
            buffer.clear();
        }
        Ok(())
    }
}

// Simple abstraction over `unicode_segmentation::split_word_bounds` for `imara_diff::TokenSource`
struct SplitWordBoundaries<'a>(&'a str);

impl<'a> gix_imara_diff::TokenSource for SplitWordBoundaries<'a> {
    type Token = &'a str;
    type Tokenizer = unicode_segmentation::UWordBounds<'a>;

    fn tokenize(&self) -> Self::Tokenizer {
        self.0.split_word_bounds()
    }

    fn estimate_tokens(&self) -> u32 {
        // https://www.wyliecomm.com/2021/11/whats-the-best-length-of-a-word-online/
        (self.0.len() as f32 / 4.7f32) as u32
    }
}

// Determine if a hunk contains any diff marker (+, -) in the underline inputs
fn contains_diff_marker(input: &InternedInput<&str>, mut hunk: Hunk) -> bool {
    let contains_diff_marker = |idx: u32, source: &[Token]| {
        let line = &input.interner[source[idx as usize]];
        line.starts_with('+') || line.starts_with('-')
    };

    hunk.before.any(|i| contains_diff_marker(i, &input.before))
        || hunk.after.any(|i| contains_diff_marker(i, &input.after))
}

// Function to create an <a> link to a GitHub compare
fn a_github_compare(class: &str, owner: &str, repo: &str, base: &str, head: &str) -> String {
    format!(
        r#"<a href="https://github.com/{owner}/{repo}/compare/{base}..{head}" class="compare {class}">{base_6}..{head_6}</a>"#,
        base_6 = &base[..base.len().min(7)],
        head_6 = &head[..head.len().min(7)]
    )
}
