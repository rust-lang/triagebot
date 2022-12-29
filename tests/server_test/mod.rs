//! Tests that exercise the webhook behavior of the triagebot server.
//!
//! These tests exercise the behavior of the `triagebot` server by injecting
//! webhook events into it, and then observing its behavior and response. They
//! involve setting up HTTP servers, launching the `triagebot` process,
//! injecting the webhook JSON object, and validating the result.
//!
//! The [`TestBuilder`] is used for configuring the test, and producing a
//! [`ServerTestCtx`], which provides access to triagebot.
//!
//! See [`crate::github_client`] and [`TestBuilder`] for a discussion of how
//! to set up API handlers.
//!
//! These tests require that PostgreSQL is installed and available in your
//! PATH. The test sets up a little sandbox where a fresh PostgreSQL database
//! is created and managed.
//!
//! To get the webhook JSON data to inject with
//! [`ServerTestCtx::send_webook`], I recommend recording it by first running
//! the triagebot server against the real github.com site with the
//! `TRIAGEBOT_TEST_RECORD` environment variable set. See the README.md file
//! for how to set up and run triagebot against one of your own repositories.
//!
//! The recording will save `.json` files in the current directory of all the
//! events received. You can then move and rename those files into the
//! `tests/server_test` directory. You usually should modify the JSON to
//! rename the repository to `rust-lang/rust`.
//!
//! At the end of the test, you should call `ctx.events.assert_eq()` to
//! validate that the correct HTTP actions were actually performed by
//! triagebot. If you are uncertain about what to put in there, just start
//! with an empty list, and the error will tell you what to add.

mod shortcut;

use super::common::{
    Events, HttpServer, HttpServerHandle, Method, Method::*, Response, TestBuilder,
};
use std::collections::HashMap;
use std::io::Read;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Mutex};
use std::thread;

static NEXT_TCP_PORT: AtomicU32 = AtomicU32::new(50000);
static TEST_COUNTER: AtomicU32 = AtomicU32::new(1);

const WEBHOOK_SECRET: &str = "secret";

/// A context used for running a test.
///
/// This is used for interacting with the triagebot process and handling API requests.
struct ServerTestCtx {
    child: Child,
    stdout: Arc<Mutex<Vec<u8>>>,
    stderr: Arc<Mutex<Vec<u8>>>,
    db_dir: PathBuf,
    triagebot_addr: SocketAddr,
    #[allow(dead_code)] // held for drop
    api_server: HttpServerHandle,
    #[allow(dead_code)] // held for drop
    raw_server: HttpServerHandle,
    #[allow(dead_code)] // held for drop
    teams_server: HttpServerHandle,
    events: Events,
}

impl TestBuilder {
    fn new() -> TestBuilder {
        let tb = TestBuilder::default();
        tb.api_handler(GET, "rate_limit", |_req| {
            Response::new().body(include_bytes!("rate_limit.json"))
        })
    }

    fn build(mut self) -> ServerTestCtx {
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

        if let Some(config) = self.config {
            self.raw_handlers.insert(
                (GET, "rust-lang/rust/master/triagebot.toml"),
                Box::new(|_req| Response::new().body(config.as_bytes())),
            );
        }

        let events = Events::new();
        let api_server = HttpServer::new(self.api_handlers, events.clone());
        let raw_server = HttpServer::new(self.raw_handlers, events.clone());

        // Add users to the teams data here if you need them. At this time,
        // GET teams.json is not included in Events since the notifications
        // code always fetches the teams, even for `@rustbot` mentions (to
        // determine if `rustbot` is a team member). That's not interesting,
        // so it is excluded for now.
        let mut teams_handlers = HashMap::new();
        teams_handlers.insert(
            (GET, "teams.json"),
            Box::new(|_req| Response::new_from_path("tests/server_test/teams.json"))
                as super::common::RequestCallback,
        );
        let teams_server = HttpServer::new(teams_handlers, Events::new());

        // TODO: This is a poor way to choose a TCP port, as it could already
        // be in use by something else.
        let triagebot_port = NEXT_TCP_PORT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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
            .env("GITHUB_API_URL", format!("http://{}", api_server.addr))
            .env(
                "GITHUB_GRAPHQL_API_URL",
                format!("http://{}/graphql", api_server.addr),
            )
            .env("GITHUB_RAW_URL", format!("http://{}", raw_server.addr))
            .env("TEAMS_API_URL", format!("http://{}", teams_server.addr))
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
            api_server,
            teams_server,
            raw_server,
            events,
        }
    }
}

impl ServerTestCtx {
    fn send_webook(&self, json: &'static [u8]) {
        let hmac = triagebot::payload::sign(WEBHOOK_SECRET, json);
        let sha1 = hex::encode(&hmac);
        let client = reqwest::blocking::Client::new();
        let response = client
            .post(format!("http://{}/github-hook", self.triagebot_addr))
            .header("X-GitHub-Event", "issue_comment")
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
