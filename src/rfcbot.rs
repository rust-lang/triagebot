use reqwest::Url;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub struct FCP {
    pub id: u32,
    pub fk_issue: u32,
    pub fk_initiator: u32,
    pub fk_initiating_comment: u32,
    pub disposition: Option<String>,
    pub fk_bot_tracking_comment: u32,
    pub fcp_start: Option<String>,
    pub fcp_closed: bool,
}
#[derive(Serialize, Deserialize, Debug)]
pub struct Reviewer {
    pub id: u32,
    pub login: String,
}
#[derive(Serialize, Deserialize, Debug)]
pub struct Review {
    pub reviewer: Reviewer,
    pub approved: bool,
}
#[derive(Serialize, Deserialize, Debug)]
pub struct FCPIssue {
    pub id: u32,
    pub number: u32,
    pub fk_milestone: Option<String>,
    pub fk_user: u32,
    pub fk_assignee: Option<String>,
    pub open: bool,
    pub is_pull_request: bool,
    pub title: String,
    pub body: String,
    pub locked: bool,
    pub closed_at: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub labels: Vec<String>,
    pub repository: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct StatusComment {
    pub id: u64,
    pub fk_issue: u32,
    pub fk_user: u32,
    pub body: String,
    pub created_at: String,
    pub updated_at: Option<String>,
    pub repository: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct FullFCP {
    pub fcp: FCP,
    pub reviews: Vec<Review>,
    pub issue: FCPIssue,
    pub status_comment: StatusComment,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct FCPDecorator {
    pub number: u64,
    pub title: String,
    pub html_url: String,
    pub repo_name: String,
    pub labels: String,
    pub assignees: String,
    pub updated_at: String,

    pub bot_tracking_comment: String,
    pub bot_tracking_comment_content: String,
    pub initiating_comment: String,
    pub initiating_comment_content: String,
}
