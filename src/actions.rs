use chrono::{DateTime, Utc};
use std::collections::HashMap;

use async_trait::async_trait;

use reqwest::Client;
use tera::{Context, Tera};

use crate::github::{self, GithubClient, Repository};

#[async_trait]
pub trait Action {
    async fn call(&self) -> String;
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

pub enum QueryKind {
    List,
    Count,
}

pub struct QueryMap<'a> {
    pub name: &'a str,
    pub kind: QueryKind,
    pub query: Box<dyn github::IssuesQuery + Send + Sync>,
}

#[derive(Debug, serde::Serialize)]
pub struct IssueDecorator {
    pub number: u64,
    pub title: String,
    pub html_url: String,
    pub repo_name: String,
    pub labels: String,
    pub assignees: String,
    pub updated_at: String,
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
    async fn call(&self) -> String {
        let gh = GithubClient::new_with_default_token(Client::new());

        let mut context = Context::new();
        let mut results = HashMap::new();

        for Query { repos, queries } in &self.actions {
            for repo in repos {
                let repository = Repository {
                    full_name: format!("{}/{}", repo.0, repo.1),
                };

                for QueryMap { name, kind, query } in queries {
                    let issues = query.query(&repository, &gh).await;

                    match issues {
                        Ok(issues_decorator) => match kind {
                            QueryKind::List => {
                                results
                                    .entry(*name)
                                    .or_insert(Vec::new())
                                    .extend(issues_decorator);
                            }
                            QueryKind::Count => {
                                let count = issues_decorator.len();
                                let result = if let Some(value) = context.get(*name) {
                                    value.as_u64().unwrap() + count as u64
                                } else {
                                    count as u64
                                };

                                context.insert(*name, &result);
                            }
                        },
                        Err(err) => {
                            eprintln!("ERROR: {}", err);
                            err.chain()
                                .skip(1)
                                .for_each(|cause| eprintln!("because: {}", cause));
                            std::process::exit(1);
                        }
                    }
                }
            }
        }

        for (name, issues) in &results {
            if name == &"proposed_fcp" {
                let fcp_map = crate::rfcbot::get_all_fcps().await.unwrap();
                println!("fcp_map: {:#?}", fcp_map);
                let mut fcp_results = Vec::new();
                for issue_decorator in issues.into_iter() {
                    println!("issue_decorator: {:#?}", &issue_decorator);
                    println!("issue_decorator.html_url: {:#?}", &issue_decorator.html_url);
                    let key = format!(
                        "rust-lang/{}:{}:{}",
                        issue_decorator.repo_name.clone(),
                        issue_decorator.number.clone(),
                        issue_decorator.title.clone(),
                    );
                    if let Some(fcp) = fcp_map.get(&key) {
                        let fcp_detail =
                            crate::rfcbot::FCPDecorator::from_issue_fcp(&fcp, &issue_decorator);

                        fcp_results.push(fcp_detail);
                    }
                }
                context.insert(*name, &fcp_results);
            } else {
                context.insert(*name, issues);
            }
        }

        println!("self.name: {}", self.name);
        println!("context: {:#?}", context);

        TEMPLATES
            .render(&format!("{}.tt", self.name), &context)
            .unwrap()
    }
}
