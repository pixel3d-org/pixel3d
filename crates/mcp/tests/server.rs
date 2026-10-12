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
    assert_eq!(names.len(), 13, "{names:?}");
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
