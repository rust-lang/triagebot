use anyhow::Context;
use async_trait::async_trait;

use super::GithubClient;
use super::Repository;
use crate::team_data::TeamClient;

#[async_trait]
pub trait IssuesQuery {
    async fn query<'a>(
        &'a self,
        repo: &'a Repository,
        gh_client: &'a GithubClient,
        team_client: &'a TeamClient,
    ) -> anyhow::Result<Vec<crate::actions::IssueDecorator>>;
}

pub struct Query<'a> {
    // key/value filter
    pub filters: Vec<(&'a str, &'a str)>,
    pub include_labels: Vec<&'a str>,
    pub exclude_labels: Vec<&'a str>,
}

#[async_trait]
impl IssuesQuery for Query<'_> {
    async fn query<'a>(
        &'a self,
        repo: &'a Repository,
        gh_client: &'a GithubClient,
        _team_client: &'a TeamClient,
    ) -> anyhow::Result<Vec<crate::actions::IssueDecorator>> {
        let issues = repo
            .get_issues(gh_client, self)
            .await
            .with_context(|| "Unable to get issues.")?;

        let mut issues_decorator = Vec::new();
        for issue in issues {
            let labels = issue
                .labels
                .iter()
                .map(|l| l.name.as_ref())
                .collect::<Vec<&str>>();

            // guess the team this issue belongs to:
            // get the first T-* label associated with it
            let t_label = labels
                .iter()
                .find(|s| s.starts_with("T-"))
                .map_or("", |v| v);

            issues_decorator.push(crate::actions::IssueDecorator {
                title: issue.title.clone(),
                number: issue.number,
                html_url: issue.html_url.clone(),
                repo_name: repo.name().to_owned(),
                labels: labels.join(","),
                assignees: issue
                    .assignees
                    .iter()
                    .map(|u| u.login.as_ref())
                    .collect::<Vec<_>>()
                    .join(", "),
                author: issue.user.login,
                team: t_label.to_string(),
                updated_at_hts: crate::actions::to_human(issue.updated_at),
                created_at_hts: crate::actions::to_human(issue.created_at),
                is_blocked: false,
            });
        }

        Ok(issues_decorator)
    }
}
