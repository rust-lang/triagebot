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
    files: &[FileDiff],
    config: &LargePullRequests,
) -> Option<String> {
    let exclude_matcher = ModifiedPathMatcher::new(&config.exclude_files);

    let number_of_changed_lines: usize = files
        .iter()
        .filter_map(|file| {
            if !exclude_matcher.is_match(&file.filename) {
                // In the patch, we only care about additions (starting with +) and deletions (-)
                // dbg!("@");
                Some(
                    file.patch
                        .lines()
                        .filter(|&line| ['-', '+'].iter().any(|&s| line.trim().starts_with(s)))
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
        threshold: 100,
    };

    assert!(large_pull_requests(&files, &no_excluded_threshold_100).is_some());

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

    assert!(large_pull_requests(&files, &no_excluded_threshold_100).is_none());

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

    assert!(large_pull_requests(&files, &no_excluded_threshold_100).is_some());

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
        threshold: 100u64,
    };

    assert!(large_pull_requests(&files, &jpeg_excluded_threshold_100).is_none());
}
