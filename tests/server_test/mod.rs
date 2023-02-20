//! Tests that exercise the webhook behavior of the triagebot server.
//!
//! These tests exercise the behavior of the `triagebot` server by injecting
//! webhook events into it, and then observing its behavior and response. They
//! involve setting up HTTP servers, launching the `triagebot` process,
//! injecting the webhook JSON object, and validating the result.
//!
//! The [`run_test`] function is used to set up the test and run it to
//! completion using pre-recorded JSON data.
//!
//! To write one of these tests, you'll need to use the recording function
//! against the live GitHub site to fetch what the actual JSON objects should
//! look like. To write a test, follow these steps:
//!
//! 1. Prepare a test repo on GitHub for exercising whatever action you want
//!    to test (for example, your personal fork of `rust-lang/rust). Get
//!    everything ready, such as opening a PR or whatever you need for your
//!    test.
//!
//! 2. Manually launch the triagebot server with the `TRIAGEBOT_TEST_RECORD`
//!    environment variable set to the path of where you want to store the
//!    recorded JSON. Use a separate directory for each test. It may look
//!    something like:
//!
//!    ```sh
//!    TRIAGEBOT_TEST_RECORD=server_test/shortcut/author cargo run --bin triagebot
//!    ```
//!
//!    Look at `README.md` for instructions for running triagebot against the
//!    live GitHub site. You'll need to have webhook forwarding running in the
//!    background.
//!
//!  3. Perform the action you want to test on GitHub. For example, post a
//!     comment with `@rustbot ready` to test the "ready" command.
//!
//!  4. Stop the triagebot server (hit CTRL-C or whatever).
//!
//!  5. The JSON for the interaction should now be stored in the directory.
//!     Peruse the JSON to make sure the expected actions are there.
//!
//!  6. Add a test to replay that action you just performed. All you need to
//!     do is call something like `run_test("shortcut/ready");` with the path
//!     to the JSON (relative to the `server_test` directory).
//!
//!  7. Run your test to make sure it works:
//!
//!     ```sh
//!     cargo test --test testsuite -- server_test::shortcut::ready
//!     ```
//!
//!     with the name of your test.
//!
//! ## Databases
//!
//! By default, the server tests will use Postgres if it is installed. If it
//! doesn't appear to be installed, then it will use SQLite instead. If you
//! want to force it to use SQLite, you can set the
//! TRIAGEBOT_TEST_FORCE_SQLITE environment variable.
//!
//! ## Scheduled Jobs
//!
//! Scheduled jobs get automatically disabled when recording or running tests
//! (via the `TRIAGEBOT_TEST_DISABLE_JOBS` environment variable). If you want
//! to write a test for a scheduled job, you'll need to set up a mechanism to
//! manually trigger the job (which could be useful outside of testing).

mod mentions;
mod shortcut;

use super::{HttpServer, HttpServerHandle};
use std::io::Read;
use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::AtomicU16;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use triagebot::test_record::Activity;

/// The webhook secret used to validate that the webhook events are coming
/// from the expected source.
const WEBHOOK_SECRET: &str = "secret";

/// A context used for running a test.
///
/// This is used for interacting with the triagebot process and handling API requests.
struct ServerTestCtx {
    /// The triagebot process handle.
    child: Child,
    /// Stdout received from triagebot, used for debugging.
    stdout: Arc<Mutex<Vec<u8>>>,
    /// Stderr received from triagebot, used for debugging.
    stderr: Arc<Mutex<Vec<u8>>>,
    /// Directory for the temporary Postgres database.
    ///
    /// `None` if using sqlite.
    db_dir: Option<PathBuf>,
    /// The address for sending webhooks into the triagebot binary.
    triagebot_addr: SocketAddr,
    /// The handle to the mock server which simulates GitHub.
    #[allow(dead_code)] // held for drop
    server: HttpServerHandle,
}

/// The main entry point for a test.
///
/// Pass the name of the test as the first parameter.
fn run_test(test_name: &str) {
    crate::assert_single_record();
    let activities = crate::load_activities("tests/server_test", test_name);
    if !matches!(activities[0], Activity::Webhook { .. }) {
        panic!("expected first activity to be a webhook event");
    }
    let ctx = build(activities);
    // Wait for the server side to find a webhook. This will then send the
    // webhook to the triagebot binary.
    loop {
        let activity = ctx
            .server
            .hook_recv
            .recv_timeout(Duration::new(60, 0))
            .unwrap();
        match activity {
            Activity::Webhook {
                webhook_event,
                payload,
            } => {
                eprintln!("sending webhook {webhook_event}");
                let payload = serde_json::to_vec(&payload).unwrap();
                ctx.send_webhook(&webhook_event, payload);
            }
            Activity::Error { message } => {
                panic!("unexpected server error: {message}");
            }
            Activity::Finished => break,
            a => panic!("unexpected activity {a:?}"),
        }
    }
}

fn build(activities: Vec<Activity>) -> ServerTestCtx {
    let db_sqlite = || {
        crate::test_dir()
            .join("triagebot.sqlite3")
            .to_str()
            .unwrap()
            .to_string()
    };
    let (db_dir, database_url) = if std::env::var_os("TRIAGEBOT_TEST_FORCE_SQLITE").is_some() {
        (None, db_sqlite())
    } else {
        match crate::db::setup_postgres() {
            Some(db_dir) => {
                let database_url = crate::db::postgres_database_url(&db_dir);
                (Some(db_dir), database_url)
            }
            None if std::env::var_os("CI").is_some() => panic!("expected postgres in CI"),
            None => (None, db_sqlite()),
        }
    };

    let server = HttpServer::new(activities);
    let triagebot_port = next_triagebot_port();
    let mut child = Command::new(env!("CARGO_BIN_EXE_triagebot"))
        .env(
            "GITHUB_API_TOKEN",
            "ghp_123456789012345678901234567890123456",
        )
        .env("GITHUB_WEBHOOK_SECRET", WEBHOOK_SECRET)
        .env("DATABASE_URL", database_url)
        .env("PORT", triagebot_port.to_string())
        .env("GITHUB_API_URL", format!("http://{}", server.addr))
        .env(
            "GITHUB_GRAPHQL_API_URL",
            format!("http://{}/graphql", server.addr),
        )
        .env("GITHUB_RAW_URL", format!("http://{}", server.addr))
        .env("TEAMS_API_URL", format!("http://{}/v1", server.addr))
        // We don't want jobs randomly running while testing.
        .env("TRIAGEBOT_TEST_DISABLE_JOBS", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Spawn some threads to capture output which can be used for debugging.
    let stdout = Arc::new(Mutex::new(Vec::new()));
    let stderr = Arc::new(Mutex::new(Vec::new()));
    let consumer = |mut source: Box<dyn Read + Send>, dest: Arc<Mutex<Vec<u8>>>| {
        move || {
            let mut dest = dest.lock().unwrap();
            if let Err(e) = source.read_to_end(&mut dest) {
                eprintln!("process reader failed: {e}");
            };
        }
    };
    thread::spawn(consumer(
        Box::new(child.stdout.take().unwrap()),
        stdout.clone(),
    ));
    thread::spawn(consumer(
        Box::new(child.stderr.take().unwrap()),
        stderr.clone(),
    ));
    let triagebot_addr = format!("127.0.0.1:{triagebot_port}").parse().unwrap();
    // Wait for the triagebot process to start up.
    for _ in 0..30 {
        match std::net::TcpStream::connect(&triagebot_addr) {
            Ok(_) => break,
            Err(e) => match e.kind() {
                std::io::ErrorKind::ConnectionRefused => {
                    std::thread::sleep(std::time::Duration::new(1, 0))
                }
                _ => panic!("unexpected error testing triagebot connection: {e:?}"),
            },
        }
    }

    ServerTestCtx {
        child,
        stdout,
        stderr,
        db_dir,
        triagebot_addr,
        server,
    }
}

impl ServerTestCtx {
    /// Sends a webhook into the triagebot binary.
    fn send_webhook(&self, event: &str, json: Vec<u8>) {
        let hmac = triagebot::payload::sign(WEBHOOK_SECRET, &json);
        let sha1 = hex::encode(&hmac);
        let client = reqwest::blocking::Client::new();
        let response = client
            .post(format!("http://{}/github-hook", self.triagebot_addr))
            .header("X-GitHub-Event", event)
            .header("X-Hub-Signature", format!("sha1={sha1}"))
            .body(json)
            .send()
            .unwrap();
        if !response.status().is_success() {
            let text = response.text().unwrap();
            panic!("webhook failed to get successful status: {text}");
        }
    }
}

impl Drop for ServerTestCtx {
    fn drop(&mut self) {
        if let Some(db_dir) = &self.db_dir {
            crate::db::stop_postgres(db_dir);
        }
        // Shut down triagebot.
        let _ = self.child.kill();
        // Display triagebot's output for debugging.
        if let Ok(stderr) = self.stderr.lock() {
            if let Ok(s) = std::str::from_utf8(&stderr) {
                eprintln!("{}", s);
            }
        }
        if let Ok(stdout) = self.stdout.lock() {
            if let Ok(s) = std::str::from_utf8(&stdout) {
                println!("{}", s);
            }
        }
    }
}

/// Returns a free port for the next triagebot process to use.
fn next_triagebot_port() -> u16 {
    static NEXT_TCP_PORT: AtomicU16 = AtomicU16::new(50000);
    loop {
        // This depends on SO_REUSEADDR being set.
        //
        // This is inherently racey, as the port may become unavailable
        // in-between the time it is checked here and triagebot actually binds
        // to it.
        //
        // TODO: This may not work on Windows, may need investigation/fixing.
        let triagebot_port = NEXT_TCP_PORT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if triagebot_port == 0 {
            panic!("can't find port to listen on");
        }
        if TcpListener::bind(format!("127.0.0.1:{triagebot_port}")).is_ok() {
            return triagebot_port;
        }
    }
}
