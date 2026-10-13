//! Loopback JSON-lines control server and client.
//!
//! Layer L5 (frontends). See `docs/architecture.md`.
//!
//! One request per line, `{"id": 1, "method": "ui.inspect", "params": {}}`;
//! one response per line, `{"id": 1, "ok": true, "result": ...}` or
//! `{"id": 1, "ok": false, "error": "...", "status": "..."}`. The server
//! listens on 127.0.0.1 only, serves at most [`MAX_CONNECTIONS`] clients,
//! and hands requests to the app over a channel; the app answers from its
//! UI thread. A request the app does not answer within [`TIMEOUT`] gets
//! status `not-run` if the app never picked it up, `may-have-run` if it did
//! (cadcraft's protocol).
//!
//! **Access.** Loopback alone is not a boundary: any local process, and any
//! web page through a browser request, can reach 127.0.0.1. So every
//! connection must first send `{"method": "hello", "params": {"token": ...}}`
//! with the server's random token; anything else closes it. The app
//! publishes the token in a file only the user can read ([`token_path`]),
//! which `pixel3d-cli mcp --connect` reads ([`Client::connect_published`]).
//! A line that is not JSON also closes the connection, so an HTTP request
//! (its first line is `POST / HTTP/1.1`) never gets to its body.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

/// The first request of every connection.
pub const HELLO: &str = "hello";

/// Where the app publishes the token of its server on `port`:
/// `<local data dir>/control/<port>.token` (for example
/// `~/.local/share/pixel3d/control/7879.token` on Linux).
pub fn token_path(port: u16) -> Option<PathBuf> {
    // The same place as the app's own data (app id `dev.pixel3d.pixel3d`).
    let dirs = directories::ProjectDirs::from("dev", "pixel3d", "pixel3d")?;
    Some(dirs.data_local_dir().join("control").join(format!("{port}.token")))
}

/// 32 random bytes from the operating system, as hex.
fn new_token() -> Result<String, ControlError> {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).map_err(|e| ControlError::Io(std::io::Error::other(e.to_string())))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// Equal strings, compared without an early exit.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Writes a file only the current user can read (owner read and write on
/// Unix; the per-user data directory's permissions on Windows).
fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // A stale file may carry wider permissions: start fresh.
    let _ = std::fs::remove_file(path);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(text.as_bytes())
}

/// The port the app listens on when agents are allowed to connect from the
/// menu (`pixel3d-cli mcp --connect` uses it when given only a host).
pub const DEFAULT_PORT: u16 = 7879;
/// Most clients served at once.
pub const MAX_CONNECTIONS: usize = 16;
/// How long a client waits for the app to answer.
pub const TIMEOUT: Duration = Duration::from_secs(60);
/// Longest request line accepted (bytes).
pub const MAX_LINE: usize = 16 * 1024 * 1024;

/// Errors starting the server or talking to it.
#[derive(Debug, thiserror::Error)]
pub enum ControlError {
    #[error("control server I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("bad response: {0}")]
    Protocol(String),
    #[error("{error} ({status})")]
    Failed { error: String, status: String },
}

/// A request waiting for the app's answer.
pub struct Request {
    pub method: String,
    pub params: Value,
    reply: Sender<Result<Value, String>>,
}

impl Request {
    /// Sends the answer back to the client (dropped if it gave up).
    pub fn reply(self, result: Result<Value, String>) {
        // The client may have timed out and left; nothing to do then.
        let _ = self.reply.send(result);
    }
}

impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Request").field("method", &self.method).finish()
    }
}

/// A request queued with the flag that tells whether the app took it.
type Queued = (Request, Arc<AtomicBool>);

/// Wakes the app's UI thread when a request arrives.
type Waker = Arc<Mutex<Option<Box<dyn Fn() + Send>>>>;

/// The running server: the address it listens on, its access token and
/// the request queue.
pub struct Server {
    addr: SocketAddr,
    token: Arc<str>,
    published: Mutex<Option<PathBuf>>,
    requests: Mutex<Receiver<Queued>>,
    stop: Arc<AtomicBool>,
    waker: Waker,
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Server").field("addr", &self.addr).finish()
    }
}

impl Server {
    /// Listens on 127.0.0.1:`port` (0 picks a free port) with a new random
    /// token. Clients need the token: [`Server::publish`] it for
    /// `pixel3d-cli mcp --connect`, or hand [`Server::token`] over directly.
    pub fn start(port: u16) -> Result<Server, ControlError> {
        let token: Arc<str> = new_token()?.into();
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        let addr = listener.local_addr()?;
        let token2 = Arc::clone(&token);
        let (tx, rx) = mpsc::channel::<Queued>();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = Arc::clone(&stop);
        let open = Arc::new(AtomicUsize::new(0));
        let waker: Waker = Arc::new(Mutex::new(None));
        let waker2 = Arc::clone(&waker);
        std::thread::Builder::new().name("control server".into()).spawn(move || {
            for stream in listener.incoming() {
                if stop2.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                if open.load(Ordering::Relaxed) >= MAX_CONNECTIONS {
                    let mut s = stream;
                    let _ = writeln!(s, "{}", json!({ "id": null, "ok": false, "error": "too many connections", "status": "not-run" }));
                    continue;
                }
                open.fetch_add(1, Ordering::Relaxed);
                let (tx, open, waker, token) = (tx.clone(), Arc::clone(&open), Arc::clone(&waker2), Arc::clone(&token2));
                let spawned = std::thread::Builder::new().name("control client".into()).spawn(move || {
                    if let Err(e) = serve(stream, &tx, &waker, &token) {
                        tracing::debug!(error = %e, "control client left");
                    }
                    open.fetch_sub(1, Ordering::Relaxed);
                });
                if spawned.is_err() {
                    tracing::warn!("could not start a control client thread");
                }
            }
        })?;
        tracing::info!(%addr, "control server listening");
        Ok(Server { addr, token, published: Mutex::new(None), requests: Mutex::new(rx), stop, waker })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The token clients must send in their `hello`.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Writes the token to [`token_path`] for this port, readable by the
    /// current user only; the file goes away with the server.
    ///
    /// Complexity: O(1) file operations.
    pub fn publish(&self) -> Result<PathBuf, ControlError> {
        let path =
            token_path(self.addr.port()).ok_or_else(|| ControlError::Io(std::io::Error::other("no user data directory to publish the token in")))?;
        write_private(&path, &self.token)?;
        if let Ok(mut p) = self.published.lock() {
            *p = Some(path.clone());
        }
        Ok(path)
    }

    /// Sets what wakes the app when a request arrives (an idle UI does not
    /// draw frames on its own).
    pub fn set_waker(&self, wake: impl Fn() + Send + 'static) {
        if let Ok(mut w) = self.waker.lock() {
            *w = Some(Box::new(wake));
        }
    }

    /// The next waiting request, if any. Call from the UI thread.
    pub fn next(&self) -> Option<Request> {
        let rx = self.requests.lock().ok()?;
        match rx.try_recv() {
            Ok((req, taken)) => {
                taken.store(true, Ordering::Relaxed);
                Some(req)
            }
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => None,
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Ok(mut p) = self.published.lock()
            && let Some(path) = p.take()
        {
            let _ = std::fs::remove_file(path);
        }
        // Wake the accept loop so it sees the flag.
        let _ = TcpStream::connect(self.addr);
    }
}

/// Serves one client: a `hello` with the token, then one request per line,
/// answered in order. A wrong or missing token, or a line that is not JSON,
/// ends the connection.
fn serve(stream: TcpStream, tx: &Sender<Queued>, waker: &Waker, token: &str) -> std::io::Result<()> {
    let mut out = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let mut allowed = false;
    loop {
        line.clear();
        let n = (&mut reader).take(MAX_LINE as u64 + 1).read_line(&mut line)?;
        if n == 0 {
            return Ok(());
        }
        if line.len() > MAX_LINE {
            writeln!(out, "{}", json!({ "id": null, "ok": false, "error": "request line too long", "status": "not-run" }))?;
            return Ok(());
        }
        if line.trim().is_empty() {
            continue;
        }
        let req: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                writeln!(out, "{}", json!({ "id": null, "ok": false, "error": format!("not JSON: {e}"), "status": "not-run" }))?;
                return Ok(());
            }
        };
        if !allowed {
            let id = req.get("id").cloned().unwrap_or(Value::Null);
            let given = req.get("params").and_then(|p| p.get("token")).and_then(Value::as_str).unwrap_or("");
            if req.get("method").and_then(Value::as_str) != Some(HELLO) || !same(given, token) {
                let error = "send {\"method\": \"hello\", \"params\": {\"token\": ...}} first, with the token the app published";
                writeln!(out, "{}", json!({ "id": id, "ok": false, "error": error, "status": "not-run" }))?;
                return Ok(());
            }
            allowed = true;
            writeln!(out, "{}", json!({ "id": id, "ok": true, "result": { "app": "pixel3d" } }))?;
            out.flush()?;
            continue;
        }
        let response = answer(req, tx, waker);
        writeln!(out, "{response}")?;
        out.flush()?;
    }
}

/// Queues one request and waits for its answer.
fn answer(req: Value, tx: &Sender<Queued>, waker: &Waker) -> Value {
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    let Some(method) = req.get("method").and_then(Value::as_str) else {
        return json!({ "id": id, "ok": false, "error": "method is required", "status": "not-run" });
    };
    let (reply, wait) = mpsc::channel();
    let taken = Arc::new(AtomicBool::new(false));
    let request = Request { method: method.to_owned(), params: req.get("params").cloned().unwrap_or(Value::Null), reply };
    if tx.send((request, Arc::clone(&taken))).is_err() {
        return json!({ "id": id, "ok": false, "error": "the app is closing", "status": "not-run" });
    }
    if let Ok(w) = waker.lock()
        && let Some(wake) = w.as_ref()
    {
        wake();
    }
    match wait.recv_timeout(TIMEOUT) {
        Ok(Ok(result)) => json!({ "id": id, "ok": true, "result": result }),
        Ok(Err(error)) => json!({ "id": id, "ok": false, "error": error, "status": "failed" }),
        Err(_) => {
            let status = if taken.load(Ordering::Relaxed) { "may-have-run" } else { "not-run" };
            json!({ "id": id, "ok": false, "error": "the app did not answer in time", "status": status })
        }
    }
}

/// A client of a running app (tests, `pixel3d-cli mcp --connect`).
pub struct Client {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    next_id: u64,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client").field("next_id", &self.next_id).finish()
    }
}

impl Client {
    /// Connects and introduces itself with the server's token.
    pub fn connect(addr: SocketAddr, token: &str) -> Result<Client, ControlError> {
        let stream = TcpStream::connect(addr)?;
        stream.set_read_timeout(Some(TIMEOUT + Duration::from_secs(5)))?;
        let mut c = Client { writer: stream.try_clone()?, reader: BufReader::new(stream), next_id: 1 };
        c.call(HELLO, json!({ "token": token }))?;
        Ok(c)
    }

    /// Connects with the token the app published for this port
    /// ([`token_path`]); the app must be on this machine.
    pub fn connect_published(addr: SocketAddr) -> Result<Client, ControlError> {
        let path = token_path(addr.port()).ok_or_else(|| ControlError::Protocol("no user data directory to read the app's token from".into()))?;
        let token = std::fs::read_to_string(&path).map_err(|e| {
            ControlError::Io(std::io::Error::new(
                e.kind(),
                format!("no access token at {} ({e}); is the app listening on port {}?", path.display(), addr.port()),
            ))
        })?;
        Client::connect(addr, token.trim())
    }

    /// Calls `method` and returns its result.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, ControlError> {
        let id = self.next_id;
        self.next_id += 1;
        writeln!(self.writer, "{}", json!({ "id": id, "method": method, "params": params }))?;
        self.writer.flush()?;
        let mut line = String::new();
        self.reader.read_line(&mut line)?;
        let v: Value = serde_json::from_str(&line).map_err(|e| ControlError::Protocol(e.to_string()))?;
        if v.get("ok") == Some(&Value::Bool(true)) {
            return Ok(v.get("result").cloned().unwrap_or(Value::Null));
        }
        let text = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
        Err(ControlError::Failed { error: text("error"), status: text("status") })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    #[test]
    fn requests_reach_the_app_and_answers_come_back() {
        let server = Server::start(0).unwrap();
        let addr = server.addr();
        let token = server.token().to_owned();
        // The "app": answers echo the method, refuses "fail".
        let app = std::thread::spawn(move || {
            let mut answered = 0;
            while answered < 2 {
                if let Some(req) = server.next() {
                    let r = if req.method == "fail" { Err("refused".to_owned()) } else { Ok(json!({ "echo": req.method, "params": req.params })) };
                    req.reply(r);
                    answered += 1;
                } else {
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
            server
        });
        let mut c = Client::connect(addr, &token).unwrap();
        assert_eq!(c.call("ui.inspect", json!({ "a": 1 })).unwrap(), json!({ "echo": "ui.inspect", "params": { "a": 1 } }));
        match c.call("fail", Value::Null) {
            Err(ControlError::Failed { error, status }) => assert_eq!((error.as_str(), status.as_str()), ("refused", "failed")),
            other => panic!("{other:?}"),
        }
        let _server = app.join().unwrap();
    }

    /// Lines the server sends back on a raw connection until it closes.
    fn exchange(addr: SocketAddr, text: &str) -> Vec<Value> {
        let mut s = TcpStream::connect(addr).unwrap();
        s.write_all(text.as_bytes()).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        BufReader::new(s).lines().map_while(Result::ok).map(|l| serde_json::from_str(&l).unwrap()).collect()
    }

    #[test]
    fn strangers_and_web_pages_never_reach_the_app() {
        let server = Server::start(0).unwrap();
        let addr = server.addr();
        let call = r#"{"id": 1, "method": "mcp.call", "params": {"tool": "run"}}"#;
        // No hello, or a wrong token: one refusal, then the server hangs up.
        let replies = exchange(addr, &format!("{call}\n{call}\n"));
        assert_eq!(replies.len(), 1, "{replies:?}");
        assert_eq!((replies[0]["ok"].clone(), replies[0]["status"].clone()), (json!(false), json!("not-run")));
        let wrong = format!("{{\"method\": \"hello\", \"params\": {{\"token\": \"{}\"}}}}\n{call}\n", "0".repeat(64));
        assert_eq!(exchange(addr, &wrong).len(), 1);
        // A browser's POST: the request line is not JSON, so its JSON body is never read.
        let post = format!("POST / HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: text/plain\r\n\r\n{call}\n");
        let replies = exchange(addr, &post);
        assert_eq!(replies.len(), 1);
        assert!(replies[0]["error"].as_str().is_some_and(|e| e.starts_with("not JSON")), "{replies:?}");
        // Garbage after a good hello also ends the connection.
        let hello = format!("{{\"method\": \"hello\", \"params\": {{\"token\": \"{}\"}}}}\n", server.token());
        let replies = exchange(addr, &format!("{hello}not json\n{call}\n"));
        assert_eq!(replies.len(), 2, "{replies:?}");
        assert_eq!(replies[0]["result"], json!({ "app": "pixel3d" }));
        // Nothing reached the app.
        assert!(server.next().is_none());
    }

    #[test]
    fn tokens_are_random_and_files_private() {
        let (a, b) = (new_token().unwrap(), new_token().unwrap());
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
        assert!(same(&a, &a) && !same(&a, &b) && !same(&a, &a[..63]));
        let dir = std::env::temp_dir().join(format!("p3d-control-{}", std::process::id()));
        let path = dir.join("nested").join("7.token");
        write_private(&path, &a).unwrap();
        write_private(&path, &b).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), b);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(token_path(7879).is_some_and(|p| p.ends_with("control/7879.token")));
    }
}
