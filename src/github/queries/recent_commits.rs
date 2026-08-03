use std::collections::HashSet;

use anyhow::Context as _;
use chrono::{DateTime, Utc};

use crate::github::{GithubClient, Repository};

#[derive(Debug)]
pub struct RecentCommit {
    pub title: String,
    pub pr_num: Option<i32>,
    pub oid: String,
    pub committed_date: DateTime<Utc>,
}

mod objects {
    use chrono::Utc;
    use serde::{Deserialize, Serialize};

    /// Custom scalar type or simple transparent wrapper
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct GitObjectID(pub String);

    /// Top-level GraphQL JSON response wrapper
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct RecentCommits {
        pub repository: Option<Repository>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Repository {
        #[serde(rename = "ref")]
        pub ref_: Option<Ref>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Ref {
        pub target: Option<GitObject>,
    }

    /// Represents the GraphQL union `GitObject`.
    /// Unrecognized types will deserialize into `Other`.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(tag = "__typename")]
    pub enum GitObject {
        Commit(Commit),
        #[serde(other)]
        Other,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Commit {
        pub history: CommitHistoryConnection,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct PageInfo {
        pub has_next_page: bool,
        pub end_cursor: Option<String>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct CommitHistoryConnection {
        pub total_count: i32,
        pub page_info: PageInfo,
        #[serde(default)]
        pub nodes: Vec<Commit2>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Commit2 {
        pub oid: GitObjectID,
        pub parents: CommitConnection,
        pub committed_date: chrono::DateTime<Utc>,
        pub message_headline: String,
        pub associated_pull_requests: Option<PullRequestConnection>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct CommitConnection {
        #[serde(default)]
        pub nodes: Vec<Commit3>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Commit3 {
        pub oid: GitObjectID,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct PullRequestConnection {
        #[serde(default)]
        pub nodes: Vec<PullRequest>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct PullRequest {
        pub number: i32,
        pub title: String,
    }
}

impl Repository {
    /// Returns a list of recent commits on the given branch.
    ///
    /// Returns results in the OID range `oldest` (exclusive) to `newest`
    /// (inclusive).
    pub async fn recent_commits(
        &self,
        client: &GithubClient,
        branch: &str,
        oldest: &str,
        newest: &str,
    ) -> anyhow::Result<Vec<RecentCommit>> {
        // This is used to deduplicate the results (so that a PR with multiple
        // commits will only show up once).
        let mut prs_seen = HashSet::new();
        let mut recent_commits = Vec::new(); // This is the final result.

        let mut after = None;
        let mut found_newest = false;
        let mut found_oldest = false;
        // This simulates --first-parent. We only care about top-level commits.
        // Unfortunately the GitHub API doesn't provide anything like that.
        let mut next_first_parent = None;
        // Search for `oldest` within 3 pages (300 commits).
        for _ in 0..3 {
            let mut data = client
                .graphql_query(
                    r#"
query RecentCommits($name: String!, $owner: String!, $branch: String!, $after: String) {
  repository(name: $name, owner: $owner) {
    ref(qualifiedName: $branch) {
      target {
        __typename
        ... on Commit {
          history(first: 100, after: $after) {
            totalCount
            pageInfo {
              hasNextPage
              endCursor
            }
            nodes {
              oid
              parents(first: 1) {
                nodes {
                  oid
                }
              }
              committedDate
              messageHeadline
              associatedPullRequests(first: 1) {
                nodes {
                  number
                  title
                }
              }
            }
          }
        }
      }
    }
  }
}
"#,
                    serde_json::json!({
                        "name": self.name(),
                        "owner": self.owner(),
                        "branch": branch,
                        "after": after,
                    }),
                )
                .await
                .with_context(|| {
                    format!(
                        "{} failed to get recent commits branch={branch}",
                        self.full_name
                    )
                })?;

            let response: objects::RecentCommits =
                serde_json::from_value(data["data"].take()).context("failed to deserialize")?;

            let target = response
                .repository
                .context("No repository.")?
                .ref_
                .context("No ref.")?
                .target
                .context("No target.")?;
            let objects::GitObject::Commit(commit) = target else {
                anyhow::bail!("unexpected target type {target:?}")
            };
            let commits = commit
                .history
                .nodes
                .into_iter()
                // Don't include anything newer than `newest`
                .skip_while(|node| {
                    if found_newest || node.oid.0 == newest {
                        found_newest = true;
                        false
                    } else {
                        // This should only happen if there is a commit that arrives
                        // between the time that `update_submodules` fetches the latest
                        // ref, and this runs. This window should be a few seconds, so it
                        // should be unlikely. This warning is here in case my assumptions
                        // about how things work is not correct.
                        tracing::warn!(
                            "unexpected race with submodule history, newest oid={newest} skipping oid={}",
                            node.oid.0
                        );
                        true
                    }
                })
                // Skip nodes that aren't the first parent
                .filter(|node| {
                    let this_first_parent = node.parents.nodes
                        .first()
                        .map(|parent| parent.oid.0.clone());

                    if let Some(first_parent) = &next_first_parent {
                        if first_parent == &node.oid.0 {
                            // Found the next first parent, include it and
                            // set next_first_parent to look for this
                            // commit's first parent.
                            next_first_parent = this_first_parent;
                            true
                        } else {
                            // Still looking for the next first parent.
                            false
                        }
                    } else {
                        // First commit.
                        next_first_parent = this_first_parent;
                        true
                    }
                })
                // Stop once reached the `oldest` commit
                .take_while(|node| {
                    if node.oid.0 == oldest {
                        found_oldest = true;
                        false
                    } else {
                        true
                    }
                })
                .filter_map(|node| {
                    // Determine if this is associated with a PR or not.
                    match node.associated_pull_requests
                        // Get the first PR (we only care about one)
                        .and_then(|mut pr| pr.nodes.pop()) {
                        Some(pr) => {
                            // Only include a PR once
                            if prs_seen.insert(pr.number) {
                                Some(RecentCommit {
                                    pr_num: Some(pr.number),
                                    title: pr.title,
                                    oid: node.oid.0.clone(),
                                    committed_date: node.committed_date,
                                })
                            } else {
                                None
                            }
                        }
                        None => {
                            // This is an unassociated commit, possibly
                            // created without a PR.
                            Some(RecentCommit {
                                pr_num: None,
                                title: node.message_headline,
                                oid: node.oid.0,
                                committed_date: node.committed_date,
                            })
                        }
                    }
                });
            recent_commits.extend(commits);
            let page_info = commit.history.page_info;
            if found_oldest || !page_info.has_next_page || page_info.end_cursor.is_none() {
                break;
            }
            after = page_info.end_cursor;
        }
        if !found_oldest {
            // This should probably do something more than log a warning, but
            // I don't think it is too important at this time (the log message
            // is only informational, and this should be unlikely to happen).
            tracing::warn!(
                "{} failed to find oldest commit sha={oldest} branch={branch}",
                self.full_name
            );
        }
        Ok(recent_commits)
    }
}
