//! SCHEDULED JOBS
//!
//! The metadata is a serde_json::Value
//! Please refer to https://docs.rs/serde_json/latest/serde_json/value/fn.from_value.html
//! on how to interpret it as an instance of type T, implementing Serialize/Deserialize.
//!
//! The schedule is a cron::Schedule
//! Please refer to https://docs.rs/cron/latest/cron/struct.Schedule.html for further info
//!
//! For example, if we want to sends a Zulip message every Friday at 11:30am ET into #t-release
//! with a @T-release meeting! content, we should create some JobSchedule like:
//!
//!    #[derive(Serialize, Deserialize)]
//!    struct ZulipMetadata {
//!      pub message: String
//!    }
//!
//!    let metadata = serde_json::value::to_value(ZulipMetadata {
//!      message: "@T-release meeting!".to_string()
//!    }).unwrap();
//!
//!    let schedule = Schedule::from_str("0 30 11 * * FRI *").unwrap();
//!    
//!    let new_job = JobSchedule {
//!      name: "send_zulip_message".to_owned(),
//!      schedule: schedule,
//!      metadata: metadata
//!    }
//!
//! and include it in the below vector in jobs():
//!
//!   jobs.push(new_job);
//!
//! ... fianlly, add the corresponding "send_zulip_message" handler in src/handlers/jobs.rs

use crate::db::jobs::JobSchedule;

// How often new cron-based jobs will be placed in the queue.
// This is the minimum period *between* a single cron task's executions.
pub const JOB_SCHEDULING_CADENCE_IN_SECS: u64 = 1800;

// How often the database is inspected for jobs which need to execute.
// This is the granularity at which events will occur.
pub const JOB_PROCESSING_CADENCE_IN_SECS: u64 = 60;

pub fn jobs() -> Vec<JobSchedule> {
    // Add to this vector any new cron task you want (as explained above)
    let jobs: Vec<JobSchedule> = Vec::new();

    jobs
}
