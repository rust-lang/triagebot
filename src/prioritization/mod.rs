use futures::executor::block_on;
use reqwest::Client;
use std::env;
use std::fs::File;
use std::io::Read;

use crate::github::{GithubClient, Issue, Repository};

pub mod config;

pub struct Meeting<A: Action> {
    pub steps: Vec<A>,
}

pub trait Action {
    fn call(&self) -> String;
}

pub struct Step<'a> {
    pub name: &'a str,
    pub actions: Vec<RepoQuery<'a>>,
}

pub struct RepoQuery<'a> {
    pub repo: &'a str,
    pub queries: Vec<NamedQuery<'a>>,
}

pub struct NamedQuery<'a> {
    pub name: &'a str,
    pub query: Query<'a>,
}

pub struct Query<'a> {
    pub filters: Vec<&'a str>,
    pub include_labels: Vec<&'a str>,
    pub exclude_labels: Vec<&'a str>,
}

pub trait Template {
    fn render(&self) -> String;
}

pub struct FileTemplate<'a> {
    name: &'a str,
    map: Vec<(&'a str, Vec<Issue>)>,
}

impl<'a> Action for Step<'a> {
    fn call(&self) -> String {
        let gh = GithubClient::new(
            Client::new(),
            env::var("GITHUB_API_TOKEN").expect("Missing GITHUB_API_TOKEN"),
        );

        let map = self
            .actions
            .iter()
            .flat_map(|RepoQuery { repo, queries }| {
                let repository = Repository {
                    full_name: repo.to_string(),
                };

                queries
                    .iter()
                    .map(|NamedQuery { name, query }| {
                        let filters = query
                            .filters
                            .iter()
                            .map(|s| s.to_string())
                            .chain(
                                query
                                    .include_labels
                                    .iter()
                                    .map(|label| format!("label:{}", label)),
                            )
                            .chain(
                                query
                                    .exclude_labels
                                    .iter()
                                    .map(|label| format!("-label:{}", label)),
                            )
                            .chain(std::iter::once(format!("repo:{}", repository.full_name)))
                            .collect::<Vec<_>>();
                        let issues_search_result = block_on(
                            repository
                                .get_issues(&gh, &filters.iter().map(|s| s.as_ref()).collect()),
                        );

                        if let Err(err) = issues_search_result {
                            eprintln!("ERROR: {}", err);
                            err.chain()
                                .skip(1)
                                .for_each(|cause| eprintln!("because: {}", cause));
                            std::process::exit(1);
                        }

                        (*name, issues_search_result.unwrap().items)
                    })
                    .collect::<Vec<_>>()
            })
            .collect();

        let template = FileTemplate::new(self.name, map);
        template.render()
    }
}

impl<'a> FileTemplate<'a> {
    fn new(name: &'a str, map: Vec<(&'a str, Vec<Issue>)>) -> Self {
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

        for (var, issues) in &self.map {
            let var = format!("{{{}}}", var);
            if !issues.is_empty() {
                let issues = issues
                    .iter()
                    .map(|issue| {
                        format!(
                            "- \"{}\" [#{}]({})",
                            issue.title, issue.number, issue.html_url
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                contents = contents.replace(&var, &format!("{}", issues));
            } else {
                contents = contents.replace(&var, &format!("Empty"));
            }
        }

        contents
    }
}
