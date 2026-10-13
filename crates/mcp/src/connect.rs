//! `pixel3d-cli mcp --connect`: the tools act on a running desktop app
//! through its control server (project spec 17.14), so the user watches the
//! agent work live. The app answers [`METHOD`] with the same [`Agent`] on
//! its own session ([`answer`]); three tools exist only when connected and
//! read or drive the interface: `ui_state`, `ui_command` and
//! `ui_screenshot`.

use std::net::{SocketAddr, ToSocketAddrs};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::Arc;

use p3d_control::{Client, ControlError, DEFAULT_PORT};
use p3d_engine::Session;
use p3d_ops::{ErrorCode, OpError, parse_params};
use serde_json::{Value, json};

use crate::agent::{Agent, ToolInfo, ToolOutput, schema};
use crate::args::{UiCommandArgs, UiScreenshotArgs, UiStateArgs};

/// The control method the app answers tool calls on.
pub const METHOD: &str = "mcp.call";

/// The tools only a connected server has, after the shared ones.
pub fn ui_tools() -> Vec<ToolInfo> {
    vec![
        ToolInfo {
            name: "ui_state",
            description: "The desktop app's interface state: mode, tool, workspace, selection, picked elements, view and transform settings.",
            schema: schema::<UiStateArgs>(),
        },
        ToolInfo {
            name: "ui_command",
            description: "Run a desktop command as its menus do (ui.mode, ui.workspace, view.frame, file.save, any op id); no id lists them.",
            schema: schema::<UiCommandArgs>(),
        },
        ToolInfo {
            name: "ui_screenshot",
            description: "A PNG of the desktop app's window (or only its viewport) to see what the user sees.",
            schema: schema::<UiScreenshotArgs>(),
        },
    ]
}

/// Reads the `--connect` address: `PORT`, `HOST` or `HOST:PORT`, with
/// 127.0.0.1 and [`DEFAULT_PORT`] filling in what is missing.
pub fn parse_addr(s: &str) -> Result<SocketAddr, String> {
    let s = s.trim();
    let spec = if let Ok(port) = s.parse::<u16>() {
        format!("127.0.0.1:{port}")
    } else if s.contains(':') {
        s.to_owned()
    } else {
        format!("{s}:{DEFAULT_PORT}")
    };
    let mut addrs = spec.to_socket_addrs().map_err(|e| format!("`{s}` is not an address ({e}); give PORT or HOST:PORT"))?;
    addrs.next().ok_or_else(|| format!("`{s}` resolves to no address"))
}

/// An agent for a host that owns its session and lends it per call.
pub fn host_agent(session: &Session) -> Agent {
    Agent::new(Session::with_registry(Arc::clone(session.registry())))
}

/// The app's side of [`METHOD`]: runs `params.tool` with `params.args` on
/// the host's `session` and answers in the form [`Remote`] reads.
pub fn answer(agent: &mut Agent, session: &mut Session, params: &Value) -> Result<Value, String> {
    let tool = params.get("tool").and_then(Value::as_str).ok_or("tool is required")?;
    let args = params.get("args").filter(|a| !a.is_null()).cloned().unwrap_or_else(|| json!({}));
    agent.swap_session(session);
    let out = catch_unwind(AssertUnwindSafe(|| agent.call(tool, args)));
    // Hand the session back before anything else, even after a panic.
    agent.swap_session(session);
    let out = out.unwrap_or_else(|_| ToolOutput::error(&OpError::internal(format!("tool {tool} failed unexpectedly; the scene is unchanged"))));
    Ok(out.to_wire())
}

/// Forwards tool calls to the app at one address; connects again on the
/// next call after the app restarted.
#[derive(Debug)]
pub struct Remote {
    addr: SocketAddr,
    /// A fixed token (tests); otherwise the one the app published, read
    /// again on every reconnect since a restarted app has a new one.
    token: Option<String>,
    client: Option<Client>,
    shots: u64,
}

impl Remote {
    /// Connects now with the token the app published, so a wrong address
    /// or a closed app fails at startup.
    pub fn connect(addr: SocketAddr) -> Result<Remote, ControlError> {
        Ok(Remote { addr, token: None, client: Some(Client::connect_published(addr)?), shots: 0 })
    }

    /// Connects with a known token (for hosts that start their own server).
    pub fn connect_with_token(addr: SocketAddr, token: &str) -> Result<Remote, ControlError> {
        Ok(Remote { addr, token: Some(token.to_owned()), client: Some(Client::connect(addr, token)?), shots: 0 })
    }

    fn open(&self) -> Result<Client, ControlError> {
        match &self.token {
            Some(t) => Client::connect(self.addr, t),
            None => Client::connect_published(self.addr),
        }
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Calls a tool in the app. Errors come back as an error output.
    pub fn call(&mut self, tool: &str, args: Value) -> ToolOutput {
        let r = match tool {
            "ui_state" => parse_params::<UiStateArgs>(&args).and_then(|_| self.request("ui.state", Value::Null)).map(|state| {
                let text = serde_json::to_string(&state).unwrap_or_default();
                ToolOutput::ok(text, state)
            }),
            "ui_command" => parse_params::<UiCommandArgs>(&args).and_then(|a| self.command(a)),
            "ui_screenshot" => parse_params::<UiScreenshotArgs>(&args).and_then(|a| self.screenshot(&a)),
            _ => self.request(METHOD, json!({ "tool": tool, "args": args })).and_then(|v| {
                ToolOutput::from_wire(&v)
                    .ok_or_else(|| OpError::internal("the app answered in another form; are the app and pixel3d-cli the same version?"))
            }),
        };
        r.unwrap_or_else(|e| ToolOutput::error(&e))
    }

    fn command(&mut self, a: UiCommandArgs) -> Result<ToolOutput, OpError> {
        let Some(id) = a.id else {
            let list = self.request("ui.commands", Value::Null)?;
            let lines: Vec<String> = list
                .get("commands")
                .and_then(Value::as_array)
                .map(|cs| {
                    cs.iter()
                        .map(|c| {
                            let s = |k: &str| c.get(k).and_then(Value::as_str).unwrap_or("");
                            match c.get("params").filter(|p| p.as_object().is_some_and(|m| !m.is_empty())) {
                                Some(p) => format!("{} {p} ({}): {}", s("id"), s("label"), s("desc")),
                                None => format!("{} ({}): {}", s("id"), s("label"), s("desc")),
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            return Ok(ToolOutput::ok(lines.join("\n"), list));
        };
        let params = if a.params.is_null() { json!({}) } else { a.params };
        let result = self.request("ui.command", json!({ "id": id, "params": params }))?;
        Ok(ToolOutput::ok(format!("ok {id}: {}", serde_json::to_string(&result).unwrap_or_default()), result))
    }

    fn screenshot(&mut self, a: &UiScreenshotArgs) -> Result<ToolOutput, OpError> {
        self.shots += 1;
        let path = std::env::temp_dir().join(format!("pixel3d-mcp-{}-{}.png", std::process::id(), self.shots));
        let method = if a.viewport_only { "ui.render" } else { "ui.screenshot" };
        let r = self.request(method, json!({ "path": path.display().to_string() }))?;
        let written = r.get("path").and_then(Value::as_str).map_or(path, PathBuf::from);
        let png =
            std::fs::read(&written).map_err(|e| OpError::new(ErrorCode::Io, format!("cannot read the screenshot {}: {e}", written.display())))?;
        // Best effort: a leftover file in the temp folder is harmless.
        let _ = std::fs::remove_file(&written);
        let what = if a.viewport_only { "viewport" } else { "window" };
        let text = match png_size(&png) {
            Some((w, h)) => format!("{what} screenshot, {w} x {h} px"),
            None => format!("{what} screenshot"),
        };
        Ok(ToolOutput { text, structured: json!({ "what": what, "bytes": png.len() }), images: vec![png], is_error: false })
    }

    /// One control request; connects first when the last call lost the app.
    fn request(&mut self, method: &str, params: Value) -> Result<Value, OpError> {
        let addr = self.addr;
        let lost = |e: &dyn std::fmt::Display| {
            OpError::new(ErrorCode::Io, format!("no desktop app at {addr}: {e}"))
                .with_hint(format!("start it with `pixel3d --control {}` or turn on File > Allow agents to connect, then call again", addr.port()))
                .retryable()
        };
        let mut client = match self.client.take() {
            Some(c) => c,
            None => self.open().map_err(|e| lost(&e))?,
        };
        match client.call(method, params) {
            Ok(v) => {
                self.client = Some(client);
                Ok(v)
            }
            Err(ControlError::Failed { error, status }) => {
                self.client = Some(client);
                Err(match status.as_str() {
                    "not-run" => OpError::new(ErrorCode::Timeout, format!("the app was busy and did not run it: {error}")).retryable(),
                    "may-have-run" => OpError::new(ErrorCode::Timeout, format!("the app did not answer in time and the call may have run: {error}"))
                        .with_hint("check with scene_overview or history before calling again"),
                    _ => OpError::new(ErrorCode::InvalidParam, error),
                })
            }
            // The connection is gone (or out of step): the next call connects again.
            Err(e) => Err(lost(&e)),
        }
    }
}

/// Width and height from a PNG's header.
fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    let be = |at: usize| png.get(at..at + 4).and_then(|b| b.try_into().ok()).map(u32::from_be_bytes);
    (png.get(1..4) == Some(b"PNG")).then_some(())?;
    Some((be(16)?, be(20)?))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    #[test]
    fn addresses_fill_in_host_and_port() {
        assert_eq!(parse_addr("7000").unwrap(), "127.0.0.1:7000".parse::<std::net::SocketAddr>().unwrap());
        assert_eq!(parse_addr("127.0.0.1:7001").unwrap(), "127.0.0.1:7001".parse::<std::net::SocketAddr>().unwrap());
        assert_eq!(parse_addr("127.0.0.1").unwrap(), format!("127.0.0.1:{DEFAULT_PORT}").parse::<std::net::SocketAddr>().unwrap());
        assert!(parse_addr("not an address:x").is_err());
    }

    #[test]
    fn png_header_gives_the_size() {
        let mut png = vec![0x89, b'P', b'N', b'G', 13, 10, 26, 10, 0, 0, 0, 13, b'I', b'H', b'D', b'R'];
        png.extend(640u32.to_be_bytes());
        png.extend(480u32.to_be_bytes());
        assert_eq!(png_size(&png), Some((640, 480)));
        assert_eq!(png_size(b"not a png"), None);
    }

    #[test]
    fn host_lends_its_session_for_one_call() {
        let mut session = Session::new();
        let mut agent = host_agent(&session);
        let made =
            answer(&mut agent, &mut session, &json!({ "tool": "run", "args": { "ops": [{ "op": "prim.box", "args": { "name": "crate" } }] } }))
                .unwrap();
        let made = ToolOutput::from_wire(&made).unwrap();
        assert!(!made.is_error, "{}", made.text);
        // The host has the change; the agent holds only its spare session.
        assert_eq!(session.revision(), 1);
        assert_eq!(agent.session().doc().object_count(), 0);
        let seen = ToolOutput::from_wire(&answer(&mut agent, &mut session, &json!({ "tool": "scene_overview" })).unwrap()).unwrap();
        assert!(seen.text.contains("crate"), "{}", seen.text);
        let bad = ToolOutput::from_wire(&answer(&mut agent, &mut session, &json!({ "tool": "nope" })).unwrap()).unwrap();
        assert!(bad.is_error);
        assert!(answer(&mut agent, &mut session, &json!({})).is_err());
    }

    #[test]
    fn wire_form_round_trips() {
        let out = ToolOutput { text: "t".into(), structured: json!({ "a": 1 }), images: vec![vec![1, 2, 3]], is_error: true };
        assert_eq!(ToolOutput::from_wire(&out.to_wire()), Some(out));
        assert_eq!(ToolOutput::from_wire(&json!({ "nope": 1 })), None);
    }
}
