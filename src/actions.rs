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
    pub repos: Vec<&'a str>,
    pub queries: Vec<QueryMap<'a>>,
}

pub struct QueryMap<'a> {
    pub name: &'a str,
    pub query: github::Query<'a>,
}

#[derive(serde::Serialize)]
pub struct IssueDecorator {
    pub number: u64,
    pub title: String,
    pub html_url: String,
    pub repo_name: String,
    pub labels: String,
    pub assignees: String,
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

#[async_trait]
impl<'a> Action for Step<'a> {
    async fn call(&self) -> String {
        let gh = GithubClient::new_with_default_token(Client::new());

        let mut context = Context::new();
        let mut results = HashMap::new();

        for Query { repos, queries} in &self.actions {

            for repo in repos.iter() {
                let repository = Repository {
                    full_name: repo.to_string(),
                };

                for QueryMap { name, query } in queries {
                    match query.kind {
                        github::QueryKind::List => {
                            let issues_search_result = repository.get_issues(&gh, &query).await;

                            match issues_search_result {
                                Ok(issues) => {
                                    let issues_decorator: Vec<_> = issues
                                        .iter()
                                        .map(|issue| IssueDecorator {
                                            title: issue.title.clone(),
                                            number: issue.number,
                                            html_url: issue.html_url.clone(),
                                            repo_name: repository
                                                .full_name
                                                .split("/")
                                                .last()
                                                .expect("Failed to split repository name")
                                                .to_string(),
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

                        github::QueryKind::Count => {
                            let count = repository.get_issues_count(&gh, &query).await;

                            match count {
                                Ok(count) => {
                                    *context.entry(*name).or_insert(0) += count;
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
