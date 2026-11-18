use anyhow::Context as _;

use crate::github::GithubClient;

#[derive(Debug, serde::Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum GitHubOpinionatedReviewState {
    Commented,
    Approved,
    ChangesRequested,
    Dismissed,
}

#[derive(Debug, serde::Deserialize, Clone, PartialEq, Eq)]
pub(crate) struct OpinionatedReview {
    pub id: String,
    pub state: GitHubOpinionatedReviewState,
}

impl GithubClient {
    pub(crate) async fn opinionated_reviews(
        &self,
        owner: &str,
        repo: &str,
        pr: u64,
    ) -> anyhow::Result<Vec<OpinionatedReview>> {
        let mut after = None;
        let mut reviews = Vec::new();

        loop {
            let mut data = self
                .graphql_query(
                    r#"
query OpinionatedReviews($owner: String!, $repo: String!, $prNumber: Int!, $after: String) {
  repository(owner: $owner, name: $repo) {
    pullRequest(number: $prNumber) {
      latestOpinionatedReviews(first: 100, writersOnly: true, after: $after) {
        nodes {
          id
          state
        }
        pageInfo {
          hasNextPage
          endCursor
        }
      }
    }
  }
}
                "#,
                    serde_json::json!({
                        "owner": owner,
                        "repo": repo,
                        "prNumber": pr,
                        "after": after,
                    }),
                )
                .await
                .context("failed to fetch opinionated reviews")?;

            let mut lor =
                data["data"]["repository"]["pullRequest"]["latestOpinionatedReviews"].take();

            let has_next_page = lor["pageInfo"]["hasNextPage"]
                .as_bool()
                .context("hasNextPage missing")?;
            let end_cursor = lor["pageInfo"]["endCursor"].as_str().map(|c| c.to_string());

            let new_reviews: Vec<OpinionatedReview> =
                serde_json::from_value(lor["nodes"].take())
                    .context("couldn't deserialize opinionated reviews")?;

            reviews.extend(new_reviews);

            if !has_next_page || end_cursor.is_none() {
                break;
            }

            after = Some(end_cursor);
        }

        Ok(reviews)
    }

    pub(crate) async fn dismiss_review(
        &self,
        review_id: &str,
        message: &str,
    ) -> anyhow::Result<()> {
        self
            .graphql_query(
                r"mutation($reviewId: ID!, $message: String!) {
                    dismissPullRequestReview(input: { pullRequestReviewId: $reviewId, message: $message }) {
                        clientMutationId
                    }
                }",
                serde_json::json!({
                    "reviewId": review_id,
                    "message": message
                }),
            )
            .await
            .with_context(|| format!("failed to dismiss review: {review_id:?}"))?;

        Ok(())
    }
}
