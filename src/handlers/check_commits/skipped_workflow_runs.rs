use crate::{config::SkippedWorkflowRunsConfig, github::GithubCommit};

// https://docs.github.com/en/actions/how-tos/manage-workflow-runs/skip-workflow-runs
const INSTRUCTIONS_ANYWHERE: &[&str] = &[
    "[skip ci]",
    "[ci skip]",
    "[no ci]",
    "[skip actions]",
    "[actions skip]",
];

const INSTRUCTIONS_TRAILER: &[&str] = &["skip-checks:true", "skip-checks: true"];

const WARNING_MESSAGE: &str = "The last commit's message of this PR contains an instruction to [skip workflow runs](https://docs.github.com/en/actions/how-tos/manage-workflow-runs/skip-workflow-runs) for this PR (i.e., skip running CI).\nThis is usually not advisable, as it bypasses automated testing.";

/// Check if the HEAD commit has instruction to skip workflow runs
pub(super) fn skipped_workflow_runs(
    _config: &SkippedWorkflowRunsConfig,
    commits: &[GithubCommit],
) -> Option<String> {
    let msg = commits.last()?.commit.message.as_str();

    // First check instructions that can happen anywhere
    if INSTRUCTIONS_ANYWHERE.iter().any(|ins| msg.contains(ins)) {
        return Some(WARNING_MESSAGE.to_string());
    }

    // Second let's check the trailer section (instructions at the very end of the commit message)

    let mut lines = msg.lines().rev();
    let last_line = lines.next()?;

    // Per GitHub docs only the last trailer instruction is taken into account for skipping checks
    // so we also only check that one
    if !INSTRUCTIONS_TRAILER.iter().any(|ins| last_line == *ins) {
        return None;
    }

    let empty_lines_before_trailer = lines
        // let's skip all the other trailers
        .skip_while(|l| !l.trim().is_empty())
        .take_while(|l| l.trim().is_empty())
        .count();

    // GitHub says that the trailer section should also "be preceded by two empty lines"
    if empty_lines_before_trailer >= 2 {
        Some(WARNING_MESSAGE.to_string())
    } else {
        None
    }
}

#[test]
fn with_instructions() {
    // [skip ci]
    assert!(
        skipped_workflow_runs(
            &SkippedWorkflowRunsConfig {},
            &[super::dummy_commit_from_body(
                "123",
                "This is a title\nLet's [skip ci]\nAnd no we have no CI"
            )]
        )
        .is_some()
    );

    // [ci skip]
    assert!(
        skipped_workflow_runs(
            &SkippedWorkflowRunsConfig {},
            &[super::dummy_commit_from_body(
                "123",
                "This is a title\n[ci skip]"
            )]
        )
        .is_some()
    );

    // [no ci]
    assert!(
        skipped_workflow_runs(
            &SkippedWorkflowRunsConfig {},
            &[super::dummy_commit_from_body(
                "123",
                "[no ci] This is my title\n"
            )]
        )
        .is_some()
    );

    // skip-checks: true
    assert!(
        skipped_workflow_runs(
            &SkippedWorkflowRunsConfig {},
            &[super::dummy_commit_from_body(
                "123",
                "This is my title\n\n\nskip-checks: true"
            )]
        )
        .is_some()
    );

    // skip-checks:true with other trailer
    assert!(
        skipped_workflow_runs(
            &SkippedWorkflowRunsConfig {},
            &[super::dummy_commit_from_body(
                "123",
                "This is my title\n\n\nSigned-by: Foo\nskip-checks:true\n"
            )]
        )
        .is_some()
    );
}

#[test]
fn without_instructions() {
    assert!(
        skipped_workflow_runs(
            &SkippedWorkflowRunsConfig {},
            &[super::dummy_commit_from_body(
                "123",
                "This is a dummy message\n it doesn't have any instructions."
            )]
        )
        .is_none()
    );

    assert!(
        skipped_workflow_runs(
            &SkippedWorkflowRunsConfig {},
            &[super::dummy_commit_from_body(
                "123",
                "This is dummy message has a fake skip ci instuction (missing braces)."
            )]
        )
        .is_none()
    );
}

#[test]
fn not_in_head() {
    assert!(
        skipped_workflow_runs(
            &SkippedWorkflowRunsConfig {},
            &[
                super::dummy_commit_from_body("123", "[skip ci] instruction"),
                super::dummy_commit_from_body(
                    "124",
                    "This is head, but without the ci skip instrunction."
                )
            ]
        )
        .is_none()
    );
}

#[test]
fn not_two_empty_lines_before_trailer() {
    assert!(
        skipped_workflow_runs(
            &SkippedWorkflowRunsConfig {},
            &[super::dummy_commit_from_body(
                "123",
                "This is head.\nskip-checks: true"
            )]
        )
        .is_none()
    );

    assert!(
        skipped_workflow_runs(
            &SkippedWorkflowRunsConfig {},
            &[super::dummy_commit_from_body(
                "123",
                "This is head.\n\nskip-checks: true"
            )]
        )
        .is_none()
    );
}
