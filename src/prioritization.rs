use crate::github;
use crate::meeting::{Meeting, Query, QueryMap, Step};

pub fn prepare_meeting<'a>() -> Meeting<Step<'a>> {
    Meeting {
        steps: vec![
            unpri_i_prioritize(),
            regressions(),
            nominations(),
            prs_waiting_on_team(),
            agenda(),
            final_review(),
        ],
    }
}

pub fn unpri_i_prioritize<'a>() -> Step<'a> {
    let mut queries = Vec::new();

    queries.push(QueryMap {
        name: "unpri_i_prioritize.all",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["I-prioritize"],
            exclude_labels: vec!["P-critical", "P-high", "P-medium", "P-low"],
        },
    });

    queries.push(QueryMap {
        name: "unpri_i_prioritize.t_compiler",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["I-prioritize", "T-compiler"],
            exclude_labels: vec!["P-critical", "P-high", "P-medium", "P-low"],
        },
    });

    queries.push(QueryMap {
        name: "unpri_i_prioritize.libs_impl",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["I-prioritize", "libs-impl"],
            exclude_labels: vec!["P-critical", "P-high", "P-medium", "P-low"],
        },
    });

    Step {
        name: "unpri_i_prioritize",
        actions: vec![Query {
            repo: "rust-lang/rust",
            queries,
        }],
    }
}

pub fn regressions<'a>() -> Step<'a> {
    let mut queries = Vec::new();

    queries.push(QueryMap {
        name: "regressions.stable_to_beta",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["regression-from-stable-to-beta"],
            exclude_labels: vec![
                "P-critical",
                "P-high",
                "P-medium",
                "P-low",
                "T-infra",
                "T-libs",
                "T-release",
                "T-rustdoc",
            ],
        },
    });

    queries.push(QueryMap {
        name: "regressions.stable_to_nightly",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["regression-from-stable-to-nightly"],
            exclude_labels: vec![
                "P-critical",
                "P-high",
                "P-medium",
                "P-low",
                "T-infra",
                "T-libs",
                "T-release",
                "T-rustdoc",
            ],
        },
    });

    queries.push(QueryMap {
        name: "regressions.stable_to_stable",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["regression-from-stable-to-stable"],
            exclude_labels: vec![
                "P-critical",
                "P-high",
                "P-medium",
                "P-low",
                "T-infra",
                "T-libs",
                "T-release",
                "T-rustdoc",
            ],
        },
    });

    Step {
        name: "regressions",
        actions: vec![Query {
            repo: "rust-lang/rust",
            queries,
        }],
    }
}

pub fn nominations<'a>() -> Step<'a> {
    let mut queries = Vec::new();

    queries.push(QueryMap {
        name: "nominations.stable_nominated",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![],
            include_labels: vec!["stable-nominated"],
            exclude_labels: vec!["stable-accepted"],
        },
    });

    queries.push(QueryMap {
        name: "nominations.beta_nominated",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![],
            include_labels: vec!["beta-nominated"],
            exclude_labels: vec!["beta-accepted"],
        },
    });

    queries.push(QueryMap {
        name: "nominations.i_nominated",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["I-nominated"],
            exclude_labels: vec![],
        },
    });

    queries.push(QueryMap {
        name: "nominations.i_nominated_t_compiler",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["I-nominated", "T-compiler"],
            exclude_labels: vec![],
        },
    });

    queries.push(QueryMap {
        name: "nominations.i_nominated_libs_impl",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["I-nominated", "libs-impl"],
            exclude_labels: vec![],
        },
    });

    Step {
        name: "nominations",
        actions: vec![Query {
            repo: "rust-lang/rust",
            queries,
        }],
    }
}

pub fn prs_waiting_on_team<'a>() -> Step<'a> {
    let mut queries = Vec::new();

    queries.push(QueryMap {
        name: "prs_waiting_on_team.all",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["S-waiting-on-team"],
            exclude_labels: vec![],
        },
    });

    queries.push(QueryMap {
        name: "prs_waiting_on_team.t_compiler",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["S-waiting-on-team", "T-compiler"],
            exclude_labels: vec![],
        },
    });

    queries.push(QueryMap {
        name: "prs_waiting_on_team.libs_impl",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["S-waiting-on-team", "libs-impl"],
            exclude_labels: vec![],
        },
    });

    Step {
        name: "prs_waiting_on_team",
        actions: vec![Query {
            repo: "rust-lang/rust",
            queries,
        }],
    }
}

pub fn agenda<'a>() -> Step<'a> {
    let mut queries = Vec::new();
    let mut actions = Vec::new();

    queries.push(QueryMap {
        name: "mcp.seconded",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["major-change", "final-comment-period"],
            exclude_labels: vec![],
        },
    });

    queries.push(QueryMap {
        name: "mcp.new_not_seconded",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["major-change", "to-announce"],
            exclude_labels: vec!["final-comment-period"],
        },
    });

    queries.push(QueryMap {
        name: "mcp.old_not_seconded",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["major-change"],
            exclude_labels: vec!["to-announce", "final-comment-period"],
        },
    });

    actions.push(Query {
        repo: "rust-lang/compiler-team",
        queries,
    });

    let mut queries = Vec::new();

    queries.push(QueryMap {
        name: "beta_nominated.t_compiler",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![],
            include_labels: vec!["beta-nominated", "T-compiler"],
            exclude_labels: vec!["beta-accepted"],
        },
    });

    queries.push(QueryMap {
        name: "beta_nominated.libs_impl",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![],
            include_labels: vec!["beta-nominated", "libs-impl"],
            exclude_labels: vec!["beta-accepted"],
        },
    });

    queries.push(QueryMap {
        name: "beta_nominated.t_rustdoc",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![],
            include_labels: vec!["beta-nominated", "T-rustdoc"],
            exclude_labels: vec!["beta-accepted"],
        },
    });

    queries.push(QueryMap {
        name: "stable_nominated.t_compiler",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![],
            include_labels: vec!["stable-nominated", "T-compiler"],
            exclude_labels: vec!["stable-accepted"],
        },
    });

    queries.push(QueryMap {
        name: "stable_nominated.t_rustdoc",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![],
            include_labels: vec!["stable-nominated", "T-rustdoc"],
            exclude_labels: vec!["stable-accepted"],
        },
    });

    queries.push(QueryMap {
        name: "stable_nominated.libs_impl",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![],
            include_labels: vec!["stable-nominated", "libs-impl"],
            exclude_labels: vec!["stable-accepted"],
        },
    });

    queries.push(QueryMap {
        name: "prs_waiting_on_team.t_compiler",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["S-waiting-on-team", "T-compiler"],
            exclude_labels: vec![],
        },
    });

    queries.push(QueryMap {
        name: "prs_waiting_on_team.libs_impl",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["S-waiting-on-team", "libs-impl"],
            exclude_labels: vec![],
        },
    });

    queries.push(QueryMap {
        name: "issues_of_note.p_critical",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["T-compiler", "P-critical"],
            exclude_labels: vec![],
        },
    });

    queries.push(QueryMap {
        name: "issues_of_note.unassigned_p_critical",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open"), ("no", "assignee")],
            include_labels: vec!["T-compiler", "P-critical"],
            exclude_labels: vec![],
        },
    });

    queries.push(QueryMap {
        name: "issues_of_note.p_high",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["T-compiler", "P-high"],
            exclude_labels: vec![],
        },
    });

    queries.push(QueryMap {
        name: "issues_of_note.unassigned_p_high",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open"), ("no", "assignee")],
            include_labels: vec!["T-compiler", "P-high"],
            exclude_labels: vec![],
        },
    });

    // - [N regression-from-stable-to-stable](https://github.com/rust-lang/rust/labels/regression-from-stable-to-stable)
    //   - [M of those are not prioritized](https://github.com/rust-lang/rust/issues?q=is%3Aopen+label%3Aregression-from-stable-to-stable+-label%3AP-critical+-label%3AP-high+-label%3AP-medium+-label%3AP-low).
    //
    // There are N (more|less) `P-critical` issues and M (more|less) `P-high` issues in comparison with last week.
    queries.push(QueryMap {
        name: "issues_of_note.regression_from_stable_to_beta",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["regression-from-stable-to-beta"],
            exclude_labels: vec![],
        },
    });

    queries.push(QueryMap {
        name: "issues_of_note.regression_from_stable_to_nightly",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["regression-from-stable-to-nightly"],
            exclude_labels: vec![],
        },
    });

    queries.push(QueryMap {
        name: "issues_of_note.regression_from_stable_to_stable",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["regression-from-stable-to-stable"],
            exclude_labels: vec![],
        },
    });

    queries.push(QueryMap {
        name: "p_critical.t_compiler",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["T-compiler", "P-critical"],
            exclude_labels: vec![],
        },
    });

    queries.push(QueryMap {
        name: "p_critical.libs_impl",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["libs-impl", "P-critical"],
            exclude_labels: vec![],
        },
    });

    queries.push(QueryMap {
        name: "p_critical.t_rustdoc",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["T-rustdoc", "P-critical"],
            exclude_labels: vec![],
        },
    });

    queries.push(QueryMap {
        name: "beta_regressions.unassigned_p_high",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open"), ("no", "assignee")],
            include_labels: vec!["regression-from-stable-to-beta", "P-high"],
            exclude_labels: vec!["T-infra", "T-release"],
        },
    });

    queries.push(QueryMap {
        name: "nightly_regressions.unassigned_p_high",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open"), ("no", "assignee")],
            include_labels: vec!["regression-from-stable-to-nightly", "P-high"],
            exclude_labels: vec!["T-infra", "T-release"],
        },
    });

    queries.push(QueryMap {
        name: "i_nominated.t_compiler",
        query: github::Query {
            kind: github::QueryKind::List,
            filters: vec![("state", "open")],
            include_labels: vec!["I-nominated", "T-compiler"],
            exclude_labels: vec![],
        },
    });

    queries.push(QueryMap {
        name: "i_nominated.libs_impl",
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

    Step {
        name: "agenda",
        actions,
    }
}

pub fn final_review<'a>() -> Step<'a> {
    Step {
        name: "final_review",
        actions: vec![],
    }
}
