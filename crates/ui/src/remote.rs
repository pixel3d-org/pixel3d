//! The app's side of the control server (project spec 5.3, 20.2): agents
//! and UI tests read the state and the on-screen elements, drive the
//! interface with real input events (clicks, drags, keys, text), take
//! screenshots and run any command. Requests are drained on the UI thread
//! in `logic()`; input is fed to egui over the next frames, as a person
//! would type and click.
//!
//! Methods: `ui.inspect` (alias `ui.state`), `ui.elements`, `ui.click`,
//! `ui.drag`, `ui.key`, `ui.text`, `ui.screenshot`, `ui.render` (the
//! viewport only), `ui.command` (any command or op), `ui.commands` (the
//! list) and `mcp.call` (an MCP tool on this app's scene, for
//! `pixel3d-cli mcp --connect`).

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;

use egui::{Event, Modifiers, PointerButton, Pos2, pos2};
use p3d_control::{Request, Server};
use serde_json::{Value, json};

use crate::PixelApp;
use crate::i18n::t;

/// Most requests answered per frame.
const PER_FRAME: usize = 32;

/// The server, the input still to feed, and screenshots being taken.
#[derive(Default)]
pub struct Remote {
    server: Option<Server>,
    /// One batch of events per frame.
    input: VecDeque<Vec<Event>>,
    shots: VecDeque<(Request, PathBuf, Option<egui::Rect>)>,
    waiting: Option<(Request, PathBuf)>,
    next_shot: u64,
    /// The server wakes this app's UI thread.
    waker_set: bool,
    /// Answers `mcp.call` on the app's session (made on the first call).
    agent: Option<p3d_mcp::Agent>,
}

impl Remote {
    /// The address the server listens on.
    pub fn addr(&self) -> Option<std::net::SocketAddr> {
        self.server.as_ref().map(Server::addr)
    }
}

/// Where to put the pointer: an element `id` (hex, from `ui.elements`), an
/// element `label`, or a position `at: [x, y]` in points.
fn point(params: &Value, key: &str, elements: &[crate::control::Widget]) -> Result<Pos2, String> {
    let p = params.get(key).unwrap_or(params);
    if let Some([x, y]) = p.as_array().map(Vec::as_slice).and_then(|a| a.get(..2)).map(|a| [a[0].as_f64(), a[1].as_f64()])
        && let (Some(x), Some(y)) = (x, y)
    {
        return Ok(pos2(x as f32, y as f32));
    }
    let id = p.get("id").and_then(Value::as_str);
    let label = p.get("label").and_then(Value::as_str);
    if let Some(at) = p.get("at").and_then(Value::as_array)
        && let (Some(x), Some(y)) = (at.first().and_then(Value::as_f64), at.get(1).and_then(Value::as_f64))
    {
        return Ok(pos2(x as f32, y as f32));
    }
    let found = elements.iter().find(|w| id.is_some_and(|id| format!("{:016x}", w.id.value()) == id) || label.is_some_and(|l| w.label == l));
    match found {
        Some(w) if w.enabled => Ok(w.rect.center()),
        Some(w) => Err(format!("{} `{}` {}", t("element"), w.label, t("is disabled"))),
        None => Err(t("no element matches; pass id, label or at: [x, y] (see ui.elements)").to_owned()),
    }
}

fn modifiers(params: &Value) -> Modifiers {
    let m = params.get("modifiers");
    let on = |k: &str| m.and_then(|m| m.get(k)).and_then(Value::as_bool).unwrap_or(false);
    Modifiers {
        alt: on("alt"),
        ctrl: on("ctrl"),
        shift: on("shift"),
        mac_cmd: on("cmd") && cfg!(target_os = "macos"),
        command: on("cmd") || on("command"),
    }
}

fn button(params: &Value) -> PointerButton {
    match params.get("button").and_then(Value::as_str) {
        Some("secondary" | "right") => PointerButton::Secondary,
        Some("middle") => PointerButton::Middle,
        _ => PointerButton::Primary,
    }
}

impl PixelApp {
    /// Starts the control server on 127.0.0.1:`port` (0 picks a free one).
    pub fn start_control(&mut self, port: u16) -> Result<std::net::SocketAddr, String> {
        if let Some(addr) = self.remote.addr() {
            return Ok(addr);
        }
        let server = Server::start(port).map_err(|e| e.to_string())?;
        let addr = server.addr();
        // `pixel3d-cli mcp --connect` reads the token from this user-only
        // file; unit tests hand the token over directly instead.
        #[cfg(not(test))]
        server.publish().map_err(|e| format!("cannot publish the agent access token: {e}"))?;
        self.remote.server = Some(server);
        self.remote.waker_set = false;
        self.set_status(format!("{} {addr}", t("Agents can connect at")));
        Ok(addr)
    }

    /// Stops the control server.
    pub fn stop_control(&mut self) {
        self.remote.server = None;
    }

    /// Feeds this frame's synthetic input. Call from the raw input hook.
    pub(crate) fn feed_remote_input(&mut self, raw: &mut egui::RawInput) {
        if let Some(batch) = self.remote.input.pop_front() {
            raw.events.extend(batch);
        }
    }

    /// Answers waiting requests and finished screenshots. Call once per
    /// frame from `logic()`.
    ///
    /// Complexity: O(requests this frame x their work).
    pub(crate) fn drain_control(&mut self, ctx: &egui::Context) {
        if !self.remote.waker_set
            && let Some(server) = &self.remote.server
        {
            let ctx = ctx.clone();
            server.set_waker(move || ctx.request_repaint());
            self.remote.waker_set = true;
        }
        if !self.remote.input.is_empty() || self.remote.waiting.is_some() || !self.remote.shots.is_empty() {
            ctx.request_repaint();
        }
        // A finished screenshot answers its request.
        if let Some((req, path)) = self.remote.waiting.take() {
            match self.shots.done.take() {
                Some(r) => req.reply(r.map(|p| json!({ "path": p.display().to_string() }))),
                None => self.remote.waiting = Some((req, path)),
            }
        }
        if self.remote.waiting.is_none()
            && !self.shots.busy()
            && let Some((req, path, crop)) = self.remote.shots.pop_front()
        {
            self.shots.crop = crop;
            self.shots.request(path.clone());
            self.remote.waiting = Some((req, path));
        }
        let Some(server) = &self.remote.server else { return };
        let mut taken = Vec::new();
        while taken.len() < PER_FRAME {
            match server.next() {
                Some(r) => taken.push(r),
                None => break,
            }
        }
        let elements = crate::control::widgets(ctx);
        for req in taken {
            let params = req.params.clone();
            match req.method.as_str() {
                "ui.screenshot" | "ui.render" => {
                    let path = params.get("path").and_then(Value::as_str).map(PathBuf::from).unwrap_or_else(|| {
                        self.remote.next_shot += 1;
                        std::env::temp_dir().join(format!("pixel3d-{}-{}.png", std::process::id(), self.remote.next_shot))
                    });
                    let crop = (req.method == "ui.render").then_some(self.viewport.rect);
                    self.remote.shots.push_back((req, path, crop));
                }
                _ => {
                    let r = self.control_call(&req.method, &params, &elements);
                    req.reply(r);
                }
            }
        }
    }

    /// One control request answered right away.
    fn control_call(&mut self, method: &str, params: &Value, elements: &[crate::control::Widget]) -> Result<Value, String> {
        match method {
            "ui.inspect" | "ui.state" => Ok(self.inspect()),
            "ui.elements" => {
                let role = params.get("role").and_then(Value::as_str);
                let text = params.get("label").and_then(Value::as_str).map(str::to_lowercase);
                let list: Vec<Value> = elements
                    .iter()
                    .filter(|w| role.is_none_or(|r| w.role == r))
                    .filter(|w| text.as_ref().is_none_or(|s| w.label.to_lowercase().contains(s)))
                    .map(crate::control::widget_json)
                    .collect();
                Ok(json!({ "elements": list }))
            }
            "ui.click" => {
                let at = point(params, "target", elements)?;
                let (b, m) = (button(params), modifiers(params));
                let press = |pressed| Event::PointerButton { pos: at, button: b, pressed, modifiers: m };
                self.remote.input.extend([vec![Event::PointerMoved(at)], vec![press(true)], vec![press(false)]]);
                if params.get("double").and_then(Value::as_bool).unwrap_or(false) {
                    self.remote.input.extend([vec![press(true)], vec![press(false)]]);
                }
                Ok(json!({ "at": [at.x, at.y] }))
            }
            "ui.drag" => {
                let (from, to) = (point(params, "from", elements)?, point(params, "to", elements)?);
                let (b, m) = (button(params), modifiers(params));
                let steps = params.get("steps").and_then(Value::as_u64).unwrap_or(8).clamp(1, 120) as usize;
                self.remote.input.push_back(vec![Event::PointerMoved(from)]);
                self.remote.input.push_back(vec![Event::PointerButton { pos: from, button: b, pressed: true, modifiers: m }]);
                for i in 1..=steps {
                    self.remote.input.push_back(vec![Event::PointerMoved(from + (to - from) * (i as f32 / steps as f32))]);
                }
                // `hold` keeps the button down (to look at a drag in progress).
                if !params.get("hold").and_then(Value::as_bool).unwrap_or(false) {
                    self.remote.input.push_back(vec![Event::PointerButton { pos: to, button: b, pressed: false, modifiers: m }]);
                }
                Ok(json!({ "from": [from.x, from.y], "to": [to.x, to.y], "frames": steps + 3 }))
            }
            "ui.key" => {
                let name = params.get("key").and_then(Value::as_str).ok_or(t("key is required, for example \"Tab\" or \"A\""))?;
                let key = egui::Key::from_name(name).ok_or_else(|| format!("{} `{name}`", t("unknown key")))?;
                let m = modifiers(params);
                let ev = |pressed| Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: m };
                self.remote.input.extend([vec![ev(true)], vec![ev(false)]]);
                Ok(json!({ "key": name }))
            }
            "ui.text" => {
                let text = params.get("text").and_then(Value::as_str).ok_or(t("text is required"))?;
                self.remote.input.push_back(vec![Event::Text(text.to_owned())]);
                Ok(json!({ "text": text }))
            }
            "ui.command" => {
                let id = params.get("id").and_then(Value::as_str).ok_or(t("id is required (a command or op id)"))?;
                let params = params.get("params").cloned().unwrap_or(Value::Null);
                self.check_agent_path(id, &params)?;
                self.run(id, params)
            }
            "ui.commands" => {
                let list: Vec<Value> = crate::commands::index(self)
                    .iter()
                    .map(|c| json!({ "id": c.id, "label": c.label, "desc": c.desc, "params": (c.params)() }))
                    .collect();
                Ok(json!({ "commands": list }))
            }
            p3d_mcp::connect::METHOD => self.agent_call(params),
            other => Err(format!("{} `{other}`", t("unknown control method"))),
        }
    }

    /// Agents name files only inside the folders the user allowed (the
    /// session's file access, `files.rs`): a `path` given to `file.open`,
    /// `file.import`, `file.save` or `file.save_as` must lie in one, so
    /// opening or saving a file cannot widen the folders. Without a path
    /// these commands ask the user in a dialog.
    ///
    /// Complexity: O(folders + path length).
    fn check_agent_path(&self, id: &str, params: &Value) -> Result<(), String> {
        let Some(path) = params.get("path").and_then(Value::as_str) else { return Ok(()) };
        let files = self.session.file_access();
        let checked = match id {
            "file.open" | "file.import" => files.readable(path).map(|_| ()),
            "file.save" | "file.save_as" => files.writable(path).map(|_| ()),
            _ => Ok(()),
        };
        checked.map_err(|e| format!("{}: {e}", t("agents may use only the folders the user allowed")))
    }

    /// Runs an MCP tool on this app's scene (`pixel3d-cli mcp --connect`):
    /// the agent's changes land in the same history as the user's, objects
    /// it creates become the selection, and the console and status bar say
    /// what it did.
    ///
    /// Complexity: the tool's work plus O(n log n) in objects after a change.
    fn agent_call(&mut self, params: &Value) -> Result<Value, String> {
        let before = Arc::clone(self.session.doc());
        let agent = self.remote.agent.get_or_insert_with(|| p3d_mcp::connect::host_agent(&self.session));
        let out = p3d_mcp::connect::answer(agent, &mut self.session, params)?;
        let tool = params.get("tool").and_then(Value::as_str).unwrap_or("?");
        let first = out.get("text").and_then(Value::as_str).and_then(|s| s.lines().next()).unwrap_or("");
        let line = format!("{} {tool}: {}", t("Agent"), first.chars().take(160).collect::<String>());
        if Arc::ptr_eq(&before, self.session.doc()) {
            self.log(line);
            return Ok(out);
        }
        let old: std::collections::BTreeSet<_> = before.objects().map(|(k, _)| k).collect();
        let doc = Arc::clone(self.session.doc());
        let created: Vec<_> = doc.objects().map(|(k, _)| k).filter(|k| !old.contains(k)).collect();
        if created.is_empty() {
            self.prune_selection();
        } else {
            self.active = created.first().copied();
            self.selection = created.into_iter().collect();
        }
        self.ui.start_page = false;
        self.set_status(line);
        Ok(out)
    }

    /// The interface state for agents: modes, selection, settings, scene.
    pub fn inspect(&self) -> Value {
        let doc = self.doc();
        let ids = |keys: &mut dyn Iterator<Item = p3d_scene::ObjectKey>| -> Vec<String> {
            keys.filter_map(|k| doc.object(k)).map(|o| o.public_id()).collect()
        };
        let (picked, picked_mode) = self.edit.picked();
        json!({
            "mode": self.ui.mode,
            "tool": self.ui.tool,
            "workspace": self.ui.workspace,
            "startPage": self.ui.start_page,
            "selection": ids(&mut self.selection.iter().copied()),
            "active": self.active_object().and_then(|k| doc.object(k)).map(|o| o.public_id()),
            "hidden": ids(&mut self.hidden.iter().copied()),
            "edit": {
                "object": self.edit.object.and_then(|k| doc.object(k)).map(|o| o.public_id()),
                "picked": picked,
                "pickedMode": picked_mode,
            },
            "view": self.ui.view,
            "transform": self.ui.transform,
            "camera": { "yaw": self.viewport.camera.yaw.to_degrees(), "pitch": self.viewport.camera.pitch.to_degrees(), "distance": self.viewport.camera.distance, "ortho": self.viewport.camera.ortho },
            "viewport": [self.viewport.rect.left(), self.viewport.rect.top(), self.viewport.rect.width(), self.viewport.rect.height()],
            "scene": { "objects": doc.object_count(), "revision": doc.revision, "dirty": self.is_dirty(), "path": self.path.as_ref().map(|p| p.display().to_string()) },
            "allowedFolders": self.session.file_access().roots().iter().map(|r| r.display().to_string()).collect::<Vec<_>>(),
            "shapes": self.sdf_state(),
            "jobCard": self.job_card_state(),
            "palette": self.palette.is_some(),
            "status": self.status_message(),
            "gizmoDragging": self.gizmo.dragging(),
            "frameMs": self.frame_ms,
            "fps": self.viewport.fps,
            "benchmark": self.bench_result,
            "gizmo": crate::gizmo::handles_json(self),
            "activeTransform": self.active_object().and_then(|k| doc.object(k)).map(|o| json!({
                "position": o.transform.translation.to_array(),
                "rotation": p3d_math::transform::rotation_to_euler_deg(o.transform.rotation),
                "scale": o.transform.scale.to_array(),
            })),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use p3d_control::Client;

    #[test]
    fn agents_drive_the_app_through_the_server() {
        let mut app = PixelApp::new(p3d_engine::Session::new(), crate::Services::default());
        let addr = app.start_control(0).unwrap();
        let token = app.remote.server.as_ref().map(|s| s.token().to_owned()).unwrap();
        let ctx = egui::Context::default();
        let client = std::thread::spawn(move || {
            let mut c = Client::connect(addr, &token).unwrap();
            let made = c.call("ui.command", json!({ "id": "prim.box", "params": { "name": "crate" } })).unwrap();
            let state = c.call("ui.inspect", Value::Null).unwrap();
            let key = c.call("ui.key", json!({ "key": "Tab" })).unwrap();
            let bad = c.call("ui.fly", Value::Null).err().map(|e| e.to_string());
            (made, state, key, bad)
        });
        // The app's frames: drain requests until the client is done.
        while !client.is_finished() {
            app.drain_control(&ctx);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let (made, state, key, bad) = client.join().unwrap();
        assert!(made.is_object());
        assert_eq!(state["scene"]["objects"], json!(1));
        assert_eq!(state["selection"].as_array().map(Vec::len), Some(1), "the new box is selected");
        assert_eq!(key, json!({ "key": "Tab" }));
        assert!(bad.is_some_and(|e| e.contains("unknown control method")));
        // The key goes in as real input over the next frames: Tab toggles Edit Mode.
        let mut raw = egui::RawInput::default();
        app.feed_remote_input(&mut raw);
        app.filter_input(&ctx, &mut raw);
        assert!(app.ui.mode.is_edit());
    }

    #[test]
    fn agents_get_the_users_folders_and_never_more() {
        let (project, other) = (crate::files::tests::temp_dir("agent-project"), crate::files::tests::temp_dir("agent-other"));
        crate::files::tests::write_obj(&project.join("tri.obj"));
        crate::files::tests::write_obj(&other.join("tri.obj"));
        let mut app = PixelApp::new(p3d_engine::Session::new(), crate::Services::default());
        // The user saves the scene: its folder is the project folder.
        app.run("file.save", json!({ "path": project.join("scene.p3d").display().to_string() })).unwrap();
        let addr = app.start_control(0).unwrap();
        let token = app.remote.server.as_ref().map(|s| s.token().to_owned()).unwrap();
        let ctx = egui::Context::default();
        let (inside, outside) = (project.join("tri.obj").display().to_string(), other.join("tri.obj").display().to_string());
        let elsewhere = other.join("scene.p3d").display().to_string();
        let client = std::thread::spawn(move || {
            let mut c = Client::connect(addr, &token).unwrap();
            let tool = |c: &mut Client, path: &str| {
                let out = c.call("mcp.call", json!({ "tool": "import", "args": { "path": path } })).unwrap();
                p3d_mcp::ToolOutput::from_wire(&out).unwrap()
            };
            let ok = tool(&mut c, &inside);
            let refused = tool(&mut c, &outside);
            let save = c.call("ui.command", json!({ "id": "file.save_as", "params": { "path": elsewhere } })).err().map(|e| e.to_string());
            let open = c.call("ui.command", json!({ "id": "file.open", "params": { "path": outside } })).err().map(|e| e.to_string());
            let state = c.call("ui.state", Value::Null).unwrap();
            (ok, refused, save, open, state)
        });
        while !client.is_finished() {
            app.drain_control(&ctx);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let (ok, refused, save, open, state) = client.join().unwrap();
        assert!(!ok.is_error, "{}", ok.text);
        assert!(refused.is_error && refused.text.contains("outside the allowed folders"), "{}", refused.text);
        for e in [save, open] {
            assert!(e.is_some_and(|e| e.contains("only the folders the user allowed")), "saving or opening elsewhere cannot widen the folders");
        }
        assert_eq!(state["allowedFolders"], json!([project.display().to_string()]));
        assert!(app.session.file_access().readable(&other.join("tri.obj").display().to_string()).is_err());
        for d in [project, other] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    #[test]
    fn mcp_tools_run_on_the_app_scene() {
        let mut app = PixelApp::new(p3d_engine::Session::new(), crate::Services::default());
        let addr = app.start_control(0).unwrap();
        let token = app.remote.server.as_ref().map(|s| s.token().to_owned()).unwrap();
        let ctx = egui::Context::default();
        let client = std::thread::spawn(move || {
            let mut c = Client::connect(addr, &token).unwrap();
            let run = json!({ "tool": "run", "args": { "ops": [{ "op": "prim.box", "args": { "name": "crate" } }] } });
            let made = c.call("mcp.call", run).unwrap();
            let seen = c.call("mcp.call", json!({ "tool": "scene_overview" })).unwrap();
            let commands = c.call("ui.commands", Value::Null).unwrap();
            (made, seen, commands)
        });
        while !client.is_finished() {
            app.drain_control(&ctx);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let (made, seen, commands) = client.join().unwrap();
        let made = p3d_mcp::ToolOutput::from_wire(&made).unwrap();
        assert!(!made.is_error, "{}", made.text);
        assert!(p3d_mcp::ToolOutput::from_wire(&seen).unwrap().text.contains("crate"));
        assert_eq!(app.doc().object_count(), 1, "the change is in the app's own session");
        assert_eq!(app.selection.len(), 1, "what the agent made is selected");
        assert!(app.session.can_undo(), "and undoable like the user's edits");
        let ids: Vec<&str> = commands["commands"].as_array().unwrap().iter().filter_map(|c| c["id"].as_str()).collect();
        assert!(ids.contains(&"view.frame") && ids.contains(&"ui.mode"), "{ids:?}");
    }
}
