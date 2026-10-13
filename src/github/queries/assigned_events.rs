use anyhow::Context as _;
use chrono::{DateTime, Utc};

use crate::github::{GithubClient, IssueRepository};

#[derive(Debug, serde::Deserialize)]
struct AssignedEvent {
    #[serde(rename = "createdAt")]
    created_at: DateTime<Utc>,
    assignee: Option<AssignedEventAssignee>,
}

#[derive(Debug, serde::Deserialize)]
struct AssignedEventAssignee {
    login: String,
}

#[derive(Debug, serde::Deserialize)]
struct AssignedEvents {
    nodes: Vec<Option<AssignedEvent>>,
}

fn latest_assignment(events: &[Option<AssignedEvent>], login: &str) -> Option<DateTime<Utc>> {
    events
        .iter()
        .flatten()
        .filter(|event| {
            event
                .assignee
                .as_ref()
                .is_some_and(|assignee| assignee.login.eq_ignore_ascii_case(login))
        })
        .map(|event| event.created_at)
        .max()
}

impl GithubClient {
    pub async fn last_assigned_at(
        &self,
        repo: &IssueRepository,
        issue_number: u64,
        login: &str,
    ) -> anyhow::Result<Option<DateTime<Utc>>> {
        let mut data = self
            .graphql_query(
                r#"
query ($owner: String!, $repo: String!, $issue: Int!) {
  repository(owner: $owner, name: $repo) {
    issue(number: $issue) {
      timelineItems(last: 50, itemTypes: [ASSIGNED_EVENT]) {
        nodes {
          ... on AssignedEvent {
            createdAt
            assignee {
              ... on User {
                login
              }
              ... on Bot {
                login
              }
            }
          }
        }
      }
    }
  }
}
"#,
                serde_json::json!({
                    "owner": repo.organization,
                    "repo": repo.repository,
                    "issue": issue_number,
                }),
            )
            .await
            .context("failed to load assignment events")?;

        let events: AssignedEvents =
            serde_json::from_value(data["data"]["repository"]["issue"]["timelineItems"].take())
                .context("failed to deserialize assignment events")?;

        Ok(latest_assignment(&events.nodes, login))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn event(login: &str, timestamp: i64) -> Option<AssignedEvent> {
        Some(AssignedEvent {
            created_at: Utc.timestamp_opt(timestamp, 0).unwrap(),
            assignee: Some(AssignedEventAssignee {
                login: login.to_string(),
            }),
        })
    }

    #[test]
    fn picks_latest_assignment_for_login() {
        let events = vec![event("foo", 100), event("bar", 200), event("foo", 300)];

        assert_eq!(
            latest_assignment(&events, "foo"),
            Some(Utc.timestamp_opt(300, 0).unwrap())
        );
    }

    #[test]
    fn matches_login_case_insensitively() {
        let events = vec![event("Foo", 100)];

        assert_eq!(
            latest_assignment(&events, "foo"),
            Some(Utc.timestamp_opt(100, 0).unwrap())
        );
    }

    #[test]
    fn ignores_other_assignees() {
        let events = vec![event("foo", 100), event("bar", 300)];

        assert_eq!(
            latest_assignment(&events, "foo"),
            Some(Utc.timestamp_opt(100, 0).unwrap())
        );
    }
}
