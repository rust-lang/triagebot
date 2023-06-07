//! `GithubClient` tests.
//!
//! These tests exercise the behavior of `GithubClient`. They involve setting
//! up HTTP servers, creating a `GithubClient` to connect to those servers,
//! executing some action, and validating the result.
//!
//! The [`run_test`] function is used to set up the test and give you a
//! [`GithubClient`] to perform some action and validate its result.
//!
//! To write one of these tests, you'll need to use the recording function
//! against the live GitHub site to fetch what the actual JSON objects should
//! look like. To write a test, follow these steps:
//!
//! 1. Choose a repo in your GitHub account to run tests against. I use either
//!    `ehuss/rust` or `ehuss/triagebot-test`, but any in your account will do.
//!
//!    **WARNING**: Do not write tests that modify the rust-lang org repos
//!    like rust-lang/rust. Write those tests against your own fork (like
//!    `ehuss/rust`). We don't want to pollute the real repos with things like
//!    test PRs.
//!
//! 2. Make sure your `.env` file is configured with a `GITHUB_API_TOKEN` that
//!    can access the repo you chose. See `README.md` for more about setting
//!    this up.
//!
//! 3. Create a `#[test]` function following the form of the other tests below
//!    where inside the `run_test` callback, execute the function you want to
//!    exercise.
//!
//! 4. Run just a single test with recording enabled:
//!
//!    ```sh
//!    TRIAGEBOT_TEST_RECORD_DIR=github_client/TEST_NAME_HERE cargo test \
//!        --test testsuite -- --exact github_client::TEST_NAME_HERE
//!    ```
//!
//!    Replace TEST_NAME_HERE with the name of your test. This will run the
//!    command against the live site and store the JSON in a directory with
//!    the given TEST_NAME_HERE.
//!
//!    Note that recording is only supported for one test at time.
//!
//! 5. Add some asserts to the result of the return value from the function.
//!
//! 6. Do a final test to make sure everything is working:
//!
//!    ```sh
//!    cargo test --test testsuite -- --exact github_client::TEST_NAME_HERE
//!    ```

use super::{HttpServer, HttpServerHandle};
use futures::Future;
use std::sync::mpsc;
use std::time::Duration;
use triagebot::github::GithubClient;
use triagebot::test_record::{self, Activity};

/// A context used for running a test.
struct GhTestCtx {
    gh: GithubClient,
    #[allow(dead_code)] // held for drop
    server: HttpServerHandle,
}

/// Checks that the server didn't generate any errors, and that it finished
/// processing all recorded events.
fn assert_no_error(hook_recv: &mpsc::Receiver<Activity>) {
    if test_record::is_recording() {
        return;
    }
    loop {
        let activity = hook_recv.recv_timeout(Duration::new(60, 0)).unwrap();
        match activity {
            Activity::Error { message } => {
                panic!("unexpected server error: {message}");
            }
            Activity::Finished => {
                break;
            }
            a => panic!("unexpected activity {a:?}"),
        }
    }
}

/// Creates a test context.
fn build(test_name: &str) -> GhTestCtx {
    crate::assert_single_record();
    crate::maybe_enable_logging();
    triagebot::test_record::init().unwrap();
    if test_record::is_recording() {
        // While recording, there are no activities to load.
        // Point the GithubClient to the real site.
        dotenv::dotenv().ok();
        let gh = GithubClient::new_from_env();
        // The server is unused, but needed for the context.
        let server = HttpServer::new(Vec::new());
        return GhTestCtx { gh, server };
    }

    let activities = crate::load_activities("tests/github_client", test_name);
    let server = HttpServer::new(activities);
    let gh = GithubClient::new(
        "sekrit-token".to_string(),
        format!("http://{}", server.addr),
        format!("http://{}/graphql", server.addr),
        format!("http://{}", server.addr),
    );
    GhTestCtx { gh, server }
}

/// The main entry point for a test.
///
/// Pass the name of the test as the first parameter.
fn run_test<F, Fut>(name: &str, f: F)
where
    F: Fn(GithubClient) -> Fut + Send + Sync,
    Fut: Future<Output = ()> + Send,
{
    let ctx = build(name);
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async { f(ctx.gh).await });
    assert_no_error(&ctx.server.hook_recv);
}

#[test]
fn repository() {
    run_test("repository", |gh| async move {
        let repo = gh.repository("rust-lang/rust").await.unwrap();
        assert_eq!(repo.full_name, "rust-lang/rust");
        assert_eq!(repo.default_branch, "master");
        assert_eq!(repo.fork, false);
        assert_eq!(repo.owner(), "rust-lang");
        assert_eq!(repo.name(), "rust");
    });
}

#[test]
fn is_new_contributor() {
    run_test("is_new_contributor", |gh| async move {
        let repo = gh.repository("rust-lang/rust").await.unwrap();
        assert_eq!(gh.is_new_contributor(&repo, "octocat").await, true);
        assert_eq!(gh.is_new_contributor(&repo, "brson").await, false);
    });
}

#[test]
fn bors_commits() {
    run_test("bors_commits", |gh| async move {
        let commits = gh.bors_commits().await;
        assert_eq!(commits.len(), 30);
        assert_eq!(commits[0].sha, "7f97aeaf73047268299ab55288b3dd886130be47");
        assert_eq!(
            commits[0].commit.author.date.to_string(),
            "2023-02-05 11:10:11 +00:00"
        );
        assert!(commits[0].commit.message.starts_with(
            "Auto merge of #107679 - est31:less_import_overhead, r=compiler-errors\n\n\
            Less import overhead for errors\n\n"
        ));
        assert_eq!(
            commits[0].commit.tree.sha,
            "28ef3869cb8034a8ab5e4ad389c139ec7dbd6df1"
        );
        assert_eq!(commits[0].parents.len(), 2);
        assert_eq!(
            commits[0].parents[0].sha,
            "2a6ff729233c62d1d991da5ed4d01aa29e59d637"
        );
        assert_eq!(
            commits[0].parents[1].sha,
            "580cc89e9c36a89d3cc13a352c96f874eaa76581"
        );
    });
}

#[test]
fn rust_commit() {
    run_test("rust_commit", |gh| async move {
        let commit = gh
            .rust_commit("7632db0e87d8adccc9a83a47795c9411b1455855")
            .await
            .unwrap();
        assert_eq!(commit.sha, "7632db0e87d8adccc9a83a47795c9411b1455855");
        assert_eq!(
            commit.commit.author.date.to_string(),
            "2022-12-08 07:46:42 +00:00"
        );
        assert_eq!(commit.commit.message, "Auto merge of #105415 - nikic:update-llvm-10, r=cuviper\n\nUpdate LLVM submodule\n\nThis is a rebase to LLVM 15.0.6.\n\nFixes #103380.\nFixes #104099.");
        assert_eq!(commit.parents.len(), 2);
        assert_eq!(
            commit.parents[0].sha,
            "f5418b09e84883c4de2e652a147ab9faff4eee29"
        );
        assert_eq!(
            commit.parents[1].sha,
            "530a687a4bb0bd0e8ab7b3f7d80f2c773be120ef"
        );
    });
}

#[test]
fn raw_file() {
    run_test("raw_file", |gh| async move {
        let contents =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789\n".repeat(1000);
        let body = gh
            .raw_file("ehuss/triagebot-test", "raw-file", "docs/example.txt")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(body, contents);
    });
}

#[test]
fn git_commit() {
    run_test("git_commit", |gh| async move {
        let repo = gh.repository("rust-lang/rust").await.unwrap();
        let commit = repo
            .git_commit(&gh, "109cccbe4f345c0f0785ce860788580c3e2a29f5")
            .await
            .unwrap();
        assert_eq!(commit.sha, "109cccbe4f345c0f0785ce860788580c3e2a29f5");
        assert_eq!(commit.author.date.to_string(), "2022-12-13 07:10:53 +00:00");
        assert_eq!(commit.message, "Auto merge of #105350 - compiler-errors:faster-binder-relate, r=oli-obk\n\nFast-path some binder relations\n\nA simpler approach than #104598\n\nFixes #104583\n\nr? types");
        assert_eq!(commit.tree.sha, "e67d87c61892169977204cc2e3fd89b2a19e13bb");
    });
}

#[test]
fn update_tree() {
    run_test("update_tree", |gh| async move {
        let repo = gh.repository("ehuss/rust").await.unwrap();
        let entries = vec![triagebot::github::GitTreeEntry {
            path: "src/doc/reference".to_string(),
            mode: "160000".to_string(),
            object_type: "commit".to_string(),
            sha: "b9ccb0960e5e98154020d4c02a09cc3901bc2500".to_string(),
        }];
        let tree = repo
            .update_tree(&gh, "6ebac807802fa2458d2f47c2c12fb1e62944e764", &entries)
            .await
            .unwrap();
        assert_eq!(tree.sha, "45aae523b087e418f2778d4557489de38fede6a3");
    });
}

#[test]
fn create_commit() {
    run_test("create_commit", |gh| async move {
        let repo = gh.repository("ehuss/rust").await.unwrap();
        let commit = repo
            .create_commit(
                &gh,
                "test reference commit",
                &["319b88c463fe6f51bb6badbbd3bb97252a60f3a5"],
                "45aae523b087e418f2778d4557489de38fede6a3",
            )
            .await
            .unwrap();
        assert_eq!(commit.sha, "88a426017fa4635ba42203c3b1d1c19f6a028184");
        assert_eq!(commit.author.date.to_string(), "2023-02-05 19:08:57 +00:00");
        assert_eq!(commit.message, "test reference commit");
        assert_eq!(commit.tree.sha, "45aae523b087e418f2778d4557489de38fede6a3");
    });
}

#[test]
fn get_reference() {
    run_test("get_reference", |gh| async move {
        let repo = gh.repository("rust-lang/rust").await.unwrap();
        let git_ref = repo.get_reference(&gh, "heads/stable").await.unwrap();
        assert_eq!(git_ref.refname, "refs/heads/stable");
        assert_eq!(git_ref.object.object_type, "commit");
        assert_eq!(
            git_ref.object.sha,
            "fc594f15669680fa70d255faec3ca3fb507c3405"
        );
        assert_eq!(git_ref.object.url, "https://api.github.com/repos/rust-lang/rust/git/commits/fc594f15669680fa70d255faec3ca3fb507c3405");
    });
}

#[test]
fn update_reference() {
    run_test("update_reference", |gh| async move {
        let repo = gh.repository("ehuss/rust").await.unwrap();
        repo.update_reference(
            &gh,
            "heads/docs-update",
            "88a426017fa4635ba42203c3b1d1c19f6a028184",
        )
        .await
        .unwrap();
    });
}

#[test]
fn submodule() {
    run_test("submodule", |gh| async move {
        let repo = gh.repository("rust-lang/rust").await.unwrap();
        let submodule = repo
            .submodule(&gh, "src/doc/reference", None)
            .await
            .unwrap();
        assert_eq!(submodule.name, "reference");
        assert_eq!(submodule.path, "src/doc/reference");
        assert_eq!(submodule.sha, "22882fb3f7b4d69fdc0d1731e8b9cfcb6910537d");
        assert_eq!(
            submodule.submodule_git_url,
            "https://github.com/rust-lang/reference.git"
        );
        let sub_repo = submodule.repository(&gh).await.unwrap();
        assert_eq!(sub_repo.full_name, "rust-lang/reference");
        assert_eq!(sub_repo.default_branch, "master");
        assert_eq!(sub_repo.fork, false);
    });
}

#[test]
fn new_pr() {
    run_test("new_pr", |gh| async move {
        let repo = gh.repository("ehuss/rust").await.unwrap();
        let issue = repo
            .new_pr(
                &gh,
                "example title",
                "ehuss:docs-update",
                "master",
                "example body text",
            )
            .await
            .unwrap();
        assert_eq!(issue.number, 7);
        assert_eq!(issue.body, "example body text");
        assert_eq!(issue.created_at.to_string(), "2023-02-05 19:20:58 UTC");
        assert_eq!(issue.updated_at.to_string(), "2023-02-05 19:20:58 UTC");
        assert_eq!(issue.merge_commit_sha, None);
        assert_eq!(issue.title, "example title");
        assert_eq!(issue.html_url, "https://github.com/ehuss/rust/pull/7");
        assert_eq!(issue.user.login, "ehuss");
        assert_eq!(issue.user.id, Some(43198));
        assert_eq!(issue.labels, vec![]);
        assert_eq!(issue.assignees, vec![]);
        assert!(matches!(
            issue.pull_request,
            Some(triagebot::github::PullRequestDetails {})
        ));
        assert_eq!(issue.merged, false);
        assert_eq!(issue.draft, false);
        assert_eq!(issue.base.as_ref().unwrap().git_ref, "master");
        assert_eq!(issue.base.as_ref().unwrap().repo.full_name, "ehuss/rust");
        assert_eq!(issue.head.unwrap().git_ref, "docs-update");
        assert_eq!(issue.state, triagebot::github::IssueState::Open);
    });
}

#[test]
fn merge_upstream() {
    run_test("merge_upstream", |gh| async move {
        let repo = gh.repository("ehuss/rust").await.unwrap();
        repo.merge_upstream(&gh, "docs-update").await.unwrap();
    });
}

#[test]
fn user() {
    run_test("user", |gh| async move {
        let user = triagebot::github::User::current(&gh).await.unwrap();
        assert_eq!(user.login, "ehuss");
        assert_eq!(user.id, Some(43198));
    });
}

#[test]
fn get_issues_no_search() {
    run_test("get_issues_no_search", |gh| async move {
        // get_issues where it doesn't use the search API
        let repo = gh.repository("rust-lang/rust").await.unwrap();
        let issues = repo
            .get_issues(
                &gh,
                &triagebot::github::Query {
                    filters: Vec::new(),
                    include_labels: vec!["A-coherence"],
                    exclude_labels: Vec::new(),
                },
            )
            .await
            .unwrap();
        assert_eq!(issues.len(), 3);
        assert_eq!(issues[0].number, 99554);
        assert_eq!(issues[1].number, 105782);
        assert_eq!(issues[2].number, 105787);
    });
}

#[test]
fn issue_properties() {
    run_test("issue_properties", |gh| async move {
        let repo = gh.repository("rust-lang/rust").await.unwrap();
        let issues = repo
            .get_issues(
                &gh,
                &triagebot::github::Query {
                    filters: Vec::new(),
                    include_labels: vec!["A-coherence"],
                    exclude_labels: Vec::new(),
                },
            )
            .await
            .unwrap();
        assert_eq!(issues.len(), 3);
        let issue = &issues[1];
        assert_eq!(issue.number, 105782);
        assert!(issue
            .body
            .starts_with("which is unsound during coherence, as coherence requires completeness"));
        assert_eq!(issue.created_at.to_string(), "2022-12-16 15:11:15 UTC");
        assert_eq!(issue.updated_at.to_string(), "2022-12-16 16:17:41 UTC");
        assert_eq!(issue.merge_commit_sha, None);
        assert_eq!(
            issue.title,
            "specialization: default items completely drop candidates instead of ambiguity"
        );
        assert_eq!(
            issue.html_url,
            "https://github.com/rust-lang/rust/issues/105782"
        );
        assert_eq!(issue.user.login, "lcnr");
        assert_eq!(issue.user.id, Some(29864074));
        let labels: Vec<_> = issue.labels.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            labels,
            &[
                "A-traits",
                "I-unsound",
                "A-specialization",
                "C-bug",
                "requires-nightly",
                "F-specialization",
                "A-coherence"
            ]
        );
        assert_eq!(issue.assignees, &[]);
        assert!(issue.pull_request.is_none());
        assert_eq!(issue.merged, false);
        assert_eq!(issue.draft, false);
        assert!(issue.base.is_none());
        assert!(issue.head.is_none());
        assert_eq!(issue.state, triagebot::github::IssueState::Open);

        let repo = issue.repository();
        assert_eq!(repo.organization, "rust-lang");
        assert_eq!(repo.repository, "rust");

        assert_eq!(issue.global_id(), "rust-lang/rust#105782");
        assert!(!issue.is_pr());
        assert!(issue.is_open());
        assert!(!issue.contain_assignee("octocat"));
        assert!(!issue.contain_assignee("atsuzaki"));
        assert!(!issue.contain_assignee("ATSUZAKI"));
    });
}

#[test]
fn get_issues_with_search() {
    // Tests `get_issues()` where it needs to use the search API.
    run_test("get_issues_with_search", |gh| async move {
        // get_issues where it doesn't use the search API
        let repo = gh.repository("rust-lang/rust").await.unwrap();
        let issues = repo
            .get_issues(
                &gh,
                &triagebot::github::Query {
                    filters: vec![("state", "closed"), ("is", "pull-request")],
                    include_labels: vec!["beta-nominated", "beta-accepted"],
                    exclude_labels: vec![],
                },
            )
            .await
            .unwrap();
        assert_eq!(issues.len(), 3);
        assert_eq!(issues[0].number, 107239);
        assert_eq!(issues[1].number, 107357);
        assert_eq!(issues[2].number, 107360);
        let pr = &issues[0];
        assert!(pr.is_pr());
        assert!(!pr.is_open());
        assert_eq!(pr.state, triagebot::github::IssueState::Closed);
        assert_eq!(pr.global_id(), "rust-lang/rust#107239");
        assert_eq!(pr.merge_commit_sha, None);
        assert_eq!(pr.title, "Bring tests back into rustc source tarball");
        assert_eq!(pr.html_url, "https://github.com/rust-lang/rust/pull/107239");
        assert!(matches!(
            pr.pull_request,
            Some(triagebot::github::PullRequestDetails {})
        ));
        assert_eq!(pr.merged, false);
        assert_eq!(pr.draft, false);
        // Presumably this is just information not available via the search
        // api? Or maybe since it is closed?
        assert!(pr.base.is_none());
        assert!(pr.head.is_none());
    });
}

#[test]
fn recent_commits() {
    run_test("recent_commits", |gh| async move {
        let repo = gh.repository("rust-lang/rust").await.unwrap();
        let recent = repo
            .recent_commits(
                &gh,
                "master",
                "afab3662eb066f05fcdb43c421b72dd19472e752",
                "e94bda3bf13303671427363d1cd93ac5e089f090",
            )
            .await
            .unwrap();
        assert_eq!(recent.len(), 4);
        assert_eq!(recent[0].pr_num, Some(111047));
        assert_eq!(
            recent[0].title,
            "Emit an error when return-type-notation is used with type/const params"
        );
        assert_eq!(recent[0].oid, "e94bda3bf13303671427363d1cd93ac5e089f090");
        assert_eq!(
            recent[0].committed_date.to_string(),
            "2023-06-07 09:03:33 UTC"
        );
        assert_eq!(recent[1].pr_num, Some(96875));
        assert_eq!(recent[2].pr_num, Some(111819));
        assert_eq!(recent[3].pr_num, Some(111495));
    });
}

#[test]
fn get_comment() {
    run_test("get_comment", |gh| async move {
        let repo = gh.repository("ehuss/triagebot-test").await.unwrap();
        let pr = repo.get_pr(&gh, 115).await.unwrap();
        let comment = pr.get_comment(&gh, 1581325459).await.unwrap();
        assert_eq!(comment.body, "This is a comment.");
        assert_eq!(
            comment.html_url,
            "https://github.com/ehuss/triagebot-test/pull/115#issuecomment-1581325459"
        );
        assert_eq!(comment.user.login, "ehuss");
        assert_eq!(comment.updated_at.to_string(), "2023-06-07 23:08:35 UTC");
        assert_eq!(comment.pr_review_state, None);
    });
}

#[test]
fn get_comments() {
    run_test("get_comments", |gh| async move {
        let repo = gh.repository("ehuss/triagebot-test").await.unwrap();
        let pr = repo.get_pr(&gh, 115).await.unwrap();
        let comments = pr.get_comments(&gh).await.unwrap();
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].body, "This is a comment.");
    });
}

#[test]
fn edit_body() {
    run_test("edit_body", |gh| async move {
        let repo = gh.repository("ehuss/triagebot-test").await.unwrap();
        let pr = repo.get_pr(&gh, 115).await.unwrap();
        assert_eq!(pr.body, "This is the old body.");
        pr.edit_body(&gh, "This is a new body.").await.unwrap();
        let pr = repo.get_pr(&gh, 115).await.unwrap();
        assert_eq!(pr.body, "This is a new body.");
    });
}

#[test]
fn edit_comment() {
    run_test("edit_comment", |gh| async move {
        let repo = gh.repository("ehuss/triagebot-test").await.unwrap();
        let pr = repo.get_pr(&gh, 115).await.unwrap();
        let comment = pr.get_comment(&gh, 1581325459).await.unwrap();
        assert_eq!(comment.body, "This is a comment.");
        pr.edit_comment(&gh, 1581325459, "This is a new comment.")
            .await
            .unwrap();
        let comment = pr.get_comment(&gh, 1581325459).await.unwrap();
        assert_eq!(comment.body, "This is a new comment.");
    });
}

#[test]
fn post_comment() {
    run_test("post_comment", |gh| async move {
        let repo = gh.repository("ehuss/triagebot-test").await.unwrap();
        let pr = repo.get_pr(&gh, 115).await.unwrap();
        pr.post_comment(&gh, "Posting a new comment on this PR.")
            .await
            .unwrap();
        let mut comments = pr.get_comments(&gh).await.unwrap();
        let comment = comments.pop().unwrap();
        assert_eq!(comment.body, "Posting a new comment on this PR.");
    });
}

#[test]
fn add_labels() {
    use triagebot::github::Label;
    run_test("add_labels", |gh| async move {
        let repo = gh.repository("ehuss/triagebot-test").await.unwrap();
        let pr = repo.get_pr(&gh, 115).await.unwrap();
        assert_eq!(pr.labels.len(), 0);
        pr.add_labels(
            &gh,
            vec![Label {
                name: "enhancement".to_string(),
            }],
        )
        .await
        .unwrap();
        let pr = repo.get_pr(&gh, 115).await.unwrap();
        assert_eq!(pr.labels.len(), 1);
        assert_eq!(pr.labels[0].name, "enhancement");

        // Try adding an unknown label.
        match pr
            .add_labels(
                &gh,
                vec![Label {
                    name: "does-not-exist".to_string(),
                }],
            )
            .await
        {
            Ok(_) => panic!("did not expect success"),
            Err(e) => {
                assert_eq!(e.to_string(), "Unknown labels: does-not-exist");
            }
        }
    });
}

#[test]
fn remove_label() {
    run_test("remove_label", |gh| async move {
        let repo = gh.repository("ehuss/triagebot-test").await.unwrap();
        let pr = repo.get_pr(&gh, 115).await.unwrap();
        assert_eq!(pr.labels.len(), 1);
        pr.remove_label(&gh, "enhancement").await.unwrap();
        let pr = repo.get_pr(&gh, 115).await.unwrap();
        assert_eq!(pr.labels.len(), 0);

        // Unknown labels are ignored.
        pr.remove_label(&gh, "does-not-exist").await.unwrap();
    });
}

#[test]
fn add_assignee() {
    run_test("add_assignee", |gh| async move {
        let repo = gh.repository("ehuss/triagebot-test").await.unwrap();
        let pr = repo.get_pr(&gh, 115).await.unwrap();
        assert_eq!(pr.assignees.len(), 0);
        pr.add_assignee(&gh, "ehuss").await.unwrap();
        let pr = repo.get_pr(&gh, 115).await.unwrap();
        assert_eq!(pr.assignees.len(), 1);
        assert_eq!(pr.assignees[0].login, "ehuss");

        // Add a second user should keep old.
        pr.add_assignee(&gh, "grashgal").await.unwrap();
        let pr = repo.get_pr(&gh, 115).await.unwrap();
        assert_eq!(pr.assignees.len(), 2);
        assert_eq!(pr.assignees[0].login, "ehuss");
        assert_eq!(pr.assignees[1].login, "grashgal");

        // Add a user already assigned should have no effect.
        pr.add_assignee(&gh, "ehuss").await.unwrap();
        let pr = repo.get_pr(&gh, 115).await.unwrap();
        assert_eq!(pr.assignees.len(), 2);
        assert_eq!(pr.assignees[0].login, "ehuss");
        assert_eq!(pr.assignees[1].login, "grashgal");

        // Add nonexistent user.
        match pr.add_assignee(&gh, "ghost").await {
            Ok(_) => panic!("did not expect success"),
            Err(triagebot::github::AssignmentError::InvalidAssignee) => {}
            Err(e) => panic!("unexpected error {e:?}"),
        }
    });
}

#[test]
fn set_assignee() {
    run_test("set_assignee", |gh| async move {
        let repo = gh.repository("ehuss/triagebot-test").await.unwrap();
        let pr = repo.get_pr(&gh, 115).await.unwrap();
        assert_eq!(pr.assignees.len(), 0);
        pr.set_assignee(&gh, "ehuss").await.unwrap();
        let pr = repo.get_pr(&gh, 115).await.unwrap();
        assert_eq!(pr.assignees.len(), 1);
        assert_eq!(pr.assignees[0].login, "ehuss");

        // Change assignee
        pr.set_assignee(&gh, "grashgal").await.unwrap();
        let pr = repo.get_pr(&gh, 115).await.unwrap();
        assert_eq!(pr.assignees.len(), 1);
        assert_eq!(pr.assignees[0].login, "grashgal");

        // Nonexistent user should fail.
        match pr.add_assignee(&gh, "ghost").await {
            Ok(_) => panic!("did not expect success"),
            Err(triagebot::github::AssignmentError::InvalidAssignee) => {}
            Err(e) => panic!("unexpected error {e:?}"),
        }
    });
}

#[test]
fn remove_assignees() {
    use triagebot::github::Selection;
    run_test("remove_assignees", |gh| async move {
        let repo = gh.repository("ehuss/triagebot-test").await.unwrap();
        let pr = repo.get_pr(&gh, 115).await.unwrap();
        assert_eq!(pr.assignees.len(), 0);
        // Shouldn't care if no assignees.
        pr.remove_assignees(&gh, Selection::All).await.unwrap();

        // Remove all
        pr.add_assignee(&gh, "ehuss").await.unwrap();
        pr.add_assignee(&gh, "grashgal").await.unwrap();
        let pr = repo.get_pr(&gh, 115).await.unwrap();
        assert_eq!(pr.assignees.len(), 2);
        pr.remove_assignees(&gh, Selection::All).await.unwrap();
        let pr = repo.get_pr(&gh, 115).await.unwrap();
        assert_eq!(pr.assignees.len(), 0);

        // Remove one
        pr.add_assignee(&gh, "ehuss").await.unwrap();
        pr.add_assignee(&gh, "grashgal").await.unwrap();
        let pr = repo.get_pr(&gh, 115).await.unwrap();
        assert_eq!(pr.assignees.len(), 2);
        pr.remove_assignees(&gh, Selection::One("grashgal"))
            .await
            .unwrap();
        let pr = repo.get_pr(&gh, 115).await.unwrap();
        assert_eq!(pr.assignees.len(), 1);
        assert_eq!(pr.assignees[0].login, "ehuss");

        // Remove all except
        pr.add_assignee(&gh, "grashgal").await.unwrap();
        // Case shouldn't matter.
        pr.remove_assignees(&gh, Selection::Except("GRASHGAL"))
            .await
            .unwrap();
        let pr = repo.get_pr(&gh, 115).await.unwrap();
        assert_eq!(pr.assignees.len(), 1);
        assert_eq!(pr.assignees[0].login, "grashgal");

        // Remove all except isn't assigned
        pr.remove_assignees(&gh, Selection::Except("octocat"))
            .await
            .unwrap();
        let pr = repo.get_pr(&gh, 115).await.unwrap();
        assert_eq!(pr.assignees.len(), 0);
    });
}

#[test]
fn set_milestone() {
    run_test("set_milestone", |gh| async move {
        let repo = gh.repository("ehuss/triagebot-test").await.unwrap();
        let pr = repo.get_pr(&gh, 115).await.unwrap();
        assert!(pr.milestone.is_none());
        // Set to an existing milestone.
        pr.set_milestone(&gh, "test").await.unwrap();
        let pr = repo.get_pr(&gh, 115).await.unwrap();
        assert_eq!(pr.milestone.as_ref().unwrap().title, "test");

        // Set to a milestone that does not exist.
        pr.set_milestone(&gh, "new milestone").await.unwrap();
        let pr = repo.get_pr(&gh, 115).await.unwrap();
        assert_eq!(pr.milestone.as_ref().unwrap().title, "new milestone");
    });
}

#[test]
fn diff() {
    run_test("diff", |gh| async move {
        let repo = gh.repository("ehuss/triagebot-test").await.unwrap();
        let pr = repo.get_pr(&gh, 115).await.unwrap();
        let diff = pr.diff(&gh).await.unwrap();
        assert_eq!(
            diff.unwrap(),
            "diff --git a/README.md b/README.md\n\
            index 8533f6d..ce54f76 100644\n\
            --- a/README.md\n\
            +++ b/README.md\n\
            @@ -1,2 +1,4 @@\n \
             # triagebot-test\n \
             Triagebot testing\n\
             +\n\
            +test\n\
            "
        );
    });
}

#[test]
fn commits() {
    run_test("commits", |gh| async move {
        let repo = gh.repository("ehuss/triagebot-test").await.unwrap();
        let pr = repo.get_pr(&gh, 115).await.unwrap();
        let commits = pr.commits(&gh).await.unwrap();
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].sha, "f39d908ac31a6bccc4a0bba0497d3626756dacd7");
        assert_eq!(
            commits[0].commit.author.date.to_string(),
            "2023-06-06 00:33:20 +00:00"
        );
        assert_eq!(commits[0].commit.message, "test");
        assert_eq!(
            commits[0].commit.tree.sha,
            "4645075b7c02de65a59592f9d124b57f3c4160a7"
        );
        assert_eq!(commits[0].parents.len(), 1);
        assert_eq!(
            commits[0].parents[0].sha,
            "9a00ee2312071126101bb900f363210d6f3d1402"
        );
    });
}

#[test]
fn files() {
    run_test("files", |gh| async move {
        let repo = gh.repository("ehuss/triagebot-test").await.unwrap();
        let pr = repo.get_pr(&gh, 115).await.unwrap();
        let files = pr.files(&gh).await.unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].sha, "ce54f760cbea22cc8e2ba87f8e1d420348d051eb");
        assert_eq!(files[0].filename, "README.md");
        assert_eq!(files[0].blob_url, "https://github.com/ehuss/triagebot-test/blob/f39d908ac31a6bccc4a0bba0497d3626756dacd7/README.md");
    });
}

#[test]
fn close() {
    run_test("close", |gh| async move {
        let repo = gh.repository("ehuss/triagebot-test").await.unwrap();
        let pr = repo.get_pr(&gh, 115).await.unwrap();
        assert_eq!(pr.state, triagebot::github::IssueState::Open);
        pr.close(&gh).await.unwrap();

        let pr = repo.get_pr(&gh, pr.number).await.unwrap();
        assert_eq!(pr.state, triagebot::github::IssueState::Closed);
    });
}
