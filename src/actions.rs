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

// Group all the pending FCP for these teams
pub const IN_PRE_FCP: &str = "in_pre_fcp";
pub const IN_FCP: &str = "in_fcp";
pub const FCP_FINISHED: &str = "fcp_finished";

pub const IN_PRE_FCP_COMPILER_TEAM: &str = "in_pre_fcp_compiler_team";
pub const IN_PRE_FCP_RUST: &str = "in_pre_fcp_rust";
pub const IN_PRE_FCP_FORGE: &str = "in_pre_fcp_forge";
const PENDINGFCP: [&str; 3] = [IN_PRE_FCP_COMPILER_TEAM, IN_PRE_FCP_RUST, IN_PRE_FCP_FORGE];

// Group all the FCP for these teams
pub const IN_FCP_COMPILER_TEAM: &str = "in_fcp_compiler_team";
pub const IN_FCP_RUST: &str = "in_fcp_rust";
pub const IN_FCP_FORGE: &str = "in_fcp_forge";
const THINGSINFCP: [&str; 3] = [IN_FCP_COMPILER_TEAM, IN_FCP_RUST, IN_FCP_FORGE];

// Group all the Finalized FCP for these teams
pub const FCP_FINISHED_COMPILER_TEAM: &str = "fcp_finished_compiler_team";
pub const FCP_FINISHED_RUST: &str = "fcp_finished_rust";
pub const FCP_FINISHED_FORGE: &str = "fcp_finished_forge";
const FINALIZEDFCP: [&str; 3] = [
    FCP_FINISHED_COMPILER_TEAM,
    FCP_FINISHED_RUST,
    FCP_FINISHED_FORGE,
];

#[async_trait]
impl<'a> Action for Step<'a> {
    async fn call(&self) -> String {
        let gh = GithubClient::new_with_default_token(Client::new());

        let mut context = Context::new();

        let mut all_pending_fcp: Vec<(&str, Vec<IssueDecorator>)> = vec![];
        let mut all_things_in_fcp: Vec<(&str, Vec<IssueDecorator>)> = vec![];
        let mut all_finalized_fcp: Vec<(&str, Vec<IssueDecorator>)> = vec![];

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

                                // group query results for multiline FCP and add them later
                                if PENDINGFCP.contains(name) {
                                    all_pending_fcp.push((IN_PRE_FCP, issues_decorator));
                                } else if THINGSINFCP.contains(name) {
                                    all_things_in_fcp.push((IN_FCP, issues_decorator));
                                } else if FINALIZEDFCP.contains(name) {
                                    all_finalized_fcp.push((FCP_FINISHED, issues_decorator));
                                } else {
                                    context.insert(*name, &issues_decorator);
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

        // Add to a single template the aggregate of each group
        for (name, issue_decorator) in all_pending_fcp {
            context.insert(name, &issue_decorator);
        }
        for (name, issue_decorator) in all_things_in_fcp {
            context.insert(name, &issue_decorator);
        }
        for (name, issue_decorator) in all_finalized_fcp {
            context.insert(name, &issue_decorator);
        }

        TEMPLATES
            .render(&format!("{}.tt", self.name), &context)
            .unwrap()
    }
}
