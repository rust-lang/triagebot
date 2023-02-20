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
//!    live GitHub site. You'll need to have webhook forwarding and Postgres
//!    running in the background.
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
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU16, AtomicU32};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use triagebot::test_record::Activity;

/// Counter used to give each test a unique sandbox directory in the
/// `target/tmp` directory.
static TEST_COUNTER: AtomicU32 = AtomicU32::new(1);
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
    db_dir: PathBuf,
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
    // Set up a directory where this test can store all its stuff.
    let tmp_dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("local");
    let test_num = TEST_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let test_dir = tmp_dir.join(format!("t{test_num}"));
    if test_dir.exists() {
        std::fs::remove_dir_all(&test_dir).unwrap();
    }
    std::fs::create_dir_all(&test_dir).unwrap();

    let db_dir = test_dir.join("db");
    setup_postgres(&db_dir);

    let server = HttpServer::new(activities);
    let triagebot_port = next_triagebot_port();
    let mut child = Command::new(env!("CARGO_BIN_EXE_triagebot"))
        .env(
            "GITHUB_API_TOKEN",
            "ghp_123456789012345678901234567890123456",
        )
        .env("GITHUB_WEBHOOK_SECRET", WEBHOOK_SECRET)
        .env(
            "DATABASE_URL",
            format!(
                "postgres:///triagebot?user=triagebot&host={}",
                db_dir.display()
            ),
        )
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
        // Shut down postgres.
        let pg_dir = find_postgres();
        match Command::new(pg_dir.join("pg_ctl"))
            .args(&["-D", self.db_dir.to_str().unwrap(), "stop"])
            .output()
        {
            Ok(output) => {
                if !output.status.success() {
                    eprintln!(
                        "failed to stop postgres:\n\
                        ---stdout\n\
                        {}\n\
                        ---stderr\n\
                        {}\n\
                        ",
                        std::str::from_utf8(&output.stdout).unwrap(),
                        std::str::from_utf8(&output.stderr).unwrap()
                    );
                }
            }
            Err(e) => eprintln!("could not run pg_ctl to stop: {e}"),
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

fn run_command(command: &Path, args: &[&str], cwd: &Path) {
    eprintln!("running {command:?}: {args:?}");
    let output = Command::new(command)
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap_or_else(|e| panic!("`{command:?}` failed to run: {e}"));
    if !output.status.success() {
        panic!(
            "{command:?} failed:\n\
            ---stdout\n\
            {}\n\
            ---stderr\n\
            {}\n\
            ",
            std::str::from_utf8(&output.stdout).unwrap(),
            std::str::from_utf8(&output.stderr).unwrap()
        );
    }
}

fn setup_postgres(db_dir: &Path) {
    std::fs::create_dir(&db_dir).unwrap();
    let db_dir_str = db_dir.to_str().unwrap();
    let pg_dir = find_postgres();
    run_command(
        &pg_dir.join("initdb"),
        &["--auth=trust", "--username=triagebot", "-D", db_dir_str],
        db_dir,
    );
    run_command(
        &pg_dir.join("pg_ctl"),
        &[
            // -h '' tells it to not listen on TCP
            // -k tells it where to place the unix-domain socket
            "-o",
            &format!("-h '' -k {db_dir_str}"),
            // -D is the data dir where everything is stored
            "-D",
            db_dir_str,
            // -l enables logging to a file instead of stdout
            "-l",
            db_dir.join("postgres.log").to_str().unwrap(),
            "start",
        ],
        db_dir,
    );
    run_command(
        &pg_dir.join("createdb"),
        &["--user", "triagebot", "-h", db_dir_str, "triagebot"],
        db_dir,
    );
}

/// Finds the root for PostgreSQL commands.
///
/// For various reasons, some Linux distros hide some postgres commands and
/// don't put them on PATH, making them difficult to access.
fn find_postgres() -> PathBuf {
    // Check if on PATH first.
    if let Ok(o) = Command::new("initdb").arg("-V").output() {
        if o.status.success() {
            return PathBuf::new();
        }
    }
    if let Ok(dirs) = std::fs::read_dir("/usr/lib/postgresql") {
        let mut versions: Vec<_> = dirs
            .filter_map(|entry| {
                let entry = entry.unwrap();
                // Versions are generally of the form 9.3 or 14, but this
                // might be broken if other forms are used.
                if let Ok(n) = entry.file_name().to_str().unwrap().parse::<f32>() {
                    Some((n, entry.path()))
                } else {
                    None
                }
            })
            .collect();
        if !versions.is_empty() {
            versions.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            return versions.last().unwrap().1.join("bin");
        }
    }
    panic!(
        "Could not find PostgreSQL binaries.\n\
        Make sure to install PostgreSQL.\n\
        If PostgreSQL is installed, update this function to match where they \
        are located on your system.\n\
        Or, add them to your PATH."
    );
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
