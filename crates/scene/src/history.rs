//! Per-object operation history, recorded as a graph of op nodes.
//!
//! Phase 1 records a linear chain: every op applied to an object appends a
//! node. The node graph editor later shows and edits the same data.

use serde::{Deserialize, Serialize};

/// One applied operation: op id, op version, canonical parameters, label.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpNode {
    pub op: String,
    pub version: u16,
    pub params: serde_json::Value,
    pub label: String,
}

/// History of one object. Nodes are in application order; each node's input
/// is the previous node's output.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OpGraph {
    pub nodes: Vec<OpNode>,
}

impl OpGraph {
    pub fn push(&mut self, node: OpNode) {
        self.nodes.push(node);
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}
