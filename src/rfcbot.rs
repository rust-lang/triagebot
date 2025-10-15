use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Debug, Clone)]
#[allow(clippy::struct_field_names, reason = "the names come from an API")]
#[expect(
    clippy::upper_case_acronyms,
    reason = "https://github.com/rust-lang/triagebot/pull/2181#discussion_r2417056288"
)]
pub struct FCP {
    pub id: u32,
    pub fk_issue: u32,
    pub fk_initiator: u32,
    pub fk_initiating_comment: u64,
    pub disposition: Option<String>,
    pub fk_bot_tracking_comment: u64,
    pub fcp_start: Option<String>,
    pub fcp_closed: bool,
}
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Reviewer {
    pub id: u64,
    pub login: String,
}
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Review {
    pub reviewer: Reviewer,
    pub approved: bool,
}
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Concern {
    pub name: String,
    pub comment: StatusComment,
    pub reviewer: Reviewer,
}
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FCPIssue {
    pub id: u32,
    pub number: u32,
    pub fk_milestone: Option<u32>,
    pub fk_user: u32,
    pub fk_assignee: Option<u32>,
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

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct StatusComment {
    pub id: u64,
    pub fk_issue: u32,
    pub fk_user: u32,
    pub body: String,
    pub created_at: String,
    pub updated_at: Option<String>,
    pub repository: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FullFCP {
    pub fcp: FCP,
    pub reviews: Vec<Review>,
    pub concerns: Vec<Concern>,
    pub issue: FCPIssue,
    pub status_comment: StatusComment,
}

pub async fn get_all_fcps() -> anyhow::Result<HashMap<String, FullFCP>> {
    let url = Url::parse("https://rfcbot.rs/api/all")?;
    let res = reqwest::get(url).await?.json::<Vec<FullFCP>>().await?;
    let mut map: HashMap<String, FullFCP> = HashMap::new();
    for full_fcp in res {
        map.insert(
            format!(
                "{}:{}:{}",
                full_fcp.issue.repository.clone(),
                full_fcp.issue.number.clone(),
                full_fcp.issue.title.clone(),
            ),
            full_fcp,
        );
    }

    Ok(map)
}
