use crate::{
    config::{IssueLinksCheckCommitsConfig, IssueLinksConfig},
    github::GithubCommit,
    handlers::{Context, check_commits::MERGE_IGNORE_LIST},
};

pub(super) async fn branch_links_in_commits(
    ctx: &Context,
    conf: &IssueLinksConfig,
    commits: &[GithubCommit],
) -> Option<String> {
    match conf.check_commits {
        IssueLinksCheckCommitsConfig::Off => return None,
        IssueLinksCheckCommitsConfig::All | IssueLinksCheckCommitsConfig::Uncanonicalized => {}
    }

    let branch_links_commits = futures::future::join_all(
        commits
            .iter()
            .filter(|c| {
                !MERGE_IGNORE_LIST
                    .iter()
                    .any(|i| c.commit.message.starts_with(i))
            })
            .map(|c| async {
                let mapping = super::super::issue_links::collect_branch_sha_links_mapping(
                    &ctx,
                    &c.commit.message,
                )
                .await;
                if mapping.is_empty() {
                    None
                } else {
                    Some(format!("- {}\n", c.sha))
                }
            }),
    )
    .await
    .into_iter()
    .flatten()
    .collect::<String>();

    if branch_links_commits.is_empty() {
        None
    } else {
        Some(format!(
            r"There are links that are not permanent in the commit message of the following commits. Please use a permalink instead.
{branch_links_commits}",
        ))
    }
}
