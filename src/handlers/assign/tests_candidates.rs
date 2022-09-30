//! Tests for `candidate_reviewers_from_names`

use super::*;

/// Basic test function for testing `candidate_reviewers_from_names`.
fn test_from_names(
    teams: Option<toml::Value>,
    config: toml::Value,
    issue: serde_json::Value,
    names: &[&str],
    expected: &[&str],
) {
    let (teams, config, issue) = convert_simplified(teams, config, issue);
    let names: Vec<_> = names.iter().map(|n| n.to_string()).collect();
    let candidates = candidate_reviewers_from_names(&teams, &config, &issue, &names).unwrap();
    let mut candidates: Vec<_> = candidates.into_iter().collect();
    candidates.sort();
    let expected: Vec<_> = expected.iter().map(|x| *x).collect();
    assert_eq!(candidates, expected);
}

/// Convert the simplified input in preparation for `candidate_reviewers_from_names`.
fn convert_simplified(
    teams: Option<toml::Value>,
    config: toml::Value,
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
    test_from_names(None, config, issue, &["compiler"], &[]);
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
    test_from_names(None, config, issue, &["c"], &["nrc", "pnkfelix"]);
}

#[test]
fn candidate_filtered_author_only_candidate() {
    // When the author is the only candidate.
    let config = toml::toml!(
        [adhoc_groups]
        compiler = ["nikomatsakis"]
    );
    let issue = generic_issue("nikomatsakis", "rust-lang/rust");
    test_from_names(None, config, issue, &["compiler"], &[]);
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
        &["user1", "user3", "user4"],
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
    test_from_names(None, config, issue, &["compiler"], &["user4"]);
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
        &["t-user1", "t-user2", "user1", "user3"],
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
        &["user2"],
    );
    test_from_names(
        Some(teams.clone()),
        config.clone(),
        issue.clone(),
        &["rust-lang/compiler"],
        &["user2"],
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
        &["foo-user"],
    );
    // Since this is rust-lang-nursery, it uses the rust-lang team, not the group.
    test_from_names(
        Some(teams.clone()),
        config.clone(),
        issue.clone(),
        &["rust-lang/compiler"],
        &["t-user1"],
    );
    test_from_names(
        Some(teams.clone()),
        config.clone(),
        issue.clone(),
        &["rust-lang-nursery/compiler"],
        &["user2"],
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
    let (teams, config, issue) = convert_simplified(Some(teams), config, issue);
    let names = vec!["github/compiler".to_string()];
    match candidate_reviewers_from_names(&teams, &config, &issue, &names) {
        Ok(x) => panic!("expected err, got {x:?}"),
        Err(FindReviewerError::TeamNotFound(_)) => {}
        Err(e) => panic!("unexpected error {e:?}"),
    }
}
