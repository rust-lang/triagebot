//! Tests for `candidate_reviewers_from_names`

use super::super::*;

/// Basic test function for testing `candidate_reviewers_from_names`.
fn test_from_names(
    teams: Option<toml::Table>,
    config: toml::Table,
    issue: serde_json::Value,
    names: &[&str],
    expected: Result<&[&str], FindReviewerError>,
) {
    let (teams, config, issue) = convert_simplified(teams, config, issue);
    let names: Vec<_> = names.iter().map(|n| n.to_string()).collect();
    match (
        candidate_reviewers_from_names(&teams, &config, &issue, &names),
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

/// Convert the simplified input in preparation for `candidate_reviewers_from_names`.
fn convert_simplified(
    teams: Option<toml::Table>,
    config: toml::Table,
    issue: serde_json::Value,
) -> (Teams, AssignConfig, Issue) {
    // Convert the simplified team config to a real team config.
    // This uses serde_json since it is easier to manipulate than toml.
    let teams: serde_json::Value = match teams {
        Some(teams) => teams.try_into().unwrap(),
        None => serde_json::json!({}),
    };
    let mut teams_config = serde_json::json!({});
    for (team_name, members) in teams.as_object().unwrap() {
        let members: Vec<_> = members.as_array().unwrap().iter().map(|member| {
            serde_json::json!({"name": member, "github": member, "github_id": 1, "is_lead": false})
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
    let teams = serde_json::value::from_value(teams_config).unwrap();
    let config = config.try_into().unwrap();
    let issue = serde_json::value::from_value(issue).unwrap();
    (teams, config, issue)
}

fn generic_issue(author: &str, repo: &str) -> serde_json::Value {
    serde_json::json!({
        "number": 1234,
        "created_at": "2022-06-26T21:31:31Z",
        "updated_at": "2022-06-26T21:31:31Z",
        "title": "Example PR",
        "body": "PR body",
        "html_url": "https://github.com/rust-lang/rust/pull/1234",
        "user": {
            "login": author,
            "id": 583231,
        },
        "labels": [],
        "assignees": [],
        "comments_url": format!("https://api.github.com/repos/{repo}/pull/1234/comments"),
        "state": "open",
    })
}

#[test]
fn circular_groups() {
    // A cycle in the groups map.
    let config = toml::toml!(
        [adhoc_groups]
        compiler = ["other"]
        other = ["compiler"]
    );
    let issue = generic_issue("octocat", "rust-lang/rust");
    test_from_names(
        None,
        config,
        issue,
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
    let issue = generic_issue("octocat", "rust-lang/rust");
    test_from_names(None, config, issue, &["c"], Ok(&["nrc", "pnkfelix"]));
}

#[test]
fn candidate_filtered_author_only_candidate() {
    // When the author is the only candidate.
    let config = toml::toml!(
        [adhoc_groups]
        compiler = ["nikomatsakis"]
    );
    let issue = generic_issue("nikomatsakis", "rust-lang/rust");
    test_from_names(
        None,
        config,
        issue,
        &["compiler"],
        Err(FindReviewerError::AllReviewersFiltered {
            initial: vec!["compiler".to_string()],
            filtered: vec!["nikomatsakis".to_string()],
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
    let issue = generic_issue("user2", "rust-lang/rust");
    test_from_names(
        None,
        config,
        issue,
        &["compiler"],
        Ok(&["user1", "user3", "user4"]),
    );
}

#[test]
fn candidate_filtered_assignee() {
    // Filter out an existing assignee from the candidates.
    let config = toml::toml!(
        [adhoc_groups]
        compiler = ["user1", "user2", "user3", "user4"]
    );
    let mut issue = generic_issue("user2", "rust-lang/rust");
    issue["assignees"] = serde_json::json!([
        {"login": "user1", "id": 1},
        {"login": "user3", "id": 3},
    ]);
    test_from_names(None, config, issue, &["compiler"], Ok(&["user4"]));
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
    let issue = generic_issue("octocat", "rust-lang/rust");
    test_from_names(
        Some(teams),
        config,
        issue,
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
    let issue = generic_issue("octocat", "rust-lang/rust");
    test_from_names(
        Some(teams.clone()),
        config.clone(),
        issue.clone(),
        &["compiler"],
        Ok(&["user2"]),
    );
    test_from_names(
        Some(teams.clone()),
        config.clone(),
        issue.clone(),
        &["rust-lang/compiler"],
        Ok(&["user2"]),
    );
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
    let issue = generic_issue("octocat", "rust-lang-nursery/rust");
    // Random slash names should work from groups.
    test_from_names(
        Some(teams.clone()),
        config.clone(),
        issue.clone(),
        &["foo/bar"],
        Ok(&["foo-user"]),
    );
    // Since this is rust-lang-nursery, it uses the rust-lang team, not the group.
    test_from_names(
        Some(teams.clone()),
        config.clone(),
        issue.clone(),
        &["rust-lang/compiler"],
        Ok(&["t-user1"]),
    );
    test_from_names(
        Some(teams.clone()),
        config.clone(),
        issue.clone(),
        &["rust-lang-nursery/compiler"],
        Ok(&["user2"]),
    );
}

#[test]
fn invalid_org_doesnt_match() {
    let teams = toml::toml!(compiler = ["t-user1"]);
    let config = toml::toml!(
        [adhoc_groups]
        compiler = ["user2"]
    );
    let issue = generic_issue("octocat", "rust-lang/rust");
    test_from_names(
        Some(teams),
        config,
        issue,
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
    let issue = generic_issue("octocat", "rust-lang/rust");

    // Test that `r? user` falls through to assigning from the team.
    // See `determine_assignee` - ideally we would test that function directly instead of indirectly through `find_reviewer_from_names`.
    let err_names = vec!["jyn514".into()];
    test_from_names(
        Some(teams.clone()),
        config.clone(),
        issue.clone(),
        &["jyn514"],
        Err(FindReviewerError::AllReviewersFiltered {
            initial: err_names.clone(),
            filtered: err_names,
        }),
    );

    // Test that `r? bootstrap` doesn't assign from users on vacation.
    test_from_names(
        Some(teams.clone()),
        config.clone(),
        issue,
        &["bootstrap"],
        Ok(&["Mark-Simulacrum"]),
    );
}
