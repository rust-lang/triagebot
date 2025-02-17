use crate::github::{Issue, IssueState, PullRequestDetails, User};
use bon::builder;
use chrono::Utc;

pub fn default_test_user() -> User {
    User {
        login: "triagebot-tester".to_string(),
        id: 1,
    }
}

pub fn user(login: &str, id: u64) -> User {
    User {
        login: login.to_string(),
        id,
    }
}

#[builder]
pub fn issue(
    state: Option<IssueState>,
    number: Option<u64>,
    author: Option<User>,
    body: Option<&str>,
    assignees: Option<Vec<User>>,
    pr: Option<bool>,
) -> Issue {
    let number = number.unwrap_or(1);
    let state = state.unwrap_or(IssueState::Open);
    let author = author.unwrap_or(default_test_user());
    let body = body.unwrap_or("").to_string();
    let assignees = assignees.unwrap_or_default();
    let pull_request = if pr.unwrap_or(false) {
        Some(PullRequestDetails::new())
    } else {
        None
    };

    Issue {
        number,
        body,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        merge_commit_sha: None,
        title: format!("Issue #{number}"),
        html_url: "<html-url>".to_string(),
        user: author,
        labels: vec![],
        assignees,
        pull_request,
        merged: false,
        draft: false,
        comments: None,
        comments_url: "".to_string(),
        repository: Default::default(),
        base: None,
        head: None,
        state,
        milestone: None,
        mergeable: None,
    }
}

#[builder]
pub fn pull_request(
    state: Option<IssueState>,
    number: Option<u64>,
    author: Option<User>,
    body: Option<&str>,
    assignees: Option<Vec<User>>,
) -> Issue {
    issue()
        .maybe_state(state)
        .maybe_number(number)
        .maybe_author(author)
        .maybe_body(body)
        .maybe_assignees(assignees)
        .pr(true)
        .call()
}
