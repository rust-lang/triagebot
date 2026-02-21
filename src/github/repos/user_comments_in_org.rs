/// A comment made by a user on an issue or PR.
#[derive(Debug, Clone)]
pub struct UserComment {
    pub issue_title: String,
    pub issue_url: String,
    pub comment_url: String,
    pub body: String,
    pub created_at: Option<DateTime<Utc>>,
}

impl GithubClient {
    /// Fetches recent comments made by a user in a GitHub organization.
    ///
    /// Returns up to `limit` comments, sorted by creation date (most recent first).
    /// Each comment includes the URL, body snippet, and the issue/PR title it was made on.
    pub async fn user_comments_in_org(
        &self,
        username: &str,
        org: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<UserComment>> {
        // GitHub's GraphQL API doesn't seem to support filtering comments by author directly.
        // We can use two endpoints here - either use the search query and filter by commented and
        // organization, or use the user query and access its issueComments connection.
        // The user endpoint would be more efficient, in theory. However, if the user makes a lot of
        // comments in different organizations, we might load a lot of data before we get to their
        // comments in the given organization. So instead we use the search endpoint.

        // The endpoint loads issues (and PRs), not comments.
        // The `commenter:` filter guarantees each returned issue has at least one comment
        // from the user. So we fetch `limit` issues and a small number of recent comments
        // per issue, then filter to only the user's comments.
        let search_query = format!("commenter:{username} org:{org} sort:updated-desc");
        let issues_to_fetch = limit;
        let comments_per_issue = 100;

        let data = self
            .graphql_query(
                r#"
query($query: String!, $issueLimit: Int!, $commentLimit: Int!) {
  search(query: $query, type: ISSUE, first: $issueLimit) {
    nodes {
      ... on Issue {
        url
        title
        comments(first: $commentLimit, orderBy: {field: UPDATED_AT, direction: DESC}) {
          nodes {
            author { login }
            body
            url
            createdAt
          }
        }
      }
      ... on PullRequest {
        url
        title
        comments(first: $commentLimit, orderBy: {field: UPDATED_AT, direction: DESC}) {
          nodes {
            author { login }
            body
            url
            createdAt
          }
        }
      }
    }
  }
}
                "#,
                serde_json::json!({
                    "query": search_query,
                    "issueLimit": issues_to_fetch,
                    "commentLimit": comments_per_issue,
                }),
            )
            .await
            .context("failed to search for user comments")?;

        let mut all_comments: Vec<UserComment> = Vec::new();

        if let Some(nodes) = data["data"]["search"]["nodes"].as_array() {
            for node in nodes {
                let issue_title = node["title"].as_str().unwrap_or("Unknown");
                let issue_url = node["url"].as_str().unwrap_or("");

                if let Some(comments) = node["comments"]["nodes"].as_array() {
                    for comment in comments {
                        // Filter to only comments by the target user
                        let author = comment["author"]["login"].as_str().unwrap_or("");
                        if !author.eq_ignore_ascii_case(username) {
                            continue;
                        }

                        let body = comment["body"].as_str().unwrap_or("");
                        let url = comment["url"].as_str().unwrap_or("");
                        let created_at = comment["createdAt"]
                            .as_str()
                            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                            .map(|dt| dt.with_timezone(&chrono::Utc));

                        all_comments.push(UserComment {
                            issue_title: issue_title.to_string(),
                            issue_url: issue_url.to_string(),
                            comment_url: url.to_string(),
                            body: body.to_string(),
                            created_at,
                        });
                    }
                }
            }
        }

        // Sort by creation date (most recent first) and take the limit
        all_comments.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        all_comments.truncate(limit);

        Ok(all_comments)
    }
}