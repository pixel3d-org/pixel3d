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

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

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

/// The running server: the address it listens on and the request queue.
pub struct Server {
    addr: SocketAddr,
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
    /// Listens on 127.0.0.1:`port` (0 picks a free port).
    pub fn start(port: u16) -> Result<Server, ControlError> {
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        let addr = listener.local_addr()?;
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
                let (tx, open, waker) = (tx.clone(), Arc::clone(&open), Arc::clone(&waker2));
                let spawned = std::thread::Builder::new().name("control client".into()).spawn(move || {
                    if let Err(e) = serve(stream, &tx, &waker) {
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
        Ok(Server { addr, requests: Mutex::new(rx), stop, waker })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
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
        // Wake the accept loop so it sees the flag.
        let _ = TcpStream::connect(self.addr);
    }
}

/// Serves one client: one request per line, answered in order.
fn serve(stream: TcpStream, tx: &Sender<Queued>, waker: &Waker) -> std::io::Result<()> {
    let mut out = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
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
        let response = answer(&line, tx, waker);
        writeln!(out, "{response}")?;
        out.flush()?;
    }
}

/// Queues one request line and waits for its answer.
fn answer(line: &str, tx: &Sender<Queued>, waker: &Waker) -> Value {
    let req: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => return json!({ "id": null, "ok": false, "error": format!("not JSON: {e}"), "status": "not-run" }),
    };
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
    pub fn connect(addr: SocketAddr) -> Result<Client, ControlError> {
        let stream = TcpStream::connect(addr)?;
        stream.set_read_timeout(Some(TIMEOUT + Duration::from_secs(5)))?;
        Ok(Client { writer: stream.try_clone()?, reader: BufReader::new(stream), next_id: 1 })
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
        let mut c = Client::connect(addr).unwrap();
        assert_eq!(c.call("ui.inspect", json!({ "a": 1 })).unwrap(), json!({ "echo": "ui.inspect", "params": { "a": 1 } }));
        match c.call("fail", Value::Null) {
            Err(ControlError::Failed { error, status }) => assert_eq!((error.as_str(), status.as_str()), ("refused", "failed")),
            other => panic!("{other:?}"),
        }
        let _server = app.join().unwrap();
        // Garbage gets a protocol answer, not a hang.
        let mut s = TcpStream::connect(addr).unwrap();
        writeln!(s, "not json").unwrap();
        let mut line = String::new();
        BufReader::new(s).read_line(&mut line).unwrap();
        let v: Value = serde_json::from_str(&line).unwrap();
        assert_eq!((v["ok"].clone(), v["status"].clone()), (json!(false), json!("not-run")));
    }
}
