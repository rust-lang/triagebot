use anyhow::Context as _;
use serde::Deserialize;

use crate::github::GithubClient;

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct QueryResponse {
    pub repository: Option<Repository>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Repository {
    pub pull_requests: PullRequestConnection,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestConnection {
    pub page_info: PageInfo,
    pub nodes: Vec<PullRequest>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PageInfo {
    pub has_next_page: bool,
    pub end_cursor: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    pub number: i32,
    pub author: Option<Actor>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub url: String,
    pub title: String,
    pub is_draft: bool,
    pub labels: Option<Connection<Label>>,
    pub assignees: Connection<User>,
    pub comments: ConnectionWithCount<IssueComment>,
    pub latest_reviews: Option<ConnectionWithCount<PullRequestReview>>,
}

#[derive(Deserialize, Debug)]
pub struct Connection<T> {
    pub nodes: Vec<T>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionWithCount<T> {
    pub total_count: i32,
    pub nodes: Vec<T>,
}

#[derive(Deserialize, Debug)]
pub struct Actor {
    pub login: String,
}

#[derive(Deserialize, Debug)]
pub struct Label {
    pub name: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub login: String,
    pub database_id: Option<i32>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct IssueComment {
    pub author: Option<Actor>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestReview {
    pub author: Option<Actor>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl GithubClient {
    pub async fn least_recently_reviewed_prs(
        &self,
        owner: &str,
        name: &str,
    ) -> anyhow::Result<Vec<PullRequest>> {
        let mut prs = Vec::new();
        let mut after = Option::<String>::None;

        loop {
            let mut data = self
                .graphql_query(
                    r#"
        query LeastRecentlyReviewedPullRequests(
            $repository_owner: String!,
            $repository_name: String!,
            $after: String
        ) {
            repository(owner: $repository_owner, name: $repository_name) {
                pullRequests(
                    states: [OPEN],
                    first: 100,
                    after: $after,
                    labels: ["S-waiting-on-review"],
                    orderBy: { direction: ASC, field: UPDATED_AT }
                ) {
                    totalCount
                    pageInfo {
                        hasNextPage
                        endCursor
                    }
                    nodes {
                        number
                        author {
                            login
                        }
                        createdAt
                        url
                        title
                        isDraft
                        labels(first: 100) {
                            nodes {
                                name
                            }
                        }
                        assignees(first: 100) {
                            nodes {
                                login
                                databaseId
                            }
                        }
                        comments(first: 100, orderBy: { direction: DESC, field: UPDATED_AT }) {
                            totalCount
                            nodes {
                                author {
                                    login
                                }
                                createdAt
                            }
                        }
                        latestReviews(last: 20) {
                            totalCount
                            nodes {
                                author {
                                    login
                                }
                                createdAt
                            }
                        }
                    }
                }
            }
        }
    "#,
                    serde_json::json!({
                        "repository_owner": owner.to_string(),
                        "repository_name": name.to_string(),
                        "after": after,
                    }),
                )
                .await
                .context("failed to query the least recently reviewed prs")?;

            let response: QueryResponse =
                serde_json::from_value(data["data"].take()).context("failed to deserialize")?;

            let repository = response.repository.context("No repository.")?;
            prs.extend(repository.pull_requests.nodes);

            let page_info = repository.pull_requests.page_info;
            if !page_info.has_next_page || page_info.end_cursor.is_none() {
                break;
            }
            after = page_info.end_cursor;
        }

        Ok(prs)
    }
}
