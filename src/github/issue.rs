/// An issue or pull request.
///
/// For convenience, since issues and pull requests share most of their
/// fields, this struct is used for both. The `pull_request` field can be used
/// to determine which it is. Some fields are only available on pull requests
/// (but not always, check the GitHub API for details).
#[derive(Debug, serde::Deserialize)]
pub struct Issue {
    pub number: u64,
    #[serde(deserialize_with = "opt_string")]
    pub body: String,
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
    /// The SHA for a merge commit.
    ///
    /// This field is complicated, see the [Pull Request
    /// docs](https://docs.github.com/en/rest/pulls/pulls#get-a-pull-request)
    /// for details.
    #[serde(default)]
    pub merge_commit_sha: Option<String>,
    pub title: String,
    /// The common URL for viewing this issue or PR.
    ///
    /// Example: `https://github.com/octocat/Hello-World/pull/1347`
    pub html_url: String,
    // User performing an `action` (or PR/issue author)
    pub user: User,
    pub labels: Vec<Label>,
    // Users assigned to the issue/pr after `action` has been performed issue
    // (PR reviewers or issue assignees)
    // These are NOT the same as `IssueEvent.assignee`
    pub assignees: Vec<User>,
    /// Indicator if this is a pull request.
    ///
    /// This is `Some` if this is a PR (as opposed to an issue). Note that
    /// this does not always get filled in by GitHub, and must be manually
    /// populated (because some webhook events do not set it).
    pub pull_request: Option<PullRequestDetails>,
    /// Whether or not the pull request was merged.
    #[serde(default)]
    pub merged: bool,
    #[serde(default)]
    pub draft: bool,

    /// Number of comments
    pub comments: Option<u32>,

    /// The API URL for discussion comments.
    ///
    /// Example: `https://api.github.com/repos/octocat/Hello-World/issues/1347/comments`
    pub comments_url: String,
    /// The repository for this issue.
    ///
    /// Note that this is constructed via the [`Issue::repository`] method.
    /// It is not deserialized from the GitHub API.
    #[serde(skip)]
    pub repository: OnceLock<IssueRepository>,

    /// The base commit for a PR (the branch of the destination repo).
    #[serde(default)]
    pub base: Option<CommitBase>,
    /// The head commit for a PR (the branch from the source repo).
    #[serde(default)]
    pub head: Option<CommitBase>,
    /// Whether it is open or closed.
    pub state: IssueState,
    pub milestone: Option<Milestone>,
    /// Whether a PR has merge conflicts.
    pub mergeable: Option<bool>,

    /// How the author is associated with the repository
    pub author_association: AuthorAssociation,
}

#[derive(PartialEq, Eq, Debug, Clone, Ord, PartialOrd, serde::Deserialize)]
pub struct Label {
    pub name: String,
}

#[derive(Debug, serde::Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum IssueState {
    Open,
    Closed,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct CommitBase {
    pub sha: String,
    #[serde(rename = "ref")]
    pub git_ref: String,
    pub repo: Option<Repository>,
}

/// An indicator used to differentiate between an issue and a pull request.
///
/// Some webhook events include a `pull_request` field in the Issue object,
/// and some don't. GitHub does include a few fields here, but they aren't
/// needed at this time (merged_at, diff_url, html_url, patch_url, url).
#[derive(Debug, serde::Deserialize)]
#[cfg_attr(test, derive(Default))]
pub struct PullRequestDetails {
    /// This is a slot to hold the diff for a PR.
    ///
    /// This will be filled in only once as an optimization since multiple
    /// handlers want to see PR changes, and getting the diff can be
    /// expensive.
    #[serde(skip)]
    compare: tokio::sync::OnceCell<GithubCompare>,
}

impl PullRequestDetails {
    pub fn new() -> PullRequestDetails {
        PullRequestDetails {
            compare: tokio::sync::OnceCell::new(),
        }
    }
}

/// The return from GitHub compare API
#[derive(Debug, serde::Deserialize)]
pub struct GithubCompare {
    /// The base commit of the PR
    pub base_commit: GithubCommit,
    /// The merge base commit
    ///
    /// See <https://git-scm.com/docs/git-merge-base> for more details
    pub merge_base_commit: GithubCommit,
    /// List of file differences
    pub files: Vec<FileDiff>,
    /// List of commits
    pub commits: Vec<GithubCommit>,
}

/// Representation of a diff to a single file.
#[derive(Debug, serde::Deserialize)]
pub struct FileDiff {
    /// The fullname path of the file.
    pub filename: String,
    /// The patch/diff for the file.
    ///
    /// Can be empty when there isn't any changes to the content of the file
    /// (like when a file is renamed without it's content being modified).
    #[serde(default)]
    pub patch: String,
}

impl Issue {
    pub fn repository(&self) -> &IssueRepository {
        self.repository.get_or_init(|| {
            // https://api.github.com/repos/rust-lang/rust/issues/69257/comments
            log::trace!("get repository for {}", self.comments_url);
            let url = url::Url::parse(&self.comments_url).unwrap();
            let mut segments = url.path_segments().unwrap();
            let _comments = segments.next_back().unwrap();
            let _number = segments.next_back().unwrap();
            let _issues_or_prs = segments.next_back().unwrap();
            let repository = segments.next_back().unwrap();
            let organization = segments.next_back().unwrap();
            IssueRepository {
                organization: organization.into(),
                repository: repository.into(),
            }
        })
    }

    pub fn global_id(&self) -> String {
        format!("{}#{}", self.repository(), self.number)
    }

    pub fn is_pr(&self) -> bool {
        self.pull_request.is_some()
    }

    pub fn is_open(&self) -> bool {
        self.state == IssueState::Open
    }

    pub async fn edit_body(&self, client: &GithubClient, body: &str) -> anyhow::Result<()> {
        let edit_url = format!("{}/issues/{}", self.repository().url(client), self.number);
        #[derive(serde::Serialize)]
        struct ChangedIssue<'a> {
            body: &'a str,
        }
        client
            .send_req(client.patch(&edit_url).json(&ChangedIssue { body }))
            .await
            .context("failed to edit issue body")?;
        Ok(())
    }

    pub async fn edit_review(
        &self,
        client: &GithubClient,
        id: u64,
        new_body: &str,
    ) -> anyhow::Result<()> {
        let comment_url = format!(
            "{}/pulls/{}/reviews/{}",
            self.repository().url(client),
            self.number,
            id
        );
        #[derive(serde::Serialize)]
        struct NewComment<'a> {
            body: &'a str,
        }
        client
            .send_req(
                client
                    .put(&comment_url)
                    .json(&NewComment { body: new_body }),
            )
            .await
            .context("failed to edit review comment")?;
        Ok(())
    }

    pub async fn remove_labels(
        &self,
        client: &GithubClient,
        labels: Vec<Label>,
    ) -> anyhow::Result<()> {
        log::info!("remove_labels from {}: {:?}", self.global_id(), labels);

        // Don't try to remove labels not already present on this issue.
        let labels = labels
            .into_iter()
            .filter(|l| self.labels().contains(l))
            .collect::<Vec<_>>();

        log::info!(
            "remove_labels: {} filtered to {:?}",
            self.global_id(),
            labels
        );

        if labels.is_empty() {
            return Ok(());
        }

        // There is no API to remove all labels at once, so we issue as many
        // API requests are required in parallel.
        let requests = labels.into_iter().map(|label| async move {
            // DELETE /repos/:owner/:repo/issues/:number/labels/{name}
            let url = format!(
                "{repo_url}/issues/{number}/labels/{name}",
                repo_url = self.repository().url(client),
                number = self.number,
                name = label.name,
            );

            client
                .send_req(client.delete(&url))
                .await
                .with_context(|| format!("failed to remove {label:?}"))
        });

        futures::future::try_join_all(requests).await?;

        Ok(())
    }

    pub async fn add_labels(
        &self,
        client: &GithubClient,
        labels: Vec<Label>,
    ) -> anyhow::Result<()> {
        log::info!("add_labels: {} +{:?}", self.global_id(), labels);
        // POST /repos/:owner/:repo/issues/:number/labels
        // repo_url = https://api.github.com/repos/Codertocat/Hello-World
        let url = format!(
            "{repo_url}/issues/{number}/labels",
            repo_url = self.repository().url(client),
            number = self.number
        );

        // Don't try to add labels already present on this issue.
        let labels = labels
            .into_iter()
            .filter(|l| !self.labels().contains(l))
            .map(|l| l.name)
            .collect::<Vec<_>>();

        log::info!("add_labels: {} filtered to {:?}", self.global_id(), labels);

        if labels.is_empty() {
            return Ok(());
        }

        let mut unknown_labels = vec![];
        let mut known_labels = vec![];
        for label in labels {
            if self.repository().has_label(client, &label).await? {
                known_labels.push(label);
            } else {
                unknown_labels.push(label);
            }
        }

        if !unknown_labels.is_empty() {
            return Err(UserError::UnknownLabels {
                labels: unknown_labels,
            }
            .into());
        }

        #[derive(serde::Serialize)]
        struct LabelsReq {
            labels: Vec<String>,
        }

        client
            .send_req(client.post(&url).json(&LabelsReq {
                labels: known_labels,
            }))
            .await
            .context("failed to add labels")?;

        Ok(())
    }

    pub fn labels(&self) -> &[Label] {
        &self.labels
    }

    pub fn contain_assignee(&self, user: &str) -> bool {
        self.assignees
            .iter()
            .any(|a| a.login.to_lowercase() == user.to_lowercase())
    }

    pub async fn remove_assignees(
        &self,
        client: &GithubClient,
        selection: Selection<'_, str>,
    ) -> Result<(), AssignmentError> {
        log::info!("remove {:?} assignees for {}", selection, self.global_id());
        let url = format!(
            "{repo_url}/issues/{number}/assignees",
            repo_url = self.repository().url(client),
            number = self.number
        );

        let assignees = match selection {
            Selection::All => self
                .assignees
                .iter()
                .map(|u| u.login.as_str())
                .collect::<Vec<_>>(),
            Selection::One(user) => vec![user],
            Selection::Except(user) => self
                .assignees
                .iter()
                .map(|u| u.login.as_str())
                .filter(|&u| u.to_lowercase() != user.to_lowercase())
                .collect::<Vec<_>>(),
        };

        #[derive(serde::Serialize)]
        struct AssigneeReq<'a> {
            assignees: &'a [&'a str],
        }

        client
            .send_req(client.delete(&url).json(&AssigneeReq {
                assignees: &assignees[..],
            }))
            .await
            .map_err(AssignmentError::Other)?;

        Ok(())
    }

    pub async fn add_assignee(
        &self,
        client: &GithubClient,
        user: &str,
    ) -> Result<(), AssignmentError> {
        log::info!("add_assignee {} for {}", user, self.global_id());
        let url = format!(
            "{repo_url}/issues/{number}/assignees",
            repo_url = self.repository().url(client),
            number = self.number
        );

        #[derive(serde::Serialize)]
        struct AssigneeReq<'a> {
            assignees: &'a [&'a str],
        }

        let result: Issue = client
            .json(client.post(&url).json(&AssigneeReq { assignees: &[user] }))
            .await
            .map_err(AssignmentError::Other)?;

        // Invalid assignees are silently ignored. We can just check if the user is now
        // contained in the assignees list.
        let success = result
            .assignees
            .iter()
            .any(|u| u.login.as_str().to_lowercase() == user.to_lowercase());

        if success {
            Ok(())
        } else {
            Err(AssignmentError::InvalidAssignee)
        }
    }

    pub async fn set_assignee(
        &self,
        client: &GithubClient,
        user: &str,
    ) -> Result<(), AssignmentError> {
        log::info!("set_assignee for {} to {}", self.global_id(), user);
        self.add_assignee(client, user).await?;
        self.remove_assignees(client, Selection::Except(user))
            .await?;
        Ok(())
    }

    /// Sets the milestone of the issue or PR.
    ///
    /// This will create the milestone if it does not exist. The new milestone
    /// will start in the "open" state.
    pub async fn set_milestone(&self, client: &GithubClient, title: &str) -> anyhow::Result<()> {
        log::trace!(
            "Setting milestone for rust-lang/rust#{} to {}",
            self.number,
            title
        );

        let full_repo_name = self.repository().full_repo_name();
        let milestone = client
            .get_or_create_milestone(&full_repo_name, title, "open")
            .await?;

        client
            .set_milestone(&full_repo_name, &milestone, self.number)
            .await?;
        Ok(())
    }

    pub async fn close(&self, client: &GithubClient) -> anyhow::Result<()> {
        let edit_url = format!("{}/issues/{}", self.repository().url(client), self.number);
        #[derive(serde::Serialize)]
        struct CloseIssue<'a> {
            state: &'a str,
        }
        client
            .send_req(
                client
                    .patch(&edit_url)
                    .json(&CloseIssue { state: "closed" }),
            )
            .await
            .context("failed to close issue")?;
        Ok(())
    }

    /// Returns the diff in this event, for Open and Synchronize events for now.
    ///
    /// Returns `None` if the issue is not a PR.
    pub async fn diff(&self, client: &GithubClient) -> anyhow::Result<Option<&[FileDiff]>> {
        Ok(self.compare(client).await?.map(|c| c.files.as_ref()))
    }

    /// Returns the comparison of this event.
    ///
    /// Returns `None` if the issue is not a PR.
    pub async fn compare(&self, client: &GithubClient) -> anyhow::Result<Option<&GithubCompare>> {
        let Some(pr) = &self.pull_request else {
            return Ok(None);
        };
        let (before, after) = if let (Some(base), Some(head)) = (&self.base, &self.head) {
            (&base.sha, &head.sha)
        } else {
            return Ok(None);
        };

        let compare = pr
            .compare
            .get_or_try_init::<anyhow::Error, _, _>(|| async move {
                let req = client.get(&format!(
                    "{}/compare/{before}...{after}",
                    self.repository().url(client)
                ));
                client.json(req).await
            })
            .await?;
        Ok(Some(compare))
    }

    /// Returns the commits from this pull request (no commits are returned if this `Issue` is not
    /// a pull request).
    pub async fn commits(&self, client: &GithubClient) -> anyhow::Result<Vec<GithubCommit>> {
        if !self.is_pr() {
            return Ok(vec![]);
        }

        let mut commits = Vec::new();
        let mut page = 1;
        loop {
            let req = client.get(&format!(
                "{}/pulls/{}/commits?page={page}&per_page=100",
                self.repository().url(client),
                self.number
            ));

            let new: Vec<_> = client.json(req).await?;
            if new.is_empty() {
                break;
            }
            commits.extend(new);

            page += 1;
        }
        Ok(commits)
    }

    /// Returns the GraphQL ID of this issue.
    async fn graphql_issue_id(&self, client: &GithubClient) -> anyhow::Result<String> {
        let repo = self.repository();
        let mut issue_id = client
            .graphql_query(
                "query($owner:String!, $repo:String!, $issueNum:Int!) {
                    repository(owner: $owner, name: $repo) {
                        issue(number: $issueNum) {
                            id
                        }
                    }
                }
                ",
                serde_json::json!({
                    "owner": repo.organization,
                    "repo": repo.repository,
                    "issueNum": self.number,
                }),
            )
            .await?;
        let serde_json::Value::String(issue_id) =
            issue_id["data"]["repository"]["issue"]["id"].take()
        else {
            anyhow::bail!("expected issue id, got {issue_id}");
        };
        Ok(issue_id)
    }

    /// Transfers this issue to the given repository.
    pub async fn transfer(
        &self,
        client: &GithubClient,
        owner: &str,
        repo: &str,
    ) -> anyhow::Result<()> {
        let issue_id = self.graphql_issue_id(client).await?;
        let repo_id = client.graphql_repo_id(owner, repo).await?;
        client
            .graphql_query(
                "mutation ($issueId: ID!, $repoId: ID!) {
                  transferIssue(
                    input: {createLabelsIfMissing: false, issueId: $issueId, repositoryId: $repoId}
                  ) {
                    issue {
                      id
                    }
                  }
                }",
                serde_json::json!({
                    "issueId": issue_id,
                    "repoId": repo_id,
                }),
            )
            .await?;
        Ok(())
    }
}

// Comments

#[derive(Debug, serde::Deserialize)]
pub struct Comment {
    pub id: u64,
    pub node_id: String,
    #[serde(deserialize_with = "opt_string")]
    pub body: String,
    pub html_url: String,
    pub user: User,
    #[serde(default, alias = "submitted_at")] // for pull-request review comments
    pub created_at: Option<chrono::DateTime<Utc>>,
    #[serde(default)]
    pub updated_at: Option<chrono::DateTime<Utc>>,
    #[serde(default, rename = "state")]
    pub pr_review_state: Option<PullRequestReviewState>,
    pub author_association: AuthorAssociation,
}

#[derive(Debug, serde::Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PullRequestReviewState {
    Approved,
    ChangesRequested,
    Commented,
    Dismissed,
    Pending,
}

impl Issue {
    pub async fn get_comment(&self, client: &GithubClient, id: u64) -> anyhow::Result<Comment> {
        let comment_url = format!("{}/issues/comments/{}", self.repository().url(client), id);
        let comment = client.json(client.get(&comment_url)).await?;
        Ok(comment)
    }

    pub async fn get_first100_comments(
        &self,
        client: &GithubClient,
    ) -> anyhow::Result<Vec<Comment>> {
        let comment_url = format!(
            "{}/issues/{}/comments?page=1&per_page=100",
            self.repository().url(client),
            self.number,
        );
        client.json::<Vec<Comment>>(client.get(&comment_url)).await
    }

    pub async fn edit_comment(
        &self,
        client: &GithubClient,
        id: u64,
        new_body: &str,
    ) -> anyhow::Result<Comment> {
        let comment_url = format!("{}/issues/comments/{}", self.repository().url(client), id);
        #[derive(serde::Serialize)]
        struct NewComment<'a> {
            body: &'a str,
        }
        let comment = client
            .json(
                client
                    .patch(&comment_url)
                    .json(&NewComment { body: new_body }),
            )
            .await
            .context("failed to edit comment")?;
        Ok(comment)
    }

    pub async fn post_comment(&self, client: &GithubClient, body: &str) -> anyhow::Result<Comment> {
        #[derive(serde::Serialize)]
        struct PostComment<'a> {
            body: &'a str,
        }
        let comments_path = self
            .comments_url
            .strip_prefix("https://api.github.com")
            .expect("expected api host");
        let comments_url = format!("{}{comments_path}", client.api_url);
        let comment = client
            .json(client.post(&comments_url).json(&PostComment { body }))
            .await
            .context("failed to post comment")?;
        Ok(comment)
    }
}

// Hide comment

#[derive(Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReportedContentClassifiers {
    Abuse,
    Duplicate,
    OffTopic,
    Outdated,
    Resolved,
    Spam,
}

impl Issue {
    pub async fn hide_comment(
        &self,
        client: &GithubClient,
        node_id: &str,
        reason: ReportedContentClassifiers,
    ) -> anyhow::Result<()> {
        client
            .graphql_query(
                "mutation($node_id: ID!, $reason: ReportedContentClassifiers!) {
                    minimizeComment(input: {subjectId: $node_id, classifier: $reason}) {
                        __typename
                    }
                }",
                serde_json::json!({
                    "node_id": node_id,
                    "reason": reason,
                }),
            )
            .await?;
        Ok(())
    }
}

// Lock

#[derive(Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
pub enum LockReason {
    #[serde(rename = "off-topic")]
    OffTopic,
    #[serde(rename = "too heated")]
    TooHeated,
    #[serde(rename = "resolved")]
    Resolved,
    #[serde(rename = "spam")]
    Spam,
}

impl Issue {
    /// Lock an issue with an optional reason.
    pub async fn lock(
        &self,
        client: &GithubClient,
        reason: Option<LockReason>,
    ) -> anyhow::Result<()> {
        let lock_url = format!(
            "{}/issues/{}/lock",
            self.repository().url(client),
            self.number
        );
        #[derive(serde::Serialize)]
        struct LockReasonIssue {
            lock_reason: LockReason,
        }
        client
            .send_req({
                let req = client.put(&lock_url);

                if let Some(lock_reason) = reason {
                    req.json(&LockReasonIssue { lock_reason })
                } else {
                    req
                }
            })
            .await
            .context("failed to lock issue")?;
        Ok(())
    }
}

// Pull-request files

#[derive(Debug, serde::Deserialize)]
pub struct PullRequestFile {
    pub sha: String,
    pub filename: String,
    pub blob_url: String,
    pub additions: u64,
    pub deletions: u64,
    pub changes: u64,
}

impl Issue {
    pub async fn files(&self, client: &GithubClient) -> anyhow::Result<Vec<PullRequestFile>> {
        if !self.is_pr() {
            return Ok(vec![]);
        }

        let req = client.get(&format!(
            "{}/pulls/{}/files",
            self.repository().url(client),
            self.number
        ));
        client.json(req).await
    }
}

// Zulip <-> GitHub

/// Contains only the parts of `Issue` that are needed for turning the issue title into a Zulip
/// topic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZulipGitHubReference {
    pub number: u64,
    pub title: String,
    pub repository: IssueRepository,
}

impl ZulipGitHubReference {
    pub fn zulip_topic_reference(&self) -> String {
        let repo = &self.repository;
        if repo.organization == "rust-lang" {
            if repo.repository == "rust" {
                format!("#{}", self.number)
            } else {
                format!("{}#{}", repo.repository, self.number)
            }
        } else {
            format!("{}/{}#{}", repo.organization, repo.repository, self.number)
        }
    }
}

impl Issue {
    pub fn to_zulip_github_reference(&self) -> ZulipGitHubReference {
        ZulipGitHubReference {
            number: self.number,
            title: self.title.clone(),
            repository: self.repository().clone(),
        }
    }
}
