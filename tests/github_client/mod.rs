//! `GithubClient` tests.
//!
//! These tests exercise the behavior of `GithubClient`. They involve setting
//! up HTTP servers, creating a `GithubClient` to connect to those servers,
//! executing some action, and validating the result.
//!
//! The [`TestBuilder`] is used for configuring the test, and producing a
//! [`GhTestCtx`], which provides access to the HTTP servers.
//!
//! For issuing requests, you'll need to define the server-side behavior. This
//! is usually done by calling [`TestBuilder::api_handler`] which adds a route
//! handler for an API request. The handler should validate its input, and
//! usually return a JSON response (using a [`Response`] object).
//!
//! To get the proper contents for the JSON response, I recommend using the
//! [`gh api`](https://cli.github.com/) command, and save the output to a
//! file. For example:
//!
//! ```sh
//! gh api repos/rust-lang/rust > repository_rust.json
//! ```
//!
//! Since some API commands mutate state, I recommend running them against a
//! repository in your user account. It can be your fork of the `rust` repo,
//! or any other repo. For example, to get the response of deleting a label:
//!
//! ```sh
//! gh api repos/ehuss/triagebot-test/issues/labels/bar -X DELETE
//! ```
//!
//! JSON properties can be passed with the `-f` or `-F` flags. This example
//! creates a low-level tag object in the git database:
//!
//! ```sh
//! gh api repos/ehuss/triagebot-test/git/tags -X POST \
//!    -F tag="v0.0.1" -F message="my tag" \
//!    -F object="bc1db30cf2a3fbac1dfb964e39881e6d47475e11" -F type="commit"
//! ```
//!
//! Beware that `-f` currently doesn't handle arrays properly. In those cases,
//! you'll need to write the JSON manually and pipe it into the command. This
//! example adds a label to an issue:
//!
//! ```sh
//! echo '{"labels": ["S-waiting-on-author"]}' | gh api repos/rust-lang/rust/issues/104171/labels --input -
//! ```
//!
//! Check out the help page for `gh api` for more information.
//!
//! If you are saving output for a repository other than `rust-lang/rust`, you
//! can leave it as-is, or you can edit the JSON to change the repository name
//! to `rust-lang/rust`. It will depend if the function you are calling cares
//! about that or not.

use super::common::{Events, HttpServer, HttpServerHandle, Method::*, Response, TestBuilder};
use std::fs;
use triagebot::github::GithubClient;

/// A context used for running a test.
///
/// This provides access to performing the test actions.
struct GhTestCtx {
    gh: GithubClient,
    #[allow(dead_code)] // held for drop
    api_server: HttpServerHandle,
    #[allow(dead_code)] // held for drop
    raw_server: HttpServerHandle,
}

impl TestBuilder {
    fn new_gh() -> TestBuilder {
        let tb = TestBuilder::default();
        // Many of the tests need a repo.
        tb.api_handler(GET, "repos/rust-lang/rust", |_req| {
            Response::new().body(include_bytes!("repository_rust.json"))
        })
    }

    fn build_gh(self) -> GhTestCtx {
        self.maybe_enable_logging();
        let events = Events::new();
        let api_server = HttpServer::new(self.api_handlers, events.clone());
        let raw_server = HttpServer::new(self.raw_handlers, events.clone());
        let gh = GithubClient::new(
            "sekrit-token".to_string(),
            format!("http://{}", api_server.addr),
            format!("http://{}/graphql", api_server.addr),
            format!("http://{}", raw_server.addr),
        );
        GhTestCtx {
            gh,
            api_server,
            raw_server,
        }
    }
}

#[tokio::test]
async fn repository() {
    let ctx = TestBuilder::new_gh().build_gh();
    let repo = ctx.gh.repository("rust-lang/rust").await.unwrap();
    assert_eq!(repo.full_name, "rust-lang/rust");
    assert_eq!(repo.default_branch, "master");
    assert_eq!(repo.fork, false);
    assert_eq!(repo.owner(), "rust-lang");
    assert_eq!(repo.name(), "rust");
}

#[tokio::test]
async fn is_new_contributor() {
    let ctx = TestBuilder::new_gh()
        .api_handler(GET, "repos/rust-lang/rust/commits", |req| {
            let author = req
                .query
                .iter()
                .find(|(k, _v)| k == "author")
                .map(|(_k, v)| v)
                .unwrap();
            let body = fs::read(format!(
                "tests/github_client/is_new_contributor_{author}.json"
            ))
            .unwrap();
            Response::new().body(&body)
        })
        .build_gh();

    let repo = ctx.gh.repository("rust-lang/rust").await.unwrap();
    assert_eq!(ctx.gh.is_new_contributor(&repo, "octocat").await, true);
    assert_eq!(ctx.gh.is_new_contributor(&repo, "brson").await, false);
}

#[tokio::test]
async fn bors_commits() {
    let ctx = TestBuilder::new_gh()
        .api_handler(GET, "repos/rust-lang/rust/commits", |req| {
            assert_eq!(req.query_string(), "author=bors");
            Response::new().body(include_bytes!("commits_bors.json"))
        })
        .build_gh();
    let commits = ctx.gh.bors_commits().await;
    assert_eq!(commits.len(), 30);
    assert_eq!(commits[0].sha, "37d7de337903a558dbeb1e82c844fe915ab8ff25");
    assert_eq!(
        commits[0].commit.author.date.to_string(),
        "2022-12-12 10:38:31 +00:00"
    );
    assert!(commits[0].commit.message.starts_with("Auto merge of #105252 - bjorn3:codegen_less_pair_values, r=nagisa\n\nUse struct types during codegen in less places\n"));
    assert_eq!(
        commits[0].commit.tree.sha,
        "d4919a64af3b34d516f096975fb26454240aeaa5"
    );
    assert_eq!(commits[0].parents.len(), 2);
    assert_eq!(
        commits[0].parents[0].sha,
        "2176e3a7a4a8dfbea92f3104244fbf8fad4faf9a"
    );
    assert_eq!(
        commits[0].parents[1].sha,
        "262ace528425e6e22ccc0a5afd6321a566ab18d7"
    );
}

#[tokio::test]
async fn rust_commit() {
    let ctx = TestBuilder::new_gh()
        .api_handler(GET, "repos/rust-lang/rust/commits/{sha}", |req| {
            let sha = &req.components["sha"];
            let body = fs::read(format!("tests/github_client/commits_{sha}.json")).unwrap();
            Response::new().body(&body)
        })
        .build_gh();
    let commit = ctx
        .gh
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
}

#[tokio::test]
async fn raw_file() {
    let contents = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789".repeat(1000);
    let send_contents = contents.clone();
    let ctx = TestBuilder::new_gh()
        .raw_handler(GET, "rust-lang/rust/master/example.txt", move |_req| {
            Response::new().body(&send_contents)
        })
        .build_gh();
    let body = ctx
        .gh
        .raw_file("rust-lang/rust", "master", "example.txt")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(body, contents);
}

#[tokio::test]
async fn git_commit() {
    let ctx = TestBuilder::new_gh()
        .api_handler(GET, "repos/rust-lang/rust/git/commits/{sha}", |req| {
            let sha = &req.components["sha"];
            let body = fs::read(format!("tests/github_client/git_commits_{sha}.json")).unwrap();
            Response::new().body(&body)
        })
        .build_gh();
    let repo = ctx.gh.repository("rust-lang/rust").await.unwrap();
    let commit = repo
        .git_commit(&ctx.gh, "109cccbe4f345c0f0785ce860788580c3e2a29f5")
        .await
        .unwrap();
    assert_eq!(commit.sha, "109cccbe4f345c0f0785ce860788580c3e2a29f5");
    assert_eq!(commit.author.date.to_string(), "2022-12-13 07:10:53 +00:00");
    assert_eq!(commit.message, "Auto merge of #105350 - compiler-errors:faster-binder-relate, r=oli-obk\n\nFast-path some binder relations\n\nA simpler approach than #104598\n\nFixes #104583\n\nr? types");
    assert_eq!(commit.tree.sha, "e67d87c61892169977204cc2e3fd89b2a19e13bb");
}

#[tokio::test]
async fn create_commit() {
    let ctx = TestBuilder::new_gh()
        .api_handler(POST, "repos/rust-lang/rust/git/commits", |req| {
            let data = req.json();
            assert_eq!(data["message"].as_str().unwrap(), "test reference commit");
            let parents = data["parents"].as_array().unwrap();
            assert_eq!(parents.len(), 1);
            assert_eq!(
                parents[0].as_str().unwrap(),
                "b7bc90fea3b441234a84b49fdafeb75815eebbab"
            );
            assert_eq!(data["tree"], "bef1883908d15f4c900cd8229c9331bacade900a");
            Response::new().body(include_bytes!("git_commits_post.json"))
        })
        .build_gh();
    let repo = ctx.gh.repository("rust-lang/rust").await.unwrap();
    let commit = repo
        .create_commit(
            &ctx.gh,
            "test reference commit",
            &["b7bc90fea3b441234a84b49fdafeb75815eebbab"],
            "bef1883908d15f4c900cd8229c9331bacade900a",
        )
        .await
        .unwrap();
    assert_eq!(commit.sha, "525144116ef5d2f324a677c4e918246d52f842b0");
    assert_eq!(commit.author.date.to_string(), "2022-12-13 15:11:44 +00:00");
    assert_eq!(commit.message, "test reference commit");
    assert_eq!(commit.tree.sha, "bef1883908d15f4c900cd8229c9331bacade900a");
}

#[tokio::test]
async fn get_reference() {
    let ctx = TestBuilder::new_gh()
        .api_handler(GET, "repos/rust-lang/rust/git/ref/heads/stable", |_req| {
            Response::new().body(include_bytes!("get_reference.json"))
        })
        .build_gh();
    let repo = ctx.gh.repository("rust-lang/rust").await.unwrap();
    let git_ref = repo.get_reference(&ctx.gh, "heads/stable").await.unwrap();
    assert_eq!(git_ref.refname, "refs/heads/stable");
    assert_eq!(git_ref.object.object_type, "commit");
    assert_eq!(
        git_ref.object.sha,
        "69f9c33d71c871fc16ac445211281c6e7a340943"
    );
    assert_eq!(git_ref.object.url, "https://api.github.com/repos/rust-lang/rust/git/commits/69f9c33d71c871fc16ac445211281c6e7a340943");
}

#[tokio::test]
async fn update_reference() {
    let ctx = TestBuilder::new_gh()
        .api_handler(
            PATCH,
            "repos/rust-lang/rust/git/refs/heads/docs-update",
            |req| {
                let data = req.json();
                assert_eq!(
                    data["sha"].as_str().unwrap(),
                    "b7bc90fea3b441234a84b49fdafeb75815eebbab"
                );
                assert_eq!(data["force"].as_bool().unwrap(), true);
                Response::new().body(include_bytes!("update_reference_patch.json"))
            },
        )
        .build_gh();
    let repo = ctx.gh.repository("rust-lang/rust").await.unwrap();
    repo.update_reference(
        &ctx.gh,
        "heads/docs-update",
        "b7bc90fea3b441234a84b49fdafeb75815eebbab",
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn update_tree() {
    let ctx = TestBuilder::new_gh()
        .api_handler(POST, "repos/rust-lang/rust/git/trees", |req| {
            let data = req.json();
            assert_eq!(
                data["base_tree"].as_str().unwrap(),
                "aba475763a52ff6fcad1c617234288ac9880b8e3"
            );
            let tree = data["tree"].as_array().unwrap();
            assert_eq!(tree.len(), 1);
            assert_eq!(tree[0]["path"].as_str().unwrap(), "src/doc/reference");
            Response::new().body(include_bytes!("update_tree_post.json"))
        })
        .build_gh();
    let repo = ctx.gh.repository("rust-lang/rust").await.unwrap();
    let entries = vec![triagebot::github::GitTreeEntry {
        path: "src/doc/reference".to_string(),
        mode: "160000".to_string(),
        object_type: "commit".to_string(),
        sha: "6a5431b863f61b86dcac70ee2ab377152f40f66e".to_string(),
    }];
    let tree = repo
        .update_tree(
            &ctx.gh,
            "aba475763a52ff6fcad1c617234288ac9880b8e3",
            &entries,
        )
        .await
        .unwrap();
    assert_eq!(tree.sha, "bef1883908d15f4c900cd8229c9331bacade900a");
}

#[tokio::test]
async fn submodule() {
    let ctx = TestBuilder::new_gh()
        .api_handler(GET, "repos/rust-lang/reference", |_req| {
            Response::new().body(include_bytes!("repository_reference.json"))
        })
        .api_handler(
            GET,
            "repos/rust-lang/rust/contents/src/doc/reference",
            |_req| Response::new().body(include_bytes!("submodule_get.json")),
        )
        .build_gh();
    let repo = ctx.gh.repository("rust-lang/rust").await.unwrap();
    let submodule = repo
        .submodule(&ctx.gh, "src/doc/reference", None)
        .await
        .unwrap();
    assert_eq!(submodule.name, "reference");
    assert_eq!(submodule.path, "src/doc/reference");
    assert_eq!(submodule.sha, "9f0cc13ffcd27c1fbe1ab766a9491e15ddcf4d19");
    assert_eq!(
        submodule.submodule_git_url,
        "https://github.com/rust-lang/reference.git"
    );
    let sub_repo = submodule.repository(&ctx.gh).await.unwrap();
    assert_eq!(sub_repo.full_name, "rust-lang/reference");
    assert_eq!(sub_repo.default_branch, "master");
    assert_eq!(sub_repo.fork, false);
}

#[tokio::test]
async fn new_pr() {
    let ctx = TestBuilder::new_gh()
        .api_handler(POST, "repos/rust-lang/rust/pulls", |req| {
            let data = req.json();
            assert_eq!(data["title"].as_str().unwrap(), "example title");
            assert_eq!(data["head"].as_str().unwrap(), "ehuss:docs-update");
            assert_eq!(data["base"].as_str().unwrap(), "master");
            assert_eq!(data["body"].as_str().unwrap(), "example body text");
            Response::new().body(include_bytes!("new_pr.json"))
        })
        .build_gh();
    let repo = ctx.gh.repository("rust-lang/rust").await.unwrap();
    let issue = repo
        .new_pr(
            &ctx.gh,
            "example title",
            "ehuss:docs-update",
            "master",
            "example body text",
        )
        .await
        .unwrap();
    assert_eq!(issue.number, 6);
    assert_eq!(issue.body, "example body text");
    assert_eq!(issue.created_at.to_string(), "2022-12-14 03:05:59 UTC");
    assert_eq!(issue.updated_at.to_string(), "2022-12-14 03:05:59 UTC");
    assert_eq!(issue.merge_commit_sha, None);
    assert_eq!(issue.title, "example title");
    assert_eq!(issue.html_url, "https://github.com/rust-lang/rust/pull/6");
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
    assert_eq!(
        issue.base.as_ref().unwrap().repo.full_name,
        "rust-lang/rust"
    );
    assert_eq!(issue.head.unwrap().git_ref, "docs-update");
    assert_eq!(issue.state, triagebot::github::IssueState::Open);
}

#[tokio::test]
async fn merge_upstream() {
    let ctx = TestBuilder::new_gh()
        .api_handler(POST, "repos/rust-lang/rust/merge-upstream", |req| {
            let data = req.json();
            assert_eq!(data["branch"].as_str().unwrap(), "docs-update");
            Response::new().body(include_bytes!("merge_upstream.json"))
        })
        .build_gh();
    let repo = ctx.gh.repository("rust-lang/rust").await.unwrap();
    repo.merge_upstream(&ctx.gh, "docs-update").await.unwrap();
}

#[tokio::test]
async fn user() {
    let ctx = TestBuilder::new_gh()
        .api_handler(GET, "user", |_req| {
            Response::new().body(include_bytes!("user.json"))
        })
        .build_gh();
    let user = triagebot::github::User::current(&ctx.gh).await.unwrap();
    assert_eq!(user.login, "ehuss");
    assert_eq!(user.id, Some(43198));
}

#[tokio::test]
async fn get_issues_no_search() {
    // get_issues where it doesn't use the search API
    let ctx = TestBuilder::new_gh()
        .api_handler(GET, "repos/rust-lang/rust/issues", |req| {
            assert_eq!(
                req.query_string(),
                "labels=A-coherence&filter=all&sort=created&direction=asc&per_page=100"
            );
            Response::new().body(include_bytes!("issues.json"))
        })
        .build_gh();

    let repo = ctx.gh.repository("rust-lang/rust").await.unwrap();
    let issues = repo
        .get_issues(
            &ctx.gh,
            &triagebot::github::Query {
                filters: Vec::new(),
                include_labels: vec!["A-coherence"],
                exclude_labels: Vec::new(),
            },
        )
        .await
        .unwrap();
    assert_eq!(issues.len(), 2);
    assert_eq!(issues[0].number, 105782);
    assert_eq!(issues[1].number, 105787);
}

#[tokio::test]
async fn issue_properties() {
    let ctx = TestBuilder::new_gh()
        .api_handler(GET, "repos/rust-lang/rust/issues", |_req| {
            Response::new().body(include_bytes!("issues.json"))
        })
        .build_gh();

    let repo = ctx.gh.repository("rust-lang/rust").await.unwrap();
    let issues = repo
        .get_issues(
            &ctx.gh,
            &triagebot::github::Query {
                filters: Vec::new(),
                include_labels: vec!["A-coherence"],
                exclude_labels: Vec::new(),
            },
        )
        .await
        .unwrap();
    assert_eq!(issues.len(), 2);
    let issue = &issues[0];
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
}
