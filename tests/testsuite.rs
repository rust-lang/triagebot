//! Triagebot integration testsuite.
//!
//! There is currently one type of test here:
//!
//! * `github_client` — This tests the behavior `GithubClient`.
//!
//! See the individual modules for an introduction to writing these tests.
//!
//! The tests generally work by launching an HTTP server and intercepting HTTP
//! requests that would normally go to external sites like
//! https://api.github.com.
//!
//! If you need help with debugging, set the `TRIAGEBOT_TEST_LOG=trace`
//! environment variable to display log information.

mod github_client;

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use triagebot::test_record::{self, Activity};
use url::Url;

/// A request received on the HTTP server.
#[derive(Clone, Debug)]
pub struct Request {
    /// The path of the request, such as `repos/rust-lang/rust/labels`.
    pub path: String,
    /// The HTTP method.
    pub method: String,
    /// The query components of the URL (the stuff after `?`).
    pub query: Option<String>,
    /// HTTP headers.
    pub headers: HashMap<String, String>,
    /// The body of the HTTP request (usually a JSON blob).
    pub body: Vec<u8>,
}

/// The response the HTTP server should send to the client.
pub struct Response {
    pub code: u16,
    pub headers: Vec<String>,
    pub body: Vec<u8>,
}

/// A primitive HTTP server.
pub struct HttpServer {
    listener: TcpListener,
    /// A sequence of activities that the server should expect, and the
    /// responses it should give.
    activities: Vec<Activity>,
    /// Which activity in `activities` is currently being processed.
    current: usize,
    /// Channel for sending notifications to the main thread.
    hook_transmit: mpsc::Sender<Activity>,
}

/// A reference on how to connect to the test HTTP server.
pub struct HttpServerHandle {
    pub addr: SocketAddr,
    /// Channel for receiving notifications from the server.
    pub hook_recv: mpsc::Receiver<Activity>,
}

impl Drop for HttpServerHandle {
    fn drop(&mut self) {
        if let Ok(mut stream) = TcpStream::connect(self.addr) {
            // shut down the server
            let _ = stream.write_all(b"STOP");
            let _ = stream.flush();
        }
    }
}

/// Enables logging if `TRIAGEBOT_TEST_LOG` is set. This can help with
/// debugging a test.
pub fn maybe_enable_logging() {
    const LOG_VAR: &str = "TRIAGEBOT_TEST_LOG";
    use std::sync::Once;
    static DO_INIT: Once = Once::new();
    if std::env::var_os(LOG_VAR).is_some() {
        DO_INIT.call_once(|| {
            dotenv::dotenv().ok();
            tracing_subscriber::fmt::Subscriber::builder()
                .with_env_filter(tracing_subscriber::EnvFilter::from_env(LOG_VAR))
                .with_ansi(std::env::var_os("DISABLE_COLOR").is_none())
                .try_init()
                .unwrap();
        });
    }
}

/// Makes sure recording is only being done for one test (recording multiple
/// tests isn't supported).
pub fn assert_single_record() {
    static RECORDING: AtomicBool = AtomicBool::new(false);
    if test_record::is_recording() {
        if RECORDING.swap(true, Ordering::SeqCst) {
            panic!(
                "More than one test appears to be recording.\n\
                TRIAGEBOT_TEST_RECORD only supports recording one test at a time.\n\
                Make sure to pass the exact name of the test to `cargo test` with the \
                `--exact` flag to run only one test."
            );
        }
    }
}

/// Loads all JSON [`Activity`] blobs from a directory.
pub fn load_activities(test_dir: &str, test_path: &str) -> Vec<Activity> {
    let mut activity_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    activity_path.push(test_dir);
    activity_path.push(test_path);
    let mut activity_paths: Vec<_> = std::fs::read_dir(&activity_path)
        .map_err(|e| {
            format!(
                "failed to read test activity directory {activity_path:?}: {e}\n\
                Be sure to set the environment variable TRIAGEBOT_TEST_RECORD to the \
                path to record the initial test data against the live GitHub site.\n\
                See the test docs in testsuite.rs for more."
            )
        })
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().and_then(|p| p.to_str()) == Some("json"))
        .map(|entry| entry.path())
        .collect();
    if activity_paths.is_empty() {
        panic!("expected at least one activity in {activity_path:?}");
    }
    activity_paths.sort();
    activity_paths
        .into_iter()
        .map(|path| {
            let contents = std::fs::read_to_string(path).unwrap();
            let mut activity = serde_json::from_str(&contents).unwrap();
            if let Activity::Request {
                path,
                response_body,
                ..
            } = &mut activity
            {
                if path == "/v1/teams.json" {
                    let mut teams_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
                    teams_path.push("tests/server_test/shared/teams.json");
                    let body = std::fs::read_to_string(teams_path).unwrap();
                    let body = serde_json::from_str(&body).unwrap();
                    *response_body = body;
                }
            }
            activity
        })
        .collect()
}

impl HttpServer {
    /// Creates the server and launches it in the background.
    pub fn new(activities: Vec<Activity>) -> HttpServerHandle {
        let (hook_transmit, hook_recv) = mpsc::channel();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let mut server = HttpServer {
            listener,
            activities,
            current: 0,
            hook_transmit,
        };
        std::thread::spawn(move || server.start());
        HttpServerHandle { addr, hook_recv }
    }

    fn start(&mut self) {
        if let Some(activity @ Activity::Webhook { .. }) = self.activities.get(0) {
            self.hook_transmit.send(activity.clone()).unwrap();
            self.current += 1;
        }
        let mut line = String::new();
        'server: loop {
            let (socket, _) = self.listener.accept().unwrap();
            let mut buf = BufReader::new(socket);
            line.clear();
            if buf.read_line(&mut line).unwrap() == 0 {
                // Connection terminated.
                eprintln!("unexpected client drop");
                continue;
            }
            // Read the "GET path HTTP/1.1" line.
            let mut parts = line.split_ascii_whitespace();
            let method = parts.next().unwrap().to_ascii_uppercase();
            if method == "STOP" {
                // Shutdown the server.
                return;
            }
            let path = parts.next().unwrap();
            // The host here doesn't matter, we're just interested in parsing
            // the query string.
            let url = Url::parse(&format!("https://api.github.com{path}")).unwrap();

            let mut headers = HashMap::new();
            let mut content_len = None;
            loop {
                line.clear();
                if buf.read_line(&mut line).unwrap() == 0 {
                    continue 'server;
                }
                if line == "\r\n" {
                    // End of headers.
                    line.clear();
                    break;
                }
                let (name, value) = line.split_once(':').unwrap();
                let name = name.trim().to_ascii_lowercase();
                let value = value.trim().to_string();
                match name.as_str() {
                    "content-length" => content_len = Some(value.parse::<u64>().unwrap()),
                    _ => {}
                }
                headers.insert(name, value);
            }
            let mut body = vec![0u8; content_len.unwrap_or(0) as usize];
            buf.read_exact(&mut body).unwrap();

            let path = url.path().to_string();
            let query = url.query().map(|s| s.to_string());
            eprintln!("got request {method} {path}");
            let request = Request {
                path,
                method,
                query,
                headers,
                body,
            };
            let response = self.process_request(request);

            let buf = buf.get_mut();
            write!(buf, "HTTP/1.1 {}\r\n", response.code).unwrap();
            write!(buf, "Content-Length: {}\r\n", response.body.len()).unwrap();
            write!(buf, "Connection: close\r\n").unwrap();
            for header in response.headers {
                write!(buf, "{}\r\n", header).unwrap();
            }
            write!(buf, "\r\n").unwrap();
            buf.write_all(&response.body).unwrap();
            buf.flush().unwrap();
            self.next_activity();
        }
    }

    fn next_activity(&mut self) {
        loop {
            self.current += 1;
            match self.activities.get(self.current) {
                Some(activity @ Activity::Webhook { .. }) => {
                    self.hook_transmit.send(activity.clone()).unwrap();
                }
                Some(_) => break,
                None => {
                    self.hook_transmit.send(Activity::Finished).unwrap();
                    break;
                }
            }
        }
    }

    fn process_request(&self, request: Request) -> Response {
        let Some(activity) = self.activities.get(self.current) else {
            let msg = format!("error: not enough activities\n\
                Make sure the activity log is complete.\n\
                Request was {request:?}\n");
            self.report_err(&msg);
            return Response {
                code: 500,
                headers: Vec::new(),
                body: msg.into(),
            };
        };
        match activity {
            Activity::Webhook { .. } => {
                panic!("unexpected webhook")
            }
            Activity::Request {
                method,
                path,
                query,
                request_body,
                response_code,
                response_body,
            } => {
                if method != &request.method || path != &request.path {
                    return self.report_err(&format!(
                        "expected next request to be {method} {path},\n\
                        got {} {}",
                        request.method, request.path
                    ));
                }
                if query != &request.query {
                    return self.report_err(&format!(
                        "query string does not match\n\
                        expected: {query:?}\n\
                        got: {:?}\n",
                        request.query
                    ));
                }
                if request_body.as_bytes() != request.body {
                    return self.report_err(&format!(
                        "expected next request {method} {path} to have body:\n\
                        {request_body}\n\
                        \n\
                        got:\n\
                        {}",
                        String::from_utf8_lossy(&request.body)
                    ));
                }
                let body = match response_body {
                    // We overload the meaning of a string to be a raw string.
                    // I don't think GitHub's API ever returns a string as a response.
                    serde_json::Value::String(s) => s.as_bytes().to_vec(),
                    _ => serde_json::to_vec(response_body).unwrap(),
                };
                return Response {
                    code: *response_code,
                    headers: Vec::new(),
                    body,
                };
            }
            Activity::Error { .. } | Activity::Finished => {
                panic!("unexpected activity: {activity:?}");
            }
        }
    }

    fn report_err(&self, message: &str) -> Response {
        eprintln!("error: {message}");
        self.hook_transmit
            .send(Activity::Error {
                message: message.to_string(),
            })
            .unwrap();
        Response {
            code: 500,
            headers: Vec::new(),
            body: Vec::from(message.as_bytes()),
        }
    }
}
