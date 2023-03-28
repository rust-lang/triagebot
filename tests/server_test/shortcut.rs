use crate::server_test::TestPrBuilder;
use super::run_test;
use crate::server_test::ServerTestSetup;
use triagebot::github::{GithubClient, Repository};
use triagebot::github::{Issue, Label};

async fn prepare_repo_for_shortcut(setup: &ServerTestSetup, initial_label: &str) -> Issue {
    setup.config("[shortcut]").await;
    // Set up labels that all shortcut tests need.
    for label in ["S-waiting-on-review", "S-waiting-on-author", "S-blocked"] {
        if !setup.repo.has_label(&setup.gh, label).await.unwrap() {
            setup
                .repo
                .create_label(&setup.gh, label, "d3dddd", "")
                .await
                .unwrap();
        }
    }
    let pr = TestPrBuilder::new("shortcut-test", &setup.repo.default_branch)
    .create(&setup.gh, &setup.repo).await;
    pr.add_labels(
        &setup.gh,
        vec![Label {
            name: initial_label.into(),
        }],
    )
    .await
    .unwrap();
    pr
}

async fn wait_for_labels(gh: &GithubClient, repo: &Repository, pr: u64, labels: &[&str]) {
    eprintln!("waiting for labels to update");
    for _ in 0..5 {
        let pr = repo.get_pr(gh, pr).await.unwrap();
        let current_labels = pr.labels();
        eprintln!("current_labels={current_labels:?}");
        if labels
            .iter()
            .all(|expected| current_labels.iter().any(|l| l.name == *expected))
            && labels.len() == current_labels.len()
        {
            return;
        }
        tokio::time::sleep(std::time::Duration::new(5, 0)).await;
    }
    panic!("labels did not update to expected {labels:?}");
}

async fn shortcut_test(
    setup: ServerTestSetup,
    initial_label: &str,
    comment: &str,
    expected_label: &str,
) {
    let pr = prepare_repo_for_shortcut(&setup, initial_label).await;
    let ctx = setup.launch_triagebot_live();
    let gh = ctx.gh.as_ref().unwrap();
    let repo = ctx.repo.as_ref().unwrap();
    pr.post_comment(gh, comment).await.unwrap();
    wait_for_labels(gh, repo, pr.number, &[expected_label]).await;
}

#[test]
fn author() {
    run_test("shortcut/author", |setup| async move {
        shortcut_test(
            setup,
            "S-waiting-on-review",
            "@rustbot author",
            "S-waiting-on-author",
        )
        .await;
    });
}

#[test]
fn ready() {
    run_test("shortcut/ready", |setup| async move {
        shortcut_test(
            setup,
            "S-waiting-on-author",
            "@rustbot ready",
            "S-waiting-on-review",
        )
        .await;
    });
}

#[test]
fn blocked() {
    run_test("shortcut/blocked", |setup| async move {
        shortcut_test(
            setup,
            "S-waiting-on-author",
            "@rustbot blocked",
            "S-blocked",
        )
        .await;
    });
}
