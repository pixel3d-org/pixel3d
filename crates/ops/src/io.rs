//! `io.*`: bringing files in (and, from Phase 5, writing them out). Paths
//! must lie inside the session's allowed folders (project spec 3.8, 17.19).

use std::path::Path;
use std::sync::Arc;

use p3d_geo::cleanup::{CleanupSpec, cleanup};
use p3d_geo::soup::SoupReport;
use p3d_io::import::{Format, ImportError, ImportSpec, read_file};
use p3d_io::sandbox::SandboxError;
use p3d_scene::name::MAX_NAME_LEN;
use p3d_scene::{Document, ObjectKey, ObjectKind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::cleanup::would;
use crate::common::{check_length, check_new_object, label, thousands};
use crate::error::{ErrorCode, OpError};
use crate::op::{Category, Op, OpCall, OpCtx, OpExample, OpFlags, OpMeta, OpOutput, Warning};
use crate::params::Length;
use crate::tx::Tx;

/// Parameters of `io.import`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportParams {
    /// File to read: `.glb`, `.gltf`, `.fbx` or `.obj`, inside the allowed folders. Relative paths start in the project folder.
    pub path: String,
    /// Name of the object that holds the file's contents. Default: the file's single top object keeps its name; several get a parent named after the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Parent object for the import.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Vertices closer than this weld while repairing the meshes; 0 (default) welds identical positions only.
    #[serde(default)]
    pub weld: Length,
}

/// `io.import`.
pub struct Import;

/// A file's names as usable object names: no slashes, control characters
/// or id prefix, trimmed and shortened.
fn clean_name(raw: &str, fallback: &str) -> String {
    let mut s: String = raw.trim().chars().map(|c| if c == '/' || c.is_control() { '_' } else { c }).collect();
    s = s.trim().to_owned();
    if let Some(rest) = s.strip_prefix("o:") {
        s = format!("o_{rest}");
    }
    while s.len() > MAX_NAME_LEN {
        s.pop();
    }
    let s = s.trim().to_owned();
    if s.is_empty() { fallback.to_owned() } else { s }
}

fn import_error(e: ImportError, path: &str) -> OpError {
    let hint_roots = "pixel3d-cli mcp reads inside the folder it starts in; add folders with --allow-dir";
    match e {
        ImportError::Sandbox(SandboxError::NotFound(p)) => OpError::new(ErrorCode::NotFound, format!("`{p}` does not exist"))
            .at("/path")
            .with_hint("check the file name; relative paths start in the project folder"),
        ImportError::Sandbox(s @ (SandboxError::NoRoots | SandboxError::Outside { .. })) => {
            OpError::new(ErrorCode::PathNotAllowed, s.to_string()).at("/path").with_hint(hint_roots)
        }
        ImportError::Sandbox(s) => OpError::new(ErrorCode::Io, s.to_string()).at("/path"),
        ImportError::UnknownFormat(_) => OpError::new(ErrorCode::Unsupported, format!("`{path}` is not a glTF, GLB, OBJ or FBX file"))
            .at("/path")
            .with_hint("export the model from its tool as .glb or .fbx"),
        ImportError::TooLarge(n) => {
            OpError::new(ErrorCode::BudgetExceeded, format!("the file is {} bytes, more than an import reads", thousands(n as usize)))
        }
        e @ (ImportError::Parse { .. } | ImportError::Empty) => OpError::new(ErrorCode::Io, e.to_string()).at("/path"),
        e @ ImportError::Repair { .. } => {
            OpError::new(ErrorCode::NonManifoldInput, e.to_string()).at("/path").with_hint("try a small `weld`, or fix the mesh in its tool")
        }
    }
}

/// `welded 16 vertices, flipped 2 faces` for the repairs that happened.
fn repairs(r: &SoupReport) -> Vec<String> {
    let mut out = Vec::new();
    let mut add = |n: usize, what: &str| {
        if n > 0 {
            out.push(format!("{what} {}", thousands(n)));
        }
    };
    add(r.degenerate, "dropped degenerate faces:");
    add(r.duplicates, "dropped repeated faces:");
    add(r.flipped, "turned faces to agree:");
    add(r.split_edges, "split non-manifold edges:");
    add(r.split_verts, "split non-manifold vertices:");
    add(r.detached, "detached faces:");
    out
}

impl Op for Import {
    const ID: &'static str = "io.import";
    type Params = ImportParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Import",
            category: Category::Io,
            summary: "Reads a glTF, GLB, OBJ or FBX file into the scene, in meters with +Y up, repairing its meshes.",
            docs: "Reads every mesh and node of the file with its hierarchy, transforms, UVs, normals (kept as custom normals; \
                   edges where they split become sharp), vertex colors and material slots (their names are in the object's \
                   `materials` metadata). FBX and OBJ are converted to meters, +Y up and +Z front; glTF already is.\n\n\
                   File meshes are polygon soup: every mesh is repaired into a clean manifold mesh (welded seams, repeated and \
                   degenerate faces dropped, winding unified, non-manifold edges and vertices split), and the summary says what \
                   was repaired and what `cleanup.auto` would still fix. The path must be inside the allowed folders.\n\n\
                   **Use for** bringing in reference models, kitbash parts and assets to optimize, UV or bake.",
            examples: &[OpExample {
                args: r#"{"path": "props/crate.obj"}"#,
                result: "imported crate.obj (OBJ) into crate (o:1): 1 object, 1 mesh, 6 faces",
            }],
            aliases: &["import", "open model", "load gltf", "load fbx", "load obj", "file import", "append"],
            complexity: "O(file size) to read, O(C) expected to repair C corners, O(n log n) for the cleanup report",
            flags: OpFlags::UNDOABLE.union(OpFlags::CREATES_OBJECTS).union(OpFlags::LONG_RUNNING),
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &ImportParams) -> Result<(), OpError> {
        if p.path.trim().is_empty() {
            return Err(OpError::invalid("/path", "give a file path"));
        }
        if Format::from_path(Path::new(&p.path)).is_none() {
            return Err(import_error(ImportError::UnknownFormat(p.path.clone()), &p.path));
        }
        check_new_object(doc, p.name.as_deref(), p.parent.as_deref())?;
        check_length("/weld", p.weld, 0.0, 0.1)
    }

    fn apply(tx: &mut Tx, p: &ImportParams, ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let parent = check_new_object(tx.doc(), p.name.as_deref(), p.parent.as_deref())?;
        let files = ctx.files.ok_or_else(|| import_error(ImportError::Sandbox(SandboxError::NoRoots), &p.path))?;
        let (file, imported) = read_file(files, &p.path, &ImportSpec { weld: p.weld.0 as f32 }).map_err(|e| import_error(e, &p.path))?;
        ctx.check_cancel()?;
        let format = Format::from_path(&file).map_or("file", Format::name);
        let stem = clean_name(&file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(), "import");
        let file_name = file.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();

        // One top node becomes the root; several get a parent named after the file.
        let tops = imported.nodes.iter().filter(|n| n.parent.is_none()).count();
        let mut out = OpOutput::default();
        let (root, top_parent) = if tops == 1 {
            (None, parent)
        } else {
            let name = p.name.clone().unwrap_or_else(|| tx.doc().unique_name(&stem, parent));
            let k = tx.doc_mut().create_object(&name, parent, ObjectKind::Empty).map_err(|e| OpError::from_scene(e, "/name"))?;
            out.created.push(k);
            (Some(k), Some(k))
        };
        // Meshes used by several nodes are shared, like the file's instances.
        let uses: Vec<usize> = (0..imported.meshes.len()).map(|i| imported.nodes.iter().filter(|n| n.mesh == Some(i)).count()).collect();
        let mut shared: Vec<Option<p3d_scene::MeshKey>> = vec![None; imported.meshes.len()];
        let mut keys: Vec<ObjectKey> = Vec::with_capacity(imported.nodes.len());
        let mut first = None;
        for (i, n) in imported.nodes.iter().enumerate() {
            let parent_key = match n.parent {
                Some(pi) => keys.get(pi).copied(),
                None => top_parent,
            };
            let name = match (&p.name, root, n.parent) {
                (Some(want), None, None) => want.clone(),
                _ => tx.doc().unique_name(&clean_name(&n.name, &format!("node_{i}")), parent_key),
            };
            let kind = match n.mesh {
                Some(mi) if uses[mi] > 1 => {
                    let mk = match shared[mi] {
                        Some(k) => k,
                        None => {
                            let k = tx.doc_mut().add_shared_mesh(Arc::new(imported.meshes[mi].mesh.clone()));
                            shared[mi] = Some(k);
                            k
                        }
                    };
                    ObjectKind::Mesh { mesh: mk }
                }
                Some(mi) => ObjectKind::Mesh { mesh: tx.doc_mut().add_mesh(imported.meshes[mi].mesh.clone()) },
                None => ObjectKind::Empty,
            };
            let k = tx.doc_mut().create_object(&name, parent_key, kind).map_err(|e| OpError::from_scene(e, "/name"))?;
            if let Some(o) = tx.doc_mut().object_mut(k) {
                o.transform = n.transform;
                if let Some(mi) = n.mesh {
                    o.metadata.insert("materials".into(), serde_json::json!(imported.meshes[mi].materials));
                }
            }
            if first.is_none() {
                first = Some(k);
            }
            keys.push(k);
            out.created.push(k);
        }
        let top = root.or(first).ok_or_else(|| OpError::new(ErrorCode::Io, "the file has no nodes"))?;
        if let Some(o) = tx.doc_mut().object_mut(top) {
            o.metadata.insert("source".into(), serde_json::json!(file_name));
        }

        // What the repair did and what cleanup.auto would still do.
        let mut total = SoupReport::default();
        let mut faces = 0usize;
        for m in &imported.meshes {
            let r = &m.repair;
            faces += m.mesh.face_count();
            total.welded += r.welded;
            total.degenerate += r.degenerate;
            total.duplicates += r.duplicates;
            total.flipped += r.flipped;
            total.split_edges += r.split_edges;
            total.split_verts += r.split_verts;
            total.detached += r.detached;
        }
        for w in &imported.warnings {
            out.warnings.push(Warning { message: w.clone(), fix: None });
        }
        for (i, n) in imported.nodes.iter().enumerate() {
            let Some(mi) = n.mesh else { continue };
            if shared[mi].is_some() && imported.nodes.iter().position(|o| o.mesh == Some(mi)) != Some(i) {
                continue;
            }
            let mut copy = imported.meshes[mi].mesh.clone();
            let r = cleanup(&mut copy, &CleanupSpec::default());
            let text = would(&r);
            if !text.ends_with("change nothing") {
                let target = tx.doc().path(keys[i]).unwrap_or_default();
                out.warnings.push(Warning {
                    message: format!("{}: {text}", label(tx.doc(), keys[i])),
                    fix: Some(OpCall { op: "cleanup.auto".into(), args: serde_json::json!({ "target": target }) }),
                });
            }
        }
        let fixed = repairs(&total);
        let repaired = if fixed.is_empty() { String::new() } else { format!("; repaired: {}", fixed.join(", ")) };
        let meshes = imported.meshes.len();
        out.summary = format!(
            "imported {file_name} ({format}) into {}: {} object{}, {} mesh{}, {} faces{repaired}",
            label(tx.doc(), top),
            keys.len() + usize::from(root.is_some()),
            if keys.len() + usize::from(root.is_some()) == 1 { "" } else { "s" },
            meshes,
            if meshes == 1 { "" } else { "es" },
            thousands(faces)
        );
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_become_object_names() {
        assert_eq!(clean_name("  Wheel/Front\t", "x"), "Wheel_Front");
        assert_eq!(clean_name("o:12", "x"), "o_12");
        assert_eq!(clean_name("   ", "fallback"), "fallback");
        assert_eq!(clean_name(&"a".repeat(300), "x").len(), MAX_NAME_LEN);
    }
}
