use super::run_test;
use crate::assert_datetime_approx_equal;
use serde_json::json;

#[test]
fn jobs() {
    run_test(|mut connection| async move {
        // Create some jobs and check that ones scheduled in the past are returned.
        let past = chrono::Utc::now() - chrono::Duration::minutes(5);
        let future = chrono::Utc::now() + chrono::Duration::hours(1);
        connection
            .insert_job("sample_job1", &past, &json! {{"foo": 123}})
            .await
            .unwrap();
        connection
            .insert_job("sample_job2", &past, &json! {{}})
            .await
            .unwrap();
        connection
            .insert_job("sample_job1", &future, &json! {{}})
            .await
            .unwrap();
        let jobs = connection.get_jobs_to_execute().await.unwrap();
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0].name, "sample_job1");
        assert_datetime_approx_equal(&jobs[0].scheduled_at, &past);
        assert_eq!(jobs[0].metadata, json! {{"foo": 123}});
        assert_eq!(jobs[0].executed_at, None);
        assert_eq!(jobs[0].error_message, None);

        assert_eq!(jobs[1].name, "sample_job2");
        assert_datetime_approx_equal(&jobs[1].scheduled_at, &past);
        assert_eq!(jobs[1].metadata, json! {{}});
        assert_eq!(jobs[1].executed_at, None);
        assert_eq!(jobs[1].error_message, None);

        // Get job by name
        let job = connection
            .get_job_by_name_and_scheduled_at("sample_job1", &future)
            .await
            .unwrap();
        assert_eq!(job.metadata, json! {{}});
        assert_eq!(job.error_message, None);

        // Update error message
        connection
            .update_job_error_message(&job.id, "an error")
            .await
            .unwrap();
        let job = connection
            .get_job_by_name_and_scheduled_at("sample_job1", &future)
            .await
            .unwrap();
        assert_eq!(job.error_message.as_deref(), Some("an error"));

        // Delete job
        let job = connection
            .get_job_by_name_and_scheduled_at("sample_job1", &past)
            .await
            .unwrap();
        connection.delete_job(&job.id).await.unwrap();
        let jobs = connection.get_jobs_to_execute().await.unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].name, "sample_job2");
    });
}

#[test]
fn on_conflict() {
    // Verify that inserting a job with different data updates the data.
    run_test(|mut connection| async move {
        let past = chrono::Utc::now() - chrono::Duration::minutes(5);
        connection
            .insert_job("sample_job1", &past, &json! {{"foo": 123}})
            .await
            .unwrap();
        connection
            .insert_job("sample_job1", &past, &json! {{"foo": 456}})
            .await
            .unwrap();
        let job = connection
            .get_job_by_name_and_scheduled_at("sample_job1", &past)
            .await
            .unwrap();
        assert_eq!(job.metadata, json! {{"foo": 456}});
    });
}

#[test]
fn update_job_executed_at() {
    run_test(|mut connection| async move {
        let now = chrono::Utc::now();
        let past = now - chrono::Duration::minutes(5);
        connection
            .insert_job("sample_job1", &past, &json! {{"foo": 123}})
            .await
            .unwrap();
        let jobs = connection.get_jobs_to_execute().await.unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].executed_at, None);
        connection
            .update_job_executed_at(&jobs[0].id)
            .await
            .unwrap();
        let jobs = connection.get_jobs_to_execute().await.unwrap();
        assert_eq!(jobs.len(), 1);
        let executed_at = jobs[0].executed_at.expect("executed_at should be set");
        // The timestamp should be approximately "now".
        if executed_at - now > chrono::Duration::minutes(1) {
            panic!("executed_at timestamp unexpected {executed_at:?} vs {now:?}");
        }
    });
}
