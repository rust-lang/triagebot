use crate::actions::{Action, Query, QueryMap, Step};
use crate::github;
use std::collections::HashMap;

pub fn prioritization<'a>() -> Box<dyn Action> {
    let mut actions = Vec::new();

    let mut queries = Vec::new();

    // MCP/FCP queries
    queries.push(QueryMap {
        name: "mcp_new_not_seconded",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["major-change", "to-announce"],
            exclude_labels: vec![
                "proposed-final-comment-period",
                "finished-final-comment-period",
                "final-comment-period",
                "major-change-accepted",
                "t-libs",
                "t-libs-api",
            ],
            ordering: HashMap::new(),
        },
    });

    queries.push(QueryMap {
        name: "mcp_old_not_seconded",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["major-change"],
            exclude_labels: vec![
                "to-announce",
                "proposed-final-comment-period",
                "finished-final-comment-period",
                "final-comment-period",
                "t-libs",
                "t-libs-api",
            ],
            ordering: HashMap::new(),
        },
    });

    queries.push(QueryMap {
        name: "in_pre_fcp",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["proposed-final-comment-period"],
            exclude_labels: vec!["t-libs", "t-libs-api"],
            ordering: HashMap::new(),
        },
    });
    queries.push(QueryMap {
        name: "in_fcp",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["final-comment-period"],
            exclude_labels: vec!["t-libs", "t-libs-api"],
            ordering: HashMap::new(),
        },
    });

    queries.push(QueryMap {
        name: "mcp_accepted",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "all")],
            include_labels: vec!["major-change-accepted", "to-announce"],
            exclude_labels: vec!["t-libs", "t-libs-api"],
            ordering: HashMap::new(),
        },
    });

    queries.push(QueryMap {
        name: "fcp_finished",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "all")],
            include_labels: vec![
                "finished-final-comment-period",
                "disposition-merge",
                "to-announce",
            ],
            exclude_labels: vec!["t-libs", "t-libs-api"],
            ordering: HashMap::new(),
        },
    });

    actions.push(Query {
        repos: vec!["rust-lang/compiler-team"],
        queries,
    });

    let mut queries = Vec::new();

    queries.push(QueryMap {
        name: "in_pre_fcp",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["proposed-final-comment-period", "T-compiler"],
            exclude_labels: vec!["t-libs", "t-libs-api"],
            ordering: HashMap::new(),
        },
    });
    queries.push(QueryMap {
        name: "in_fcp",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["final-comment-period", "T-compiler"],
            exclude_labels: vec!["t-libs", "t-libs-api"],
            ordering: HashMap::new(),
        },
    });

    queries.push(QueryMap {
        name: "fcp_finished",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "all")],
            include_labels: vec![
                "finished-final-comment-period",
                "disposition-merge",
                "to-announce",
            ],
            exclude_labels: vec!["t-libs", "t-libs-api"],
            ordering: HashMap::new(),
        },
    });

    actions.push(Query {
        repos: vec!["rust-lang/rust"],
        queries,
    });

    let mut queries = Vec::new();

    queries.push(QueryMap {
        name: "in_pre_fcp",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["proposed-final-comment-period"],
            exclude_labels: vec!["t-libs", "t-libs-api"],
            ordering: HashMap::new(),
        },
    });
    queries.push(QueryMap {
        name: "in_fcp",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["final-comment-period"],
            exclude_labels: vec!["t-libs", "t-libs-api"],
            ordering: HashMap::new(),
        },
    });

    queries.push(QueryMap {
        name: "fcp_finished",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "all")],
            include_labels: vec![
                "finished-final-comment-period",
                "disposition-merge",
                "to-announce",
            ],
            exclude_labels: vec!["t-libs", "t-libs-api"],
            ordering: HashMap::new(),
        },
    });

    actions.push(Query {
        repos: vec!["rust-lang/rust-forge"],
        queries,
    });

    let mut queries = Vec::new();

    // beta nomination queries
    queries.push(QueryMap {
        name: "beta_nominated_t_compiler",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![],
            include_labels: vec!["beta-nominated", "T-compiler"],
            exclude_labels: vec!["beta-accepted"],
            ordering: HashMap::new(),
        },
    });

    queries.push(QueryMap {
        name: "beta_nominated_t_rustdoc",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![],
            include_labels: vec!["beta-nominated", "T-rustdoc"],
            exclude_labels: vec!["beta-accepted"],
            ordering: HashMap::new(),
        },
    });

    // stable nomination queries
    queries.push(QueryMap {
        name: "stable_nominated_t_compiler",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![],
            include_labels: vec!["stable-nominated", "T-compiler"],
            exclude_labels: vec!["stable-accepted"],
            ordering: HashMap::new(),
        },
    });

    queries.push(QueryMap {
        name: "stable_nominated_t_rustdoc",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![],
            include_labels: vec!["stable-nominated", "T-rustdoc"],
            exclude_labels: vec!["stable-accepted"],
            ordering: HashMap::new(),
        },
    });

    // prs waiting on team queries
    queries.push(QueryMap {
        name: "prs_waiting_on_team_t_compiler",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["S-waiting-on-team", "T-compiler"],
            exclude_labels: vec![],
            ordering: HashMap::new(),
        },
    });

    // issues of note queries
    queries.push(QueryMap {
        name: "issues_of_note_p_critical",
        query: github::Query {
            kind: github::QueryKind::Count,
            filters: vec![("state", "open")],
            include_labels: vec!["T-compiler", "P-critical"],
            exclude_labels: vec![],
            ordering: HashMap::new(),
        },
    });

    queries.push(QueryMap {
        name: "issues_of_note_unassigned_p_critical",
        query: github::Query {
            kind: github::QueryKind::Count,
            filters: vec![("state", "open"), ("no", "assignee")],
            include_labels: vec!["T-compiler", "P-critical"],
            exclude_labels: vec![],
            ordering: HashMap::new(),
        },
    });

    queries.push(QueryMap {
        name: "issues_of_note_p_high",
        query: github::Query {
            kind: github::QueryKind::Count,
            filters: vec![("state", "open")],
            include_labels: vec!["T-compiler", "P-high"],
            exclude_labels: vec![],
            ordering: HashMap::new(),
        },
    });

    queries.push(QueryMap {
        name: "issues_of_note_unassigned_p_high",
        query: github::Query {
            kind: github::QueryKind::Count,
            filters: vec![("state", "open"), ("no", "assignee")],
            include_labels: vec!["T-compiler", "P-high"],
            exclude_labels: vec![],
            ordering: HashMap::new(),
        },
    });

    queries.push(QueryMap {
        name: "issues_of_note_regression_from_stable_to_beta_p_critical",
        query: github::Query {
            kind: github::QueryKind::Count,
            filters: vec![("state", "open")],
            include_labels: vec!["regression-from-stable-to-beta", "P-critical"],
            exclude_labels: vec![],
            ordering: HashMap::new(),
        },
    });

    queries.push(QueryMap {
        name: "issues_of_note_regression_from_stable_to_beta_p_high",
        query: github::Query {
            kind: github::QueryKind::Count,
            filters: vec![("state", "open")],
            include_labels: vec!["regression-from-stable-to-beta", "P-high"],
            exclude_labels: vec![],
            ordering: HashMap::new(),
        },
    });

    queries.push(QueryMap {
        name: "issues_of_note_regression_from_stable_to_beta_p_medium",
        query: github::Query {
            kind: github::QueryKind::Count,
            filters: vec![("state", "open")],
            include_labels: vec!["regression-from-stable-to-beta", "P-medium"],
            exclude_labels: vec![],
            ordering: HashMap::new(),
        },
    });

    queries.push(QueryMap {
        name: "issues_of_note_regression_from_stable_to_beta_p_low",
        query: github::Query {
            kind: github::QueryKind::Count,
            filters: vec![("state", "open")],
            include_labels: vec!["regression-from-stable-to-beta", "P-low"],
            exclude_labels: vec![],
            ordering: HashMap::new(),
        },
    });

    queries.push(QueryMap {
        name: "issues_of_note_regression_from_stable_to_nightly_p_critical",
        query: github::Query {
            kind: github::QueryKind::Count,
            filters: vec![("state", "open")],
            include_labels: vec!["regression-from-stable-to-nightly", "P-critical"],
            exclude_labels: vec![],
            ordering: HashMap::new(),
        },
    });

    queries.push(QueryMap {
        name: "issues_of_note_regression_from_stable_to_nightly_p_high",
        query: github::Query {
            kind: github::QueryKind::Count,
            filters: vec![("state", "open")],
            include_labels: vec!["regression-from-stable-to-nightly", "P-high"],
            exclude_labels: vec![],
            ordering: HashMap::new(),
        },
    });

    queries.push(QueryMap {
        name: "issues_of_note_regression_from_stable_to_nightly_p_medium",
        query: github::Query {
            kind: github::QueryKind::Count,
            filters: vec![("state", "open")],
            include_labels: vec!["regression-from-stable-to-nightly", "P-medium"],
            exclude_labels: vec![],
            ordering: HashMap::new(),
        },
    });

    queries.push(QueryMap {
        name: "issues_of_note_regression_from_stable_to_nightly_p_low",
        query: github::Query {
            kind: github::QueryKind::Count,
            filters: vec![("state", "open")],
            include_labels: vec!["regression-from-stable-to-nightly", "P-low"],
            exclude_labels: vec![],
            ordering: HashMap::new(),
        },
    });

    queries.push(QueryMap {
        name: "issues_of_note_regression_from_stable_to_stable_p_critical",
        query: github::Query {
            kind: github::QueryKind::Count,
            filters: vec![("state", "open")],
            include_labels: vec!["regression-from-stable-to-stable", "P-critical"],
            exclude_labels: vec![],
            ordering: HashMap::new(),
        },
    });

    queries.push(QueryMap {
        name: "issues_of_note_regression_from_stable_to_stable_p_high",
        query: github::Query {
            kind: github::QueryKind::Count,
            filters: vec![("state", "open")],
            include_labels: vec!["regression-from-stable-to-stable", "P-high"],
            exclude_labels: vec![],
            ordering: HashMap::new(),
        },
    });

    queries.push(QueryMap {
        name: "issues_of_note_regression_from_stable_to_stable_p_medium",
        query: github::Query {
            kind: github::QueryKind::Count,
            filters: vec![("state", "open")],
            include_labels: vec!["regression-from-stable-to-stable", "P-medium"],
            exclude_labels: vec![],
            ordering: HashMap::new(),
        },
    });

    queries.push(QueryMap {
        name: "issues_of_note_regression_from_stable_to_stable_p_low",
        query: github::Query {
            kind: github::QueryKind::Count,
            filters: vec![("state", "open")],
            include_labels: vec!["regression-from-stable-to-stable", "P-low"],
            exclude_labels: vec![],
            ordering: HashMap::new(),
        },
    });

    queries.push(QueryMap {
        name: "p_critical_t_compiler",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["T-compiler", "P-critical"],
            exclude_labels: vec![],
            ordering: HashMap::new(),
        },
    });

    queries.push(QueryMap {
        name: "p_critical_t_rustdoc",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["T-rustdoc", "P-critical"],
            exclude_labels: vec![],
            ordering: HashMap::new(),
        },
    });

    queries.push(QueryMap {
        name: "beta_regressions_p_high",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["regression-from-stable-to-beta", "P-high"],
            exclude_labels: vec!["T-infra", "T-libs", "T-release", "T-rustdoc", "T-core"],
            ordering: HashMap::new(),
        },
    });

    queries.push(QueryMap {
        name: "nightly_regressions_unassigned_p_high",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open"), ("no", "assignee")],
            include_labels: vec!["regression-from-stable-to-nightly", "P-high"],
            exclude_labels: vec!["T-infra", "T-libs", "T-release", "T-rustdoc", "T-core"],
            ordering: HashMap::new(),
        },
    });

    queries.push(QueryMap {
        name: "nominated_t_compiler",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["I-nominated", "T-compiler"],
            exclude_labels: vec![],
            ordering: HashMap::new(),
        },
    });

    let mut ordering = HashMap::new();
    ordering.insert("sort", "updated");
    ordering.insert("per_page", "5");

    queries.push(QueryMap {
        name: "top_unreviewed_prs",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open"), ("is", "pull-request"), ("-is", "draft")],
            include_labels: vec!["S-waiting-on-review", "T-compiler"],
            exclude_labels: vec![],
            ordering,
        },
    });

    actions.push(Query {
        repos: vec!["rust-lang/rust"],
        queries,
    });

    // retrieve some RFCs for the T-compiler agenda

    let mut queries = Vec::new();

    // https://github.com/rust-lang/rfcs/pulls?q=is%3Aopen+label%3AI-nominated+label%3AT-compiler
    queries.push(QueryMap {
        name: "nominated_rfcs_t_compiler",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["T-compiler", "I-nominated"],
            exclude_labels: vec![],
            ordering: HashMap::new(),
        },
    });

    actions.push(Query {
        repos: vec!["rust-lang/rfcs"],
        queries,
    });

    Box::new(Step {
        name: "prioritization_agenda",
        actions,
    })
}

pub fn lang<'a>() -> Box<dyn Action> {
    Box::new(Step {
        name: "lang_agenda",
        actions: vec![
            Query {
                repos: vec!["rust-lang/lang-team"],
                queries: vec![
                    QueryMap {
                        name: "pending_project_proposals",
                        query: github::Query {
                            kind: github::QueryKind::List,
                            filters: vec![("state", "open"), ("is", "issue")],
                            include_labels: vec!["major-change"],
                            exclude_labels: vec!["charter-needed"],
                            ordering: HashMap::new(),
                        },
                    },
                    QueryMap {
                        name: "pending_lang_team_prs",
                        query: github::Query {
                            kind: github::QueryKind::List,
                            filters: vec![("state", "open"), ("is", "pull-request")],
                            include_labels: vec![],
                            exclude_labels: vec![],
                            ordering: HashMap::new(),
                        },
                    },
                    QueryMap {
                        name: "scheduled_meetings",
                        query: github::Query {
                            kind: github::QueryKind::List,
                            filters: vec![("state", "open"), ("is", "issue")],
                            include_labels: vec!["meeting-proposal", "meeting-scheduled"],
                            exclude_labels: vec![],
                            ordering: HashMap::new(),
                        },
                    },
                ],
            },
            Query {
                repos: vec!["rust-lang/rfcs"],
                queries: vec![QueryMap {
                    name: "rfcs_waiting_to_be_merged",
                    query: github::Query {
                        kind: github::QueryKind::List,
                        filters: vec![("state", "open"), ("is", "pr")],
                        include_labels: vec![
                            "disposition-merge",
                            "finished-final-comment-period",
                            "T-lang",
                        ],
                        exclude_labels: vec![],
                        ordering: HashMap::new(),
                    },
                }],
            },
            Query {
                repos: vec![
                    "rust-lang/rfcs",
                    "rust-lang/rust",
                    "rust-lang/reference",
                    "rust-lang/lang-team",
                ],
                queries: vec![
                    QueryMap {
                        name: "p_critical",
                        query: github::Query {
                            kind: github::QueryKind::List,
                            filters: vec![("state", "open")],
                            include_labels: vec!["T-lang", "P-critical"],
                            exclude_labels: vec![],
                            ordering: HashMap::new(),
                        },
                    },
                    QueryMap {
                        name: "nominated",
                        query: github::Query {
                            kind: github::QueryKind::List,
                            filters: vec![("state", "open")],
                            include_labels: vec!["T-lang", "I-nominated"],
                            exclude_labels: vec![],
                            ordering: HashMap::new(),
                        },
                    },
                    QueryMap {
                        name: "proposed_fcp",
                        query: github::Query {
                            kind: github::QueryKind::List,
                            filters: vec![("state", "open")],
                            include_labels: vec!["T-lang", "proposed-final-comment-period"],
                            exclude_labels: vec!["finished-final-comment-period"],
                            ordering: HashMap::new(),
                        },
                    },
                    QueryMap {
                        name: "in_fcp",
                        query: github::Query {
                            kind: github::QueryKind::List,
                            filters: vec![("state", "open")],
                            include_labels: vec!["T-lang", "final-comment-period"],
                            exclude_labels: vec!["finished-final-comment-period"],
                            ordering: HashMap::new(),
                        },
                    },
                    QueryMap {
                        name: "finished_fcp",
                        query: github::Query {
                            kind: github::QueryKind::List,
                            filters: vec![("state", "open")],
                            include_labels: vec!["T-lang", "finished-final-comment-period"],
                            exclude_labels: vec![],
                            ordering: HashMap::new(),
                        },
                    },
                ],
            },
        ],
    })
}

pub fn lang_planning<'a>() -> Box<dyn Action> {
    Box::new(Step {
        name: "lang_planning_agenda",
        actions: vec![
            Query {
                repos: vec!["rust-lang/lang-team"],
                queries: vec![
                    QueryMap {
                        name: "pending_project_proposals",
                        query: github::Query {
                            kind: github::QueryKind::List,
                            filters: vec![("state", "open"), ("is", "issue")],
                            include_labels: vec!["major-change"],
                            exclude_labels: vec!["charter-needed"],
                            ordering: HashMap::new(),
                        },
                    },
                    QueryMap {
                        name: "pending_lang_team_prs",
                        query: github::Query {
                            kind: github::QueryKind::List,
                            filters: vec![("state", "open"), ("is", "pr")],
                            include_labels: vec![],
                            exclude_labels: vec![],
                            ordering: HashMap::new(),
                        },
                    },
                    QueryMap {
                        name: "proposed_meetings",
                        query: github::Query {
                            kind: github::QueryKind::List,
                            filters: vec![("state", "open"), ("is", "issue")],
                            include_labels: vec!["meeting-proposal"],
                            exclude_labels: vec!["meeting-scheduled"],
                            ordering: HashMap::new(),
                        },
                    },
                ],
            },
            Query {
                repos: vec!["rust-lang/lang-team"],
                queries: vec![QueryMap {
                    name: "active_initiatives",
                    query: github::Query {
                        kind: github::QueryKind::List,
                        filters: vec![("state", "open"), ("is", "issue")],
                        include_labels: vec!["lang-initiative"],
                        exclude_labels: vec![],
                        ordering: HashMap::new(),
                    },
                }],
            },
        ],
    })
}
