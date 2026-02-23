use anyhow::Context;
use async_trait::async_trait;
use std::collections::HashMap;

use super::GithubClient;
use super::Repository;
use super::utils::find_open_concerns;
use super::utils::quote_reply;
use crate::team_data::TeamClient;

#[async_trait]
pub trait IssuesQuery {
    async fn query<'a>(
        &'a self,
        repo: &'a Repository,
        include_fcp_details: bool,
        include_mcp_details: bool,
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
        include_fcp_details: bool,
        include_mcp_details: bool,
        gh_client: &'a GithubClient,
        team_client: &'a TeamClient,
    ) -> anyhow::Result<Vec<crate::actions::IssueDecorator>> {
        let issues = repo
            .get_issues(gh_client, self)
            .await
            .with_context(|| "Unable to get issues.")?;

        let fcp_map = if include_fcp_details {
            crate::rfcbot::get_all_fcps()
                .await
                .with_context(|| "Unable to get all fcps from rfcbot.")?
        } else {
            HashMap::new()
        };

        let zulip_map = if include_fcp_details {
            Some(team_client.zulip_map().await?)
        } else {
            None
        };

        let mut issues_decorator = Vec::new();
        let re = regex::Regex::new("https://github.com/rust-lang/|/").unwrap();
        let re_zulip_link = regex::Regex::new(r"\[stream\]:\s").unwrap();
        for issue in issues {
            let fcp_details = if include_fcp_details {
                let repository_name = if let Some(repo) = issue.repository.get() {
                    repo.repository.clone()
                } else {
                    let split = re.split(&issue.html_url).collect::<Vec<&str>>();
                    split[1].to_string()
                };
                let key = format!(
                    "rust-lang/{}:{}:{}",
                    repository_name, issue.number, issue.title,
                );

                if let Some(fcp) = fcp_map.get(&key) {
                    let bot_tracking_comment_html_url = format!(
                        "{}#issuecomment-{}",
                        issue.html_url, fcp.fcp.fk_bot_tracking_comment
                    );
                    let bot_tracking_comment_content = quote_reply(&fcp.status_comment.body);
                    let fk_initiating_comment = fcp.fcp.fk_initiating_comment;
                    let (initiating_comment_html_url, initiating_comment_content) = {
                        let comment = issue
                            .get_comment(gh_client, fk_initiating_comment)
                            .await
                            .with_context(|| {
                                format!(
                                    "failed to get first comment id={} for fcp={}",
                                    fk_initiating_comment, fcp.fcp.id
                                )
                            })?;
                        (comment.html_url, quote_reply(&comment.body))
                    };

                    // TODO: agree with the team(s) a policy to emit actual mentions to remind FCP
                    // voting member to cast their vote
                    let should_mention = false;
                    Some(crate::actions::FCPDetails {
                        bot_tracking_comment_html_url,
                        bot_tracking_comment_content,
                        initiating_comment_html_url,
                        initiating_comment_content,
                        disposition: fcp
                            .fcp
                            .disposition
                            .as_deref()
                            .unwrap_or("<unknown>")
                            .to_string(),
                        should_mention,
                        pending_reviewers: fcp
                            .reviews
                            .iter()
                            .filter(|r| !r.approved)
                            .map(|r| crate::actions::FCPReviewerDetails {
                                github_login: r.reviewer.login.clone(),
                                zulip_id: zulip_map.as_ref().and_then(|map| {
                                    map.users
                                        .iter()
                                        .find(|&(_, &github)| github == r.reviewer.id)
                                        .map(|(&zulip, _)| zulip)
                                }),
                            })
                            .collect(),
                        concerns: fcp
                            .concerns
                            .iter()
                            .map(|c| crate::actions::FCPConcernDetails {
                                name: c.name.clone(),
                                reviewer_login: c.reviewer.login.clone(),
                                concern_url: format!(
                                    "{}#issuecomment-{}",
                                    issue.html_url, c.comment.id
                                ),
                            })
                            .collect(),
                    })
                } else {
                    None
                }
            } else {
                None
            };

            let mcp_details = if include_mcp_details {
                let first100_comments = issue.get_first100_comments(gh_client).await?;
                let (zulip_link, concerns) = if first100_comments.is_empty() {
                    (String::new(), None)
                } else {
                    let split = re_zulip_link
                        .split(&first100_comments[0].body)
                        .collect::<Vec<&str>>();
                    let zulip_link = (*split.last().unwrap_or(&"#")).to_string();
                    let concerns = find_open_concerns(first100_comments);
                    (zulip_link, concerns)
                };

                Some(crate::actions::MCPDetails {
                    zulip_link,
                    concerns,
                })
            } else {
                None
            };

            issues_decorator.push(crate::actions::IssueDecorator {
                title: issue.title.clone(),
                number: issue.number,
                html_url: issue.html_url.clone(),
                repo_name: repo.name().to_owned(),
                labels: issue
                    .labels
                    .iter()
                    .map(|l| l.name.as_ref())
                    .collect::<Vec<_>>()
                    .join(", "),
                assignees: issue
                    .assignees
                    .iter()
                    .map(|u| u.login.as_ref())
                    .collect::<Vec<_>>()
                    .join(", "),
                author: issue.user.login,
                updated_at_hts: crate::actions::to_human(issue.updated_at),
                fcp_details,
                mcp_details,
            });
        }

        Ok(issues_decorator)
    }
}

pub struct LeastRecentlyReviewedPullRequests;
#[async_trait]
impl IssuesQuery for LeastRecentlyReviewedPullRequests {
    async fn query<'a>(
        &'a self,
        repo: &'a Repository,
        _include_fcp_details: bool,
        _include_mcp_details: bool,
        client: &'a GithubClient,
        _team_client: &'a TeamClient,
    ) -> anyhow::Result<Vec<crate::actions::IssueDecorator>> {
        use cynic::QueryBuilder;
        use github_graphql::queries;

        let repository_owner = repo.owner();
        let repository_name = repo.name();

        let mut prs: Vec<queries::PullRequest> = vec![];

        let mut args = queries::LeastRecentlyReviewedPullRequestsArguments {
            repository_owner,
            repository_name,
            after: None,
        };
        loop {
            let query = queries::LeastRecentlyReviewedPullRequests::build(args.clone());
            let req = client.post(&client.graphql_url);
            let req = req.json(&query);

            let data: cynic::GraphQlResponse<queries::LeastRecentlyReviewedPullRequests> =
                client.json(req).await?;
            if let Some(errors) = data.errors {
                anyhow::bail!("There were graphql errors. {errors:?}");
            }
            let repository = data
                .data
                .context("No data returned.")?
                .repository
                .context("No repository.")?;
            prs.extend(repository.pull_requests.nodes);
            let page_info = repository.pull_requests.page_info;
            if !page_info.has_next_page || page_info.end_cursor.is_none() {
                break;
            }
            args.after = page_info.end_cursor;
        }

        let mut prs: Vec<_> = prs
            .into_iter()
            .filter_map(|pr| {
                if pr.is_draft {
                    return None;
                }
                let labels = pr
                    .labels
                    .map(|l| l.nodes)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|node| node.name)
                    .collect::<Vec<_>>();
                if !labels.iter().any(|label| label == "T-compiler") {
                    return None;
                }
                let labels = labels.join(", ");

                let assignees: Vec<_> = pr
                    .assignees
                    .nodes
                    .into_iter()
                    .map(|user| user.login)
                    .collect();

                let mut reviews = pr
                    .latest_reviews
                    .map(|connection| connection.nodes)
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|node| {
                        let created_at = node.created_at;
                        node.author.map(|author| (author, created_at))
                    })
                    .map(|(author, created_at)| (author.login, created_at))
                    .collect::<Vec<_>>();

                reviews.sort_by_key(|r| r.1);

                let mut comments = pr
                    .comments
                    .nodes
                    .into_iter()
                    .filter_map(|node| {
                        let created_at = node.created_at;
                        node.author.map(|author| (author, created_at))
                    })
                    .map(|(author, created_at)| (author.login, created_at))
                    .filter(|comment| assignees.contains(&comment.0))
                    .collect::<Vec<_>>();

                comments.sort_by_key(|c| c.1);

                let updated_at = std::cmp::max(
                    reviews.last().map(|t| t.1).unwrap_or(pr.created_at),
                    comments.last().map(|t| t.1).unwrap_or(pr.created_at),
                );
                let assignees = assignees.join(", ");
                let author = pr.author.expect("checked");

                Some((
                    updated_at,
                    pr.number as u64,
                    pr.title,
                    pr.url.0,
                    repository_name,
                    labels,
                    author.login,
                    assignees,
                ))
            })
            .collect();
        prs.sort_by_key(|pr| pr.0);

        let prs: Vec<_> = prs
            .into_iter()
            .take(50)
            .map(
                |(updated_at, number, title, html_url, repo_name, labels, author, assignees)| {
                    let updated_at_hts = crate::actions::to_human(updated_at);

                    crate::actions::IssueDecorator {
                        number,
                        title,
                        html_url,
                        repo_name: repo_name.to_string(),
                        labels,
                        author,
                        assignees,
                        updated_at_hts,
                        fcp_details: None,
                        mcp_details: None,
                    }
                },
            )
            .collect();

        Ok(prs)
    }
}
