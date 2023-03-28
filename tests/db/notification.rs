use super::run_test;
use crate::assert_datetime_approx_equal;
use std::num::NonZeroUsize;
use triagebot::db::notifications::{Identifier, Notification};

#[test]
fn notification() {
    run_test(|mut connection| async move {
        let now = chrono::Utc::now();
        connection
            .record_username(43198, "ehuss".to_string())
            .await
            .unwrap();
        connection
            .record_username(14314532, "weihanglo".to_string())
            .await
            .unwrap();
        connection
            .record_ping(&Notification {
                user_id: 43198,
                origin_url: "https://github.com/rust-lang/rust/issues/1".to_string(),
                origin_html: "This comment mentions @ehuss.".to_string(),
                short_description: Some("Comment on some issue".to_string()),
                time: now.into(),
                team_name: None,
            })
            .await
            .unwrap();

        connection
            .record_ping(&Notification {
                user_id: 43198,
                origin_url: "https://github.com/rust-lang/rust/issues/2".to_string(),
                origin_html: "This comment mentions @rust-lang/cargo.".to_string(),
                short_description: Some("Comment on some issue".to_string()),
                time: now.into(),
                team_name: Some("cargo".to_string()),
            })
            .await
            .unwrap();
        connection
            .record_ping(&Notification {
                user_id: 14314532,
                origin_url: "https://github.com/rust-lang/rust/issues/2".to_string(),
                origin_html: "This comment mentions @rust-lang/cargo.".to_string(),
                short_description: Some("Comment on some issue".to_string()),
                time: now.into(),
                team_name: Some("cargo".to_string()),
            })
            .await
            .unwrap();

        let notifications = connection.get_notifications("ehuss").await.unwrap();
        assert_eq!(notifications.len(), 2);
        assert_eq!(
            notifications[0].origin_url,
            "https://github.com/rust-lang/rust/issues/1"
        );
        assert_eq!(
            notifications[0].origin_text,
            "This comment mentions @ehuss."
        );
        assert_eq!(
            notifications[0].short_description.as_deref(),
            Some("Comment on some issue")
        );
        assert_datetime_approx_equal(&notifications[0].time, &now);
        assert_eq!(notifications[0].metadata, None);

        assert_eq!(
            notifications[1].origin_url,
            "https://github.com/rust-lang/rust/issues/2"
        );
        assert_eq!(
            notifications[1].origin_text,
            "This comment mentions @rust-lang/cargo."
        );
        assert_eq!(
            notifications[1].short_description.as_deref(),
            Some("Comment on some issue")
        );
        assert_datetime_approx_equal(&notifications[1].time, &now);
        assert_eq!(notifications[1].metadata, None);

        let notifications = connection.get_notifications("weihanglo").await.unwrap();
        assert_eq!(notifications.len(), 1);
        assert_eq!(
            notifications[0].origin_url,
            "https://github.com/rust-lang/rust/issues/2"
        );
        assert_eq!(
            notifications[0].origin_text,
            "This comment mentions @rust-lang/cargo."
        );
        assert_eq!(
            notifications[0].short_description.as_deref(),
            Some("Comment on some issue")
        );
        assert_datetime_approx_equal(&notifications[0].time, &now);
        assert_eq!(notifications[0].metadata, None);

        let notifications = connection.get_notifications("octocat").await.unwrap();
        assert_eq!(notifications.len(), 0);
    });
}

#[test]
fn delete_ping() {
    run_test(|mut connection| async move {
        connection
            .record_username(43198, "ehuss".to_string())
            .await
            .unwrap();
        let now = chrono::Utc::now();
        for x in 1..4 {
            connection
                .record_ping(&Notification {
                    user_id: 43198,
                    origin_url: x.to_string(),
                    origin_html: "@ehuss {n}".to_string(),
                    short_description: Some("Comment on some issue".to_string()),
                    time: now.into(),
                    team_name: None,
                })
                .await
                .unwrap();
        }

        let notifications = connection.get_notifications("ehuss").await.unwrap();
        assert_eq!(notifications.len(), 3);
        assert_eq!(notifications[0].origin_url, "1");
        assert_eq!(notifications[1].origin_url, "2");
        assert_eq!(notifications[2].origin_url, "3");

        match connection
            .delete_ping(43198, Identifier::Index(NonZeroUsize::new(5).unwrap()))
            .await
        {
            Err(e) => assert_eq!(e.to_string(), "No such notification with index 5"),
            Ok(deleted) => panic!("did not expect success {deleted:?}"),
        }

        let deleted = connection
            .delete_ping(43198, Identifier::Index(NonZeroUsize::new(2).unwrap()))
            .await
            .unwrap();
        assert_eq!(deleted.len(), 1);
        assert_eq!(deleted[0].origin_url, "2");
        let notifications = connection.get_notifications("ehuss").await.unwrap();
        assert_eq!(notifications.len(), 2);
        assert_eq!(notifications[0].origin_url, "1");
        assert_eq!(notifications[1].origin_url, "3");

        let deleted = connection
            .delete_ping(43198, Identifier::Url("1"))
            .await
            .unwrap();
        assert_eq!(deleted.len(), 1);
        assert_eq!(deleted[0].origin_url, "1");
        let notifications = connection.get_notifications("ehuss").await.unwrap();
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].origin_url, "3");

        for x in 4..6 {
            connection
                .record_ping(&Notification {
                    user_id: 43198,
                    origin_url: x.to_string(),
                    origin_html: "@ehuss {n}".to_string(),
                    short_description: Some("Comment on some issue".to_string()),
                    time: now.into(),
                    team_name: None,
                })
                .await
                .unwrap();
        }

        let notifications = connection.get_notifications("ehuss").await.unwrap();
        assert_eq!(notifications.len(), 3);
        assert_eq!(notifications[0].origin_url, "3");
        assert_eq!(notifications[1].origin_url, "4");
        assert_eq!(notifications[2].origin_url, "5");

        let deleted = connection
            .delete_ping(43198, Identifier::Index(NonZeroUsize::new(2).unwrap()))
            .await
            .unwrap();
        assert_eq!(deleted.len(), 1);
        assert_eq!(deleted[0].origin_url, "4");

        let deleted = connection
            .delete_ping(43198, Identifier::All)
            .await
            .unwrap();
        assert_eq!(deleted.len(), 2);
        assert_eq!(deleted[0].origin_url, "3");
        assert_eq!(deleted[1].origin_url, "5");

        let notifications = connection.get_notifications("ehuss").await.unwrap();
        assert_eq!(notifications.len(), 0);
    });
}

#[test]
fn meta_notification() {
    run_test(|mut connection| async move {
        let now = chrono::Utc::now();
        connection
            .record_username(43198, "ehuss".to_string())
            .await
            .unwrap();
        connection
            .record_ping(&Notification {
                user_id: 43198,
                origin_url: "1".to_string(),
                origin_html: "This comment mentions @ehuss.".to_string(),
                short_description: Some("Comment on some issue".to_string()),
                time: now.into(),
                team_name: None,
            })
            .await
            .unwrap();
        connection
            .add_metadata(43198, 0, Some("metadata 1"))
            .await
            .unwrap();
        let notifications = connection.get_notifications("ehuss").await.unwrap();
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].metadata.as_deref(), Some("metadata 1"));
    });
}

#[test]
fn move_indices() {
    run_test(|mut connection| async move {
        let now = chrono::Utc::now();
        connection
            .record_username(43198, "ehuss".to_string())
            .await
            .unwrap();
        for x in 1..4 {
            connection
                .record_ping(&Notification {
                    user_id: 43198,
                    origin_url: x.to_string(),
                    origin_html: "@ehuss {n}".to_string(),
                    short_description: Some("Comment on some issue".to_string()),
                    time: now.into(),
                    team_name: None,
                })
                .await
                .unwrap();
        }
        connection.move_indices(43198, 1, 0).await.unwrap();
        let notifications = connection.get_notifications("ehuss").await.unwrap();
        assert_eq!(notifications.len(), 3);
        assert_eq!(notifications[0].origin_url, "2");
        assert_eq!(notifications[1].origin_url, "1");
        assert_eq!(notifications[2].origin_url, "3");
    });
}
