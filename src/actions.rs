use async_trait::async_trait;

use reqwest::Client;
use std::env;
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
    pub repo: &'a str,
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
    pub pr: String,
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
        let gh = GithubClient::new(
            Client::new(),
            env::var("GITHUB_API_TOKEN").expect("Missing GITHUB_API_TOKEN"),
        );

        let mut context = Context::new();

        for Query { repo, queries } in &self.actions {
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
                                    .map(|issue| {
                                        let pr = if issue.pull_request.is_some() {
                                            // FIXME: link to PR.
                                            // We need to tweak PullRequestDetails for this
                                            "[has_pr] "
                                        } else {
                                            ""
                                        }
                                        .to_string();

                                        IssueDecorator {
                                            title: issue.title.clone(),
                                            number: issue.number,
                                            html_url: issue.html_url.clone(),
                                            pr,
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
                                        }
                                    })
                                    .collect();

                                context.insert(*name, &issues_decorator);
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
                                context.insert(*name, &count);
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

        TEMPLATES
            .render(&format!("{}.tt", self.name), &context)
            .unwrap()
    }
}
