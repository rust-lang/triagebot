#[cynic::schema_for_derives(file = "src/github/github.graphql", module = "schema")]
mod queries {
    use super::schema;

    pub type DateTime = chrono::DateTime<chrono::Utc>;

    cynic::impl_scalar!(DateTime, schema::DateTime);

    #[derive(cynic::FragmentArguments, Debug)]
    pub struct LeastRecentlyReviewedPullRequestsArguments {
        pub after: Option<String>,
    }

    #[derive(cynic::QueryFragment, Debug)]
    #[cynic(graphql_type = "Query", argument_struct = "LeastRecentlyReviewedPullRequestsArguments")]
    pub struct LeastRecentlyReviewedPullRequests {
        #[arguments(owner = "rust-lang", name = "rust")]
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
}

mod schema {
    cynic::use_schema!("src/github/github.graphql");
}
