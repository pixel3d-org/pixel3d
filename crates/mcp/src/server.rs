//! The MCP server: rmcp over stdio (project spec 17.14). The [`Agent`]
//! lives on its own thread (the session is single-owner); tool calls reach
//! it over a channel and the async side awaits the answer. Connected to a
//! desktop app, that thread forwards the calls to the app instead.

use std::net::SocketAddr;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::mpsc;

use base64::Engine;
use p3d_engine::Session;
use p3d_ops::OpError;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation, ListResourcesResult, ListToolsResult,
    PaginatedRequestParams, ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, Resource, ResourceContents, ServerCapabilities,
    ServerConfig, Tool, ToolAnnotations,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, ServiceExt};
use serde_json::{Map, Value};
use tokio::sync::oneshot;

use crate::agent::{Agent, ToolInfo, ToolOutput, tools};
use crate::connect::{Remote, ui_tools};
use crate::docs;

/// What the server tells clients when they connect.
pub const INSTRUCTIONS: &str = "pixel3d builds 3D game assets from typed ops and lets you check them as text, without screenshots. \
Read `docs` (topic quickstart) once. Loop: `expect add` tests from the request, `run` ops in one batch, read the problems \
and test lines at the end of each answer, `perceive` or `describe` to look closer, `diagnose` for fixes with predicted results. \
Units are meters (or strings like \"12mm\"), +Y is up, +Z is the front.";

/// Added to [`INSTRUCTIONS`] when connected to the desktop app.
const CONNECTED: &str = " You are connected to the pixel3d desktop app: the user sees every change live and may edit the scene \
between your calls, so read it again before relying on an older answer. `ui_state`, `ui_command` and `ui_screenshot` read and \
drive the app's interface.";

type Request = (String, Value, oneshot::Sender<ToolOutput>);

/// Why the server stopped with an error.
#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    #[error("runtime: {0}")]
    Runtime(std::io::Error),
    #[error("mcp: {0}")]
    Mcp(String),
    #[error("cannot reach the desktop app at {addr}: {reason}; start it with `pixel3d --control {port}` or turn on File > Allow agents to connect", port = addr.port())]
    Connect { addr: SocketAddr, reason: String },
}

/// The rmcp handler.
pub struct Server {
    tx: mpsc::Sender<Request>,
    tools: Vec<Tool>,
    connected: bool,
}

/// Tools that never change the scene (`query` does with `save_as`, `expect`
/// with add and remove).
const READ_ONLY: [&str; 9] = ["scene_overview", "describe", "perceive", "analyze", "list_ops", "describe_op", "docs", "ui_state", "ui_screenshot"];

impl Server {
    /// Starts the session thread.
    pub fn new(session: Session) -> Result<Server, ServeError> {
        let mut agent = Agent::new(session);
        Self::start("pixel3d-session", tools(), false, move |tool, args| agent.call(tool, args))
    }

    /// Forwards every tool to the desktop app listening at `addr` and adds
    /// the `ui_*` tools. Fails when nothing listens there.
    pub fn connect(addr: SocketAddr) -> Result<Server, ServeError> {
        let mut remote = Remote::connect(addr).map_err(|e| ServeError::Connect { addr, reason: e.to_string() })?;
        let mut list = tools();
        list.extend(ui_tools());
        Self::start("pixel3d-connect", list, true, move |tool, args| remote.call(tool, args))
    }

    /// Starts the thread that answers tool calls with `answer`.
    fn start(
        thread: &str,
        list: Vec<ToolInfo>,
        connected: bool,
        mut answer: impl FnMut(&str, Value) -> ToolOutput + Send + 'static,
    ) -> Result<Server, ServeError> {
        let (tx, rx) = mpsc::channel::<Request>();
        std::thread::Builder::new()
            .name(thread.into())
            .spawn(move || {
                while let Ok((tool, args, reply)) = rx.recv() {
                    let out = catch_unwind(AssertUnwindSafe(|| answer(&tool, args))).unwrap_or_else(|_| {
                        ToolOutput::error(&OpError::internal(format!("tool {tool} failed unexpectedly; the scene is unchanged")))
                    });
                    // The client may have gone; nothing to do then.
                    let _ = reply.send(out);
                }
            })
            .map_err(ServeError::Runtime)?;
        let tools = list
            .into_iter()
            .map(|t| {
                let schema = match t.schema {
                    Value::Object(m) => m,
                    _ => Map::new(),
                };
                let read_only = READ_ONLY.contains(&t.name);
                Tool::new(t.name, t.description, Arc::new(schema)).with_annotations(ToolAnnotations::new().read_only(read_only).open_world(false))
            })
            .collect();
        Ok(Server { tx, tools, connected })
    }

    /// Asks the session thread to run a tool.
    async fn ask(&self, tool: &str, args: Value) -> Result<ToolOutput, McpError> {
        let (tx, rx) = oneshot::channel();
        self.tx.send((tool.to_owned(), args, tx)).map_err(|_| McpError::internal_error("the session stopped", None))?;
        rx.await.map_err(|_| McpError::internal_error("the session stopped", None))
    }
}

/// Resources: the guides, the op list and the live scene overview.
fn resources() -> Vec<Resource> {
    let mut out: Vec<Resource> = docs::GUIDES
        .iter()
        .map(|(name, about, _)| Resource::new(format!("pixel3d://guide/{name}"), *name).with_description(*about).with_mime_type("text/markdown"))
        .collect();
    out.push(Resource::new("pixel3d://ops", "ops").with_description("every op with a one-line summary").with_mime_type("text/markdown"));
    out.push(Resource::new("pixel3d://scene", "scene").with_description("the current scene overview").with_mime_type("text/plain"));
    out
}

impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().enable_resources().build())
            .with_server_info(Implementation::new("pixel3d", env!("CARGO_PKG_VERSION")))
            .with_instructions(if self.connected { format!("{INSTRUCTIONS}{CONNECTED}") } else { INSTRUCTIONS.to_owned() })
    }

    async fn list_tools(&self, _request: Option<PaginatedRequestParams>, _context: RequestContext<RoleServer>) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(self.tools.clone()))
    }

    async fn call_tool(&self, request: CallToolRequestParams, _context: RequestContext<RoleServer>) -> Result<CallToolResponse, McpError> {
        let args = request.arguments.map_or_else(|| Value::Object(Map::new()), Value::Object);
        let out = self.ask(&request.name, args).await?;
        let mut content = vec![ContentBlock::text(out.text)];
        for png in &out.images {
            content.push(ContentBlock::image(base64::engine::general_purpose::STANDARD.encode(png), "image/png"));
        }
        let mut result = if out.is_error { CallToolResult::error(content) } else { CallToolResult::success(content) };
        result.structured_content = Some(out.structured);
        Ok(result.into())
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        Ok(ListResourcesResult::with_all_items(resources()))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, McpError> {
        let uri = request.uri.as_str();
        let text = if uri == "pixel3d://scene" {
            self.ask("scene_overview", Value::Object(Map::new())).await?.text
        } else {
            let topic = uri.strip_prefix("pixel3d://guide/").or_else(|| uri.strip_prefix("pixel3d://")).unwrap_or(uri);
            docs::topic(topic).ok_or_else(|| McpError::resource_not_found(format!("no resource {uri}"), None))?
        };
        Ok(ReadResourceResult::new(vec![ResourceContents::text(text, uri)]).into())
    }
}

/// Serves MCP over stdin and stdout until the client disconnects.
pub fn serve_stdio(session: Session) -> Result<(), ServeError> {
    serve(|| Server::new(session))
}

/// Serves MCP over stdin and stdout with every tool acting on the desktop
/// app at `addr` (`pixel3d-cli mcp --connect`).
pub fn serve_connected_stdio(addr: SocketAddr) -> Result<(), ServeError> {
    serve(|| Server::connect(addr))
}

fn serve(server: impl FnOnce() -> Result<Server, ServeError>) -> Result<(), ServeError> {
    let server = server()?;
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(ServeError::Runtime)?;
    rt.block_on(async {
        let running = server.serve(rmcp::transport::stdio()).await.map_err(|e| ServeError::Mcp(e.to_string()))?;
        running.waiting().await.map_err(|e| ServeError::Mcp(e.to_string()))?;
        Ok(())
    })
}
