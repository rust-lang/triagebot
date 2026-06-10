use std::sync::{Arc, LazyLock};

use compiler_p_high::actions::{Action, Query, QueryKind, QueryMap, Step};
use compiler_p_high::github::default_token_from_env;
use compiler_p_high::github::issue_query::Query as IssueQuery;
use octocrab::Octocrab;
use tera::Tera;

pub static TEMPLATES: LazyLock<Tera> = LazyLock::new(|| match Tera::new("templates/*") {
    Ok(t) => t,
    Err(e) => {
        println!("Parsing error(s): {e}");
        ::std::process::exit(1);
    }
});

pub fn p_high_issues() -> Box<dyn Action> {
    let all_tlabels = vec![
        "T-bootstrap",
        "T-cargo",
        "T-clippy",
        "T-community",
        "T-compiler",
        "T-core",
        "T-crates-io",
        "T-dev-tools",
        "T-docs-rs",
        "T-edition",
        "T-fls",
        "T-infra",
        "T-infra-italians",
        "T-lang",
        "T-lang-docs",
        "T-leadership-council",
        "T-libs",
        "T-libs-api",
        "T-opsem",
        "T-release",
        "T-rust-analyzer",
        "T-rustdoc",
        "T-rustdoc-frontend",
        "T-rustdoc-internals",
        "T-rustdoc-json-backend",
        "T-rustfmt",
        "T-rustup",
        "T-spec",
        "T-style",
        "T-testing-devex",
        "T-triagebot",
        "T-types",
    ];

    let mut other_teams_tlabels = all_tlabels.clone();
    other_teams_tlabels.retain(|&x| x != "T-compiler");

    Box::new(Step {
        name: "p_high_issues",
        actions: vec![
            Query {
                repos: vec![("rust-lang", "rust")],
                queries: vec![QueryMap {
                    name: "p_high_tcompiler_unassigned",
                    kind: QueryKind::List,
                    query: Arc::new(IssueQuery {
                        filters: vec![("state", "open"), ("no", "assignee")],
                        include_labels: vec!["P-high", "T-compiler"],
                        exclude_labels: vec![],
                    }),
                }],
            },
            Query {
                repos: vec![("rust-lang", "rust")],
                queries: vec![QueryMap {
                    name: "p_high_tcompiler_assigned",
                    kind: QueryKind::List,
                    query: Arc::new(IssueQuery {
                        filters: vec![("state", "open"), ("has", "assignee")],
                        include_labels: vec!["P-high", "T-compiler"],
                        exclude_labels: other_teams_tlabels,
                    }),
                }],
            },
            Query {
                repos: vec![("rust-lang", "rust")],
                queries: vec![QueryMap {
                    name: "p_high_without_tlabel",
                    kind: QueryKind::List,
                    query: Arc::new(IssueQuery {
                        filters: vec![("state", "open")],
                        include_labels: vec!["P-high"],
                        exclude_labels: all_tlabels,
                    }),
                }],
            },
        ],
    })
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let octo_client = Octocrab::builder()
        .user_access_token(default_token_from_env())
        .build()
        .unwrap();
    let query = p_high_issues();
    print!("{}", query.call(&octo_client).await?);
    Ok(())
}
