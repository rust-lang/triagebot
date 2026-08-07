use anyhow::Context;

use crate::github::GithubClient;

const ORG: &str = "rust-lang";
const REPO: &str = "goals";
const LABEL: &str = "C-tracking-issue";

pub struct GoalIssue {
    pub number: u64,
    pub title: String,
    pub assignees: Vec<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub labels: Vec<String>,
    pub last_comment: Option<LastGoalComment>,
}

pub struct LastGoalComment {
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(serde::Deserialize)]
struct GraphQlConnection {
    nodes: Vec<Option<GraphQlIssue>>,
    #[serde(rename = "pageInfo")]
    page_info: GraphQlPageInfo,
}

#[derive(serde::Deserialize)]
struct GraphQlPageInfo {
    #[serde(rename = "hasNextPage")]
    has_next_page: bool,
    #[serde(rename = "endCursor")]
    end_cursor: Option<String>,
}

#[derive(serde::Deserialize)]
struct GraphQlIssue {
    number: u64,
    title: String,
    #[serde(rename = "createdAt")]
    created_at: chrono::DateTime<chrono::Utc>,
    assignees: GraphQlNodes<GraphQlUser>,
    labels: Option<GraphQlNodes<GraphQlLabel>>,
    comments: GraphQlNodes<GraphQlComment>,
}

#[derive(serde::Deserialize)]
struct GraphQlNodes<T> {
    nodes: Vec<Option<T>>,
}

#[derive(serde::Deserialize)]
struct GraphQlUser {
    login: String,
}

#[derive(serde::Deserialize)]
struct GraphQlLabel {
    name: String,
}

#[derive(serde::Deserialize)]
struct GraphQlComment {
    #[serde(rename = "createdAt")]
    created_at: chrono::DateTime<chrono::Utc>,
}

impl From<GraphQlIssue> for GoalIssue {
    fn from(issue: GraphQlIssue) -> Self {
        let assignees = issue
            .assignees
            .nodes
            .into_iter()
            .flatten()
            .map(|assignee| assignee.login)
            .collect();

        let labels = issue
            .labels
            .map(|labels| {
                labels
                    .nodes
                    .into_iter()
                    .flatten()
                    .map(|label| label.name)
                    .collect()
            })
            .unwrap_or_default();

        let last_comment = issue
            .comments
            .nodes
            .into_iter()
            .flatten()
            .next()
            .map(|comment| LastGoalComment {
                created_at: comment.created_at,
            });

        Self {
            number: issue.number,
            title: issue.title,
            assignees,
            created_at: issue.created_at,
            labels,
            last_comment,
        }
    }
}

impl GithubClient {
    /// Get every open tracking issue in `rust-lang/goals`,
    /// including the latest comment's date.
    pub async fn open_goal_issues(&self) -> anyhow::Result<Vec<GoalIssue>> {
        let mut cursor = None::<String>;
        let mut issues = Vec::new();

        loop {
            let mut response = self
                .graphql_query(
                    r#"
query (
  $owner: String!
  $repo: String!
  $label: String!
  $cursor: String
) {
  repository(owner: $owner, name: $repo) {
    issues(
      first: 100
      after: $cursor
      states: [OPEN]
      labels: [$label]
      orderBy: {
        field: CREATED_AT
        direction: ASC
      }
    ) {
      nodes {
        number
        title
        createdAt
        assignees(first: 100) {
          nodes {
            login
          }
        }
        labels(first: 100) {
          nodes {
            name
          }
        }
        comments(last: 1) {
          nodes {
            createdAt
          }
        }
      }
      pageInfo {
        hasNextPage
        endCursor
      }
    }
  }
}
"#,
                    serde_json::json!({
                        "owner": ORG,
                        "repo": REPO,
                        "label": LABEL,
                        "cursor": cursor.as_deref(),
                    }),
                )
                .await
                .context("failed to fetch goal issues")?;

            let page = response
                .pointer_mut("/data/repository/issues")
                .context("data.repository.issues is missing from response")?
                .take();

            let page: GraphQlConnection =
                serde_json::from_value(page).context("failed to deserialize page")?;

            issues.extend(page.nodes.into_iter().flatten().map(GoalIssue::from));

            if !page.page_info.has_next_page {
                break;
            }

            cursor = page.page_info.end_cursor;
        }

        Ok(issues)
    }
}
