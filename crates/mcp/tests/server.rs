//! The MCP server end to end in one process: an rmcp client talks to the
//! server over an in-memory pipe.

// Test helpers fail the test by panicking; clippy only exempts `#[test]` functions themselves.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use p3d_engine::Session;
use p3d_mcp::Server;
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ReadResourceRequestParams};
use serde_json::json;

fn args(v: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    v.as_object().cloned().unwrap()
}

#[tokio::test(flavor = "current_thread")]
async fn tools_resources_and_calls() {
    let (server_io, client_io) = tokio::io::duplex(1 << 16);
    let server = tokio::spawn(async move {
        let running = Server::new(Session::new()).unwrap().serve(server_io).await.unwrap();
        running.waiting().await.unwrap();
    });
    let client = ().serve(client_io).await.unwrap();

    let info = client.peer_info().expect("server info");
    assert_eq!(info.server_info.as_ref().map(|i| i.name.as_str()), Some("pixel3d"));
    let tools = client.list_all_tools().await.unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    assert_eq!(names.len(), 16, "{names:?}");
    assert!(names.contains(&"run") && names.contains(&"perceive"));
    let run = tools.iter().find(|t| t.name == "run").unwrap();
    assert_eq!(run.input_schema.get("additionalProperties"), Some(&json!(false)));

    let r = client
        .call_tool(CallToolRequestParams::new("run").with_arguments(args(json!({ "ops": [{ "op": "prim.box", "args": { "name": "crate" } }] }))))
        .await
        .unwrap();
    assert_ne!(r.is_error, Some(true));
    let text = r.content[0].as_text().unwrap().text.clone();
    assert!(text.starts_with("ok rev 1"), "{text}");
    assert_eq!(r.structured_content.as_ref().unwrap()["revision"], json!(1));

    let r = client.call_tool(CallToolRequestParams::new("describe").with_arguments(args(json!({ "target": "crat" })))).await.unwrap();
    assert_eq!(r.is_error, Some(true));
    assert!(r.content[0].as_text().unwrap().text.contains("did you mean: crate"));

    let resources = client.list_all_resources().await.unwrap();
    assert!(resources.iter().any(|r| r.uri == "pixel3d://guide/quickstart"));
    let g = client.read_resource(ReadResourceRequestParams::new("pixel3d://guide/quickstart")).await.unwrap();
    assert!(format!("{:?}", g.contents[0]).contains("pixel3d quickstart"));
    let s = client.read_resource(ReadResourceRequestParams::new("pixel3d://scene")).await.unwrap();
    assert!(format!("{:?}", s.contents[0]).contains("crate (o:1)"));
    assert!(client.read_resource(ReadResourceRequestParams::new("pixel3d://nothing")).await.is_err());

    client.cancel().await.unwrap();
    server.await.unwrap();
}

/// A stand-in for the desktop app: a control server whose "UI thread"
/// answers `mcp.call` on its own session, as the app does.
fn fake_app() -> (std::net::SocketAddr, String, std::sync::Arc<std::sync::atomic::AtomicBool>, std::thread::JoinHandle<Session>) {
    use std::sync::atomic::{AtomicBool, Ordering};
    let server = p3d_control::Server::start(0).unwrap();
    let addr = server.addr();
    let token = server.token().to_owned();
    let stop = std::sync::Arc::new(AtomicBool::new(false));
    let stop2 = std::sync::Arc::clone(&stop);
    let app = std::thread::spawn(move || {
        let mut session = Session::new();
        let mut agent = p3d_mcp::connect::host_agent(&session);
        while !stop2.load(Ordering::Relaxed) {
            let Some(req) = server.next() else {
                std::thread::sleep(std::time::Duration::from_millis(2));
                continue;
            };
            let r = match req.method.as_str() {
                "mcp.call" => p3d_mcp::connect::answer(&mut agent, &mut session, &req.params),
                "ui.state" => Ok(json!({ "mode": "object", "objects": session.doc().object_count() })),
                "ui.commands" => Ok(json!({ "commands": [{ "id": "view.frame", "label": "Frame selected", "desc": "fit the view" }] })),
                "ui.command" if req.params["id"] == "view.frame" => Ok(json!({ "framed": true })),
                "ui.command" => Err(format!("unknown command `{}`", req.params["id"].as_str().unwrap_or(""))),
                "ui.screenshot" => {
                    let path = req.params["path"].as_str().unwrap().to_owned();
                    let mut png = vec![0x89, b'P', b'N', b'G', 13, 10, 26, 10, 0, 0, 0, 13, b'I', b'H', b'D', b'R'];
                    png.extend(64u32.to_be_bytes());
                    png.extend(32u32.to_be_bytes());
                    std::fs::write(&path, png).unwrap();
                    Ok(json!({ "path": path }))
                }
                other => Err(format!("unknown control method `{other}`")),
            };
            req.reply(r);
        }
        session
    });
    (addr, token, stop, app)
}

#[tokio::test(flavor = "current_thread")]
async fn connected_tools_act_on_the_app() {
    let (addr, token, stop, app) = fake_app();
    let (server_io, client_io) = tokio::io::duplex(1 << 16);
    let server = tokio::spawn(async move {
        let running = Server::connect_with_token(addr, &token).unwrap().serve(server_io).await.unwrap();
        running.waiting().await.unwrap();
    });
    let client = ().serve(client_io).await.unwrap();
    let info = client.peer_info().expect("server info");
    assert!(info.instructions.as_deref().unwrap_or("").contains("desktop app"));
    let tools = client.list_all_tools().await.unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    assert_eq!(names.len(), 19, "{names:?}");
    assert!(names.ends_with(&["ui_state", "ui_command", "ui_screenshot"]), "{names:?}");

    // Scene tools run in the app's session.
    let r = client
        .call_tool(CallToolRequestParams::new("run").with_arguments(args(json!({ "ops": [{ "op": "prim.box", "args": { "name": "crate" } }] }))))
        .await
        .unwrap();
    assert_ne!(r.is_error, Some(true), "{:?}", r.content);
    let s = client.read_resource(ReadResourceRequestParams::new("pixel3d://scene")).await.unwrap();
    assert!(format!("{:?}", s.contents[0]).contains("crate (o:1)"));

    let r = client.call_tool(CallToolRequestParams::new("ui_state").with_arguments(args(json!({})))).await.unwrap();
    assert_eq!(r.structured_content, Some(json!({ "mode": "object", "objects": 1 })));
    let r = client.call_tool(CallToolRequestParams::new("ui_command").with_arguments(args(json!({})))).await.unwrap();
    assert!(r.content[0].as_text().unwrap().text.starts_with("view.frame (Frame selected)"));
    let r = client.call_tool(CallToolRequestParams::new("ui_command").with_arguments(args(json!({ "id": "view.frame" })))).await.unwrap();
    assert_eq!(r.structured_content, Some(json!({ "framed": true })));
    let r = client.call_tool(CallToolRequestParams::new("ui_command").with_arguments(args(json!({ "id": "view.fly" })))).await.unwrap();
    assert_eq!(r.is_error, Some(true));
    assert!(r.content[0].as_text().unwrap().text.contains("unknown command `view.fly`"));
    let r = client.call_tool(CallToolRequestParams::new("ui_screenshot").with_arguments(args(json!({})))).await.unwrap();
    assert_eq!(r.content[0].as_text().unwrap().text, "window screenshot, 64 x 32 px");
    assert!(r.content[1].as_image().is_some());
    let r = client.call_tool(CallToolRequestParams::new("ui_state").with_arguments(args(json!({ "x": 1 })))).await.unwrap();
    assert_eq!(r.is_error, Some(true), "arguments stay strict");

    client.cancel().await.unwrap();
    server.await.unwrap();
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let session = app.join().unwrap();
    assert_eq!(session.doc().object_count(), 1, "the app's session holds the agent's change");
}

#[test]
fn connecting_to_nothing_explains_how_to_start_the_app() {
    // Bind and drop: the port is free and nothing listens there.
    let addr = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
    let err = Server::connect(addr).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(err.contains(&format!("pixel3d --control {}", addr.port())), "{err}");
}
