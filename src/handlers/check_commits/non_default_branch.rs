use crate::{config::WarnNonDefaultBranchException, github::IssuesEvent};

const NON_DEFAULT_BRANCH: &str =
    "Pull requests are usually filed against the {default} branch for this repo, \
     but this one is against {target}. \
     Please double check that you specified the right target!";

const NON_DEFAULT_BRANCH_EXCEPTION: &str =
    "Pull requests targetting the {default} branch are usually filed against the {default} \
     branch, but this one is against {target}. \
     Please double check that you specified the right target!";

/// Returns a message if the PR is opened against the non-default branch (or the exception branch
/// if it's an exception).
pub(super) fn non_default_branch(
    exceptions: &[WarnNonDefaultBranchException],
    event: &IssuesEvent,
) -> Option<String> {
    let target_branch = &event.issue.base.as_ref().unwrap().git_ref;
    let (default_branch, warn_msg) = exceptions
        .iter()
        .find(|e| event.issue.title.contains(&e.title))
        .map_or_else(
            || (&event.repository.default_branch, NON_DEFAULT_BRANCH),
            |e| (&e.branch, NON_DEFAULT_BRANCH_EXCEPTION),
        );
    if target_branch == default_branch {
        return None;
    }
    Some(
        warn_msg
            .replace("{default}", default_branch)
            .replace("{target}", target_branch),
    )
}
