use std::sync::Arc;

use crate::actions::{Action, Query, QueryKind, QueryMap, Step};
use crate::errors::AppError;
use crate::github::issue_query::LeastRecentlyReviewedPullRequests;
use crate::github::issue_query::Query as IssueQuery;

pub async fn types_planning_http() -> axum::response::Result<String, AppError> {
    Ok(types_planning().call().await?)
}

pub fn prioritization() -> Box<dyn Action> {
    Box::new(Step {
        name: "prioritization_agenda",
        actions: vec![
            Query {
                repos: vec![("rust-lang", "compiler-team")],
                queries: vec![
                    // MCP/FCP queries
                    QueryMap {
                        name: "mcp_new_not_seconded",
                        kind: QueryKind::List,
                        query: Arc::new(IssueQuery {
                            filters: vec![("state", "open")],
                            include_labels: vec!["major-change", "to-announce"],
                            exclude_labels: vec![
                                "proposed-final-comment-period",
                                "finished-final-comment-period",
                                "final-comment-period",
                                "major-change-accepted",
                                "t-libs",
                                "t-libs-api",
                                "t-rustdoc",
                            ],
                        }),
                    },
                    QueryMap {
                        name: "mcp_old_not_seconded",
                        kind: QueryKind::List,
                        query: Arc::new(IssueQuery {
                            filters: vec![("state", "open")],
                            include_labels: vec!["major-change"],
                            exclude_labels: vec![
                                "to-announce",
                                "proposed-final-comment-period",
                                "finished-final-comment-period",
                                "final-comment-period",
                                "t-libs",
                                "t-libs-api",
                                "t-rustdoc",
                            ],
                        }),
                    },
                    QueryMap {
                        name: "in_pre_fcp",
                        kind: QueryKind::List,
                        query: Arc::new(IssueQuery {
                            filters: vec![("state", "open")],
                            include_labels: vec!["proposed-final-comment-period"],
                            exclude_labels: vec!["t-libs", "t-libs-api", "t-rustdoc"],
                        }),
                    },
                    QueryMap {
                        name: "in_fcp",
                        kind: QueryKind::List,
                        query: Arc::new(IssueQuery {
                            filters: vec![("state", "open")],
                            include_labels: vec!["final-comment-period"],
                            exclude_labels: vec!["t-libs", "t-libs-api", "t-rustdoc"],
                        }),
                    },
                    QueryMap {
                        name: "mcp_accepted",
                        kind: QueryKind::List,
                        query: Arc::new(IssueQuery {
                            filters: vec![("state", "all")],
                            include_labels: vec!["major-change-accepted", "to-announce"],
                            exclude_labels: vec!["t-libs", "t-libs-api", "t-rustdoc"],
                        }),
                    },
                    QueryMap {
                        name: "fcp_finished",
                        kind: QueryKind::List,
                        query: Arc::new(IssueQuery {
                            filters: vec![("state", "all")],
                            include_labels: vec![
                                "finished-final-comment-period",
                                "disposition-merge",
                                "to-announce",
                            ],
                            exclude_labels: vec!["t-libs", "t-libs-api", "t-rustdoc"],
                        }),
                    },
                ],
            },
            Query {
                repos: vec![("rust-lang", "rust")],
                queries: vec![
                    QueryMap {
                        name: "in_pre_fcp",
                        kind: QueryKind::List,
                        query: Arc::new(IssueQuery {
                            filters: vec![("state", "open")],
                            include_labels: vec!["proposed-final-comment-period", "T-compiler"],
                            exclude_labels: vec!["t-libs", "t-libs-api", "t-rustdoc"],
                        }),
                    },
                    QueryMap {
                        name: "in_fcp",
                        kind: QueryKind::List,
                        query: Arc::new(IssueQuery {
                            filters: vec![("state", "open")],
                            include_labels: vec!["final-comment-period", "T-compiler"],
                            exclude_labels: vec!["t-libs", "t-libs-api", "t-rustdoc"],
                        }),
                    },
                    QueryMap {
                        name: "fcp_finished_tcompiler",
                        kind: QueryKind::List,
                        query: Arc::new(IssueQuery {
                            filters: vec![("state", "all")],
                            include_labels: vec![
                                "finished-final-comment-period",
                                "disposition-merge",
                                "to-announce",
                            ],
                            exclude_labels: vec![
                                "t-libs",
                                "t-libs-api",
                                "t-rustdoc",
                                "t-lang",
                                "t-style",
                            ],
                        }),
                    },
                    QueryMap {
                        name: "fcp_finished_not_tcompiler",
                        kind: QueryKind::List,
                        query: Arc::new(IssueQuery {
                            filters: vec![("state", "all")],
                            include_labels: vec![
                                "finished-final-comment-period",
                                "disposition-merge",
                                "to-announce",
                            ],
                            exclude_labels: vec!["t-libs", "t-libs-api", "t-rustdoc", "t-compiler"],
                        }),
                    },
                ],
            },
            Query {
                repos: vec![("rust-lang", "rust-forge")],
                queries: vec![
                    QueryMap {
                        name: "in_pre_fcp",
                        kind: QueryKind::List,
                        query: Arc::new(IssueQuery {
                            filters: vec![("state", "open")],
                            include_labels: vec!["proposed-final-comment-period"],
                            exclude_labels: vec!["t-libs", "t-libs-api", "t-rustdoc"],
                        }),
                    },
                    QueryMap {
                        name: "in_fcp",
                        kind: QueryKind::List,
                        query: Arc::new(IssueQuery {
                            filters: vec![("state", "open")],
                            include_labels: vec!["final-comment-period"],
                            exclude_labels: vec!["t-libs", "t-libs-api", "t-rustdoc"],
                        }),
                    },
                    QueryMap {
                        name: "fcp_finished",
                        kind: QueryKind::List,
                        query: Arc::new(IssueQuery {
                            filters: vec![("state", "all")],
                            include_labels: vec![
                                "finished-final-comment-period",
                                "disposition-merge",
                                "to-announce",
                            ],
                            exclude_labels: vec!["t-libs", "t-libs-api", "t-rustdoc"],
                        }),
                    },
                ],
            },
            Query {
                repos: vec![("rust-lang", "rust")],
                queries: vec![
                    // beta nomination queries
                    QueryMap {
                        name: "beta_nominated_t_compiler",
                        kind: QueryKind::List,
                        query: Arc::new(IssueQuery {
                            filters: vec![],
                            include_labels: vec!["beta-nominated", "T-compiler"],
                            exclude_labels: vec!["beta-accepted"],
                        }),
                    },
                    // stable nomination queries
                    QueryMap {
                        name: "stable_nominated_t_compiler",
                        kind: QueryKind::List,
                        query: Arc::new(IssueQuery {
                            filters: vec![],
                            include_labels: vec!["stable-nominated", "T-compiler"],
                            exclude_labels: vec!["stable-accepted"],
                        }),
                    },
                    // beta nomination t-types
                    QueryMap {
                        name: "beta_nominated_t_types",
                        kind: QueryKind::List,
                        query: Arc::new(IssueQuery {
                            filters: vec![],
                            include_labels: vec!["beta-nominated", "T-types"],
                            exclude_labels: vec!["beta-accepted"],
                        }),
                    },
                    // stable nomination queries
                    QueryMap {
                        name: "stable_nominated_t_types",
                        kind: QueryKind::List,
                        query: Arc::new(IssueQuery {
                            filters: vec![],
                            include_labels: vec!["stable-nominated", "T-Types"],
                            exclude_labels: vec!["stable-accepted"],
                        }),
                    },
                    // prs waiting on team queries
                    QueryMap {
                        name: "prs_waiting_on_team_t_compiler",
                        kind: QueryKind::List,
                        query: Arc::new(IssueQuery {
                            filters: vec![("state", "open")],
                            include_labels: vec!["S-waiting-on-t-compiler"],
                            exclude_labels: vec![],
                        }),
                    },
                    // issues of note queries
                    QueryMap {
                        name: "issues_of_note_p_critical",
                        kind: QueryKind::Count,
                        query: Arc::new(IssueQuery {
                            filters: vec![("state", "open")],
                            include_labels: vec!["T-compiler", "P-critical"],
                            exclude_labels: vec![],
                        }),
                    },
                    QueryMap {
                        name: "issues_of_note_unassigned_p_critical",
                        kind: QueryKind::Count,
                        query: Arc::new(IssueQuery {
                            filters: vec![("state", "open"), ("no", "assignee")],
                            include_labels: vec!["T-compiler", "P-critical"],
                            exclude_labels: vec![],
                        }),
                    },
                    QueryMap {
                        name: "issues_of_note_p_high",
                        kind: QueryKind::Count,
                        query: Arc::new(IssueQuery {
                            filters: vec![("state", "open")],
                            include_labels: vec!["T-compiler", "P-high"],
                            exclude_labels: vec![],
                        }),
                    },
                    QueryMap {
                        name: "issues_of_note_unassigned_p_high",
                        kind: QueryKind::Count,
                        query: Arc::new(IssueQuery {
                            filters: vec![("state", "open"), ("no", "assignee")],
                            include_labels: vec!["T-compiler", "P-high"],
                            exclude_labels: vec![],
                        }),
                    },
                    QueryMap {
                        name: "issues_of_note_regression_from_stable_to_beta_p_critical",
                        kind: QueryKind::Count,
                        query: Arc::new(IssueQuery {
                            filters: vec![("state", "open")],
                            include_labels: vec!["regression-from-stable-to-beta", "P-critical"],
                            exclude_labels: vec![],
                        }),
                    },
                    QueryMap {
                        name: "issues_of_note_regression_from_stable_to_beta_p_high",
                        kind: QueryKind::Count,
                        query: Arc::new(IssueQuery {
                            filters: vec![("state", "open")],
                            include_labels: vec!["regression-from-stable-to-beta", "P-high"],
                            exclude_labels: vec![],
                        }),
                    },
                    QueryMap {
                        name: "issues_of_note_regression_from_stable_to_beta_p_medium",
                        kind: QueryKind::Count,
                        query: Arc::new(IssueQuery {
                            filters: vec![("state", "open")],
                            include_labels: vec!["regression-from-stable-to-beta", "P-medium"],
                            exclude_labels: vec![],
                        }),
                    },
                    QueryMap {
                        name: "issues_of_note_regression_from_stable_to_beta_p_low",
                        kind: QueryKind::Count,
                        query: Arc::new(IssueQuery {
                            filters: vec![("state", "open")],
                            include_labels: vec!["regression-from-stable-to-beta", "P-low"],
                            exclude_labels: vec![],
                        }),
                    },
                    QueryMap {
                        name: "issues_of_note_regression_from_stable_to_nightly_p_critical",
                        kind: QueryKind::Count,
                        query: Arc::new(IssueQuery {
                            filters: vec![("state", "open")],
                            include_labels: vec!["regression-from-stable-to-nightly", "P-critical"],
                            exclude_labels: vec![],
                        }),
                    },
                    QueryMap {
                        name: "issues_of_note_regression_from_stable_to_nightly_p_high",
                        kind: QueryKind::Count,
                        query: Arc::new(IssueQuery {
                            filters: vec![("state", "open")],
                            include_labels: vec!["regression-from-stable-to-nightly", "P-high"],
                            exclude_labels: vec![],
                        }),
                    },
                    QueryMap {
                        name: "issues_of_note_regression_from_stable_to_nightly_p_medium",
                        kind: QueryKind::Count,
                        query: Arc::new(IssueQuery {
                            filters: vec![("state", "open")],
                            include_labels: vec!["regression-from-stable-to-nightly", "P-medium"],
                            exclude_labels: vec![],
                        }),
                    },
                    QueryMap {
                        name: "issues_of_note_regression_from_stable_to_nightly_p_low",
                        kind: QueryKind::Count,
                        query: Arc::new(IssueQuery {
                            filters: vec![("state", "open")],
                            include_labels: vec!["regression-from-stable-to-nightly", "P-low"],
                            exclude_labels: vec![],
                        }),
                    },
                    QueryMap {
                        name: "issues_of_note_regression_from_stable_to_stable_p_critical",
                        kind: QueryKind::Count,
                        query: Arc::new(IssueQuery {
                            filters: vec![("state", "open")],
                            include_labels: vec!["regression-from-stable-to-stable", "P-critical"],
                            exclude_labels: vec![],
                        }),
                    },
                    QueryMap {
                        name: "issues_of_note_regression_from_stable_to_stable_p_high",
                        kind: QueryKind::Count,
                        query: Arc::new(IssueQuery {
                            filters: vec![("state", "open")],
                            include_labels: vec!["regression-from-stable-to-stable", "P-high"],
                            exclude_labels: vec![],
                        }),
                    },
                    QueryMap {
                        name: "issues_of_note_regression_from_stable_to_stable_p_medium",
                        kind: QueryKind::Count,
                        query: Arc::new(IssueQuery {
                            filters: vec![("state", "open")],
                            include_labels: vec!["regression-from-stable-to-stable", "P-medium"],
                            exclude_labels: vec![],
                        }),
                    },
                    QueryMap {
                        name: "issues_of_note_regression_from_stable_to_stable_p_low",
                        kind: QueryKind::Count,
                        query: Arc::new(IssueQuery {
                            filters: vec![("state", "open")],
                            include_labels: vec!["regression-from-stable-to-stable", "P-low"],
                            exclude_labels: vec![],
                        }),
                    },
                    QueryMap {
                        name: "p_critical_t_compiler",
                        kind: QueryKind::List,
                        query: Arc::new(IssueQuery {
                            filters: vec![("state", "open")],
                            include_labels: vec!["T-compiler", "P-critical"],
                            exclude_labels: vec![],
                        }),
                    },
                    QueryMap {
                        name: "p_critical_t_types",
                        kind: QueryKind::List,
                        query: Arc::new(IssueQuery {
                            filters: vec![("state", "open")],
                            include_labels: vec!["T-types", "P-critical"],
                            exclude_labels: vec![],
                        }),
                    },
                    QueryMap {
                        name: "beta_regressions_p_high",
                        kind: QueryKind::List,
                        query: Arc::new(IssueQuery {
                            filters: vec![("state", "open")],
                            include_labels: vec!["regression-from-stable-to-beta", "P-high"],
                            exclude_labels: vec![
                                "T-infra",
                                "T-libs",
                                "T-libs-api",
                                "T-release",
                                "T-rustdoc",
                            ],
                        }),
                    },
                    QueryMap {
                        name: "nightly_regressions_unassigned_p_high",
                        kind: QueryKind::List,
                        query: Arc::new(IssueQuery {
                            filters: vec![("state", "open"), ("no", "assignee")],
                            include_labels: vec!["regression-from-stable-to-nightly", "P-high"],
                            exclude_labels: vec![
                                "T-infra",
                                "T-libs",
                                "T-libs-api",
                                "T-release",
                                "T-rustdoc",
                            ],
                        }),
                    },
                    QueryMap {
                        name: "nominated_t_compiler",
                        kind: QueryKind::List,
                        query: Arc::new(IssueQuery {
                            filters: vec![("state", "open")],
                            include_labels: vec!["I-compiler-nominated"],
                            exclude_labels: vec![],
                        }),
                    },
                    QueryMap {
                        name: "top_unreviewed_prs",
                        kind: QueryKind::List,
                        query: Arc::new(LeastRecentlyReviewedPullRequests),
                    },
                ],
            },
            Query {
                repos: vec![("rust-lang", "rfcs")],
                queries: vec![
                    // retrieve some RFCs for the T-compiler agenda
                    // https://github.com/rust-lang/rfcs/pulls?q=is%3Aopen+label%3AI-compiler-nominated
                    QueryMap {
                        name: "nominated_rfcs_t_compiler",
                        kind: QueryKind::List,
                        query: Arc::new(IssueQuery {
                            filters: vec![("state", "open")],
                            include_labels: vec!["I-compiler-nominated"],
                            exclude_labels: vec![],
                        }),
                    },
                ],
            },
        ],
    })
}

pub fn types_planning() -> Box<dyn Action + Send + Sync> {
    Box::new(Step {
        name: "types_planning_agenda",
        actions: vec![
            Query {
                repos: vec![("rust-lang", "types-team")],
                queries: vec![
                    QueryMap {
                        name: "roadmap_tracking_issues",
                        kind: QueryKind::List,
                        query: Arc::new(IssueQuery {
                            filters: vec![("state", "open"), ("is", "issue")],
                            include_labels: vec!["roadmap-tracking-issue"],
                            exclude_labels: vec![],
                        }),
                    },
                    QueryMap {
                        name: "deep_dive_proposals",
                        kind: QueryKind::List,
                        query: Arc::new(IssueQuery {
                            filters: vec![("state", "open"), ("is", "issue")],
                            include_labels: vec!["deep-dive-proposal"],
                            exclude_labels: vec![],
                        }),
                    },
                    QueryMap {
                        name: "major_changes",
                        kind: QueryKind::List,
                        query: Arc::new(IssueQuery {
                            filters: vec![("state", "open"), ("is", "issue")],
                            include_labels: vec!["major-change"],
                            exclude_labels: vec![],
                        }),
                    },
                ],
            },
            Query {
                repos: vec![("rust-lang", "rust")],
                queries: vec![
                    QueryMap {
                        name: "nominated_issues",
                        kind: QueryKind::List,
                        query: Arc::new(IssueQuery {
                            filters: vec![("state", "open"), ("is", "issue")],
                            include_labels: vec!["I-types-nominated"],
                            exclude_labels: vec![],
                        }),
                    },
                    QueryMap {
                        name: "types_fcps",
                        kind: QueryKind::List,
                        query: Arc::new(IssueQuery {
                            filters: vec![("state", "open")],
                            include_labels: vec!["T-types", "proposed-final-comment-period"],
                            exclude_labels: vec![],
                        }),
                    },
                ],
            },
        ],
    })
}

// Things to add (maybe):
// - Compiler RFCs
// - P-high issues
pub fn compiler_backlog_bonanza() -> Box<dyn Action> {
    Box::new(Step {
        name: "compiler_backlog_bonanza",
        actions: vec![Query {
            repos: vec![("rust-lang", "rust")],
            queries: vec![QueryMap {
                name: "tracking_issues",
                kind: QueryKind::List,
                query: Arc::new(IssueQuery {
                    filters: vec![("state", "open")],
                    include_labels: vec!["C-tracking-issue"],
                    exclude_labels: vec!["T-libs-api", "T-libs", "T-lang", "T-rustdoc"],
                }),
            }],
        }],
    })
}

// Lists available agenda pages
pub static INDEX: &str = r#"
<html>
<body>
<ul>
    <li><a href="/agenda/types/planning">T-types planning agenda</a></li>
</ul>
</body>
</html>
"#;
