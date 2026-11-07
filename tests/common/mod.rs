#![allow(dead_code)]

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;

use assert_cmd::Command;
use tiny_http::{Header, Server};

pub struct Reply {
    pub status: u16,
    pub headers: Vec<(&'static str, String)>,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn ok(content_type: &str, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: 200,
            headers: vec![("Content-Type", content_type.into())],
            body: body.into(),
        }
    }

    pub fn status(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    pub fn header(mut self, name: &'static str, value: &str) -> Self {
        self.headers.push((name, value.into()));
        self
    }

    pub fn body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }
}

pub struct Seen {
    pub path: String,
    pub headers: Vec<(String, String)>,
}

impl Seen {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

pub type Handler = Box<dyn Fn(&Seen) -> Reply + Send + Sync>;

pub fn always(reply: impl Fn() -> Reply + Send + Sync + 'static) -> Handler {
    Box::new(move |_| reply())
}

pub struct TestServer {
    base: String,
    server: Arc<Server>,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl TestServer {
    pub fn start(routes: Vec<(&'static str, Handler)>) -> Self {
        let server = Arc::new(Server::http("127.0.0.1:0").unwrap());
        let base = format!("http://{}", server.server_addr().to_ip().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (srv, log) = (Arc::clone(&server), Arc::clone(&seen));
        thread::spawn(move || {
            for request in srv.incoming_requests() {
                let path = request.url().split('?').next().unwrap_or("").to_owned();
                let record = Seen {
                    path: path.clone(),
                    headers: request
                        .headers()
                        .iter()
                        .map(|h| (h.field.to_string(), h.value.to_string()))
                        .collect(),
                };
                let reply = routes
                    .iter()
                    .find(|(p, _)| *p == path)
                    .map_or_else(|| Reply::status(404), |(_, handler)| handler(&record));
                log.lock().unwrap().push(record);
                let mut response =
                    tiny_http::Response::from_data(reply.body).with_status_code(reply.status);
                for (name, value) in reply.headers {
                    response.add_header(Header::from_bytes(name, value).unwrap());
                }
                let _ = request.respond(response);
            }
        });
        Self { base, server, seen }
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    pub fn hits(&self, path: &str) -> usize {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.path == path)
            .count()
    }

    pub fn last_header(&self, path: &str, name: &str) -> Option<String> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|s| s.path == path)
            .and_then(|s| s.header(name).map(str::to_owned))
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.server.unblock();
    }
}

pub const TEST_AGENT: &str = "TestAgent/1.0 (+mailto:tests@example.com)";

pub fn noted(db: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_noted"));
    cmd.env("NOTED_DB", db)
        .env("NOTED_CONFIG_DIR", config_dir(db))
        .env("NOTED_USER_AGENT", TEST_AGENT);
    cmd
}

pub fn config_dir(db: &Path) -> std::path::PathBuf {
    db.with_extension("config")
}

pub fn answer_once_then_reset(body: &'static str) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/page", listener.local_addr().unwrap());
    thread::spawn(move || {
        let mut connections = listener.incoming();
        if let Some(Ok(mut stream)) = connections.next() {
            let mut request = [0; 4096];
            let _ = std::io::Read::read(&mut stream, &mut request);
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = std::io::Write::write_all(&mut stream, reply.as_bytes());
        }
        for stream in connections {
            drop(stream);
        }
    });
    url
}
