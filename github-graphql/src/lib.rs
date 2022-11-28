//! Definitions for GitHub GraphQL.
//!
//! See <https://docs.github.com/en/graphql> for more GitHub's GraphQL API.

// This schema can be downloaded from https://docs.github.com/public/schema.docs.graphql
#[cynic::schema_for_derives(file = "src/github.graphql", module = "schema")]
pub mod queries {
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
        #[arguments(last = 20)]
        pub latest_reviews: Option<PullRequestReviewConnection>,
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

#[cynic::schema_for_derives(file = "src/github.graphql", module = "schema")]
pub mod docs_update_queries {
    use super::queries::{DateTime, PageInfo};
    use super::schema;

    #[derive(cynic::FragmentArguments, Debug)]
    pub struct RecentCommitsArguments {
        pub branch: String,
        pub name: String,
        pub owner: String,
        pub after: Option<String>,
    }

    /// Query for fetching recent commits and their associated PRs.
    ///
    /// This query is built from:
    ///
    /// ```text
    /// query RecentCommits($name: String!, $owner: String!, $branch: String!, $after: String) {
    ///   repository(name: $name, owner: $owner) {
    ///     ref(qualifiedName: $branch) {
    ///       target {
    ///         ... on Commit {
    ///           history(first: 100, after: $after) {
    ///             totalCount
    ///             pageInfo {
    ///               hasNextPage
    ///               endCursor
    ///             }
    ///             nodes {
    ///               oid
    ///               parents(first: 1) {
    ///                 nodes {
    ///                   oid
    ///                 }
    ///               }
    ///               committedDate
    ///               messageHeadline
    ///               associatedPullRequests(first: 1) {
    ///                 nodes {
    ///                   number
    ///                   title
    ///                 }
    ///               }
    ///             }
    ///           }
    ///         }
    ///       }
    ///     }
    ///   }
    /// }
    /// ```
    #[derive(cynic::QueryFragment, Debug)]
    #[cynic(graphql_type = "Query", argument_struct = "RecentCommitsArguments")]
    pub struct RecentCommits {
        #[arguments(name = &args.name, owner = &args.owner)]
        pub repository: Option<Repository>,
    }

    #[derive(cynic::QueryFragment, Debug)]
    #[cynic(argument_struct = "RecentCommitsArguments")]
    pub struct Repository {
        #[arguments(qualified_name = &args.branch)]
        #[cynic(rename = "ref")]
        pub ref_: Option<Ref>,
    }

    #[derive(cynic::QueryFragment, Debug)]
    #[cynic(argument_struct = "RecentCommitsArguments")]
    pub struct Ref {
        pub target: Option<GitObject>,
    }

    #[derive(cynic::QueryFragment, Debug)]
    #[cynic(argument_struct = "RecentCommitsArguments")]
    pub struct Commit {
        #[arguments(first = 100, after = &args.after)]
        pub history: CommitHistoryConnection,
    }

    #[derive(cynic::QueryFragment, Debug)]
    pub struct CommitHistoryConnection {
        pub total_count: i32,
        pub page_info: PageInfo,
        pub nodes: Option<Vec<Option<Commit2>>>,
    }

    #[derive(cynic::QueryFragment, Debug)]
    #[cynic(graphql_type = "Commit")]
    pub struct Commit2 {
        pub oid: GitObjectID,
        #[arguments(first = 1)]
        pub parents: CommitConnection,
        pub committed_date: DateTime,
        pub message_headline: String,
        #[arguments(first = 1)]
        pub associated_pull_requests: Option<PullRequestConnection>,
    }

    #[derive(cynic::QueryFragment, Debug)]
    pub struct PullRequestConnection {
        pub nodes: Option<Vec<Option<PullRequest>>>,
    }

    #[derive(cynic::QueryFragment, Debug)]
    pub struct PullRequest {
        pub number: i32,
        pub title: String,
    }

    #[derive(cynic::QueryFragment, Debug)]
    pub struct CommitConnection {
        pub nodes: Option<Vec<Option<Commit3>>>,
    }

    #[derive(cynic::QueryFragment, Debug)]
    #[cynic(graphql_type = "Commit")]
    pub struct Commit3 {
        pub oid: GitObjectID,
    }

    #[derive(cynic::InlineFragments, Debug)]
    #[cynic(argument_struct = "RecentCommitsArguments")]
    pub enum GitObject {
        Commit(Commit),
        // These three variants are here just to pacify cynic. I don't know
        // why it fails to compile without them.
        Tree(Tree),
        Tag(Tag),
        Blob(Blob),
    }

    #[derive(cynic::QueryFragment, Debug)]
    pub struct Tree {
        pub id: cynic::Id,
    }
    #[derive(cynic::QueryFragment, Debug)]
    pub struct Tag {
        pub id: cynic::Id,
    }
    #[derive(cynic::QueryFragment, Debug)]
    pub struct Blob {
        pub id: cynic::Id,
    }

    #[derive(cynic::Scalar, Debug, Clone)]
    pub struct GitObjectID(pub String);
}

mod schema {
    cynic::use_schema!("src/github.graphql");
}
