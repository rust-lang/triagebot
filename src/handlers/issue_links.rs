//! This handler is used to canonicalize linked GitHub issues into their long form
//! so that when pulling subtree into the main repository we don't accidentaly
//! close issues in the wrong repository.
//!
//! Example: `Fixes #123` (in rust-lang/clippy) would now become `Fixes rust-lang/clippy#123`

use std::collections::HashSet;
use std::sync::LazyLock;
use std::{borrow::Cow, collections::HashMap};

use regex::{Captures, Regex};

use crate::{
    config::IssueLinksConfig,
    github::{IssuesAction, IssuesEvent},
    handlers::Context,
};

static LINKED_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\B(?P<issue>#[0-9]+)\b").unwrap());

static LINKED_FILE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"https://github\.com/(?P<org>[^/\s]+)/(?P<repo>[^/\s]+)/blob/(?P<ref>[^/\s]+)/(?P<filepath>[^\s]+)\b",
    )
    .unwrap()
});

type BranchShaMappings<'a> = HashMap<
    (
        /* org */ &'a str,
        /* repo */ &'a str,
        /* branch name */ &'a str,
    ),
    /* sha */ String,
>;

pub(super) struct IssueLinksInput {}

pub(super) async fn parse_input(
    _ctx: &Context,
    event: &IssuesEvent,
    config: Option<&IssueLinksConfig>,
) -> Result<Option<IssueLinksInput>, String> {
    if !event.issue.is_pr() {
        return Ok(None);
    }

    if !matches!(
        event.action,
        IssuesAction::Opened | IssuesAction::Reopened | IssuesAction::Edited
    ) {
        return Ok(None);
    }

    // Require a `[issue-links]` (or it's alias `[canonicalize-issue-links]`)
    // configuration block to enable the handler.
    if config.is_none() {
        return Ok(None);
    }

    Ok(Some(IssueLinksInput {}))
}

pub(super) async fn handle_input(
    ctx: &Context,
    _config: &IssueLinksConfig,
    e: &IssuesEvent,
    _input: IssueLinksInput,
) -> anyhow::Result<()> {
    let full_repo_name = e.issue.repository().full_repo_name();

    let shas = collect_branch_sha_links_mapping(ctx, &e.issue.body).await;

    let new_body = fix_linked_issues(&e.issue.body, full_repo_name.as_str());
    let new_body = fix_linked_files(&new_body, shas);

    if e.issue.body != new_body {
        e.issue.edit_body(&ctx.github, &new_body).await?;
    }

    Ok(())
}

fn fix_linked_issues<'a>(body: &'a str, full_repo_name: &str) -> Cow<'a, str> {
    let replace_by = format!("{full_repo_name}${{issue}}");
    parser::replace_all_outside_ignore_blocks(&LINKED_RE, body, replace_by)
}

fn fix_linked_files<'a>(body: &'a str, shas: BranchShaMappings<'a>) -> Cow<'a, str> {
    parser::replace_all_outside_ignore_blocks(&LINKED_FILE_RE, body, |caps: &Captures| {
        let org = &caps["org"];
        let repo = &caps["repo"];
        let ref_ = &caps["ref"];
        let filepath = &caps["filepath"];

        if let Some(sha) = shas.get(&(org, repo, ref_)) {
            format!("https://github.com/{org}/{repo}/blob/{sha}/{filepath}")
        } else {
            caps.get(0).unwrap().as_str().to_string()
        }
    })
}

pub(crate) async fn collect_branch_sha_links_mapping<'a>(
    ctx: &Context,
    body: &'a str,
) -> BranchShaMappings<'a> {
    let mut seen = HashSet::new();

    let shas: Vec<_> = LINKED_FILE_RE
        .captures_iter(body)
        .filter_map(|caps: Captures| {
            let org = caps.name("org").unwrap().as_str();
            let repo = caps.name("repo").unwrap().as_str();
            let ref_ = caps.name("ref").unwrap().as_str();

            // Let's assume that an alpha-numeric string of 40 and 64 characters
            // is most probably a SHA1 or SHA256 hash.
            let looks_like_sha = (ref_.len() == 40 || ref_.len() == 64)
                && ref_.chars().all(|c| c.is_ascii_alphanumeric());

            if looks_like_sha || !seen.insert((org, repo, ref_)) {
                return None;
            }

            Some(async move {
                match ctx
                    .github
                    .get_reference(org, repo, &format!("heads/{ref_}"))
                    .await
                {
                    Ok(git_ref) => Some(((org, repo, ref_), git_ref.object.sha)),
                    Err(err) => {
                        // maybe network error, or simply user error, either way don't fail
                        tracing::warn!("{err}");
                        None
                    }
                }
            })
        })
        // limit to maximum 10 different branches
        .take(10)
        .collect();

    futures::future::join_all(shas)
        .await
        .into_iter()
        .flatten()
        .collect()
}

#[test]
fn fixed_body_issues() {
    let full_repo_name = "rust-lang/rust";

    let body = r#"
This is a PR, which links to #123.

Fix #123
fixed #456
Fixes    #7895
Fixesd #7895
Closes: #987
resolves:   #655
Resolves #00000 Closes #888
    "#;

    let fixed_body = r#"
This is a PR, which links to rust-lang/rust#123.

Fix rust-lang/rust#123
fixed rust-lang/rust#456
Fixes    rust-lang/rust#7895
Fixesd rust-lang/rust#7895
Closes: rust-lang/rust#987
resolves:   rust-lang/rust#655
Resolves rust-lang/rust#00000 Closes rust-lang/rust#888
    "#;

    let new_body = fix_linked_issues(body, full_repo_name);
    assert_eq!(new_body, fixed_body);
}

#[test]
fn fixed_body_files() {
    let mut shas = HashMap::new();
    shas.insert(("torvalds", "linux", "master"), "123456789".to_string());

    let body = r#"
This is a PR, which links to https://github.com/torvalds/linux/blob/master/include/uapi/linux/audit.h#L389-L451.
    "#;

    let fixed_body = r#"
This is a PR, which links to https://github.com/torvalds/linux/blob/123456789/include/uapi/linux/audit.h#L389-L451.
    "#;

    let new_body = fix_linked_files(body, shas);
    assert_eq!(new_body, fixed_body);
}

#[test]
fn edge_case_body() {
    let full_repo_name = "rust-lang/rust";

    assert_eq!(
        fix_linked_issues("#132 with a end", full_repo_name),
        "rust-lang/rust#132 with a end"
    );
    assert_eq!(
        fix_linked_issues("with a start #132", full_repo_name),
        "with a start rust-lang/rust#132"
    );
    assert_eq!(
        fix_linked_issues("#132", full_repo_name),
        "rust-lang/rust#132"
    );
    assert_eq!(
        fix_linked_issues("(#132)", full_repo_name),
        "(rust-lang/rust#132)"
    );
}

#[test]
fn untouched_body() {
    let full_repo_name = "rust-lang/rust";
    let mut shas = HashMap::new();
    shas.insert(("torvalds", "linux", "master"), "123456789".to_string());

    let body = r#"
This is a PR.

Fix rust-lang#123
Resolves #abgt
Resolves: #abgt
Fixes #157a
Fixes#123
`Fixes #123`

```
Example: Fixes #123
Example2: https://github.com/torvalds/linux/blob/master/include/uapi/linux/audit.h#L389-L451
```

<!-- Fixes #123 -->
<!-- https://github.com/torvalds/linux/blob/master/include/uapi/linux/audit.h#L389-L451 -->
    "#;

    let new_body = fix_linked_issues(body, full_repo_name);
    let new_body = fix_linked_files(&new_body, shas);
    assert_eq!(new_body, body);
}
