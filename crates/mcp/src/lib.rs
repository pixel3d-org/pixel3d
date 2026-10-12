//! MCP server for agents.
//!
//! Layer L5 (frontends). See `docs/architecture.md`.
//!
//! [`Agent`] is the agent surface without a transport: it owns the
//! [`p3d_engine::Session`] and answers tool calls with compact text plus
//! structured JSON (project spec 17.13 to 17.18). The MCP server runs the
//! agent on its own thread and forwards tool calls to it.

pub mod agent;
pub mod args;
pub mod describe;
pub mod docs;
pub mod server;

pub use agent::{Agent, ToolInfo, ToolOutput, tools};
pub use server::{INSTRUCTIONS, ServeError, Server, serve_stdio};
