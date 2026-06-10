use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use async_trait::async_trait;
use tera::{Context, Tera};
use triagebot::actions::{Action, Query, QueryKind, QueryMap};
use triagebot::github::issue_query::Query as IssueQuery;
use triagebot::github::{GithubClient, Repository};
use triagebot::team_data::TeamClient;

pub static TEMPLATES: LazyLock<Tera> = LazyLock::new(|| match Tera::new("templates/*") {
    Ok(t) => t,
    Err(e) => {
        println!("Parsing error(s): {e}");
        ::std::process::exit(1);
    }
});

pub fn p_high_issues() -> Box<dyn Action> {
    Box::new(Step {
        name: "p_high_issues",
        actions: vec![Query {
            repos: vec![("rust-lang", "rust")],
            queries: vec![QueryMap {
                name: "tracking_issues",
                kind: QueryKind::List,
                query: Arc::new(IssueQuery {
                    filters: vec![("state", "open")],
                    include_labels: vec!["C-tracking-issue"],
                    exclude_labels: vec![
                        "T-libs-api",
                        "T-libs",
                        "T-lang",
                        "T-rustdoc",
                        "T-bootstrap",
                        "T-cargo",
                        "T-core",
                        "T-dev-tools",
                        "T-infra",
                        "T-lang",
                        "T-libs",
                        "T-libs-api",
                        "T-opsem",
                        "T-release",
                        "T-rustdoc",
                        "T-rustdoc-frontend",
                        "T-rustfmt",
                        "T-rust-analyzer",
                        "T-style",
                        "T-types",
                        "T-leadership-council",
                    ],
                }),
            }],
        }],
    })
}

pub struct Step<'a> {
    pub name: &'a str,
    pub actions: Vec<Query<'a>>,
}

#[async_trait]
impl Action for Step<'_> {
    async fn call(&self) -> anyhow::Result<String> {
        let mut gh = GithubClient::new_from_env();
        gh.set_retry_rate_limit(true);
        let team_api = TeamClient::new_from_env();

        let mut context = Context::new();
        let mut results = HashMap::new();

        let mut handles: Vec<tokio::task::JoinHandle<anyhow::Result<(String, QueryKind, Vec<_>)>>> =
            Vec::new();
        let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(5));

        for Query { repos, queries } in &self.actions {
            for repo in repos {
                let repository = Repository {
                    full_name: format!("{}/{}", repo.0, repo.1),
                    // These are unused for query.
                    default_branch: "master".to_string(),
                    fork: false,
                    parent: None,
                };

                for QueryMap { name, kind, query } in queries {
                    let semaphore = semaphore.clone();
                    let name = String::from(*name);
                    let kind = *kind;
                    let repository = repository.clone();
                    let gh = gh.clone();
                    let team_api = team_api.clone();
                    let query = query.clone();
                    handles.push(tokio::task::spawn(async move {
                        let _permit = semaphore.acquire().await?;
                        let issues = query
                            .query(&repository, false, false, &gh, &team_api)
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

        // let date = chrono::Utc::now().date_naive();
        // context.insert("CURRENT_DATE", &date);

        // Ok("TODO".to_string())

        Ok(TEMPLATES
            .render(&format!("{}.tt", self.name), &context)
            .unwrap())
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();
    let x = p_high_issues();
    print!("{}", x.call().await?);
    Ok(())
}
