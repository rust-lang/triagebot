use super::run_test;
use crate::assert_datetime_approx_equal;
use triagebot::db::Commit;

#[test]
fn rustc_commits() {
    run_test(|mut connection| async move {
        // Using current time since `get_commits_with_artifacts` is relative to the current time.
        let now = chrono::offset::Utc::now();
        connection
            .record_commit(&Commit {
                sha: "eebdfb55fce148676c24555505aebf648123b2de".to_string(),
                parent_sha: "73f40197ecabf77ed59028af61739404eb60dd2e".to_string(),
                time: now.into(),
                pr: Some(108228),
            })
            .await
            .unwrap();

        // A little older to ensure sorting is consistent.
        let now3 = now - chrono::Duration::hours(3);
        connection
            .record_commit(&Commit {
                sha: "73f40197ecabf77ed59028af61739404eb60dd2e".to_string(),
                parent_sha: "fcdbd1c07f0b6c8e7d8bbd727c6ca69a1af8c7e9".to_string(),
                time: now3.into(),
                pr: Some(107772),
            })
            .await
            .unwrap();

        // In the distant past, won't show up in get_commits_with_artifacts.
        connection
            .record_commit(&Commit {
                sha: "26904687275a55864f32f3a7ba87b7711d063fd5".to_string(),
                parent_sha: "3b348d932aa5c9884310d025cf7c516023fd0d9a".to_string(),
                time: "2022-02-19T23:25:06Z".parse().unwrap(),
                pr: Some(92911),
            })
            .await
            .unwrap();

        assert!(connection
            .has_commit("eebdfb55fce148676c24555505aebf648123b2de")
            .await
            .unwrap());
        assert!(connection
            .has_commit("73f40197ecabf77ed59028af61739404eb60dd2e")
            .await
            .unwrap());
        assert!(connection
            .has_commit("26904687275a55864f32f3a7ba87b7711d063fd5")
            .await
            .unwrap());
        assert!(!connection
            .has_commit("fcdbd1c07f0b6c8e7d8bbd727c6ca69a1af8c7e9")
            .await
            .unwrap());

        let missing = connection.get_missing_commits().await.unwrap();
        assert_eq!(
            &missing[..],
            [
                "fcdbd1c07f0b6c8e7d8bbd727c6ca69a1af8c7e9",
                "3b348d932aa5c9884310d025cf7c516023fd0d9a"
            ]
        );

        let commits = connection.get_commits_with_artifacts().await.unwrap();
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].sha, "eebdfb55fce148676c24555505aebf648123b2de");
        assert_eq!(
            commits[0].parent_sha,
            "73f40197ecabf77ed59028af61739404eb60dd2e"
        );
        assert_datetime_approx_equal(&commits[0].time, &now);
        assert_eq!(commits[0].pr, Some(108228));

        assert_eq!(commits[1].sha, "73f40197ecabf77ed59028af61739404eb60dd2e");
        assert_eq!(
            commits[1].parent_sha,
            "fcdbd1c07f0b6c8e7d8bbd727c6ca69a1af8c7e9"
        );
        assert_datetime_approx_equal(&commits[1].time, &now3);
        assert_eq!(commits[1].pr, Some(107772));
    });
}
