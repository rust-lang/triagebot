use anyhow::Context;
use async_trait::async_trait;

// This schema can be downloaded from https://docs.github.com/en/graphql/overview/public-schema
#[cynic::schema_for_derives(file = "src/github/github.graphql", module = "schema")]
mod queries {
    use super::schema;

    pub type DateTime = chrono::DateTime<chrono::Utc>;

    cynic::impl_scalar!(DateTime, schema::DateTime);

    #[derive(cynic::FragmentArguments, Debug)]
    pub struct LeastRecentlyReviewedPullRequestsArguments {
        pub repository_owner: String,
        pub repository_name: String,
        pub after: Option<String>,
    }

    #[derive(cynic::QueryFragment, Debug)]
    #[cynic(
        graphql_type = "Query",
        argument_struct = "LeastRecentlyReviewedPullRequestsArguments"
    )]
    pub struct LeastRecentlyReviewedPullRequests {
        #[arguments(owner = &args.repository_owner, name = &args.repository_name)]
        pub repository: Option<Repository>,
    }

    #[derive(cynic::QueryFragment, Debug)]
    #[cynic(argument_struct = "LeastRecentlyReviewedPullRequestsArguments")]
    pub struct Repository {
        #[arguments(states = Some(vec![PullRequestState::Open]), first = 100, after = &args.after, labels = Some(vec!["S-waiting-on-review".to_string()]), order_by = IssueOrder { direction: OrderDirection::Asc, field: IssueOrderField::UpdatedAt })]
        pub pull_requests: PullRequestConnection,
    }

    #[derive(cynic::QueryFragment, Debug)]
    pub struct PullRequestConnection {
        pub total_count: i32,
        pub page_info: PageInfo,
        pub nodes: Option<Vec<Option<PullRequest>>>,
    }

    #[derive(cynic::QueryFragment, Debug)]
    pub struct PullRequest {
        pub number: i32,
        pub created_at: DateTime,
        pub url: Uri,
        pub title: String,
        #[arguments(first = 100)]
        pub labels: Option<LabelConnection>,
        pub is_draft: bool,
        #[arguments(first = 100)]
        pub assignees: UserConnection,
        #[arguments(first = 100, order_by = IssueCommentOrder { direction: OrderDirection::Desc, field: IssueCommentOrderField::UpdatedAt })]
        pub comments: IssueCommentConnection,
        #[arguments(last = 5)]
        pub reviews: Option<PullRequestReviewConnection>,
    }

    #[derive(cynic::QueryFragment, Debug)]
    pub struct PullRequestReviewConnection {
        pub total_count: i32,
        pub nodes: Option<Vec<Option<PullRequestReview>>>,
    }

    #[derive(cynic::QueryFragment, Debug)]
    pub struct PullRequestReview {
        pub author: Option<Actor>,
        pub created_at: DateTime,
    }

    #[derive(cynic::QueryFragment, Debug)]
    pub struct UserConnection {
        pub nodes: Option<Vec<Option<User>>>,
    }

    #[derive(cynic::QueryFragment, Debug)]
    pub struct User {
        pub login: String,
    }

    #[derive(cynic::QueryFragment, Debug)]
    pub struct PageInfo {
        pub has_next_page: bool,
        pub end_cursor: Option<String>,
    }

    #[derive(cynic::QueryFragment, Debug)]
    pub struct LabelConnection {
        pub nodes: Option<Vec<Option<Label>>>,
    }

    #[derive(cynic::QueryFragment, Debug)]
    pub struct Label {
        pub name: String,
    }

    #[derive(cynic::QueryFragment, Debug)]
    pub struct IssueCommentConnection {
        pub total_count: i32,
        pub nodes: Option<Vec<Option<IssueComment>>>,
    }

    #[derive(cynic::QueryFragment, Debug)]
    pub struct IssueComment {
        pub author: Option<Actor>,
        pub created_at: DateTime,
    }

    #[derive(cynic::Enum, Clone, Copy, Debug)]
    pub enum IssueCommentOrderField {
        UpdatedAt,
    }

    #[derive(cynic::Enum, Clone, Copy, Debug)]
    pub enum IssueOrderField {
        Comments,
        CreatedAt,
        UpdatedAt,
    }

    #[derive(cynic::Enum, Clone, Copy, Debug)]
    pub enum OrderDirection {
        Asc,
        Desc,
    }

    #[derive(cynic::Enum, Clone, Copy, Debug)]
    pub enum PullRequestState {
        Closed,
        Merged,
        Open,
    }

    #[derive(cynic::InputObject, Debug)]
    pub struct IssueOrder {
        pub direction: OrderDirection,
        pub field: IssueOrderField,
    }

    #[derive(cynic::InputObject, Debug)]
    pub struct IssueCommentOrder {
        pub direction: OrderDirection,
        pub field: IssueCommentOrderField,
    }

    #[derive(cynic::QueryFragment, Debug)]
    pub struct Actor {
        pub login: String,
    }

    #[derive(cynic::Scalar, Debug, Clone)]
    pub struct Uri(pub String);
}

mod schema {
    cynic::use_schema!("src/github/github.graphql");
}

pub struct LeastRecentlyReviewedPullRequests;
#[async_trait]
impl super::IssuesQuery for LeastRecentlyReviewedPullRequests {
    async fn query<'a>(
        &'a self,
        repo: &'a super::Repository,
        client: &'a super::GithubClient,
    ) -> anyhow::Result<Vec<crate::actions::IssueDecorator>> {
        use cynic::QueryBuilder;

        let repository_owner = repo.owner().to_owned();
        let repository_name = repo.name().to_owned();

        let mut prs = vec![];

        let mut args = queries::LeastRecentlyReviewedPullRequestsArguments {
            repository_owner,
            repository_name: repository_name.clone(),
            after: None,
        };
        loop {
            let query = queries::LeastRecentlyReviewedPullRequests::build(&args);
            let req = client.post(super::Repository::GITHUB_GRAPHQL_API_URL);
            let req = req.json(&query);

            let (resp, req_dbg) = client._send_req(req).await?;
            let response = resp.json().await.context(req_dbg)?;
            let data: cynic::GraphQlResponse<queries::LeastRecentlyReviewedPullRequests> =
                query.decode_response(response).with_context(|| {
                    format!("failed to parse response for `LeastRecentlyReviewedPullRequests`")
                })?;
            if let Some(errors) = data.errors {
                anyhow::bail!("There were graphql errors. {:?}", errors);
            }
            let repository = data
                .data
                .ok_or_else(|| anyhow::anyhow!("No data returned."))?
                .repository
                .ok_or_else(|| anyhow::anyhow!("No repository."))?;
            prs.extend(
                repository
                    .pull_requests
                    .nodes
                    .unwrap_or_default()
                    .into_iter(),
            );
            let page_info = repository.pull_requests.page_info;
            if !page_info.has_next_page || page_info.end_cursor.is_none() {
                break;
            }
            args.after = page_info.end_cursor;
        }

        let mut prs: Vec<_> = prs
            .into_iter()
            .filter_map(|pr| pr)
            .filter_map(|pr| {
                if pr.is_draft {
                    return None;
                }
                let labels = pr
                    .labels
                    .map(|labels| {
                        labels
                            .nodes
                            .map(|nodes| {
                                nodes
                                    .into_iter()
                                    .filter_map(|node| node)
                                    .map(|node| node.name)
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default()
                    })
                    .unwrap_or_default();
                if !labels.iter().any(|label| label == "T-compiler") {
                    return None;
                }
                let labels = labels.join(", ");

                let assignees: Vec<_> = pr
                    .assignees
                    .nodes
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|user| user)
                    .map(|user| user.login)
                    .collect();

                let mut reviews = pr
                    .reviews
                    .map(|reviews| {
                        reviews
                            .nodes
                            .map(|nodes| {
                                nodes
                                    .into_iter()
                                    .filter_map(|n| n)
                                    .map(|review| {
                                        (
                                            review
                                                .author
                                                .map(|a| a.login)
                                                .unwrap_or("N/A".to_string()),
                                            review.created_at,
                                        )
                                    })
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default()
                    })
                    .unwrap_or_default();
                reviews.sort_by_key(|r| r.1);

                let comments = pr
                    .comments
                    .nodes
                    .map(|nodes| {
                        nodes
                            .into_iter()
                            .filter_map(|n| n)
                            .map(|comment| {
                                (
                                    comment.author.map(|a| a.login).unwrap_or("N/A".to_string()),
                                    comment.created_at,
                                )
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let mut comments: Vec<_> = comments
                    .into_iter()
                    .filter(|comment| assignees.contains(&comment.0))
                    .collect();
                comments.sort_by_key(|c| c.1);

                let updated_at = std::cmp::max(
                    reviews.last().map(|t| t.1).unwrap_or(pr.created_at),
                    comments.last().map(|t| t.1).unwrap_or(pr.created_at),
                );
                let assignees = assignees.join(", ");

                Some((
                    updated_at,
                    pr.number as u64,
                    pr.title,
                    pr.url.0,
                    repository_name.clone(),
                    labels,
                    assignees,
                ))
            })
            .collect();
        prs.sort_by_key(|pr| pr.0);

        let prs: Vec<_> = prs
            .into_iter()
            .take(5)
            .map(
                |(updated_at, number, title, html_url, repo_name, labels, assignees)| {
                    let updated_at = crate::actions::to_human(updated_at);

                    crate::actions::IssueDecorator {
                        number,
                        title,
                        html_url,
                        repo_name,
                        labels,
                        assignees,
                        updated_at,
                    }
                },
            )
            .collect();

        Ok(prs)
    }
}
