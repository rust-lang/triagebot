use anyhow::Context as _;

use crate::config::LargePullRequests;
use crate::github::{GithubCommit, GithubCompare, IssuesAction};
use crate::utils::ModifiedPathMatcher;
use crate::{github::Event, handlers::Context};

pub fn large_pull_request_message(threshold: u64) -> String {
    format!(
        "Seems that this commit is larger than expected (threshold: {threshold} lines of code changed) \
Big pull requests are reviewed much slower than small ones, consider splitting it."
    )
}

pub(crate) fn large_pull_requests(
    compare: &GithubCompare,
    config: &LargePullRequests,
) -> Option<String> {
    let exclude_matcher = ModifiedPathMatcher::new(&config.exclude_files);

    let number_of_changed_lines: usize = compare.files
        .iter()
        .filter_map(|file| {
            if !exclude_matcher.is_match(&file.filename) {
                // In the patch, we only care about additions (starting with +) and deletions (-)
                Some(file.patch.lines().filter(|&line| ['-', '+'].iter().any(|&s| line.starts_with(s))).count())
            } else {
                None
            }
        }).sum();

    if (number_of_changed_lines as u64) >= config.threshold {
        return Some(large_pull_request_message(config.threshold));
    }

    None
}
