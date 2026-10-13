//! Generators: meshes built from profiles and paths (lathe, sweep, loft,
//! pipe, extruded shapes) and from other meshes (array, mirror, scatter).
//!
//! Generators share the primitives' conventions: +Y up, quads by default,
//! semantic face groups and sharp flags on hard edges.

mod extrude;
mod lathe;
mod shape;

pub use extrude::{SHAPE_CORNER_DEG, ShapeSpec, extrude_shape};
pub use lathe::{CORNER_DEG, LatheSpec, MAX_PROFILE_POINTS, lathe};
pub use shape::MAX_CONTOURS;

use crate::prim::PrimError;
use crate::prim::soup::Soup;

/// Most faces a generator may produce.
pub const MAX_GEN_FACES: usize = 4_000_000;

pub(crate) fn invalid(param: &'static str, value: impl ToString, reason: impl Into<String>) -> PrimError {
    PrimError::InvalidParam { param, value: value.to_string(), reason: reason.into() }
}

/// Quads between two rows of vertices, `[a_k, b_k, b_k1, a_k1]` for each
/// column step (the last column joins the first when `wrap`). A row of one
/// vertex is a pole: its quads become triangles.
///
/// Complexity: O(columns).
pub(crate) fn strip(soup: &mut Soup, a: &[u32], b: &[u32], wrap: bool, tag: u16) {
    let cols = a.len().max(b.len());
    let steps = if wrap { cols } else { cols.saturating_sub(1) };
    let at = |r: &[u32], k: usize| r[if r.len() == 1 { 0 } else { k % r.len() }];
    for k in 0..steps {
        let q = [at(a, k), at(b, k), at(b, k + 1), at(a, k + 1)];
        if a.len() == 1 {
            soup.poly(&[q[0], q[1], q[2]], tag);
        } else if b.len() == 1 {
            soup.poly(&[q[0], q[1], q[3]], tag);
        } else {
            soup.poly(&q, tag);
        }
    }
}
