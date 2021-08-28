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
    pub query: github::GithubQuery<'a>,
}

#[derive(serde::Serialize)]
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
                    owner: repo.0.to_string(),
                    name: repo.1.to_string(),
                };

                for QueryMap { name, kind, query } in queries {
                    match query {
                        github::GithubQuery::REST(query) => {
                            match kind {
                                QueryKind::List => {
                                    let issues_search_result = repository.get_issues(&gh, &query).await;
        
                                    match issues_search_result {
                                        Ok(issues) => {
                                            let issues_decorator: Vec<_> = issues
                                                .iter()
                                                .map(|issue| IssueDecorator {
                                                    title: issue.title.clone(),
                                                    number: issue.number,
                                                    html_url: issue.html_url.clone(),
                                                    repo_name: repository.name.clone(),
                                                    labels: issue
                                                        .labels
                                                        .iter()
                                                        .map(|l| l.name.as_ref())
                                                        .collect::<Vec<_>>()
                                                        .join(", "),
                                                    assignees: issue
                                                        .assignees
                                                        .iter()
                                                        .map(|u| u.login.as_ref())
                                                        .collect::<Vec<_>>()
                                                        .join(", "),
                                                    updated_at: to_human(issue.updated_at),
                                                })
                                                .collect();
        
                                            results
                                                .entry(*name)
                                                .or_insert(Vec::new())
                                                .extend(issues_decorator);
                                        }
                                        Err(err) => {
                                            eprintln!("ERROR: {}", err);
                                            err.chain()
                                                .skip(1)
                                                .for_each(|cause| eprintln!("because: {}", cause));
                                            std::process::exit(1);
                                        }
                                    }
                                }
        
                                QueryKind::Count => {
                                    let count = repository.get_issues_count(&gh, &query).await;
        
                                    match count {
                                        Ok(count) => {
                                            let result = if let Some(value) = context.get(*name) {
                                                value.as_u64().unwrap() + count as u64
                                            } else {
                                                count as u64
                                            };
        
                                            context.insert(*name, &result);
                                        }
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
                        },
                        github::GithubQuery::GraphQL(query) => {
                            let issues = query.query(&repository, &gh).await;

                            match issues {
                                Ok(issues_decorator) => {
                                    match kind {
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
                                    }
                                }
                                Err(err) => {
                                    eprintln!("ERROR: {}", err);
                                    err.chain()
                                        .skip(1)
                                        .for_each(|cause| eprintln!("because: {}", cause));
                                    std::process::exit(1);
                                }
                            }
                        }
                    };
                }
            }
        }

        for (name, issues) in &results {
            context.insert(*name, issues);
        }

        TEMPLATES
            .render(&format!("{}.tt", self.name), &context)
            .unwrap()
    }
}
