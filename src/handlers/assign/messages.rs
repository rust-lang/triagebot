//! Assignment messages functions and constants.
//!
//! This module contains the different constants and functions related
//! to assignment messages.

pub fn new_user_welcome_message(reviewer: &str) -> String {
    format!(
        "Thanks for the pull request, and welcome! \
The Rust team is excited to review your changes, and you should hear from {reviewer} \
some time within the next two weeks."
    )
}

pub fn contribution_message(contributing_url: &str, bot: &str) -> String {
    format!(
        "Please see [the contribution \
instructions]({contributing_url}) for more information. Namely, in order to ensure the \
minimum review times lag, PR authors and assigned reviewers should ensure that the review \
label (`S-waiting-on-review` and `S-waiting-on-author`) stays updated, invoking these commands \
when appropriate:

- `@{bot} author`: the review is finished, PR author should check the comments and take action accordingly
- `@{bot} review`: the author is ready for a review, this PR will be queued again in the reviewer's queue"
    )
}

pub fn welcome_with_reviewer(assignee: &str) -> String {
    format!("@{assignee} (or someone else)")
}

pub fn returning_user_welcome_message(assignee: &str, bot: &str) -> String {
    format!(
        "r? @{assignee}

{bot} has assigned @{assignee}.
They will have a look at your PR within the next two weeks and either review your PR or \
reassign to another reviewer.

Use `r?` to explicitly pick a reviewer"
    )
}

pub fn returning_user_welcome_message_no_reviewer(pr_author: &str) -> String {
    format!("@{pr_author}: no appropriate reviewer found, use `r?` to override")
}

pub fn reviewer_off_rotation_message(username: &str) -> String {
    format!(
        r"`{username}` is not available for reviewing at the moment.

Please choose another assignee."
    )
}

pub fn reviewer_assigned_before(username: &str) -> String {
    format!(
        "Requested reviewer @{username} was already assigned before.

Please choose another assignee by using `r? @reviewer`."
    )
}

pub const WELCOME_WITHOUT_REVIEWER: &str = "@Mark-Simulacrum (NB. this repo may be misconfigured)";

pub const REVIEWER_IS_PR_AUTHOR: &str = "Pull request author cannot be assigned as reviewer.


Please choose another assignee.";

pub const REVIEWER_ALREADY_ASSIGNED: &str =
    "Requested reviewer is already assigned to this pull request.

Please choose another assignee.";
