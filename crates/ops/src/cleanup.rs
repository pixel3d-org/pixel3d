//! `cleanup.*`: the "make it clean" pass and its dry-run report.

use p3d_geo::cleanup::{CleanupReport, CleanupSpec, cleanup};
use p3d_scene::Document;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::common::{check_count, check_length, label, mesh_target, thousands};
use crate::error::OpError;
use crate::op::{Category, Op, OpCtx, OpExample, OpFlags, OpMeta, OpOutput, Warning};
use crate::params::Length;
use crate::tx::Tx;

fn tenth_mm() -> Length {
    Length(1e-4)
}

fn four() -> u32 {
    4
}

fn yes() -> bool {
    true
}

/// Which fixes to run (shared by `cleanup.auto` and `cleanup.report`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CleanupParams {
    /// Mesh object (name, path or id).
    pub target: String,
    /// Weld vertices closer than this (0 skips welding).
    #[serde(default = "tenth_mm")]
    pub weld: Length,
    /// Collapse faces with no area (their tiny edge folds away).
    #[serde(default = "yes")]
    pub degenerate: bool,
    /// Remove loose parts smaller than this across (0 keeps all).
    #[serde(default = "tenth_mm")]
    pub min_part_size: Length,
    /// Remove closed parts buried inside other closed parts (never seen).
    #[serde(default = "yes")]
    pub remove_hidden: bool,
    /// Turn inside-out parts outward.
    #[serde(default = "yes")]
    pub orient: bool,
    /// Fill holes of at most this many edges on parts that are not flat sheets (0 fills none).
    #[serde(default = "four")]
    pub max_hole_edges: u32,
    /// Join coplanar triangle pairs into quads.
    #[serde(default = "yes")]
    pub join_flat: bool,
}

impl Default for CleanupParams {
    fn default() -> Self {
        CleanupParams {
            target: String::new(),
            weld: tenth_mm(),
            degenerate: true,
            min_part_size: tenth_mm(),
            remove_hidden: true,
            orient: true,
            max_hole_edges: 4,
            join_flat: true,
        }
    }
}

impl CleanupParams {
    fn spec(&self) -> CleanupSpec {
        CleanupSpec {
            weld: self.weld.0 as f32,
            degenerate: self.degenerate,
            min_part_size: self.min_part_size.0 as f32,
            enclosed: self.remove_hidden,
            orient: self.orient,
            max_hole_edges: self.max_hole_edges,
            join_flat: self.join_flat,
        }
    }

    fn validate(&self, doc: &Document) -> Result<(), OpError> {
        mesh_target(doc, &self.target)?;
        check_length("/weld", self.weld, 0.0, 0.1)?;
        check_length("/min_part_size", self.min_part_size, 0.0, 10.0)?;
        check_count("/max_hole_edges", self.max_hole_edges, 0, 64)
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{} {}", thousands(n), if n == 1 { one } else { many })
}

/// `welded 2 vertices, filled 1 hole` (empty when nothing changed).
fn fixes(r: &CleanupReport) -> Vec<String> {
    let mut out = Vec::new();
    let mut add = |n: usize, s: String| {
        if n > 0 {
            out.push(s);
        }
    };
    add(r.welded, format!("welded {}", plural(r.welded, "vertex", "vertices")));
    add(r.degenerate_collapsed, format!("collapsed {}", plural(r.degenerate_collapsed, "degenerate face", "degenerate faces")));
    add(r.specks_removed, format!("removed {}", plural(r.specks_removed, "speck", "specks")));
    add(r.holes_filled, format!("filled {}", plural(r.holes_filled, "hole", "holes")));
    add(r.triangles_joined, format!("joined {} into quads", plural(r.triangles_joined, "triangle", "triangles")));
    add(r.enclosed_removed, format!("removed {}", plural(r.enclosed_removed, "hidden part", "hidden parts")));
    add(r.parts_flipped, format!("turned {} outward", plural(r.parts_flipped, "part", "parts")));
    out
}

/// `cleanup.auto would weld 2 vertices, fill 1 hole`, or that it would change nothing.
pub(crate) fn would(r: &CleanupReport) -> String {
    let done = fixes(r);
    if done.is_empty() {
        return "cleanup.auto would change nothing".to_owned();
    }
    // "welded 2 vertices" reads "would weld 2 vertices".
    let text = done
        .join(", ")
        .replace("welded", "weld")
        .replace("collapsed", "collapse")
        .replace("removed", "remove")
        .replace("filled", "fill")
        .replace("joined", "join")
        .replace("turned", "turn");
    format!("cleanup.auto would {text}")
}

fn warnings(r: &CleanupReport) -> Vec<Warning> {
    let mut w: Vec<Warning> = r.warnings.iter().map(|m| Warning { message: m.clone(), fix: None }).collect();
    if r.degenerate_left > 0 {
        w.push(Warning {
            message: format!(
                "{} with long edges kept (collapsing would move vertices too far)",
                plural(r.degenerate_left, "sliver face", "sliver faces")
            ),
            fix: None,
        });
    }
    w
}

// ---- cleanup.auto ------------------------------------------------------------------------

/// `cleanup.auto`.
pub struct Auto;

impl Op for Auto {
    const ID: &'static str = "cleanup.auto";
    type Params = CleanupParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Clean up",
            category: Category::Cleanup,
            summary: "Makes a mesh clean in one call: welds seams, collapses degenerate faces, removes specks and hidden parts, fills small holes, joins flat triangles into quads and turns parts outward.",
            docs: "The passes run in this order, each best effort (a pass that cannot run leaves the mesh as it was and adds \
                   a warning):\n\n\
                   1. **Weld** vertices closer than `weld` (0.1 mm), repeated until nothing moves; faces that only touch at a \
                   corner stay apart.\n\
                   2. **Degenerate faces** with no area collapse along their tiny edge; slivers with long edges are reported.\n\
                   3. **Specks**: loose parts smaller than `min_part_size` across are removed.\n\
                   4. **Small holes** up to `max_hole_edges` edges are filled, except on flat sheets (a part with no more faces than \
                   the loop has edges).\n\
                   5. **Flat triangle pairs** become quads (a limited dissolve that keeps quads).\n\
                   6. **Hidden parts**: closed parts buried in the material of another closed part are removed. Inward parts \
                   inside a closed part are cavities (hollow walls) and stay.\n\
                   7. **Orientation**: inside-out closed parts turn outward by volume; open parts turn when rays clearly see their \
                   back from outside.\n\n\
                   Running it twice changes nothing the second time. Use `cleanup.report` first to see what it would do.\n\n\
                   **Use for** imported meshes, joined parts, and as the first step of making an asset game ready.",
            examples: &[
                OpExample { args: r#"{"target": "bin"}"#, result: "bin (o:5): filled 1 hole: 6 faces" },
                OpExample { args: r#"{"target": "tiles"}"#, result: "tiles (o:6): joined 32 triangles into quads: 16 faces" },
            ],
            aliases: &["cleanup", "clean up", "mesh cleanup", "merge by distance", "fill holes", "recalculate outside", "3d print toolbox"],
            complexity: "O(n log n): welding, a BVH and part tests by ray parity",
            flags: OpFlags::UNDOABLE,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &CleanupParams) -> Result<(), OpError> {
        p.validate(doc)
    }

    fn apply(tx: &mut Tx, p: &CleanupParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        let mesh = tx.mesh_mut(key, "/target")?;
        let r = cleanup(mesh, &p.spec());
        let faces = mesh.face_count();
        let done = fixes(&r);
        let what = if done.is_empty() { "already clean".to_owned() } else { done.join(", ") };
        let mut out = OpOutput::summary(format!("{}: {what}: {}", label(tx.doc(), key), plural(faces, "face", "faces")));
        out.warnings = warnings(&r);
        if r.fixes() > 0 {
            out.changed.push(key);
        }
        Ok(out)
    }
}

// ---- cleanup.report ----------------------------------------------------------------------

/// `cleanup.report`.
pub struct Report;

impl Op for Report {
    const ID: &'static str = "cleanup.report";
    type Params = CleanupParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Cleanup report",
            category: Category::Cleanup,
            summary: "Reports a mesh's topology problems and what cleanup.auto would fix, without changing anything.",
            docs: "Runs the `cleanup.auto` passes with the same parameters on a copy and reports the fixes it would make, plus \
                   the mesh's parts, open borders and degenerate faces. Nothing changes.\n\n\
                   **Use for** checking imports and joined meshes before cleaning them, and in reports.",
            examples: &[OpExample {
                args: r#"{"target": "panels"}"#,
                result: "panels (o:9): 2 parts (0 closed), 2 open borders, 0 degenerate faces; cleanup.auto would weld 2 vertices",
            }],
            aliases: &["mesh analysis", "check mesh", "3d print checks", "non-manifold check", "cleanup preview"],
            complexity: "O(n log n), on a copy",
            flags: OpFlags::NONE,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &CleanupParams) -> Result<(), OpError> {
        p.validate(doc)
    }

    fn apply(tx: &mut Tx, p: &CleanupParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        let mesh = tx.mesh(key, "/target")?;
        let t = p3d_analyze::topology(mesh);
        let mut copy = mesh.clone();
        let r = cleanup(&mut copy, &p.spec());
        let would = would(&r);
        let mut out = OpOutput::summary(format!(
            "{}: {} ({} closed), {}, {}; {would}",
            label(tx.doc(), key),
            plural(t.parts, "part", "parts"),
            thousands(t.closed_parts),
            plural(t.boundary_loops, "open border", "open borders"),
            plural(t.degenerate_faces, "degenerate face", "degenerate faces"),
        ));
        out.warnings = warnings(&r);
        Ok(out)
    }
}
