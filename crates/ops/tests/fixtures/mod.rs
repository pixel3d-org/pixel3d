//! Scenes the op tests and benches run against: each op category's
//! examples are written for one of these fixtures.

// Shared by tests and benches; each uses a different subset.
#![allow(dead_code, clippy::panic, clippy::expect_used, clippy::unwrap_used)]

use p3d_ops::{Applied, Category, OpCtx, OpError, Registry, Tx};
use p3d_scene::Document;
use serde_json::{Value, json};

pub fn run(tx: &mut Tx, op: &str, args: Value) -> Result<Applied, OpError> {
    let r = Registry::builtin();
    let op = r.get(op)?;
    let applied = op.run(tx, &args, &OpCtx::default())?;
    tx.doc().check().map_err(OpError::internal)?;
    for (_, o) in tx.doc().objects() {
        if let Some(m) = o.mesh_key().and_then(|k| tx.doc().mesh(k)) {
            m.validate().map_err(|e| OpError::internal(e.to_string()))?;
        }
    }
    Ok(applied)
}

pub fn ok(tx: &mut Tx, op: &str, args: Value) -> String {
    run(tx, op, args).unwrap_or_else(|e| panic!("{op}: {e}")).output.summary
}

pub fn err(tx: &mut Tx, op: &str, args: Value) -> OpError {
    let before = tx.clone();
    let e = run(tx, op, args).expect_err(op);
    // A failing op leaves the working copy unchanged in these cases (validation errors).
    assert_eq!(before.doc().object_count(), tx.doc().object_count());
    e
}

/// The fixture every op example runs against: crate (o:1, 0.6 m box) with
/// child lid (o:2, box) which has child hinge (o:4, empty), and post (o:3, cylinder).
pub fn fixture() -> Tx {
    let mut tx = Tx::new(Document::new());
    ok(&mut tx, "prim.box", json!({ "name": "crate", "width": 0.6, "height": 0.6, "depth": 0.6 }));
    ok(&mut tx, "prim.box", json!({ "name": "lid", "parent": "crate", "width": 0.6, "height": 0.05, "depth": 0.6, "position": [0, 0.6, 0] }));
    ok(&mut tx, "prim.cylinder", json!({ "name": "post", "radius": 0.05, "height": 1.0, "segments": 8, "position": [2, 0, 0] }));
    ok(&mut tx, "scene.create_empty", json!({ "name": "hinge", "parent": "crate/lid" }));
    tx
}

/// The fixture plus an expectation `size` (lid dims), for `expect.*` examples.
pub fn expect_fixture() -> Tx {
    let mut tx = fixture();
    ok(&mut tx, "expect.add", json!({ "name": "size", "expect": { "dims": { "target": "lid", "height": { "max": "60mm" } } } }));
    tx
}

/// A box, an open polyline `path` (o:2) and a closed rectangle `outline` (o:3).
pub fn curve_fixture() -> Tx {
    let mut tx = Tx::new(Document::new());
    ok(&mut tx, "prim.box", json!({ "name": "crate", "width": 0.6, "height": 0.6, "depth": 0.6 }));
    ok(&mut tx, "curve.polyline", json!({ "name": "path", "points": [[0, 0, 0], [1, 0, 0], [1, 0, -1]] }));
    ok(&mut tx, "curve.rectangle", json!({ "name": "outline", "size": [0.8, 0.6] }));
    tx
}

/// The fixture plus bin (o:5), an open box without its top, tiles (o:6), a
/// triangulated 4 x 4 grid, and duct (o:7), two open tubes one above the
/// other, and panels (o:9), two touching 1 m planes joined but not welded,
/// and drill (o:11), a thin cylinder through crate,
/// for `edit.*` examples.
pub fn edit_fixture() -> Tx {
    let mut tx = fixture();
    ok(&mut tx, "prim.box", json!({ "name": "bin", "width": 0.4, "height": 0.3, "depth": 0.4, "position": [-2, 0, 0] }));
    ok(&mut tx, "edit.delete_elements", json!({ "target": "bin", "select": { "faces": { "group": "top" } } }));
    ok(&mut tx, "prim.plane", json!({ "name": "tiles", "width": 2, "depth": 2, "position": [0, 0, 3], "advanced": { "segments": [4, 4] } }));
    ok(&mut tx, "edit.triangulate", json!({ "target": "tiles" }));
    let tube = |name: &str, y: f64| json!({ "name": name, "radius": 0.1, "height": 0.4, "segments": 8, "cap": "none", "position": [3, y, 0] });
    ok(&mut tx, "prim.cylinder", tube("duct", 0.0));
    ok(&mut tx, "prim.cylinder", tube("duct_top", 0.6));
    ok(&mut tx, "scene.join", json!({ "targets": ["duct", "duct_top"] }));
    ok(&mut tx, "prim.plane", json!({ "name": "panels", "position": [5, 0, 0] }));
    ok(&mut tx, "prim.plane", json!({ "name": "panel_r", "position": [6, 0, 0] }));
    ok(&mut tx, "scene.join", json!({ "targets": ["panels", "panel_r"] }));
    ok(&mut tx, "prim.cylinder", json!({ "name": "drill", "radius": 0.1, "height": 1.0, "segments": 8, "position": [0, -0.2, 0] }));
    tx
}

/// A crate of two materials as OBJ text.
pub const CRATE_OBJ: &str = "o crate\nv -0.3 0 -0.3\nv 0.3 0 -0.3\nv 0.3 0 0.3\nv -0.3 0 0.3\nv -0.3 0.6 -0.3\nv 0.3 0.6 -0.3\nv 0.3 0.6 0.3\nv -0.3 0.6 0.3\n\
    usemtl wood\nf 1 2 3 4\nf 5 8 7 6\nf 1 5 6 2\nf 2 6 7 3\n\
    usemtl metal\nf 3 7 8 4\nf 4 8 5 1\n";

/// A fresh folder holding `props/crate.obj`, `props/open.obj` (the crate
/// without its top) and `outside.obj` next to it but outside; file access
/// is limited to the folder.
pub fn io_folder() -> (std::path::PathBuf, p3d_ops::FileAccess) {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let base = std::env::temp_dir().join(format!("p3d-ops-io-{}-{n}", std::process::id()));
    let dir = base.join("project");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(dir.join("props")).unwrap();
    std::fs::write(dir.join("props/crate.obj"), CRATE_OBJ).unwrap();
    std::fs::write(dir.join("props/open.obj"), CRATE_OBJ.replace("f 5 8 7 6\n", "")).unwrap();
    std::fs::write(base.join("outside.obj"), CRATE_OBJ).unwrap();
    let files = p3d_ops::FileAccess::new([dir.clone()]).unwrap();
    (dir, files)
}

/// The fixture an op category's examples run against.
pub fn for_category(c: Category) -> Tx {
    match c {
        Category::Prim | Category::Io => Tx::new(Document::new()),
        Category::Edit | Category::Shade | Category::Subd | Category::Deform | Category::Cleanup => edit_fixture(),
        Category::Expect => expect_fixture(),
        Category::Curve | Category::Gen | Category::Sdf => curve_fixture(),
        Category::Remesh => edit_fixture(),
        _ => fixture(),
    }
}
