#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct GitHubIssueWithComments {
    pub title: String,
    #[serde(rename = "titleHTML")]
    pub title_html: String,
    #[serde(rename = "bodyHTML")]
    pub body_html: String,
    pub state: GitHubIssueState,
    #[serde(rename = "stateReason")]
    pub state_reason: Option<GitHubIssueStateReason>,
    pub url: String,
    pub author: Option<GitHubSimplifiedAuthor>,
    #[serde(rename = "createdAt")]
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[serde(rename = "updatedAt")]
    pub updated_at: chrono::DateTime<chrono::Utc>,
    #[serde(rename = "reactionGroups")]
    pub reactions: Vec<GitHubGraphQlReactionGroup>,
    pub comments: GitHubGraphQlComments,
    #[serde(rename = "reviewThreads")]
    pub review_threads: Option<GitHubGraphQlReviewThreads>,
    pub reviews: Option<GitHubGraphQlReviews>,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct GitHubSimplifiedAuthor {
    pub login: String,
    #[serde(rename = "avatarUrl")]
    pub avatar_url: String,
}

impl Default for GitHubSimplifiedAuthor {
    fn default() -> Self {
        // Default to the "Deleted user" (https://github.com/ghost)
        GitHubSimplifiedAuthor {
            login: "ghost".to_string(),
            avatar_url: "https://avatars.githubusercontent.com/u/10137?v=4".to_string(),
        }
    }
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct GitHubGraphQlComments {
    pub nodes: Vec<GitHubGraphQlComment>,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct GitHubGraphQlComment {
    pub author: Option<GitHubSimplifiedAuthor>,
    #[serde(rename = "createdAt")]
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[serde(rename = "updatedAt")]
    pub updated_at: chrono::DateTime<chrono::Utc>,
    #[serde(rename = "isMinimized")]
    pub is_minimized: bool,
    #[serde(rename = "minimizedReason")]
    pub minimized_reason: Option<String>,
    #[serde(rename = "bodyHTML")]
    pub body_html: String,
    pub url: String,
    #[serde(rename = "reactionGroups")]
    pub reactions: Vec<GitHubGraphQlReactionGroup>,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct GitHubGraphQlReviewThreads {
    pub nodes: Vec<GitHubGraphQlReviewThread>,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct GitHubGraphQlReviewThread {
    #[serde(rename = "isCollapsed")]
    pub is_collapsed: bool,
    #[serde(rename = "isOutdated")]
    pub is_outdated: bool,
    #[serde(rename = "isResolved")]
    pub is_resolved: bool,
    pub path: String,
    pub comments: GitHubGraphQlReviewThreadComments,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct GitHubGraphQlReviewThreadComments {
    pub nodes: Vec<GitHubGraphQlReviewThreadComment>,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct GitHubGraphQlReviewThreadComment {
    pub author: Option<GitHubSimplifiedAuthor>,
    #[serde(rename = "createdAt")]
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[serde(rename = "updatedAt")]
    pub updated_at: chrono::DateTime<chrono::Utc>,
    #[serde(rename = "bodyHTML")]
    pub body_html: String,
    pub url: String,
    #[serde(rename = "reactionGroups")]
    pub reactions: Vec<GitHubGraphQlReactionGroup>,
    #[serde(rename = "pullRequestReview")]
    pub pull_request_review: GitHubGraphQlPullRequestReview,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct GitHubGraphQlPullRequestReview {
    pub id: String,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct GitHubGraphQlReviews {
    pub nodes: Vec<GitHubGraphQlReview>,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct GitHubGraphQlReview {
    pub author: Option<GitHubSimplifiedAuthor>,
    pub id: String,
    pub state: GitHubReviewState,
    #[serde(rename = "submittedAt")]
    pub submitted_at: chrono::DateTime<chrono::Utc>,
    #[serde(rename = "updatedAt")]
    pub updated_at: chrono::DateTime<chrono::Utc>,
    #[serde(rename = "bodyHTML")]
    pub body_html: String,
    #[serde(rename = "isMinimized")]
    pub is_minimized: bool,
    #[serde(rename = "minimizedReason")]
    pub minimized_reason: Option<String>,
    pub url: String,
    #[serde(rename = "reactionGroups")]
    pub reactions: Vec<GitHubGraphQlReactionGroup>,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct GitHubGraphQlReactionGroup {
    pub content: GitHubGraphQlReactionContent,
    pub users: GitHubGraphQlReactionGroupUsers,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct GitHubGraphQlReactionGroupUsers {
    #[serde(rename = "totalCount")]
    pub total_count: u32,
}

#[derive(Debug, serde::Deserialize, serde::Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GitHubGraphQlReactionContent {
    ThumbsUp,
    ThumbsDown,
    Laugh,
    Hooray,
    Confused,
    Heart,
    Rocket,
    Eyes,
}

#[derive(Debug, serde::Deserialize, serde::Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GitHubIssueState {
    Open,
    Closed,
    Merged,
}

#[derive(Debug, serde::Deserialize, serde::Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GitHubIssueStateReason {
    Completed,
    Duplicate,
    NotPlanned,
    Reopened,
}

impl GithubClient {
    pub async fn issue_with_comments(
        &self,
        owner: &str,
        repo: &str,
        issue: u64,
    ) -> anyhow::Result<GitHubIssueWithComments> {
        fn page_info(data: &serde_json::Value, key: &str) -> (bool, Option<String>) {
            if let Some(obj) = data.get(key) {
                let has_next = obj["pageInfo"]["hasNextPage"].as_bool().unwrap_or(false);
                let end_cursor = obj["pageInfo"]["endCursor"].as_str().map(|s| s.to_string());
                (has_next, end_cursor)
            } else {
                (false, None)
            }
        }

        let mut comments_cursor: Option<String> = None;
        let mut review_threads_cursor: Option<String> = None;
        let mut reviews_cursor: Option<String> = None;
        let mut all_comments = Vec::new();
        let mut all_review_threads = Vec::new();
        let mut all_reviews = Vec::new();
        let mut issue_json;

        loop {
            let mut data = self
        .graphql_query(
            "
query ($owner: String!, $repo: String!, $issueNumber: Int!, $commentsCursor: String, $reviewThreadsCursor: String, $reviewsCursor: String) {
  repository(owner: $owner, name: $repo) {
    issueOrPullRequest(number: $issueNumber) {
      ... on Issue {
        url
        state
        stateReason
        title
        titleHTML
        bodyHTML
        createdAt
        updatedAt
        author {
          login
          avatarUrl
        }
        reactionGroups {
          content
          users {
            totalCount
          }
        }
        comments(first: 100, after: $commentsCursor) {
          nodes {
            author {
              login
              avatarUrl
            }
            createdAt
            updatedAt
            isMinimized
            minimizedReason
            bodyHTML
            url
            reactionGroups {
              content
              users {
                totalCount
              }
            }
          }
          pageInfo {
            hasNextPage
            endCursor
          }
        }
      }
      ... on PullRequest {
        url
        state
        title
        titleHTML
        bodyHTML
        createdAt
        updatedAt
        author {
          login
          avatarUrl
        }
        reactionGroups {
          content
          users {
            totalCount
          }
        }
        comments(first: 100, after: $commentsCursor) {
          nodes {
            author {
              login
              avatarUrl
            }
            createdAt
            updatedAt
            isMinimized
            minimizedReason
            bodyHTML
            url
            reactionGroups {
              content
              users {
                totalCount
              }
            }
          }
          pageInfo {
            hasNextPage
            endCursor
          }
        }
        reviewThreads(first: 100, after: $reviewThreadsCursor) {
          nodes {
            isCollapsed
            isOutdated
            isResolved
            path
            comments(first: 100) {
              nodes {
                author {
                  login
                  avatarUrl
                }
                createdAt
                updatedAt
                bodyHTML
                url
                reactionGroups {
                  content
                  users {
                    totalCount
                  }
                }
                pullRequestReview {
                  id
                }
              }
            }
          }
          pageInfo {
            hasNextPage
            endCursor
          }
        }
        reviews(first: 100, after: $reviewsCursor) {
          nodes {
            author {
              login
              avatarUrl
            }
            id
            state
            submittedAt
            updatedAt
            isMinimized
            minimizedReason
            bodyHTML
            url
            reactionGroups {
              content
              users {
                totalCount
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
  }
}
                    ",
                    serde_json::json!({
                        "owner": owner,
                        "repo": repo,
                        "issueNumber": issue,
                        "commentsCursor": comments_cursor.as_deref(),
                        "reviewThreadsCursor": review_threads_cursor.as_deref(),
                        "reviewsCursor": reviews_cursor.as_deref(),
                    }),
                )
                .await
                .context("failed to fetch the issue with comments")?;

            issue_json = data["data"]["repository"]["issueOrPullRequest"].take();

            // Update all cursors from pageInfo
            let (comments_has_next, comments_end_cursor) = page_info(&issue_json, "comments");
            let comments_cursor_changed = comments_end_cursor != comments_cursor;

            let (review_threads_has_next, review_threads_end_cursor) =
                page_info(&issue_json, "reviewThreads");
            let review_threads_cursor_changed = review_threads_end_cursor != review_threads_cursor;

            let (reviews_has_next, reviews_end_cursor) = page_info(&issue_json, "reviews");
            let reviews_cursor_changed = reviews_end_cursor != reviews_cursor;

            // Update cursors for next iteration
            comments_cursor = comments_end_cursor;
            review_threads_cursor = review_threads_end_cursor;
            reviews_cursor = reviews_end_cursor;

            // Early return if first page has no more pages for any field (1 API call)
            if all_comments.is_empty()
                && all_review_threads.is_empty()
                && all_reviews.is_empty()
                && !comments_has_next
                && !review_threads_has_next
                && !reviews_has_next
            {
                return serde_json::from_value(issue_json)
                    .context("fail to deserialize the GraphQl json response");
            }

            // Only accumulate if cursor actually advanced (new page of data)
            if comments_cursor_changed {
                if let Some(comments_array) = issue_json["comments"]["nodes"].as_array_mut() {
                    all_comments.append(comments_array);
                }
            }

            // Only accumulate review threads if cursor advanced (only for PullRequest)
            if review_threads_cursor_changed {
                if let Some(threads_array) = issue_json["reviewThreads"]["nodes"].as_array_mut() {
                    all_review_threads.append(threads_array);
                }
            }

            // Only accumulate reviews if cursor advanced (only for PullRequest)
            if reviews_cursor_changed {
                if let Some(reviews_array) = issue_json["reviews"]["nodes"].as_array_mut() {
                    all_reviews.append(reviews_array);
                }
            }

            // Continue if any field has more pages
            if !comments_has_next && !review_threads_has_next && !reviews_has_next {
                break;
            }
        }

        // Reconstruct final result with all accumulated data
        let mut final_issue = issue_json;
        final_issue["comments"]["nodes"] = serde_json::Value::Array(all_comments);
        if let Some(threads) = final_issue.get_mut("reviewThreads") {
            threads["nodes"] = serde_json::Value::Array(all_review_threads);
        }
        if let Some(reviews) = final_issue.get_mut("reviews") {
            reviews["nodes"] = serde_json::Value::Array(all_reviews);
        }

        serde_json::from_value(final_issue).context("fail to deserialize final response")
    }
}