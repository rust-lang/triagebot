//! Utility code to help writing triagebot tests.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::net::{SocketAddr, TcpListener};
use std::sync::{Arc, Mutex};
use url::Url;

/// The callback type for HTTP route handlers.
pub type RequestCallback = Box<dyn Send + Fn(Request) -> Response>;

/// HTTP method.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Method {
    GET,
    POST,
    PUT,
    DELETE,
    PATCH,
}

impl Method {
    fn from_str(s: &str) -> Method {
        match s {
            "GET" => Method::GET,
            "POST" => Method::POST,
            "PUT" => Method::PUT,
            "DELETE" => Method::DELETE,
            "PATCH" => Method::PATCH,
            _ => panic!("unexpected HTTP method {s}"),
        }
    }
}

/// A builder for preparing a test.
#[derive(Default)]
pub struct TestBuilder {
    pub config: Option<&'static str>,
    pub api_handlers: HashMap<(Method, &'static str), RequestCallback>,
    pub raw_handlers: HashMap<(Method, &'static str), RequestCallback>,
}

/// A request received on the HTTP server.
#[derive(Clone, Debug)]
pub struct Request {
    /// The path of the request, such as `repos/rust-lang/rust/labels`.
    pub path: String,
    /// The HTTP method.
    pub method: Method,
    /// Components in the path that were captured with the `{foo}` syntax.
    /// See [`TestBuilder::api_handler`] for details.
    pub components: HashMap<String, String>,
    /// The query components of the URL (the stuff after `?`).
    pub query: Vec<(String, String)>,
    /// HTTP headers.
    pub headers: HashMap<String, String>,
    /// The body of the HTTP request (usually a JSON blob).
    pub body: Vec<u8>,
}

impl Request {
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap()
    }
    pub fn body_str(&self) -> String {
        String::from_utf8(self.body.clone()).unwrap()
    }

    pub fn query_string(&self) -> String {
        let vs: Vec<_> = self.query.iter().map(|(k, v)| format!("{k}={v}")).collect();
        vs.join("&")
    }
}

/// The response the HTTP server should send to the client.
pub struct Response {
    pub code: u32,
    pub headers: Vec<String>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn new() -> Response {
        Response {
            code: 200,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    pub fn new_from_path(path: &str) -> Response {
        Response {
            code: 200,
            headers: Vec::new(),
            body: std::fs::read(path).unwrap(),
        }
    }

    pub fn body(mut self, file: &[u8]) -> Self {
        self.body = Vec::from(file);
        self
    }
}

/// A recording of HTTP requests which can then be validated they are
/// performed in the correct order.
///
/// A copy of this is shared among the different HTTP servers. At the end of
/// the test, the test should call `assert_eq` to validate the correct actions
/// were performed.
#[derive(Clone)]
pub struct Events(Arc<Mutex<Vec<(Method, String)>>>);

impl Events {
    pub fn new() -> Events {
        Events(Arc::new(Mutex::new(Vec::new())))
    }

    fn push(&self, method: Method, path: String) {
        let mut es = self.0.lock().unwrap();
        es.push((method, path));
    }

    pub fn assert_eq(&self, expected: &[(Method, &str)]) {
        let es = self.0.lock().unwrap();
        for (actual, expected) in es.iter().zip(expected.iter()) {
            if actual.0 != expected.0 || actual.1 != expected.1 {
                panic!("expected request to {expected:?}, but next event was {actual:?}");
            }
        }
        if es.len() > expected.len() {
            panic!(
                "got unexpected extra requests, \
                make sure the event assertion lists all events\n\
                Extras are: {:?} ",
                &es[expected.len()..]
            );
        } else if es.len() < expected.len() {
            panic!(
                "expected additional requests that were never made, \
                make sure the event assertion lists the correct requests\n\
                Extra expected are: {:?}",
                &expected[es.len()..]
            );
        }
    }
}

/// A primitive HTTP server.
pub struct HttpServer {
    listener: TcpListener,
    /// Handlers to call for specific routes.
    handlers: HashMap<(Method, &'static str), RequestCallback>,
    /// A recording of all API requests.
    events: Events,
}

/// A reference on how to connect to the test HTTP server.
pub struct HttpServerHandle {
    pub addr: SocketAddr,
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

impl TestBuilder {
    /// Sets the config for the `triagebot.toml` file for the `rust-lang/rust`
    /// repository.
    pub fn config(mut self, config: &'static str) -> Self {
        self.config = Some(config);
        self
    }

    /// Adds an HTTP handler for https://api.github.com/
    ///
    /// The `path` is the route, like `repos/rust-lang/rust/labels`. A generic
    /// route can be configured using curly braces. For example, to get all
    /// requests for labels, use a path like `repos/rust-lang/rust/{label}`.
    /// The value of the path component can be found in
    /// [`Request::components`].
    ///
    /// If the path ends with `{...}`, then this means "the rest of the path".
    /// The rest of the path can be obtained from the `...` value in the
    /// `Request::components` map.
    pub fn api_handler<R: 'static + Send + Fn(Request) -> Response>(
        mut self,
        method: Method,
        path: &'static str,
        responder: R,
    ) -> Self {
        self.api_handlers
            .insert((method, path), Box::new(responder));
        self
    }

    /// Adds an HTTP handler for https://raw.githubusercontent.com
    pub fn raw_handler<R: 'static + Send + Fn(Request) -> Response>(
        mut self,
        method: Method,
        path: &'static str,
        responder: R,
    ) -> Self {
        self.raw_handlers
            .insert((method, path), Box::new(responder));
        self
    }

    /// Enables logging if `TRIAGEBOT_TEST_LOG` is set. This can help with
    /// debugging a test.
    pub fn maybe_enable_logging(&self) {
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
}

impl HttpServer {
    pub fn new(
        handlers: HashMap<(Method, &'static str), RequestCallback>,
        events: Events,
    ) -> HttpServerHandle {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = HttpServer {
            listener,
            handlers,
            events,
        };
        std::thread::spawn(move || server.start());
        HttpServerHandle { addr }
    }

    fn start(&self) {
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

            let method = Method::from_str(&method);
            self.events.push(method, url.path().to_string());
            let response = self.route(method, &url, headers, body);

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
        }
    }

    /// Route the request
    fn route(
        &self,
        method: Method,
        url: &Url,
        headers: HashMap<String, String>,
        body: Vec<u8>,
    ) -> Response {
        eprintln!("route {method:?} {url}",);
        let query = url
            .query_pairs()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let segments: Vec<_> = url.path_segments().unwrap().collect();
        let path = url.path().to_string();
        for ((route_method, route_pattern), responder) in &self.handlers {
            if *route_method != method {
                continue;
            }
            if let Some(components) = match_route(route_pattern, &segments) {
                let request = Request {
                    method,
                    path,
                    query,
                    components,
                    headers,
                    body,
                };
                tracing::debug!("request={request:?}");
                return responder(request);
            }
        }
        eprintln!(
            "route {method:?} {url} has no handler.\n\
            Add a handler to the context for this route."
        );
        Response {
            code: 404,
            headers: Vec::new(),
            body: b"404 not found".to_vec(),
        }
    }
}

fn match_route(route_pattern: &str, segments: &[&str]) -> Option<HashMap<String, String>> {
    let mut segments = segments.into_iter();
    let mut components = HashMap::new();
    for part in route_pattern.split('/') {
        if part == "{...}" {
            let rest: Vec<_> = segments.map(|s| *s).collect();
            components.insert("...".to_string(), rest.join("/"));
            return Some(components);
        }
        match segments.next() {
            None => return None,
            Some(actual) => {
                if part.starts_with('{') {
                    let part = part[1..part.len() - 1].to_string();
                    components.insert(part, actual.to_string());
                } else if *actual != part {
                    return None;
                }
            }
        }
    }
    if segments.next().is_some() {
        return None;
    }
    Some(components)
}
