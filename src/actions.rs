use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tera::{Context, Tera};

use crate::github::{self, GithubClient, Repository};

#[async_trait]
pub trait Action {
    async fn call(&self) -> anyhow::Result<String>;
}

pub struct Step<'a> {
    pub name: &'a str,
    pub actions: Vec<Query<'a>>,
}

pub struct Query<'a> {
    /// Vec of (owner, name)
    pub repos: Vec<(&'a str, &'a str)>,
    pub queries: Vec<QueryMap<'a>>,
}

#[derive(Copy, Clone)]
pub enum QueryKind {
    List,
    Count,
}

pub struct QueryMap<'a> {
    pub name: &'a str,
    pub kind: QueryKind,
    pub query: Arc<dyn github::IssuesQuery + Send + Sync>,
}

#[derive(Debug, serde::Serialize)]
pub struct IssueDecorator {
    pub number: u64,
    pub title: String,
    pub html_url: String,
    pub repo_name: String,
    pub labels: String,
    pub assignees: String,
    // Human (readable) timestamp
    pub updated_at_hts: String,

    pub fcp_details: Option<FCPDetails>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct FCPDetails {
    pub bot_tracking_comment_html_url: String,
    pub bot_tracking_comment_content: String,
    pub initiating_comment_html_url: String,
    pub initiating_comment_content: String,
}

lazy_static! {
    pub static ref TEMPLATES: Tera = {
        match Tera::new("templates/*") {
            Ok(t) => t,
            Err(e) => {
                println!("Parsing error(s): {}", e);
                ::std::process::exit(1);
            }
        }
    };
}

pub fn to_human(d: DateTime<Utc>) -> String {
    let d1 = chrono::Utc::now() - d;
    let days = d1.num_days();
    if days > 60 {
        format!("{} months ago", days / 30)
    } else {
        format!("about {} days ago", days)
    }
}

#[async_trait]
impl<'a> Action for Step<'a> {
    async fn call(&self) -> anyhow::Result<String> {
        let gh = GithubClient::new_with_default_token(Client::new());

        let mut context = Context::new();
        let mut results = HashMap::new();

        let mut handles: Vec<tokio::task::JoinHandle<anyhow::Result<(String, QueryKind, Vec<_>)>>> =
            Vec::new();
        let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(5));

        for Query { repos, queries } in &self.actions {
            for repo in repos {
                let repository = Repository {
                    full_name: format!("{}/{}", repo.0, repo.1),
                };

                for QueryMap { name, kind, query } in queries {
                    let semaphore = semaphore.clone();
                    let name = String::from(*name);
                    let kind = *kind;
                    let repository = repository.clone();
                    let gh = gh.clone();
                    let query = query.clone();
                    handles.push(tokio::task::spawn(async move {
                        let _permit = semaphore.acquire().await?;
                        let issues = query
                            .query(&repository, name == "proposed_fcp", &gh)
                            .await?;
                        Ok((name, kind, issues))
                    }));
                }
            }
        }

        for handle in handles {
            let (name, kind, issues) = handle.await.unwrap()?;
            match kind {
                QueryKind::List => {
                    results.entry(name).or_insert(Vec::new()).extend(issues);
                }
                QueryKind::Count => {
                    let count = issues.len();
                    let result = if let Some(value) = context.get(&name) {
                        value.as_u64().unwrap() + count as u64
                    } else {
                        count as u64
                    };

                    context.insert(name, &result);
                }
            }
        }

        for (name, issues) in &results {
            context.insert(name, issues);
        }

        Ok(TEMPLATES
            .render(&format!("{}.tt", self.name), &context)
            .unwrap())
    }
}
