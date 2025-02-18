//! This handler implements collecting release notes from issues and PRs that are tagged with
//! `relnotes`. Any such tagging will open a new issue in rust-lang/rust responsible for tracking
//! the inclusion in releases notes.
//!
//! The new issue will be closed when T-release has added the text proposed (tracked in the issue
//! description) into the final release notes PR.
//!
//! The issue description will be edited manually by teams through the GitHub UI -- in the future,
//! we might add triagebot support for maintaining that text via commands or similar.
//!
//! These issues will also be automatically milestoned when their corresponding PR or issue is. In
//! the absence of a milestone, T-release is responsible for ascertaining which release is
//! associated with the issue.

use serde::{Deserialize, Serialize};

use crate::{
    db::issue_data::IssueData,
    github::{Event, IssuesAction},
    handlers::Context,
};

const RELNOTES_KEY: &str = "relnotes";

#[derive(Debug, Default, Deserialize, Serialize)]
struct RelnotesState {
    relnotes_issue: Option<u64>,
}

const TITLE_PREFIX: &str = "Tracking issue for release notes";

pub async fn handle(ctx: &Context, event: &Event) -> anyhow::Result<()> {
    let Event::Issue(e) = event else {
        return Ok(());
    };

    let repo = e.issue.repository();
    if !(repo.organization == "rust-lang" && repo.repository == "rust") {
        return Ok(());
    }

    if e.issue.title.starts_with(TITLE_PREFIX) {
        // Ignore these issues -- they're otherwise potentially self-recursive.
        return Ok(());
    }

    let mut client = ctx.db.get().await;
    let mut state: IssueData<'_, RelnotesState> =
        IssueData::load(&mut client, &e.issue, RELNOTES_KEY).await?;

    if let Some(paired) = state.data.relnotes_issue {
        // Already has a paired release notes issue.

        if let IssuesAction::Milestoned = &e.action {
            if let Some(milestone) = &e.issue.milestone {
                ctx.github
                    .set_milestone(&e.issue.repository().to_string(), &milestone, paired)
                    .await?;
            }
        }

        return Ok(());
    }

    if let IssuesAction::Labeled { label } = &e.action {
        let is_fcp_merge = label.name == "finished-final-comment-period"
            && e.issue
                .labels
                .iter()
                .any(|label| label.name == "disposition-merge");

        if label.name == "relnotes" || label.name == "relnotes-perf" || is_fcp_merge {
            let title = format!("{TITLE_PREFIX} of #{}: {}", e.issue.number, e.issue.title);
            let body = format!(
                "
This issue tracks the release notes text for #{}.

### Steps

- [ ] Proposed text is drafted by PR author (or team) making the noteworthy change.
- [ ] Issue is nominated for release team review of clarity for wider audience.
- [ ] Release team includes text in release notes/blog posts.

### Release notes text

The responsible team for the underlying change should edit this section to replace the automatically generated link with a succinct description of what changed, drawing upon text proposed by the author (either in discussion or through direct editing).

````markdown
# Category (e.g. Language, Compiler, Libraries, Compatibility notes, ...)
- [{}]({})
````

> [!TIP]
> Use the [previous releases](https://doc.rust-lang.org/nightly/releases.html) categories to help choose which one(s) to use.
> The category will be de-duplicated with all the other ones by the release team.
>
> *More than one section can be included if needed.*

### Release blog section

If the change is notable enough for inclusion in the blog post, the responsible team should add content to this section.
*Otherwise leave it empty.*

````markdown
````

cc {} -- origin issue/PR authors and assignees for starting to draft text
",
                e.issue.number, e.issue.title, e.issue.html_url,
                [&e.issue.user].into_iter().chain(e.issue.assignees.iter())
                    .map(|v| format!("@{}", v.login)).collect::<Vec<_>>().join(", ")
            );
            let resp = ctx
                .github
                .new_issue(
                    &e.issue.repository(),
                    &title,
                    &body,
                    ["relnotes", "relnotes-tracking-issue"]
                        .into_iter()
                        .chain(e.issue.labels.iter().map(|l| &*l.name).filter(|l| {
                            l.starts_with("A-") // A-* (area)
                            || l.starts_with("F-") // F-* (feature)
                            || l.starts_with("L-") // L-* (lint)
                            || l.starts_with("O-") // O-* (OS)
                            || l.starts_with("T-") // T-* (team)
                            || l.starts_with("WG-") // WG-* (working group)
                        }))
                        .map(ToOwned::to_owned)
                        .collect::<Vec<_>>(),
                )
                .await?;
            if let Some(milestone) = &e.issue.milestone {
                ctx.github
                    .set_milestone(&e.issue.repository().to_string(), &milestone, resp.number)
                    .await?;
            }
            state.data.relnotes_issue = Some(resp.number);
            state.save().await?;
        }
    }

    Ok(())
}
