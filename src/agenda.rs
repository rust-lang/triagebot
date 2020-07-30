use crate::actions::{Action, Query, QueryMap, Step};
use crate::github;

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
            exclude_labels: vec!["final-comment-period", "major-change-accepted"],
        },
    });

    queries.push(QueryMap {
        name: "mcp_old_not_seconded",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["major-change"],
            exclude_labels: vec!["to-announce", "final-comment-period"],
        },
    });

    queries.push(QueryMap {
        name: "in_pre_fcp_compiler_team",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["proposed-final-comment-period"],
            exclude_labels: vec![],
        },
    });
    queries.push(QueryMap {
        name: "in_fcp_compiler_team",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["final-comment-period"],
            exclude_labels: vec![],
        },
    });

    queries.push(QueryMap {
        name: "mcp_accepted",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "all")],
            include_labels: vec!["major-change-accepted", "to-announce"],
            exclude_labels: vec![],
        },
    });

    actions.push(Query {
        repo: "rust-lang/compiler-team",
        queries,
    });

    let mut queries = Vec::new();

    queries.push(QueryMap {
        name: "in_pre_fcp_rust",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["proposed-final-comment-period", "T-compiler"],
            exclude_labels: vec![],
        },
    });
    queries.push(QueryMap {
        name: "in_fcp_rust",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["final-comment-period", "T-compiler"],
            exclude_labels: vec![],
        },
    });

    queries.push(QueryMap {
        name: "fcp_finished",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["finished-final-comment-period", "disposition-merge"],
            exclude_labels: vec![],
        },
    });

    actions.push(Query {
        repo: "rust-lang/rust",
        queries,
    });

    let mut queries = Vec::new();

    queries.push(QueryMap {
        name: "in_pre_fcp_forge",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["proposed-final-comment-period"],
            exclude_labels: vec![],
        },
    });
    queries.push(QueryMap {
        name: "in_fcp_forge",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["final-comment-period"],
            exclude_labels: vec![],
        },
    });

    actions.push(Query {
        repo: "rust-lang/rust-forge",
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
        },
    });

    queries.push(QueryMap {
        name: "beta_nominated_libs_impl",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![],
            include_labels: vec!["beta-nominated", "libs-impl"],
            exclude_labels: vec!["beta-accepted"],
        },
    });

    queries.push(QueryMap {
        name: "beta_nominated_t_rustdoc",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![],
            include_labels: vec!["beta-nominated", "T-rustdoc"],
            exclude_labels: vec!["beta-accepted"],
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
        },
    });

    queries.push(QueryMap {
        name: "stable_nominated_libs_impl",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![],
            include_labels: vec!["stable-nominated", "libs-impl"],
            exclude_labels: vec!["stable-accepted"],
        },
    });

    queries.push(QueryMap {
        name: "stable_nominated_t_rustdoc",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![],
            include_labels: vec!["stable-nominated", "T-rustdoc"],
            exclude_labels: vec!["stable-accepted"],
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
        },
    });

    queries.push(QueryMap {
        name: "prs_waiting_on_team_libs_impl",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["S-waiting-on-team", "libs-impl"],
            exclude_labels: vec![],
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
        },
    });

    queries.push(QueryMap {
        name: "issues_of_note_unassigned_p_critical",
        query: github::Query {
            kind: github::QueryKind::Count,
            filters: vec![("state", "open"), ("no", "assignee")],
            include_labels: vec!["T-compiler", "P-critical"],
            exclude_labels: vec![],
        },
    });

    queries.push(QueryMap {
        name: "issues_of_note_p_high",
        query: github::Query {
            kind: github::QueryKind::Count,
            filters: vec![("state", "open")],
            include_labels: vec!["T-compiler", "P-high"],
            exclude_labels: vec![],
        },
    });

    queries.push(QueryMap {
        name: "issues_of_note_unassigned_p_high",
        query: github::Query {
            kind: github::QueryKind::Count,
            filters: vec![("state", "open"), ("no", "assignee")],
            include_labels: vec!["T-compiler", "P-high"],
            exclude_labels: vec![],
        },
    });

    queries.push(QueryMap {
        name: "issues_of_note_regression_from_stable_to_beta_p_critical",
        query: github::Query {
            kind: github::QueryKind::Count,
            filters: vec![("state", "open")],
            include_labels: vec!["regression-from-stable-to-beta", "P-critical"],
            exclude_labels: vec![],
        },
    });

    queries.push(QueryMap {
        name: "issues_of_note_regression_from_stable_to_beta_p_high",
        query: github::Query {
            kind: github::QueryKind::Count,
            filters: vec![("state", "open")],
            include_labels: vec!["regression-from-stable-to-beta", "P-high"],
            exclude_labels: vec![],
        },
    });

    queries.push(QueryMap {
        name: "issues_of_note_regression_from_stable_to_beta_p_medium",
        query: github::Query {
            kind: github::QueryKind::Count,
            filters: vec![("state", "open")],
            include_labels: vec!["regression-from-stable-to-beta", "P-medium"],
            exclude_labels: vec![],
        },
    });

    queries.push(QueryMap {
        name: "issues_of_note_regression_from_stable_to_beta_p_low",
        query: github::Query {
            kind: github::QueryKind::Count,
            filters: vec![("state", "open")],
            include_labels: vec!["regression-from-stable-to-beta", "P-low"],
            exclude_labels: vec![],
        },
    });

    queries.push(QueryMap {
        name: "issues_of_note_regression_from_stable_to_nightly_p_critical",
        query: github::Query {
            kind: github::QueryKind::Count,
            filters: vec![("state", "open")],
            include_labels: vec!["regression-from-stable-to-nightly", "P-critical"],
            exclude_labels: vec![],
        },
    });

    queries.push(QueryMap {
        name: "issues_of_note_regression_from_stable_to_nightly_p_high",
        query: github::Query {
            kind: github::QueryKind::Count,
            filters: vec![("state", "open")],
            include_labels: vec!["regression-from-stable-to-nightly", "P-high"],
            exclude_labels: vec![],
        },
    });

    queries.push(QueryMap {
        name: "issues_of_note_regression_from_stable_to_nightly_p_medium",
        query: github::Query {
            kind: github::QueryKind::Count,
            filters: vec![("state", "open")],
            include_labels: vec!["regression-from-stable-to-nightly", "P-medium"],
            exclude_labels: vec![],
        },
    });

    queries.push(QueryMap {
        name: "issues_of_note_regression_from_stable_to_nightly_p_low",
        query: github::Query {
            kind: github::QueryKind::Count,
            filters: vec![("state", "open")],
            include_labels: vec!["regression-from-stable-to-nightly", "P-low"],
            exclude_labels: vec![],
        },
    });

    queries.push(QueryMap {
        name: "issues_of_note_regression_from_stable_to_stable_p_critical",
        query: github::Query {
            kind: github::QueryKind::Count,
            filters: vec![("state", "open")],
            include_labels: vec!["regression-from-stable-to-stable", "P-critical"],
            exclude_labels: vec![],
        },
    });

    queries.push(QueryMap {
        name: "issues_of_note_regression_from_stable_to_stable_p_high",
        query: github::Query {
            kind: github::QueryKind::Count,
            filters: vec![("state", "open")],
            include_labels: vec!["regression-from-stable-to-stable", "P-high"],
            exclude_labels: vec![],
        },
    });

    queries.push(QueryMap {
        name: "issues_of_note_regression_from_stable_to_stable_p_medium",
        query: github::Query {
            kind: github::QueryKind::Count,
            filters: vec![("state", "open")],
            include_labels: vec!["regression-from-stable-to-stable", "P-medium"],
            exclude_labels: vec![],
        },
    });

    queries.push(QueryMap {
        name: "issues_of_note_regression_from_stable_to_stable_p_low",
        query: github::Query {
            kind: github::QueryKind::Count,
            filters: vec![("state", "open")],
            include_labels: vec!["regression-from-stable-to-stable", "P-low"],
            exclude_labels: vec![],
        },
    });

    queries.push(QueryMap {
        name: "p_critical_t_compiler",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["T-compiler", "P-critical"],
            exclude_labels: vec![],
        },
    });

    queries.push(QueryMap {
        name: "p_critical_libs_impl",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["libs-impl", "P-critical"],
            exclude_labels: vec![],
        },
    });

    queries.push(QueryMap {
        name: "p_critical_t_rustdoc",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["T-rustdoc", "P-critical"],
            exclude_labels: vec![],
        },
    });

    queries.push(QueryMap {
        name: "beta_regressions_unassigned_p_high",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open"), ("no", "assignee")],
            include_labels: vec!["regression-from-stable-to-beta", "P-high"],
            exclude_labels: vec!["T-infra", "T-libs", "T-release", "T-rustdoc"],
        },
    });

    queries.push(QueryMap {
        name: "nightly_regressions_unassigned_p_high",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open"), ("no", "assignee")],
            include_labels: vec!["regression-from-stable-to-nightly", "P-high"],
            exclude_labels: vec!["T-infra", "T-libs", "T-release", "T-rustdoc"],
        },
    });

    queries.push(QueryMap {
        name: "i_nominated_t_compiler",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["I-nominated", "T-compiler"],
            exclude_labels: vec![],
        },
    });

    queries.push(QueryMap {
        name: "i_nominated_libs_impl",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["I-nominated", "libs-impl"],
            exclude_labels: vec![],
        },
    });

    actions.push(Query {
        repo: "rust-lang/rust",
        queries,
    });

    Box::new(Step {
        name: "prioritization_agenda",
        actions,
    })
}
