//! MCP contract tests: the real `pixel3d-cli mcp` process over stdio, with
//! every response snapshotted (project spec 22.1).

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

struct Client {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl Client {
    fn start() -> Client {
        let mut child = Command::new(env!("CARGO_BIN_EXE_pixel3d-cli"))
            .arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn pixel3d-cli mcp");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = BufReader::new(child.stdout.take().expect("stdout"));
        Client { child, stdin, stdout, next_id: 1 }
    }

    fn send(&mut self, msg: &Value) {
        writeln!(self.stdin, "{msg}").expect("write");
        self.stdin.flush().expect("flush");
    }

    /// Sends a request and returns its response.
    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        loop {
            let mut line = String::new();
            let n = self.stdout.read_line(&mut line).expect("read");
            assert!(n > 0, "the server closed stdout");
            let v: Value = serde_json::from_str(&line).expect("one JSON message per line");
            if v["id"] == json!(id) {
                return v;
            }
        }
    }

    fn call(&mut self, tool: &str, args: Value) -> Value {
        self.request("tools/call", json!({ "name": tool, "arguments": args }))
    }

    fn initialize(&mut self) -> Value {
        let r = self.request(
            "initialize",
            json!({ "protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": { "name": "contract-test", "version": "1" } }),
        );
        self.send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
        r
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Masks run timings so snapshots are stable.
fn scrub(v: &mut Value) {
    match v {
        Value::String(s) => {
            *s = s
                .lines()
                .map(|l| match (l.find(", "), l.find(" ms)")) {
                    (Some(a), Some(b)) if l.starts_with("ok rev") && a < b => format!("{}, _{}", &l[..a], &l[b..]),
                    _ => l.to_owned(),
                })
                .collect::<Vec<_>>()
                .join("\n")
                + if s.ends_with('\n') { "\n" } else { "" };
        }
        Value::Array(a) => a.iter_mut().for_each(scrub),
        Value::Object(o) => {
            if let Some(ms) = o.get_mut("ms") {
                *ms = json!("_");
            }
            o.values_mut().for_each(scrub);
        }
        _ => {}
    }
}

#[test]
fn contract() {
    let mut c = Client::start();
    let mut init = c.initialize();
    init["result"]["serverInfo"]["version"] = json!("_");
    insta::assert_json_snapshot!("initialize", init);
    let tools = c.request("tools/list", json!({}));
    insta::assert_json_snapshot!("tools_list", tools);

    // A beveled crate in three calls: build with tests, describe, diagnose.
    let mut transcript = Vec::new();
    for (tool, args) in [
        (
            "run",
            json!({ "ops": [
                { "op": "prim.box", "args": { "name": "crate", "width": "600mm", "height": "600mm", "depth": "600mm", "bevel": { "width": "8mm", "segments": 2 } } },
                { "op": "expect.add", "args": { "expect": { "dims": { "target": "crate", "width": { "min": "590mm", "max": "610mm" } } } } },
                { "op": "expect.add", "args": { "expect": { "watertight": { "target": "crate" } } } }
            ] }),
        ),
        ("describe", json!({ "target": "crate" })),
        ("diagnose", json!({})),
        ("run", json!({ "ops": [{ "op": "prim.box", "args": { "widht": 1 } }] })),
    ] {
        let mut r = c.call(tool, args.clone());
        scrub(&mut r);
        transcript.push(json!({ "call": { "tool": tool, "arguments": args }, "response": r }));
    }
    assert!(transcript[0]["response"]["result"]["content"][0]["text"].as_str().is_some_and(|t| t.contains("tests 2/2 pass")));
    assert_eq!(transcript[3]["response"]["result"]["isError"], json!(true));
    insta::assert_json_snapshot!("crate_session", transcript);

    let resources = c.request("resources/list", json!({}));
    insta::assert_json_snapshot!("resources_list", resources);
}

#[test]
fn cold_start_to_initialized() {
    // Best of three: the first spawn also pays for loading the binary from disk.
    let mut best = Duration::MAX;
    for _ in 0..3 {
        let start = Instant::now();
        let mut c = Client::start();
        let r = c.initialize();
        assert!(r["result"]["protocolVersion"].is_string(), "{r}");
        best = best.min(start.elapsed());
    }
    eprintln!("cold start to initialize response: {best:?}");
    // Project spec 21.8: under 150 ms. Unoptimized test builds get slack.
    let limit = if cfg!(debug_assertions) { Duration::from_millis(1500) } else { Duration::from_millis(150) };
    assert!(best < limit, "cold start {best:?} over {limit:?}");
}
