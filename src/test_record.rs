//! Support for recording network activity for test playback.
//!
//! See `testsuite.rs` for more information on test recording.

use crate::EventName;
use anyhow::Context;
use anyhow::Result;
use reqwest::{Request, StatusCode};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use tracing::{error, info, warn};

/// A representation of the recording of activity of triagebot.
///
/// Activities are stored as JSON on disk. The test framework injects the
/// `Webhook` and then checks for the appropriate requests and sends the
/// recorded responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Activity {
    /// A webhook event received from GitHub.
    Webhook {
        webhook_event: String,
        payload: serde_json::Value,
    },
    /// An outgoing request to api.github.com, and its response.
    ApiRequest {
        method: String,
        path: String,
        query: Option<String>,
        request_body: String,
        response_code: u16,
        response_body: serde_json::Value,
    },
    /// An outgoing request to raw.githubusercontent.com, and its response.
    RawRequest {
        path: String,
        query: Option<String>,
        response_code: u16,
        response_body: String,
    },
    /// Sent by the mock HTTP server to the test framework when it detects
    /// something is wrong.
    Error { message: String },
    /// Sent by the mock HTTP server to the test framework to tell it that all
    /// activities have been processed.
    Finished,
}

/// Information about an HTTP request that is captured before sending.
///
/// This is needed to avoid messing with cloning the Request.
pub struct RequestInfo {
    /// If this is `true`, then it is for raw.githubusercontent.com.
    /// If `false`, then it is for api.github.com.
    is_raw: bool,
    method: String,
    path: String,
    query: Option<String>,
    body: String,
}

/// Returns whether or not TRIAGEBOT_TEST_RECORD has been set to enable
/// recording.
pub fn is_recording() -> bool {
    record_dir().is_some()
}

/// The directory where the JSON recordings should be stored.
///
/// Returns `None` if recording is disabled.
fn record_dir() -> Option<PathBuf> {
    let Some(test_record) = std::env::var_os("TRIAGEBOT_TEST_RECORD") else { return None };
    let mut record_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    record_dir.push("tests");
    record_dir.push(test_record);
    Some(record_dir)
}

fn next_sequence() -> u32 {
    static NEXT_ID: AtomicU32 = AtomicU32::new(0);
    NEXT_ID.fetch_add(1, Ordering::SeqCst)
}

/// Initializes the test recording system.
///
/// This sets up the directory where JSON files are stored if the
/// `TRIAGEBOT_TEST_RECORD` environment variable is set.
pub fn init() -> Result<()> {
    let Some(record_dir) = record_dir() else { return Ok(()) };
    fs::create_dir_all(&record_dir)
        .with_context(|| format!("failed to create recording directory {record_dir:?}"))?;
    // Clear any old recording data.
    for entry in fs::read_dir(&record_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|p| p.to_str()) == Some("json") {
            warn!("deleting old recording at {path:?}");
            fs::remove_file(&path)
                .with_context(|| format!("failed to remove old recording {path:?}"))?;
        }
    }
    Ok(())
}

/// Records a webhook event for the test framework.
///
/// The recording is only saved if the `TRIAGEBOT_TEST_RECORD` environment
/// variable is set.
pub fn record_event(event: &EventName, payload: &str) {
    let Some(record_dir) = record_dir() else { return };

    let payload_json: serde_json::Value = serde_json::from_str(payload).expect("valid json");
    let name = match event {
        EventName::PullRequest => {
            let action = payload_json["action"].as_str().unwrap();
            let number = payload_json["number"].as_u64().unwrap();
            format!("pr{number}_{action}")
        }
        EventName::PullRequestReview => {
            let action = payload_json["action"].as_str().unwrap();
            let number = payload_json["pull_request"]["number"].as_u64().unwrap();
            format!("pr{number}_review_{action}")
        }
        EventName::PullRequestReviewComment => {
            let action = payload_json["action"].as_str().unwrap();
            let number = payload_json["pull_request"]["number"].as_u64().unwrap();
            format!("pr{number}_review_comment_{action}")
        }
        EventName::IssueComment => {
            let action = payload_json["action"].as_str().unwrap();
            let number = payload_json["issue"]["number"].as_u64().unwrap();
            format!("issue{number}_comment_{action}")
        }
        EventName::Issue => {
            let action = payload_json["action"].as_str().unwrap();
            let number = payload_json["issue"]["number"].as_u64().unwrap();
            format!("issue{number}_{action}")
        }
        EventName::Push => {
            let after = payload_json["after"].as_str().unwrap();
            format!("push_{after}")
        }
        EventName::Create => {
            let ref_type = payload_json["ref_type"].as_str().unwrap();
            let git_ref = payload_json["ref"].as_str().unwrap();
            format!("create_{ref_type}_{git_ref}")
        }
        EventName::Other => {
            return;
        }
    };
    let activity = Activity::Webhook {
        webhook_event: event.to_string(),
        payload: payload_json,
    };
    let filename = format!("{:02}-webhook-{name}.json", next_sequence());
    save_activity(&record_dir.join(filename), &activity);
}

/// Captures information about a Request to be used for a test recording.
///
/// This value is passed to `record_request` after the request has been sent.
pub fn capture_request(req: &Request) -> Option<RequestInfo> {
    if !is_recording() {
        return None;
    }
    let url = req.url();
    let body = req
        .body()
        .and_then(|body| body.as_bytes())
        .map(|bytes| String::from_utf8(bytes.to_vec()).unwrap())
        .unwrap_or_default();
    let is_raw = url.host_str().unwrap().contains("raw");
    let info = RequestInfo {
        is_raw,
        method: req.method().to_string(),
        path: url.path().to_string(),
        query: url.query().map(|q| q.to_string()),
        body,
    };
    Some(info)
}

/// Records an HTTP request and response for the test framework.
///
/// The recording is only saved if the `TRIAGEBOT_TEST_RECORD` environment
/// variable is set.
pub fn record_request(info: Option<RequestInfo>, status: StatusCode, body: &[u8]) {
    let Some(info) = info else { return };
    let Some(record_dir) = record_dir() else { return };
    let response_code = status.as_u16();
    let mut name = info.path.replace(['/', '.'], "_");
    if name.starts_with('_') {
        name.remove(0);
    }
    let (kind, activity) = if info.is_raw {
        (
            "raw",
            Activity::RawRequest {
                path: info.path,
                query: info.query,
                response_code,
                response_body: String::from_utf8_lossy(body).to_string(),
            },
        )
    } else {
        let json_body = if info.path == "/v1/teams.json" {
            // This is a hack to reduce the amount of data stored in the test
            // directory. This file gets requested many times, and it is very
            // large.
            serde_json::json!({})
        } else {
            match serde_json::from_slice(body) {
                Ok(json) => json,
                Err(e) => {
                    error!("failed to record API response for {}: {e:?}", info.path);
                    return;
                }
            }
        };
        name.insert(0, '-');
        name.insert_str(0, &info.method);
        (
            "api",
            Activity::ApiRequest {
                method: info.method,
                path: info.path,
                query: info.query,
                request_body: info.body,
                response_code,
                response_body: json_body,
            },
        )
    };

    let filename = format!("{:02}-{kind}-{name}.json", next_sequence());
    save_activity(&record_dir.join(filename), &activity);
}

/// Helper for saving an [`Activity`] to disk as JSON.
fn save_activity(path: &Path, activity: &Activity) {
    let save_activity_inner = || -> Result<()> {
        let file = File::create(path)?;
        let file = BufWriter::new(file);
        serde_json::to_writer_pretty(file, &activity)?;
        Ok(())
    };
    if let Err(e) = save_activity_inner() {
        error!("failed to save test activity {path:?}: {e:?}");
    };
    info!("test activity saved to {path:?}");
}
