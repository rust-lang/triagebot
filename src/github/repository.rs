use anyhow::Context;
use chrono::{DateTime, FixedOffset, Utc};
use itertools::Itertools;
use reqwest::StatusCode;
use std::collections::HashSet;
use tracing as log;

use super::issue_query::Query;
use super::{GitTreeEntry, GithubClient, Issue, IssueRepository, Milestone, PullRequestDetails};

#[derive(Clone, Debug, serde::Deserialize)]
#[cfg_attr(test, derive(Default))]
pub struct Repository {
    pub full_name: String,
    pub default_branch: String,
    #[serde(default)]
    pub fork: bool,
    pub parent: Option<Box<Repository>>,
}

impl Repository {
    fn url(&self, client: &GithubClient) -> String {
        format!("{}/repos/{}", client.api_url, self.full_name)
    }

    pub fn owner(&self) -> &str {
        self.full_name.split_once('/').unwrap().0
    }

    pub fn name(&self) -> &str {
        self.full_name.split_once('/').unwrap().1
    }

    pub async fn get_issues<'a>(
        &self,
        client: &GithubClient,
        query: &Query<'a>,
    ) -> anyhow::Result<Vec<Issue>> {
        #[derive(Debug, serde::Deserialize)]
        struct IssueSearchResult {
            total_count: u64,
            items: Vec<Issue>,
        }

        let Query {
            filters,
            include_labels,
            exclude_labels,
        } = query;

        let mut ordering = Ordering {
            sort: "created",
            direction: "asc",
            per_page: "100",
            page: 1,
        };
        let filters: Vec<_> = filters
            .clone()
            .into_iter()
            .filter(|(key, val)| {
                match *key {
                    "sort" => ordering.sort = val,
                    "direction" => ordering.direction = val,
                    "per_page" => ordering.per_page = val,
                    _ => return true,
                }
                false
            })
            .collect();

        // `is: pull-request` indicates the query to retrieve PRs only
        let is_pr = filters.contains(&("is", "pull-request"));

        // There are some cases that can only be handled by the search API:
        // 1. When using negating label filters (exclude_labels)
        // 2. When there's a key parameter key=no
        // 3. When the query is to retrieve PRs only and there are label filters
        //
        // Check https://docs.github.com/en/rest/reference/search#search-issues-and-pull-requests
        // for more information
        let use_search_api = !exclude_labels.is_empty()
            || filters.iter().any(|&(key, _)| key == "no")
            || is_pr && !include_labels.is_empty();

        // If there are more than `per_page` of issues, we need to paginate
        let mut issues = vec![];
        loop {
            let url = if use_search_api {
                self.build_search_issues_url(
                    client,
                    &filters,
                    include_labels,
                    exclude_labels,
                    ordering,
                )
            } else if is_pr {
                self.build_pulls_url(client, &filters, include_labels, ordering)
            } else {
                self.build_issues_url(client, &filters, include_labels, ordering)
            };

            let result = client.get(&url);
            if use_search_api {
                let result = client
                    .json::<IssueSearchResult>(result)
                    .await
                    .with_context(|| format!("failed to list issues from {url}"))?;
                issues.extend(result.items);
                if (issues.len() as u64) < result.total_count {
                    ordering.page += 1;
                    continue;
                }
            } else {
                // FIXME: paginate with non-search
                issues = client
                    .json(result)
                    .await
                    .with_context(|| format!("failed to list issues from {url}"))?;
            }

            break;
        }
        Ok(issues)
    }

    fn build_issues_url(
        &self,
        client: &GithubClient,
        filters: &[(&str, &str)],
        include_labels: &[&str],
        ordering: Ordering<'_>,
    ) -> String {
        self.build_endpoint_url(client, "issues", filters, include_labels, ordering)
    }

    fn build_pulls_url(
        &self,
        client: &GithubClient,
        filters: &[(&str, &str)],
        include_labels: &[&str],
        ordering: Ordering<'_>,
    ) -> String {
        self.build_endpoint_url(client, "pulls", filters, include_labels, ordering)
    }

    fn build_endpoint_url(
        &self,
        client: &GithubClient,
        endpoint: &str,
        filters: &[(&str, &str)],
        include_labels: &[&str],
        ordering: Ordering<'_>,
    ) -> String {
        let filters = filters
            .iter()
            .map(|(key, val)| format!("{key}={val}"))
            .chain([format!("labels={}", include_labels.join(","))])
            .chain(["filter=all".to_owned()])
            .chain([format!("sort={}", ordering.sort)])
            .chain([format!("direction={}", ordering.direction)])
            .chain([format!("per_page={}", ordering.per_page)])
            .format("&");
        format!(
            "{}/repos/{}/{}?{}",
            client.api_url, self.full_name, endpoint, filters
        )
    }

    fn build_search_issues_url(
        &self,
        client: &GithubClient,
        filters: &[(&str, &str)],
        include_labels: &[&str],
        exclude_labels: &[&str],
        ordering: Ordering<'_>,
    ) -> String {
        let filters = filters
            .iter()
            .filter(|filter| **filter != ("state", "all"))
            .map(|(key, val)| format!("{key}:{val}"))
            .chain(include_labels.iter().map(|label| format!("label:{label}")))
            .chain(exclude_labels.iter().map(|label| format!("-label:{label}")))
            .chain([format!("repo:{}", self.full_name)])
            .format("+");
        format!(
            "{}/search/issues?q={}&sort={}&order={}&per_page={}&page={}",
            client.api_url,
            filters,
            ordering.sort,
            ordering.direction,
            ordering.per_page,
            ordering.page,
        )
    }

    /// Creates a new git tree based on another tree.
    pub async fn update_tree(
        &self,
        client: &GithubClient,
        base_tree: &str,
        tree: &[GitTreeEntry],
    ) -> anyhow::Result<GitTreeObject> {
        let url = format!("{}/git/trees", self.url(client));
        client
            .json(client.post(&url).json(&serde_json::json!({
                "base_tree": base_tree,
                "tree": tree,
            })))
            .await
            .with_context(|| {
                format!(
                    "{} failed to update tree with base {base_tree}",
                    self.full_name
                )
            })
    }

    /// Creates a new PR.
    pub async fn new_pr(
        &self,
        client: &GithubClient,
        title: &str,
        head: &str,
        base: &str,
        body: &str,
    ) -> anyhow::Result<Issue> {
        let url = format!("{}/pulls", self.url(client));
        let mut issue: Issue = client
            .json(client.post(&url).json(&serde_json::json!({
                "title": title,
                "head": head,
                "base": base,
                "body": body,
            })))
            .await
            .with_context(|| {
                format!(
                    "{} failed to create a new PR head={head} base={base} title={title}",
                    self.full_name
                )
            })?;
        issue.pull_request = Some(PullRequestDetails::new());
        Ok(issue)
    }

    /// Synchronize a branch (in a forked repository) by pulling in its upstream contents.
    ///
    /// **Warning**: This will to a force update if there are conflicts.
    pub async fn merge_upstream(&self, client: &GithubClient, branch: &str) -> anyhow::Result<()> {
        let url = format!("{}/merge-upstream", self.url(client));
        let merge_error = match client
            .send_req(client.post(&url).json(&serde_json::json!({
                "branch": branch,
            })))
            .await
        {
            Ok(_) => return Ok(()),
            Err(e) => {
                if e.downcast_ref::<reqwest::Error>().is_some_and(|e| {
                    matches!(
                        e.status(),
                        Some(StatusCode::UNPROCESSABLE_ENTITY | StatusCode::CONFLICT)
                    )
                }) {
                    e
                } else {
                    return Err(e);
                }
            }
        };
        // 409 is a clear error that there is a merge conflict.
        // However, I don't understand how/why 422 might happen. The docs don't really say.
        // The gh cli falls back to trying to force a sync, so let's try that.
        log::info!(
            "{} failed to merge upstream branch {branch}, trying force sync: {merge_error:?}",
            self.full_name
        );
        let parent = self.parent.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "{} failed to merge upstream branch {branch}, \
                 force sync could not determine parent",
                self.full_name
            )
        })?;
        // Note: I'm not sure how to handle the case where the branch name
        // differs to the upstream. For example, if I create a branch off
        // master in my fork, somehow GitHub knows that my branch should push
        // to upstream/master (not upstream/my-branch-name). I can't find a
        // way to find that branch name. Perhaps GitHub assumes it is the
        // default branch if there is no matching branch name?
        let branch_ref = format!("heads/{branch}");
        let latest_parent_commit = parent
            .get_reference(client, &branch_ref)
            .await
            .with_context(|| {
                format!(
                    "failed to get head branch {branch} when merging upstream to {}",
                    self.full_name
                )
            })?;
        let sha = latest_parent_commit.object.sha;
        self.update_reference(client, &branch_ref, &sha)
            .await
            .with_context(|| {
                format!(
                    "failed to force update {branch} to {sha} for {}",
                    self.full_name
                )
            })?;
        Ok(())
    }

    /// Get or create a [`Milestone`].
    ///
    /// This will not change the state if it already exists.
    pub async fn get_or_create_milestone(
        &self,
        client: &GithubClient,
        title: &str,
        state: &str,
    ) -> anyhow::Result<Milestone> {
        client
            .get_or_create_milestone(&self.full_name, title, state)
            .await
    }

    /// Set the milestone of an issue or PR.
    pub async fn set_milestone(
        &self,
        client: &GithubClient,
        milestone: &Milestone,
        issue_num: u64,
    ) -> anyhow::Result<()> {
        client
            .set_milestone(&self.full_name, milestone, issue_num)
            .await
    }

    pub async fn get_issue(&self, client: &GithubClient, issue_num: u64) -> anyhow::Result<Issue> {
        let url = format!("{}/issues/{issue_num}", self.url(client));
        client
            .json(client.get(&url))
            .await
            .with_context(|| format!("{} failed to get issue {issue_num}", self.full_name))
    }

    pub async fn get_pr(&self, client: &GithubClient, pr_num: u64) -> anyhow::Result<Issue> {
        client
            .pull_request(
                &IssueRepository {
                    organization: self.owner().to_string(),
                    repository: self.name().to_string(),
                },
                pr_num,
            )
            .await
    }

    /// Returns a list of PRs "associated" with a commit.
    pub async fn pulls_for_commit(
        &self,
        client: &GithubClient,
        sha: &str,
    ) -> anyhow::Result<Vec<Issue>> {
        let url = format!("{}/commits/{sha}/pulls", self.url(client));
        client
            .json(client.get(&url))
            .await
            .with_context(|| format!("{} failed to get pulls for commit {sha}", self.full_name))
    }
}

// Reference

#[derive(Debug, serde::Deserialize)]
pub struct GitReference {
    #[serde(rename = "ref")]
    pub refname: String,
    pub object: GitObject,
}

#[derive(Debug, serde::Deserialize)]
pub struct GitObject {
    #[serde(rename = "type")]
    pub object_type: String,
    pub sha: String,
    pub url: String,
}

impl Repository {
    /// Retrieves a git reference for the given refname.
    pub async fn get_reference(
        &self,
        client: &GithubClient,
        refname: &str,
    ) -> anyhow::Result<GitReference> {
        let url = format!("{}/git/ref/{refname}", self.url(client));
        client
            .json(client.get(&url))
            .await
            .with_context(|| format!("{} failed to get git reference {refname}", self.full_name))
    }

    /// Updates an existing git reference to a new SHA.
    pub async fn update_reference(
        &self,
        client: &GithubClient,
        refname: &str,
        sha: &str,
    ) -> anyhow::Result<GitReference> {
        let url = format!("{}/git/refs/{refname}", self.url(client));
        client
            .json(client.patch(&url).json(&serde_json::json!({
                "sha": sha,
                "force": true,
            })))
            .await
            .with_context(|| {
                format!(
                    "{} failed to update reference {refname} to {sha}",
                    self.full_name
                )
            })
    }
}

// Merge conflicts

/// Information about a merge conflict on a PR.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeConflictInfo {
    /// Pull request number.
    pub number: u64,
    /// Whether this pull can be merged.
    pub mergeable: MergeableState,
    /// The branch name where this PR is requesting to be merged to.
    pub base_ref_name: String,
}

#[derive(Debug, serde::Deserialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MergeableState {
    Conflicting,
    Mergeable,
    Unknown,
}

impl Repository {
    /// Fetches information about merge conflicts on open PRs.
    pub async fn get_merge_conflict_prs(
        &self,
        client: &GithubClient,
    ) -> anyhow::Result<Vec<MergeConflictInfo>> {
        let mut prs = Vec::new();
        let mut after = None;
        loop {
            let mut data = client
                .graphql_query(
                    "query($owner:String!, $repo:String!, $after:String) {
                       repository(owner: $owner, name: $repo) {
                         pullRequests(states: OPEN, first: 100, after: $after) {
                           edges {
                             node {
                               number
                               mergeable
                               baseRefName
                             }
                           }
                           pageInfo {
                             hasNextPage
                             endCursor
                           }
                         }
                       }
                    }",
                    serde_json::json!({
                        "owner": self.owner(),
                        "repo": self.name(),
                        "after": after,
                    }),
                )
                .await?;
            let edges = data["data"]["repository"]["pullRequests"]["edges"].take();
            let serde_json::Value::Array(edges) = edges else {
                anyhow::bail!("expected array edges, got {edges:?}");
            };
            let this_page = edges
                .into_iter()
                .map(|mut edge| {
                    serde_json::from_value(edge["node"].take())
                        .with_context(|| "failed to deserialize merge conflicts")
                })
                .collect::<Result<Vec<_>, _>>()?;
            prs.extend(this_page);
            if !data["data"]["repository"]["pullRequests"]["pageInfo"]["hasNextPage"]
                .as_bool()
                .unwrap_or(false)
            {
                break;
            }
            after = Some(
                data["data"]["repository"]["pullRequests"]["pageInfo"]["endCursor"]
                    .as_str()
                    .expect("endCursor is string")
                    .to_string(),
            );
        }
        Ok(prs)
    }
}

// Commits

#[derive(Debug, serde::Deserialize)]
pub struct GithubCommit {
    pub sha: String,
    pub commit: GithubCommitCommitField,
    pub parents: Vec<Parent>,
    pub html_url: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct GithubCommitCommitField {
    pub author: GitUser,
    pub message: String,
    pub tree: GitCommitTree,
}

#[derive(Debug, serde::Deserialize)]
pub struct GitCommit {
    pub sha: String,
    pub author: GitUser,
    pub message: String,
    pub tree: GitCommitTree,
}

#[derive(Debug, serde::Deserialize)]
pub struct GitCommitTree {
    pub sha: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct GitTreeObject {
    pub sha: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct GitUser {
    pub date: DateTime<FixedOffset>,
}

#[derive(Debug, serde::Deserialize)]
pub struct Parent {
    pub sha: String,
}

impl Repository {
    /// Returns a list of commits between the SHA ranges of start (exclusive)
    /// and end (inclusive).
    pub async fn github_commits_in_range(
        &self,
        client: &GithubClient,
        start: &str,
        end: &str,
    ) -> anyhow::Result<Vec<GithubCommit>> {
        let mut commits = Vec::new();
        let mut page = 1;
        loop {
            let url = format!(
                "{}/commits?sha={end}&per_page=100&page={page}",
                self.url(client)
            );
            let mut this_page: Vec<GithubCommit> = client
                .json(client.get(&url))
                .await
                .with_context(|| format!("failed to fetch commits for {url}"))?;
            // This is a temporary debugging measure to investigate why the
            // `/commits` endpoint is not returning the expected values in
            // production.
            let v: String = this_page
                .iter()
                .map(|commit| {
                    format!(
                        "({}, {}, {:?}) ",
                        commit.sha, commit.commit.author.date, commit.parents
                    )
                })
                .collect();
            log::info!("page {page}: {v}");
            if let Some(idx) = this_page.iter().position(|commit| commit.sha == start) {
                this_page.truncate(idx);
                commits.extend(this_page);
                return Ok(commits);
            } else {
                commits.extend(this_page);
            }
            page += 1;
        }
    }

    pub async fn github_commit(
        &self,
        client: &GithubClient,
        sha: &str,
    ) -> anyhow::Result<GithubCommit> {
        let url = format!("{}/commits/{}", self.url(client), sha);
        client
            .json(client.get(&url))
            .await
            .with_context(|| format!("{} failed to get github commit {sha}", self.full_name))
    }

    /// Retrieves a git commit for the given SHA.
    pub async fn git_commit(&self, client: &GithubClient, sha: &str) -> anyhow::Result<GitCommit> {
        let url = format!("{}/git/commits/{sha}", self.url(client));
        client
            .json(client.get(&url))
            .await
            .with_context(|| format!("{} failed to get git commit {sha}", self.full_name))
    }

    /// Creates a new commit.
    pub async fn create_commit(
        &self,
        client: &GithubClient,
        message: &str,
        parents: &[&str],
        tree: &str,
    ) -> anyhow::Result<GitCommit> {
        let url = format!("{}/git/commits", self.url(client));
        client
            .json(client.post(&url).json(&serde_json::json!({
                "message": message,
                "parents": parents,
                "tree": tree,
            })))
            .await
            .with_context(|| format!("{} failed to create commit for tree {tree}", self.full_name))
    }
}

// Recent commits

pub struct RecentCommit {
    pub title: String,
    pub pr_num: Option<i32>,
    pub oid: String,
    pub committed_date: DateTime<Utc>,
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
        use cynic::QueryBuilder;
        use github_graphql::docs_update_queries::{
            GitObject, RecentCommits, RecentCommitsArguments,
        };

        let mut args = RecentCommitsArguments {
            branch,
            name: self.name(),
            owner: self.owner(),
            after: None,
        };
        let mut found_newest = false;
        let mut found_oldest = false;
        // This simulates --first-parent. We only care about top-level commits.
        // Unfortunately the GitHub API doesn't provide anything like that.
        let mut next_first_parent = None;
        // Search for `oldest` within 3 pages (300 commits).
        for _ in 0..3 {
            let query = RecentCommits::build(args.clone());
            let data = client
                .json::<cynic::GraphQlResponse<RecentCommits>>(
                    client.post(&client.graphql_url).json(&query),
                )
                .await
                .with_context(|| {
                    format!(
                        "{} failed to get recent commits branch={branch}",
                        self.full_name
                    )
                })?;

            if let Some(errors) = data.errors {
                anyhow::bail!("There were graphql errors. {errors:?}");
            }
            let target = data
                .data
                .context("No data returned.")?
                .repository
                .context("No repository.")?
                .ref_
                .context("No ref.")?
                .target
                .context("No target.")?;
            let GitObject::Commit(commit) = target else {
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
            args.after = page_info.end_cursor;
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

// Submodule

#[derive(Debug, serde::Deserialize)]
pub struct Submodule {
    pub name: String,
    pub path: String,
    pub sha: String,
    pub submodule_git_url: String,
}

impl Submodule {
    /// Returns the `Repository` this submodule points to.
    ///
    /// This assumes that the submodule is on GitHub.
    pub async fn repository(&self, client: &GithubClient) -> anyhow::Result<Repository> {
        let fullname = self
            .submodule_git_url
            .strip_prefix("https://github.com/")
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "only github submodules supported, got {}",
                    self.submodule_git_url
                )
            })?
            .strip_suffix(".git")
            .ok_or_else(|| {
                anyhow::anyhow!("expected .git suffix, got {}", self.submodule_git_url)
            })?;
        client.repository(fullname).await
    }
}

impl Repository {
    /// Returns information about the git submodule at the given path.
    ///
    /// `refname` is the ref to use for fetching information. If `None`, will
    /// use the latest version on the default branch.
    pub async fn submodule(
        &self,
        client: &GithubClient,
        path: &str,
        refname: Option<&str>,
    ) -> anyhow::Result<Submodule> {
        let mut url = format!("{}/contents/{}", self.url(client), path);
        if let Some(refname) = refname {
            url.push_str("?ref=");
            url.push_str(refname);
        }
        client.json(client.get(&url)).await.with_context(|| {
            format!(
                "{} failed to get submodule path={path} refname={refname:?}",
                self.full_name
            )
        })
    }
}

// Ordering

#[derive(Copy, Clone)]
struct Ordering<'a> {
    sort: &'a str,
    direction: &'a str,
    per_page: &'a str,
    page: u64,
}
