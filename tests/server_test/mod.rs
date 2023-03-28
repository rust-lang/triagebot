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
//! XXX
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
use futures::Future;
use std::io::Read;
use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::AtomicU16;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use triagebot::github::{GitTreeEntry, GithubClient, Issue, Repository};
use triagebot::test_record::{self, Activity};

struct ServerTestSetup {
    test_path: String,
    gh: GithubClient,
    repo: Repository,
}

/// A context used for running a test.
///
/// This is used for interacting with the triagebot process and handling API requests.
struct ServerTestCtx {
    /// The triagebot process handle.
    #[allow(dead_code)] // held for drop
    triagebot_process: Process,
    webhook_secret: String,
    /// Directory for the temporary Postgres database.
    ///
    /// `None` if using sqlite.
    db_dir: Option<PathBuf>,
    /// The address for sending webhooks into the triagebot binary.
    triagebot_addr: SocketAddr,
    /// The handle to the mock server which simulates GitHub.
    #[allow(dead_code)] // held for drop
    server: Option<HttpServerHandle>,
    // TODO
    gh: Option<GithubClient>,
    // TODO
    repo: Option<Repository>,

    #[allow(dead_code)] // held for drop
    gh_process: Option<Process>,
}

/// The main entry point for a test.
///
/// Pass the name of the test as the first parameter.
fn run_test<F, Fut>(test_path: &str, f: F)
where
    F: Fn(ServerTestSetup) -> Fut + Send + Sync,
    Fut: Future<Output = ()> + Send,
{
    crate::assert_single_record();
    if std::env::var("TRIAGEBOT_TEST_RECORD").as_deref() == Ok("1") {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let repo_name = test_record::record_live_repo().expect(
                    "TRIAGEBOT_TEST_LIVE_REPO must be set to the GitHub test repo to \
                    run tests on (\"username/reponame\" format)",
                );
                dotenv::dotenv().ok();
                let gh = GithubClient::new_from_env();
                let repo = gh.repository(&repo_name).await.unwrap();
                let setup = ServerTestSetup {
                    test_path: test_path.into(),
                    gh,
                    repo,
                };
                f(setup).await
            });
    } else {
        playback_test(test_path);
    }
}

fn playback_test(test_path: &str) {
    let activities = crate::load_activities("tests/server_test", test_path);
    if !matches!(activities[0], Activity::Webhook { .. }) {
        panic!("expected first activity to be a webhook event");
    }
    let ctx = ServerTestCtx::launch_prerecorded(activities);
    // Wait for the server side to find a webhook. This will then send the
    // webhook to the triagebot binary.
    loop {
        let activity = ctx
            .server
            .as_ref()
            .unwrap()
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

impl ServerTestCtx {
    fn launch_triagebot(server: Option<HttpServerHandle>, test_path: &str) -> ServerTestCtx {
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

        let triagebot_port = next_triagebot_port();
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_triagebot"));
        let rand_data: [u8; 8] = rand::random();
        let webhook_secret = hex::encode(&rand_data);
        // TODO: Generate the secret
        cmd.env("GITHUB_WEBHOOK_SECRET", &webhook_secret)
            .env("DATABASE_URL", database_url)
            .env("PORT", triagebot_port.to_string())
            // We don't want jobs randomly running while testing.
            .env("TRIAGEBOT_TEST_DISABLE_JOBS", "1");
        if let Some(server) = &server {
            // This is for replaying recorded data.
            // Use a fake API token to ensure it doesn't try to contact the live server.
            cmd.env(
                "GITHUB_API_TOKEN",
                "ghp_123456789012345678901234567890123456",
            )
            .env("GITHUB_API_URL", format!("http://{}", server.addr))
            .env(
                "GITHUB_GRAPHQL_API_URL",
                format!("http://{}/graphql", server.addr),
            )
            .env("GITHUB_RAW_URL", format!("http://{}", server.addr))
            .env("TEAMS_API_URL", format!("http://{}/v1", server.addr));
        } else {
            // This is for recording data from the live server.
            cmd.env("TRIAGEBOT_TEST_RECORD_DIR", format!("server_test/{test_path}"));
        }

        let mut triagebot_process = Process::spawn(cmd);
        let triagebot_addr = format!("127.0.0.1:{triagebot_port}").parse().unwrap();
        // Wait for the triagebot process to start up.
        eprintln!("waiting for triagebot to be ready");
        let mut attempts = 0;
        loop {
            if attempts > 30 {
                panic!("triagebot never responded");
            }
            match std::net::TcpStream::connect(&triagebot_addr) {
                Ok(_) => break,
                Err(e) => match e.kind() {
                    std::io::ErrorKind::ConnectionRefused => {
                        std::thread::sleep(std::time::Duration::new(1, 0))
                    }
                    _ => panic!("unexpected error testing triagebot connection: {e:?}"),
                },
            }
            if let Some(status) = triagebot_process.child.try_wait().unwrap() {
                panic!("triagebot did not start: {status}");
            }

            attempts += 1;
        }
        ServerTestCtx {
            triagebot_process,
            webhook_secret,
            db_dir,
            triagebot_addr,
            server,
            gh: None,
            repo: None,
            gh_process: None,
        }
    }

    fn launch_prerecorded(activities: Vec<Activity>) -> ServerTestCtx {
        let server = HttpServer::new(activities);
        ServerTestCtx::launch_triagebot(Some(server), "")
    }

    /// Sends a webhook into the triagebot binary.
    fn send_webhook(&self, event: &str, json: Vec<u8>) {
        let hmac = triagebot::payload::sign(&self.webhook_secret, &json);
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
        if TcpListener::bind(format!("0.0.0.0:{triagebot_port}")).is_ok() {
            return triagebot_port;
        }
    }
}

impl ServerTestSetup {
    pub async fn config(&self, config: &str) {
        eprintln!("setting up triagebot.toml on {}", self.repo.full_name);
        // Create a commit.
        // Figure out the current head.
        let default_head = format!("heads/{}", self.repo.default_branch);
        let head_ref = self
            .repo
            .get_reference(&self.gh, &default_head)
            .await
            .unwrap();
        let head_commit = self
            .repo
            .git_commit(&self.gh, &head_ref.object.sha)
            .await
            .unwrap();
        // Create a blob for the commit.
        let blob_sha = self
            .repo
            .create_blob(&self.gh, config, "utf-8")
            .await
            .unwrap();
        // Create a tree entry for this new file.
        let tree_entries = vec![GitTreeEntry {
            path: "triagebot.toml".into(),
            mode: "100644".into(),
            object_type: "blob".into(),
            sha: Some(Some(blob_sha)),
            content: None,
        }];
        let new_tree = self
            .repo
            .update_tree(&self.gh, &head_commit.tree.sha, &tree_entries)
            .await
            .unwrap();

        // Create a commit.
        let commit = self
            .repo
            .create_commit(
                &self.gh,
                &format!("Set triagebot.toml config for {}", self.test_path),
                &[&head_ref.object.sha],
                &new_tree.sha,
            )
            .await
            .unwrap();
        // Move head to the new commit.
        self.repo
            .update_reference(&self.gh, &default_head, &commit.sha)
            .await
            .unwrap();
    }

    pub fn launch_triagebot_live(self) -> ServerTestCtx {
        eprintln!("launching triagebot against live GitHub");
        let mut ctx = ServerTestCtx::launch_triagebot(None, &self.test_path);
        // Launch webhook forwarding.
        let mut gh_cmd = Command::new("gh");
        gh_cmd
            .args(&["webhook", "forward", "--events=*"])
            .arg(format!("--repo={}", self.repo.full_name))
            .arg(format!("--url=http://{}/github-hook", ctx.triagebot_addr))
            .arg(format!("--secret={}", ctx.webhook_secret));
        let mut gh_process = Process::spawn(gh_cmd);

        // Wait for `gh` to launch and configure the repo.
        eprintln!("waiting for gh webhook to be ready");
        let mut attempts = 0;
        loop {
            if attempts > 30 {
                panic!("gh webhook doesn't appear to be ready");
            }
            let stdout_lock = gh_process.stdout.lock().unwrap();
            let stdout = String::from_utf8_lossy(&stdout_lock);
            if stdout.lines().any(|line| line.starts_with("Forwarding")) {
                break;
            }
            drop(stdout_lock);
            if let Some(status) = gh_process.child.try_wait().unwrap() {
                panic!("gh webhook exited unexpectedly: {status}");
            }
            std::thread::sleep(std::time::Duration::new(1, 0));
            attempts += 1;
        }
        ctx.gh = Some(self.gh);
        ctx.repo = Some(self.repo);
        ctx.gh_process = Some(gh_process);
        ctx
    }
}

struct Process {
    description: String,
    child: Child,
    stdout: Arc<Mutex<Vec<u8>>>,
    stderr: Arc<Mutex<Vec<u8>>>,
}

impl Process {
    fn spawn(mut cmd: Command) -> Process {
        let description = format!("{cmd:?}");
        let mut child = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|e| {
                panic!(
                    "failed to spawn: `{description}`\n\
                {e}"
                );
            });
        let stdout = Arc::new(Mutex::new(Vec::new()));
        let stderr = Arc::new(Mutex::new(Vec::new()));

        let consumer = |mut source: Box<dyn Read + Send>, dest: Arc<Mutex<Vec<u8>>>| {
            move || loop {
                let mut buffer = [0; 1024];
                match source.read(&mut buffer) {
                    Ok(n) if n == 0 => break,
                    Ok(n) => {
                        let mut dest = dest.lock().unwrap();
                        dest.extend_from_slice(&buffer[0..n]);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(e) => {
                        eprintln!("failed to read output: {e}");
                        return;
                    }
                }
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

        Process {
            description,
            child,
            stdout,
            stderr,
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        // Display output for debugging.
        if let Ok(stderr) = self.stderr.lock() {
            if let Ok(s) = std::str::from_utf8(&stderr) {
                eprintln!("{} stdout:\n{s}", self.description);
            }
        }
        if let Ok(stdout) = self.stdout.lock() {
            if let Ok(s) = std::str::from_utf8(&stdout) {
                eprintln!("{} stderr:\n{s}", self.description);
            }
        }
    }
}

struct TestPrBuilder {
    title: String,
    branch: String,
    base: String,
    message_body: String,
    changes: Vec<TestPrChange>,
}

impl TestPrBuilder {
    fn new(branch: &str, base: &str) -> TestPrBuilder {
        TestPrBuilder {
            title: "Test PR".into(),
            branch: branch.into(),
            base: base.into(),
            message_body: "This is a test PR.".into(),
            changes: Vec::new(),
        }
    }

    pub fn file(mut self, path: &str, executable: bool, content: &str) -> Self {
        self.changes.push(TestPrChange::File {
            path: path.into(),
            executable,
            content: content.into(),
        });
        self
    }

    pub fn rm_file(mut self, path: &str) -> Self {
        self.changes
            .push(TestPrChange::RemoveFile { path: path.into() });
        self
    }

    pub fn submodule(mut self, path: &str, commit: &str) -> Self {
        self.changes.push(TestPrChange::Submodule {
            path: path.into(),
            commit: commit.into(),
        });
        self
    }

    pub fn symlink(mut self, path: &str, dest: &str) -> Self {
        self.changes.push(TestPrChange::Symlink {
            path: path.into(),
            dest: dest.into(),
        });
        self
    }

    pub async fn create(mut self, gh: &GithubClient, repo: &Repository) -> Issue {
        // Delete any PRs open for this branch (GitHub does not allow multiple
        // PRs to the same branch).
        close_opened_prs(gh, repo, &self.branch).await;

        // Create a commit.
        // Figure out the current head.
        let head_ref = repo
            .get_reference(gh, &format!("heads/{}", self.base))
            .await
            .unwrap();
        let head_commit = repo.git_commit(gh, &head_ref.object.sha).await.unwrap();
        // Create blobs for new/modified files.
        if self.changes.is_empty() {
            self = self.file("sample-file.md", false, "This is some sample text");
        }
        let tree_entries: Vec<_> = self
            .changes
            .into_iter()
            .map(TestPrChange::into_tree_entry)
            .collect();
        let new_tree = repo
            .update_tree(gh, &head_commit.tree.sha, &tree_entries)
            .await
            .unwrap();

        let commit = repo
            .create_commit(
                gh,
                &self.message_body,
                &[&head_ref.object.sha],
                &new_tree.sha,
            )
            .await
            .unwrap();
        // Set the branch to that commit.
        repo.create_or_update_reference(gh, &format!("heads/{}", self.branch), &commit.sha)
            .await
            .unwrap();
        // Create a pull request.
        repo.new_pr(
            gh,
            &self.title,
            &self.branch,
            &self.base,
            &self.message_body,
        )
        .await
        .unwrap()
    }
}

enum TestPrChange {
    File {
        path: String,
        executable: bool,
        content: String,
    },
    RemoveFile {
        path: String,
    },
    Submodule {
        path: String,
        commit: String,
    },
    Symlink {
        path: String,
        dest: String,
    },
}

impl TestPrChange {
    fn into_tree_entry(self) -> GitTreeEntry {
        match self {
            TestPrChange::File {
                path,
                executable,
                content,
            } => {
                let mode = if executable { "100755" } else { "100644" }.into();
                GitTreeEntry {
                    path,
                    mode,
                    object_type: "blob".into(),
                    sha: None,
                    content: Some(content),
                }
            }
            TestPrChange::RemoveFile { path } => GitTreeEntry {
                path,
                mode: "100644".into(),
                object_type: "blob".into(),
                sha: Some(None),
                content: None,
            },
            TestPrChange::Submodule { path, commit } => GitTreeEntry {
                path,
                mode: "160000".into(),
                object_type: "commit".into(),
                sha: Some(Some(commit)),
                content: None,
            },
            TestPrChange::Symlink { path, dest } => GitTreeEntry {
                path,
                mode: "120000".into(),
                object_type: "blob".into(),
                sha: None,
                content: Some(dest),
            },
        }
    }
}

pub async fn close_opened_prs(gh: &GithubClient, repo: &Repository, branch: &str) {
    let issues = repo
        .get_prs(
            gh,
            "open",
            Some(&format!("{}:{}", repo.full_name, branch)),
            Some(&repo.default_branch),
            None,
            None,
        )
        .await
        .unwrap();
    for issue in issues {
        eprintln!("closing PR {}", issue.number);
        issue.close(gh).await.unwrap();
    }
}
