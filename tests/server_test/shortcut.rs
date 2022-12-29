use super::{Method::*, Response, TestBuilder};

#[test]
fn author() {
    let ctx = TestBuilder::new()
        .config("[shortcut]")
        .api_handler(GET, "repos/rust-lang/rust/issues/103952/labels", |_req| {
            Response::new_from_path("tests/server_test/shortcut_author_get_labels.json")
        })
        .api_handler(
            GET,
            "repos/rust-lang/rust/labels/S-waiting-on-author",
            |_req| Response::new_from_path("tests/server_test/labels_S-waiting-on-author.json"),
        )
        .api_handler(
            DELETE,
            "repos/rust-lang/rust/issues/103952/labels/S-waiting-on-review",
            |_req| Response::new().body(b"[]"),
        )
        .api_handler(POST, "repos/rust-lang/rust/issues/103952/labels", |req| {
            assert_eq!(req.body_str(), r#"{"labels":["S-waiting-on-author"]}"#);
            Response::new_from_path("tests/server_test/shortcut_author_labels_response.json")
        })
        .build();
    ctx.send_webook(include_bytes!("shortcut_author_comment.json"));
    ctx.events.assert_eq(&[
        (GET, "/rust-lang/rust/master/triagebot.toml"),
        (
            DELETE,
            "/repos/rust-lang/rust/issues/103952/labels/S-waiting-on-review",
        ),
        (GET, "/repos/rust-lang/rust/labels/S-waiting-on-author"),
        (POST, "/repos/rust-lang/rust/issues/103952/labels"),
    ]);
}

#[test]
fn ready() {
    let ctx = TestBuilder::new()
        .config("[shortcut]")
        .api_handler(GET, "repos/rust-lang/rust/issues/103952/labels", |_req| {
            Response::new_from_path("tests/server_test/shortcut_ready_get_labels.json")
        })
        .api_handler(
            GET,
            "repos/rust-lang/rust/labels/S-waiting-on-review",
            |_req| Response::new_from_path("tests/server_test/labels_S-waiting-on-review.json"),
        )
        .api_handler(
            DELETE,
            "repos/rust-lang/rust/issues/103952/labels/S-waiting-on-author",
            |_req| Response::new().body(b"[]"),
        )
        .api_handler(POST, "repos/rust-lang/rust/issues/103952/labels", |req| {
            assert_eq!(req.body_str(), r#"{"labels":["S-waiting-on-review"]}"#);
            Response::new_from_path("tests/server_test/shortcut_ready_labels_response.json")
        })
        .build();
    ctx.send_webook(include_bytes!("shortcut_ready_comment.json"));
    ctx.events.assert_eq(&[
        (GET, "/rust-lang/rust/master/triagebot.toml"),
        (
            DELETE,
            "/repos/rust-lang/rust/issues/103952/labels/S-waiting-on-author",
        ),
        (GET, "/repos/rust-lang/rust/labels/S-waiting-on-review"),
        (POST, "/repos/rust-lang/rust/issues/103952/labels"),
    ]);
}

#[test]
fn blocked() {
    let ctx = TestBuilder::new()
        .config("[shortcut]")
        .api_handler(GET, "repos/rust-lang/rust/issues/103952/labels", |_req| {
            Response::new_from_path("tests/server_test/shortcut_author_get_labels.json")
        })
        .api_handler(GET, "repos/rust-lang/rust/labels/S-blocked", |_req| {
            Response::new_from_path("tests/server_test/labels_S-blocked.json")
        })
        .api_handler(
            DELETE,
            "repos/rust-lang/rust/issues/103952/labels/S-waiting-on-review",
            |_req| Response::new().body(b"[]"),
        )
        .api_handler(POST, "repos/rust-lang/rust/issues/103952/labels", |req| {
            assert_eq!(req.body_str(), r#"{"labels":["S-blocked"]}"#);
            Response::new_from_path("tests/server_test/shortcut_blocked_labels_response.json")
        })
        .build();
    ctx.send_webook(include_bytes!("shortcut_blocked_comment.json"));
    ctx.events.assert_eq(&[
        (GET, "/rust-lang/rust/master/triagebot.toml"),
        (
            DELETE,
            "/repos/rust-lang/rust/issues/103952/labels/S-waiting-on-review",
        ),
        (GET, "/repos/rust-lang/rust/labels/S-blocked"),
        (POST, "/repos/rust-lang/rust/issues/103952/labels"),
    ]);
}
