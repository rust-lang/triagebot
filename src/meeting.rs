use async_trait::async_trait;

use reqwest::Client;
use std::env;
use std::fs::File;
use std::io::Read;

use crate::github::{self, GithubClient, Issue, Repository};

pub struct Meeting<A: Action> {
    pub steps: Vec<A>,
}

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

pub trait Template: Send {
    fn render(&self) -> String;
}

pub struct FileTemplate<'a> {
    name: &'a str,
    map: Vec<(&'a str, Box<dyn Template>)>,
}

pub struct IssuesTemplate {
    issues: Vec<Issue>,
}

pub struct IssueCountTemplate {
    count: usize,
}

#[async_trait]
impl<'a> Action for Step<'a> {
    async fn call(&self) -> String {
        let gh = GithubClient::new(
            Client::new(),
            env::var("GITHUB_API_TOKEN").expect("Missing GITHUB_API_TOKEN"),
        );

        let mut map: Vec<(&str, Box<dyn Template>)> = Vec::new();

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
                                map.push((*name, Box::new(IssuesTemplate::new(issues))));
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
                                map.push((*name, Box::new(IssueCountTemplate::new(count))));
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

        let template = FileTemplate::new(self.name, map);
        template.render()
    }
}

impl<'a> FileTemplate<'a> {
    fn new(name: &'a str, map: Vec<(&'a str, Box<dyn Template>)>) -> Self {
        Self { name, map }
    }
}

impl<'a> Template for FileTemplate<'a> {
    fn render(&self) -> String {
        let relative_path = format!("templates/{}.tt", self.name);
        let path = env::current_dir().unwrap().join(relative_path);
        let path = path.as_path();
        let mut file = File::open(path).unwrap();
        let mut contents = String::new();
        file.read_to_string(&mut contents).unwrap();

        for (var, template) in &self.map {
            let var = format!("{{{}}}", var);
            contents = contents.replace(&var, &template.render());
        }

        contents
    }
}

impl IssuesTemplate {
    fn new(issues: Vec<Issue>) -> Self {
        Self { issues }
    }
}

impl Template for IssuesTemplate {
    fn render(&self) -> String {
        let mut out = String::new();

        if !self.issues.is_empty() {
            for issue in &self.issues {
                out.push_str(&format!(
                    "\"{}\" [#{}]({})\n",
                    issue.title, issue.number, issue.html_url,
                ));
            }
        } else {
            out = format!("Empty");
        }

        out
    }
}

impl IssueCountTemplate {
    fn new(count: usize) -> Self {
        Self { count }
    }
}

impl Template for IssueCountTemplate {
    fn render(&self) -> String {
        format!("\"{}\"", self.count)
    }
}
