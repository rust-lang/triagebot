//! Tests for the stale assignment warning helpers

use super::super::*;
use crate::tests::github::user;
use chrono::Duration;

const BOT: &str = "rustbot";

#[test]
fn no_assignees_means_no_assigned_user() {
    assert_eq!(assigned_user(BOT, &[]), None);
}

#[test]
fn regular_assignee_is_returned() {
    let assignees = [user("alice", 1)];
    assert_eq!(assigned_user(BOT, &assignees), Some("alice"));
}

#[test]
fn bot_assignment_is_ignored() {
    let assignees = [user("rustbot", 2)];
    assert_eq!(assigned_user(BOT, &assignees), None);
}

#[test]
fn bot_login_matches_case_insensitively() {
    let assignees = [user("RustBot", 2)];
    assert_eq!(assigned_user(BOT, &assignees), None);
}

#[test]
fn staleness_threshold_is_inclusive() {
    assert!(!is_stale(
        Duration::days(STALE_ASSIGNMENT_DAYS) - Duration::seconds(1)
    ));
    assert!(is_stale(Duration::days(STALE_ASSIGNMENT_DAYS)));
    assert!(is_stale(Duration::days(STALE_ASSIGNMENT_DAYS + 30)));
}
