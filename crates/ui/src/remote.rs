//! The app's side of the control server (project spec 5.3, 20.2): agents
//! and UI tests read the state and the on-screen elements, drive the
//! interface with real input events (clicks, drags, keys, text), take
//! screenshots and run any command. Requests are drained on the UI thread
//! in `logic()`; input is fed to egui over the next frames, as a person
//! would type and click.
//!
//! Methods: `ui.inspect` (alias `ui.state`), `ui.elements`, `ui.click`,
//! `ui.drag`, `ui.key`, `ui.text`, `ui.screenshot`, `ui.render` (the
//! viewport only) and `ui.command` (any command or op).

use std::collections::VecDeque;
use std::path::PathBuf;

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
                self.run(id, params.get("params").cloned().unwrap_or(Value::Null))
            }
            other => Err(format!("{} `{other}`", t("unknown control method"))),
        }
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
        let ctx = egui::Context::default();
        let client = std::thread::spawn(move || {
            let mut c = Client::connect(addr).unwrap();
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
}
