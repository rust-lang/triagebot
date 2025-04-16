//! Tests for `candidate_reviewers_from_names`

use super::super::*;
use crate::tests::github::{issue, user};

#[must_use]
struct TestCtx {
    teams: Teams,
    config: AssignConfig,
    issue: Issue,
}

impl TestCtx {
    fn new(config: toml::Table, issue: Issue) -> Self {
        Self {
            teams: Teams {
                teams: Default::default(),
            },
            config: config.try_into().unwrap(),
            issue,
        }
    }

    fn teams(mut self, table: &toml::Table) -> Self {
        let teams: serde_json::Value = table.clone().try_into().unwrap();
        let mut teams_config = serde_json::json!({});
        for (team_name, members) in teams.as_object().unwrap() {
            let members: Vec<_> = members.as_array().unwrap().iter().map(|member| {
                serde_json::json!({"name": member, "github": member, "github_id": 100, "is_lead": false})
            }).collect();
            teams_config[team_name] = serde_json::json!({
                "name": team_name,
                "kind": "team",
                "members": serde_json::Value::Array(members),
                "alumni": [],
                "discord": [],
                "roles": [],
            });
        }
        self.teams = serde_json::value::from_value(teams_config).unwrap();
        self
    }

    fn run(self, names: &[&str], expected: Result<&[&str], FindReviewerError>) {
        let names: Vec<_> = names.iter().map(|n| n.to_string()).collect();
        match (
            candidate_reviewers_from_names(&self.teams, &self.config, &self.issue, &names),
            expected,
        ) {
            (Ok(candidates), Ok(expected)) => {
                let mut candidates: Vec<_> = candidates.into_iter().collect();
                candidates.sort();
                let expected: Vec<_> = expected.iter().map(|x| *x).collect();
                assert_eq!(candidates, expected);
            }
            (Err(actual), Err(expected)) => {
                assert_eq!(actual, expected)
            }
            (Ok(candidates), Err(_)) => panic!("expected Err, got Ok: {candidates:?}"),
            (Err(e), Ok(_)) => panic!("expected Ok, got Err: {e}"),
        }
    }
}

/// Basic test function for testing `candidate_reviewers_from_names`.
fn test_candidates(config: toml::Table, issue: Issue) -> TestCtx {
    TestCtx::new(config, issue)
}

#[test]
fn circular_groups() {
    // A cycle in the groups map.
    let config = toml::toml!(
        [adhoc_groups]
        compiler = ["other"]
        other = ["compiler"]
    );
    test_candidates(config, issue().call()).run(
        &["compiler"],
        Err(FindReviewerError::NoReviewer {
            initial: vec!["compiler".to_string()],
        }),
    );
}

#[test]
fn nested_groups() {
    // Test choosing a reviewer from group with nested groups.
    let config = toml::toml!(
        [adhoc_groups]
        a = ["@pnkfelix"]
        b = ["@nrc"]
        c = ["a", "b"]
    );
    test_candidates(config, issue().call()).run(&["c"], Ok(&["nrc", "pnkfelix"]));
}

#[test]
fn candidate_filtered_author_only_candidate() {
    // When the author is the only candidate.
    let config = toml::toml!(
        [adhoc_groups]
        compiler = ["nikomatsakis"]
    );
    test_candidates(config, issue().author(user("nikomatsakis", 1)).call()).run(
        &["compiler"],
        Err(FindReviewerError::NoReviewer {
            initial: vec!["compiler".to_string()],
        }),
    );
}

#[test]
fn candidate_filtered_author() {
    // Filter out the author from the candidates.
    let config = toml::toml!(
        [adhoc_groups]
        compiler = ["user1", "user2", "user3", "group2"]
        group2 = ["user2", "user4"]
    );
    test_candidates(config, issue().author(user("user2", 1)).call())
        .run(&["compiler"], Ok(&["user1", "user3", "user4"]));
}

#[test]
fn candidate_filtered_assignee() {
    // Filter out an existing assignee from the candidates.
    let config = toml::toml!(
        [adhoc_groups]
        compiler = ["user1", "user2", "user3", "user4"]
    );
    let issue = issue()
        .author(user("user2", 2))
        .assignees(vec![user("user1", 1), user("user3", 3)])
        .call();
    test_candidates(config, issue).run(&["compiler"], Ok(&["user4"]));
}

#[test]
fn groups_teams_users() {
    // Assortment of groups, teams, and users all selected at once.
    let teams = toml::toml!(
        team1 = ["t-user1"]
        team2 = ["t-user2"]
    );
    let config = toml::toml!(
        [adhoc_groups]
        group1 = ["user1", "rust-lang/team2"]
    );
    test_candidates(config, issue().call()).teams(&teams).run(
        &["team1", "group1", "user3"],
        Ok(&["t-user1", "t-user2", "user1", "user3"]),
    );
}

#[test]
fn group_team_user_precedence() {
    // How it handles ambiguity when names overlap.
    let teams = toml::toml!(compiler = ["t-user1"]);
    let config = toml::toml!(
        [adhoc_groups]
        compiler = ["user2"]
    );
    test_candidates(config.clone(), issue().call())
        .teams(&teams)
        .run(&["compiler"], Ok(&["user2"]));
    test_candidates(config, issue().call())
        .teams(&teams)
        .run(&["rust-lang/compiler"], Ok(&["user2"]));
}

#[test]
fn what_do_slashes_mean() {
    // How slashed names are handled.
    let teams = toml::toml!(compiler = ["t-user1"]);
    let config = toml::toml!(
        [adhoc_groups]
        compiler = ["user2"]
        "foo/bar" = ["foo-user"]
    );
    let issue = || issue().org("rust-lang-nursery").call();

    // Random slash names should work from groups.
    test_candidates(config.clone(), issue())
        .teams(&teams)
        .run(&["foo/bar"], Ok(&["foo-user"]));

    test_candidates(config, issue())
        .teams(&teams)
        .run(&["rust-lang-nursery/compiler"], Ok(&["user2"]));
}

#[test]
fn invalid_org_doesnt_match() {
    let teams = toml::toml!(compiler = ["t-user1"]);
    let config = toml::toml!(
        [adhoc_groups]
        compiler = ["user2"]
    );
    test_candidates(config, issue().call()).teams(&teams).run(
        &["github/compiler"],
        Err(FindReviewerError::TeamNotFound(
            "github/compiler".to_string(),
        )),
    );
}

#[test]
fn vacation() {
    let teams = toml::toml!(bootstrap = ["jyn514", "Mark-Simulacrum"]);
    let config = toml::toml!(users_on_vacation = ["jyn514"]);

    // Test that `r? user` returns a specific error about the user being on vacation.
    test_candidates(config.clone(), issue().call())
        .teams(&teams)
        .run(
            &["jyn514"],
            Err(FindReviewerError::ReviewerOnVacation {
                username: "jyn514".to_string(),
            }),
        );

    test_candidates(config.clone(), issue().call())
        .teams(&teams)
        .run(&["bootstrap"], Ok(&["Mark-Simulacrum"]));
}
