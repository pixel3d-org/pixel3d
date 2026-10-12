//! Operation trait, registry, parameter schemas and errors.
//!
//! Layer L4 (engine). See `docs/architecture.md`.
//!
//! Every action in pixel3d is an op in one registry: the GUI, CLI, MCP
//! server, scripts, addons and the node graph all call the same code.

pub mod catalog;
mod common;
pub mod curve;
pub mod deform;
mod dynop;
pub mod edit;
pub mod element;
mod error;
pub mod expect;
mod op;
pub mod params;
pub mod prim;
mod registry;
pub mod scene;
pub mod select;
pub mod shade;
pub mod subd;
mod tx;

pub use dynop::{Applied, DynOp, Typed, parse_params};
pub use error::{ErrorCode, OpError};
pub use op::{Artifact, Category, GroupInfo, MetricsDelta, Op, OpCall, OpCtx, OpExample, OpFlags, OpMeta, OpOutput, Warning};
pub use registry::Registry;
pub use tx::Tx;
