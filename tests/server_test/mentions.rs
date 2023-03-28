use crate::server_test::close_opened_prs;
use super::run_test;
use crate::server_test::TestPrBuilder;
use triagebot::github::GitTreeEntry;
use triagebot::github::GithubClient;
use triagebot::github::Issue;

async fn expected_comments(gh: &GithubClient, pr: &Issue, comments: &[&str]) {
    let mut found = false;
    // This repeats to make sure that no unexpected additional comments are posted.
    for _ in 0..5 {
        let current_comments = pr.get_comments(gh).await.unwrap();
        let n_current = current_comments.len();
        let n_expected = comments.len();
        if n_current < n_expected {
        } else if n_current == n_expected {
            found = true;
        } else {
            panic!(
                "too many comments received (got {n_current} expected {n_expected}):\n\
                expected:\n\
                {comments:?}\n\
                \n\
                got:\n\
                {current_comments:#?}"
            );
        }
        tokio::time::sleep(std::time::Duration::new(5, 0)).await;
    }
    if !found {
        panic!(
            "did not find expected comments {comments:?} on PR {}",
            pr.html_url
        );
    }
}

#[test]
fn default_mention() {
    // A new PR that touches a file in the [mentions] config with the default
    // message.
    run_test("mentions/default_mention", |setup| async move {
        setup
            .config(
                r#"
                    [mentions.'foo/example1']
                    cc = ["@octocat"]
                "#,
            )
            .await;
        close_opened_prs(&setup.gh, &setup.repo, "mentions-test").await;
        let ctx = setup.launch_triagebot_live();
        let gh = ctx.gh.as_ref().unwrap();
        let repo = ctx.repo.as_ref().unwrap();
        let pr = TestPrBuilder::new("mentions-test", &repo.default_branch)
            .file("foo/example1", false, "sample text")
            .create(gh, repo)
            .await;
        expected_comments(
            gh,
            &pr,
            &["Some changes occurred in foo/example1\n\ncc @octocat"],
        ).await;
    });
}

#[test]
fn custom_message() {
    // A new PR that touches a file in the [mentions] config with a custom
    // message.
    run_test("mentions/custom_mention", |setup| async move {
        setup
            .config(
                r#"
                    [mentions.'foo/example1']
                    cc = ["@octocat"]
                    message = "a custom message"
                "#,
            )
            .await;
        close_opened_prs(&setup.gh, &setup.repo, "mentions-test").await;
        let ctx = setup.launch_triagebot_live();
        let gh = ctx.gh.as_ref().unwrap();
        let repo = ctx.repo.as_ref().unwrap();
        let pr = TestPrBuilder::new("mentions-test", &repo.default_branch)
            .file("foo/example1", false, "sample text")
            .create(gh, repo)
            .await;
        expected_comments(gh, &pr, &["a custom message\n\ncc @octocat"]).await;
    });
}

// "response_body": "[mentions.'foo/example1']\n
// cc = [\"@grashgal\", \"@ehuss\"]\n\n
// [mentions.'foo/example2']\n
// cc = [\"@grashgal\", \"@ehuss\"]\nmessage = \"a custom message\"\n"

#[test]
fn dont_mention_twice() {
    // When pushing modifications to the same files, don't mention again.
    //
    // However if a push comes in for a different file, make sure it mentions again.
    //
    // This starts with a new PR adding example1/README.md.
    // It then pushes an update to example1/README.md.
    // And then a second update to add example2/README.md.
    run_test("mentions/dont_mention_twice", |setup| async move {
        setup
            .config(
                r#"
                    [mentions.'foo/example1']
                    cc = ["@octocat"]

                    [mentions.'foo/example2']
                    cc = ["@octocat"]
                "#,
            )
            .await;
        close_opened_prs(&setup.gh, &setup.repo, "mentions-test").await;
        let ctx = setup.launch_triagebot_live();
        let gh = ctx.gh.as_ref().unwrap();
        let repo = ctx.repo.as_ref().unwrap();
        let pr = TestPrBuilder::new("mentions-test", &repo.default_branch)
            .file("foo/example1", false, "sample text")
            .create(gh, repo)
            .await;
        expected_comments(
            gh,
            &pr,
            &["Some changes occurred in foo/example1\n\ncc @octocat"],
        ).await;
        // Updating the same file should not comment again.
        let commits = pr.commits(gh).await.unwrap();
        let last_commit = commits.last().unwrap();
        let tree_entries = vec![GitTreeEntry {
            path: "foo/example1".into(),
            mode: "100644".into(),
            object_type: "blob".into(),
            sha: None,
            content: Some("this file has been edited".into()),
        }];
        let new_tree = repo
            .update_tree(gh, &last_commit.commit.tree.sha, &tree_entries)
            .await
            .unwrap();
        let commit = repo
            .create_commit(gh, "editing example1", &[&last_commit.sha], &new_tree.sha)
            .await
            .unwrap();
        repo.update_reference(gh, &format!("heads/mentions-test"), &commit.sha)
            .await
            .unwrap();
        expected_comments(
            gh,
            &pr,
            &["Some changes occurred in foo/example1\n\ncc @octocat"],
        )
        .await;

        // Updating a different file should mention again (even for the same user).
        let commits = pr.commits(gh).await.unwrap();
        let last_commit = commits.last().unwrap();
        let tree_entries = vec![GitTreeEntry {
            path: "foo/example2".into(),
            mode: "100644".into(),
            object_type: "blob".into(),
            sha: None,
            content: Some("adding a second file".into()),
        }];
        let new_tree = repo
            .update_tree(gh, &last_commit.commit.tree.sha, &tree_entries)
            .await
            .unwrap();
        let commit = repo
            .create_commit(gh, "adding example2", &[&last_commit.sha], &new_tree.sha)
            .await
            .unwrap();
        repo.update_reference(gh, &format!("heads/mentions-test"), &commit.sha)
            .await
            .unwrap();
        expected_comments(
            gh,
            &pr,
            &[
                "Some changes occurred in foo/example1\n\ncc @octocat",
                "Some changes occurred in foo/example2\n\ncc @octocat",
            ],
        )
        .await;
    });
}
