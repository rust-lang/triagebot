use crate::config::LargePullRequests;
use crate::github::FileDiff;
use crate::utils::ModifiedPathMatcher;

pub fn large_pull_request_message(threshold: u64) -> String {
    format!(
        "Seems that this commit is larger than expected (threshold: {threshold} lines of code changed) \
Big pull requests are reviewed much slower than small ones, consider splitting it."
    )
}

pub(super) fn large_pull_requests(
    pr_title: &str,
    files: &[FileDiff],
    config: &LargePullRequests,
) -> Option<String> {
    // Don't trigger if the PR has any of the excluded title segments.
    if config.exclude_titles.iter().any(|s| pr_title.contains(s)) {
        return None;
    }

    let exclude_matcher = ModifiedPathMatcher::new(&config.exclude_files);

    let number_of_changed_lines: usize = files
        .iter()
        .filter_map(|file| {
            if !exclude_matcher.is_match(&file.filename) {
                // In the patch, we only care about additions (starting with +) and deletions (-).
                // Context lines start with a space, so they must not be counted even when their
                // content begins with `+` or `-`; `+++`/`---` lines are file headers, not changes.
                Some(
                    file.patch
                        .lines()
                        .filter(|line| {
                            (!line.starts_with("+++") && line.starts_with('+'))
                                || (!line.starts_with("---") && line.starts_with('-'))
                        })
                        .count(),
                )
            } else {
                None
            }
        })
        .sum();

    if (number_of_changed_lines as u64) >= config.threshold {
        return Some(large_pull_request_message(config.threshold));
    }

    None
}

#[test]
fn enough_changed_files() {
    let files: [FileDiff; 50] = std::array::from_fn(|_| FileDiff {
        filename: "file.txt".into(),
        previous_filename: None,
        patch: r#"
+ let new_variable = 0;
- let old_variable = 1;
"#
        .into(),
    });

    let no_excluded_threshold_100 = LargePullRequests {
        exclude_files: vec![],
        exclude_titles: vec![],
        threshold: 100,
    };

    assert!(large_pull_requests("Title", &files, &no_excluded_threshold_100).is_some());

    let files: [FileDiff; 50] = std::array::from_fn(|_| FileDiff {
        filename: "file.txt".into(),
        previous_filename: None,
        patch: r#"
@@@
@@@ These do not count as addition or deletion
@@@
+ // And these are not enough!
"#
        .into(),
    });

    assert!(large_pull_requests("Title", &files, &no_excluded_threshold_100).is_none());

    let files: [FileDiff; 25] = std::array::from_fn(|_| FileDiff {
        filename: "file.txt".into(),
        previous_filename: None,
        patch: r#"
- // With only 25 of these, with four lines each, we already get to 100
+ // (●ↀωↀ●)✧
- // (=‘ｘ‘=)
+ // ฅ⊱*•ω•*⊰ฅ
"#
        .into(),
    });

    assert!(large_pull_requests("Title!", &files, &no_excluded_threshold_100).is_some());

    let files: [FileDiff; 300] = std::array::from_fn(|_| FileDiff {
        filename: "actually_ignored.jpeg".into(),
        previous_filename: None,
        patch: r#"
+ // Ignored filename! No matter how many lines, we don't cross the threshold
"#
        .into(),
    });

    let jpeg_excluded_threshold_100 = LargePullRequests {
        exclude_files: vec!["*.jpeg".into()],
        exclude_titles: vec![],
        threshold: 100u64,
    };

    assert!(large_pull_requests("Title", &files, &jpeg_excluded_threshold_100).is_none());
}

#[test]
fn context_lines_and_file_headers_are_not_counted() {
    // Context lines in a unified diff start with a space, so their content may begin with
    // `+` or `-` without being a change. `+++`/`---` are file headers, not changes either.
    let files: [FileDiff; 50] = std::array::from_fn(|_| FileDiff {
        filename: "file.txt".into(),
        previous_filename: None,
        patch: r#"
--- a/file.txt
+++ b/file.txt
@@ -1,2 +1,2 @@
 + context line whose content begins with a plus
 - context line whose content begins with a minus
"#
        .into(),
    });

    let no_excluded_threshold_100 = LargePullRequests {
        exclude_files: vec![],
        exclude_titles: vec![],
        threshold: 100,
    };

    // These 50 * 4 lines used to cross the threshold when the counter trimmed each line.
    assert!(large_pull_requests("Title", &files, &no_excluded_threshold_100).is_none());
}
