use std::fmt::Write;

use anyhow::{Context as _, bail};

use crate::{
    config::ConcernConfig,
    github::{Event, Label},
    handlers::Context,
    interactions::{EditIssueBody, ErrorComment},
};
use parser::command::concern::ConcernCommand;

const CONCERN_ISSUE_KEY: &str = "CONCERN-ISSUE";

#[derive(Debug, PartialEq, Eq, Default, Clone, serde::Serialize, serde::Deserialize)]
struct ConcernData {
    concerns: Vec<Concern>,
}

#[derive(Debug, PartialEq, Eq, Clone, serde::Serialize, serde::Deserialize)]
struct Concern {
    title: String,
    comment_url: String,
    status: ConcernStatus,
}

#[derive(Debug, PartialEq, Eq, Clone, serde::Serialize, serde::Deserialize)]
enum ConcernStatus {
    Active,
    Resolved { comment_url: String },
}

pub(super) async fn handle_command(
    ctx: &Context,
    config: &ConcernConfig,
    event: &Event,
    cmd: ConcernCommand,
) -> anyhow::Result<()> {
    let Event::IssueComment(issue_comment) = event else {
        bail!("concern issued on something other than a issue")
    };
    let Some(comment_url) = event.html_url() else {
        bail!("unable to retrieve the comment url")
    };
    let issue = &issue_comment.issue;

    // Verify that this issue isn't a rfcbot FCP, skip if it is
    match crate::rfcbot::get_all_fcps().await {
        Ok(fcps) => {
            if fcps.iter().any(|(_, fcp)| {
                fcp.issue.number as u64 == issue.number
                    && fcp.issue.repository == issue_comment.repository.full_name
            }) {
                tracing::info!(
                    "{}#{} tried to register a concern, blocked by our rfcbot FCP check",
                    issue_comment.repository.full_name,
                    issue.number,
                );
                return Ok(());
            }
        }
        Err(err) => {
            tracing::warn!(
                "unable to fetch rfcbot active FCPs: {:?}, skipping check",
                err
            );
        }
    }

    // Verify that the comment author is a team member in our team repo
    if !issue_comment
        .comment
        .user
        .is_team_member(&ctx.team)
        .await
        .context("failed to verify that the user is a team member")?
    {
        tracing::info!(
            "{}#{} tried to register a concern, but comment author {:?} is not part of the team repo",
            issue_comment.repository.full_name,
            issue.number,
            issue_comment.comment.user,
        );
        ErrorComment::new(&issue, "Only team members in the [team repo](https://github.com/rust-lang/team) can add or resolve concerns.")
            .post(&ctx.github)
            .await?;
        return Ok(());
    }

    let mut client = ctx.db.get().await;
    let mut edit: EditIssueBody<'_, ConcernData> =
        EditIssueBody::load(&mut client, &issue, CONCERN_ISSUE_KEY)
            .await
            .context("unable to fetch the concerns data")?;
    let concern_data = edit.data_mut();

    // Process the command by either adding a new concern or resolving the old one
    match cmd {
        ConcernCommand::Concern { title } => {
            // Only add a concern if it wasn't already added, we could be in an edit
            if !concern_data.concerns.iter().any(|c| c.title == title) {
                concern_data.concerns.push(Concern {
                    title,
                    status: ConcernStatus::Active,
                    comment_url: comment_url.to_string(),
                });
            } else {
                tracing::info!(
                    "concern with the same name ({title}) already exists ({:?})",
                    &concern_data.concerns
                );
            }
        }
        ConcernCommand::Resolve { title } => concern_data
            .concerns
            .iter_mut()
            .filter(|c| c.title == title)
            .for_each(|c| {
                c.status = ConcernStatus::Resolved {
                    comment_url: comment_url.to_string(),
                }
            }),
    }

    // Create the new markdown content listing all the concerns
    let new_content = markdown_content(&concern_data.concerns, &ctx.username);

    // Add or remove the labels
    if concern_data
        .concerns
        .iter()
        .any(|c| matches!(c.status, ConcernStatus::Active))
    {
        if let Err(err) = issue
            .add_labels(
                &ctx.github,
                config
                    .labels
                    .iter()
                    .map(|l| Label {
                        name: l.to_string(),
                    })
                    .collect(),
            )
            .await
        {
            tracing::error!("unable to add concern labels: {:?}", err);
            let labels = config.labels.join(", ");
            issue.post_comment(
                &ctx.github,
                &format!("*Psst, I was unable to add the labels ({labels}), could someone do it for me.*"),
            ).await.context("unable to post the comment failure it-self")?;
        }
    } else {
        issue
            .remove_labels(
                &ctx.github,
                config
                    .labels
                    .iter()
                    .map(|l| Label {
                        name: l.to_string(),
                    })
                    .collect(),
            )
            .await
            .context("unable to remove the concern labels")?;
    }

    // Apply the new markdown concerns list to the issue
    edit.apply(&ctx.github, new_content)
        .await
        .context("failed to apply the new concerns section markdown")?;

    Ok(())
}

fn markdown_content(concerns: &[Concern], bot: &str) -> String {
    if concerns.is_empty() {
        return "".to_string();
    }

    let active_concerns = concerns
        .iter()
        .filter(|c| matches!(c.status, ConcernStatus::Active))
        .count();

    let mut md = String::new();

    let _ = writeln!(md, "");

    if active_concerns > 0 {
        let _ = writeln!(md, "> [!CAUTION]");
    } else {
        let _ = writeln!(md, "> [!NOTE]");
    }

    let _ = writeln!(md, "> # Concerns ({active_concerns} active)");
    let _ = writeln!(md, ">");

    for &Concern {
        ref title,
        ref status,
        ref comment_url,
    } in concerns
    {
        let _ = match status {
            ConcernStatus::Active => {
                writeln!(md, "> - [{title}]({comment_url})")
            }
            ConcernStatus::Resolved {
                comment_url: resolved_comment_url,
            } => {
                writeln!(
                    md,
                    "> - ~~[{title}]({comment_url})~~ resolved in [this comment]({resolved_comment_url})"
                )
            }
        };
    }

    let _ = writeln!(md, ">");
    let _ = writeln!(
        md,
        "> *Managed by `@{bot}`—see [help](https://forge.rust-lang.org/triagebot/concern.html) for details.*"
    );

    md
}

#[test]
fn simple_markdown_content() {
    let concerns = &[
        Concern {
            title: "This is my concern about concern".to_string(),
            status: ConcernStatus::Active,
            comment_url: "https://github.com/fake-comment-1234".to_string(),
        },
        Concern {
            title: "This is a resolved concern".to_string(),
            status: ConcernStatus::Resolved {
                comment_url: "https://github.com/fake-comment-8888".to_string(),
            },
            comment_url: "https://github.com/fake-comment-4561".to_string(),
        },
    ];

    assert_eq!(
        markdown_content(concerns, "rustbot"),
        r#"
> [!CAUTION]
> # Concerns (1 active)
>
> - [This is my concern about concern](https://github.com/fake-comment-1234)
> - ~~[This is a resolved concern](https://github.com/fake-comment-4561)~~ resolved in [this comment](https://github.com/fake-comment-8888)
>
> *Managed by `@rustbot`—see [help](https://forge.rust-lang.org/triagebot/concern.html) for details.*
"#
    );
}

#[test]
fn resolved_concerns_markdown_content() {
    let concerns = &[Concern {
        title: "This is a resolved concern".to_string(),
        status: ConcernStatus::Resolved {
            comment_url: "https://github.com/fake-comment-8888".to_string(),
        },
        comment_url: "https://github.com/fake-comment-4561".to_string(),
    }];

    assert_eq!(
        markdown_content(concerns, "rustbot"),
        r#"
> [!NOTE]
> # Concerns (0 active)
>
> - ~~[This is a resolved concern](https://github.com/fake-comment-4561)~~ resolved in [this comment](https://github.com/fake-comment-8888)
>
> *Managed by `@rustbot`—see [help](https://forge.rust-lang.org/triagebot/concern.html) for details.*
"#
    );
}
