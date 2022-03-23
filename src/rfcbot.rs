use crate::actions::FCPDecorator;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Debug, Clone)]
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
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Reviewer {
    pub id: u32,
    pub login: String,
}
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Review {
    pub reviewer: Reviewer,
    pub approved: bool,
}
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FCPIssue {
    pub id: u32,
    pub number: u32,
    pub fk_milestone: Option<String>,
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
    pub issue: FCPIssue,
    pub status_comment: StatusComment,
}

fn quote_reply(markdown: &str) -> String {
    if markdown.is_empty() {
        String::from("*No content*")
    } else {
        format!("\n\t> {}", markdown.replace("\n", "\n\t> "))
    }
}

// #[async_trait]
// pub trait IssueToFCP {
//     async fn from_issue_fcp<'a>(
//         full_fcp: &FullFCP,
//         issue_decorator: &crate::actions::IssueDecorator,
//         client: &'a GithubClient,
//     ) -> anyhow::Result<Self>;
// }

// #[async_trait]
// impl<'q> IssueToFCP for FCPDecorator {
//     async fn from_issue_fcp<'a>(
//         full_fcp: &FullFCP,
//         issue_decorator: &crate::actions::IssueDecorator,
//         client: &'a GithubClient,
//     ) -> anyhow::Result<Self> {
//         let bot_tracking_comment_html_url = format!(
//             "{}#issuecomment-{}",
//             issue_decorator.html_url, full_fcp.fcp.fk_bot_tracking_comment
//         );
//         let bot_tracking_comment_content = quote_reply(&full_fcp.status_comment.body);
//         let fk_initiating_comment = full_fcp.fcp.fk_initiating_comment;
//         let initiating_comment_html_url = format!(
//             "{}#issuecomment-{}",
//             issue_decorator.html_url, fk_initiating_comment,
//         );
//         // TODO: get from GitHub
//         let url = format!(
//             "{}/issues/comments/{}",
//             issue_decorator.html_url, fk_initiating_comment
//         );
//         let init_comment_content = client._send_req(client.get(&url)).await?.json().await?;
//         let initiating_comment_content = quote_reply(&init_comment_content);

//         Self {
//             // shared properties with IssueDecorator
//             number: issue_decorator.number.clone(),
//             title: issue_decorator.title.clone(),
//             html_url: issue_decorator.html_url.clone(),
//             repo_name: issue_decorator.repo_name.clone(),
//             labels: issue_decorator.labels.clone(),
//             assignees: issue_decorator.assignees.clone(),
//             updated_at: issue_decorator.updated_at.clone(),

//             // additional properties from FullFCP (from rfcbot)
//             bot_tracking_comment_html_url,
//             bot_tracking_comment_content,
//             initiating_comment_html_url,
//             initiating_comment_content,
//         }
//     }
// }

impl FCPDecorator {
    pub fn from_issue_fcp(
        full_fcp: &FullFCP,
        issue_decorator: &crate::actions::IssueDecorator,
    ) -> Self {
        let bot_tracking_comment_html_url = format!(
            "{}#issuecomment-{}",
            issue_decorator.html_url, full_fcp.fcp.fk_bot_tracking_comment
        );
        let bot_tracking_comment_content = quote_reply(&full_fcp.status_comment.body);
        let initiating_comment_html_url = format!(
            "{}#issuecomment-{}",
            issue_decorator.html_url, full_fcp.fcp.fk_initiating_comment
        );
        // TODO: get from GitHub
        let initiating_comment_content = quote_reply(&String::new());

        Self {
            // shared properties with IssueDecorator
            number: issue_decorator.number.clone(),
            title: issue_decorator.title.clone(),
            html_url: issue_decorator.html_url.clone(),
            repo_name: issue_decorator.repo_name.clone(),
            labels: issue_decorator.labels.clone(),
            assignees: issue_decorator.assignees.clone(),
            updated_at: issue_decorator.updated_at.clone(),

            // additional properties from FullFCP (from rfcbot)
            bot_tracking_comment_html_url,
            bot_tracking_comment_content,
            initiating_comment_html_url,
            initiating_comment_content,
        }
    }
}

// pub struct FCPCollection {
//     pub fcps: Box<dyn HashMap<String, FullFCP> + Send + Sync>,
// }

// // pub trait FCPQuery {
// //     pub fn get<'a>(&'a self) -> anyhow::Result<HashMap<String, FullFCP>>;
// // }

// impl FCPCollection {
//     pub async fn get_all_fcps(&self) -> anyhow::Result<()> {
//         let url = Url::parse(&"https://rfcbot.rs/api/all")?;
//         let res = reqwest::get(url).await?.json::<Vec<FullFCP>>().await?;
//         let mut map: HashMap<String, FullFCP> = HashMap::new();
//         for full_fcp in res.into_iter() {
//             map.insert(
//                 format!(
//                     "{}:{}:{}",
//                     full_fcp.issue.repository.clone(),
//                     full_fcp.issue.number.clone(),
//                     full_fcp.issue.title.clone(),
//                 ),
//                 full_fcp,
//             );
//         }

//         self.fcps = Box::new(map);
//         Ok(())
//     }
// }

pub async fn get_all_fcps() -> anyhow::Result<HashMap<String, FullFCP>> {
    let url = Url::parse(&"https://rfcbot.rs/api/all")?;
    let res = reqwest::get(url).await?.json::<Vec<FullFCP>>().await?;
    let mut map: HashMap<String, FullFCP> = HashMap::new();
    for full_fcp in res.into_iter() {
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
