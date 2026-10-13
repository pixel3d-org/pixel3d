//! Reading glTF, GLB, OBJ and FBX files into pixel3d's units and axes
//! (meters, +Y up, +Z front; project spec 6.1 and 16.1). glTF goes through
//! the `gltf` crate, OBJ and FBX through ufbx. Every mesh is repaired from
//! the file's polygon soup into a 2-manifold mesh ([`p3d_geo::soup`]),
//! keeping UVs, normals (as custom split normals, with sharp edges where
//! they split), colors and material slots, and the repair is reported.

mod gltf;
mod ufbx;

use std::path::{Path, PathBuf};

use glam::{Vec2, Vec3, Vec4};
use p3d_geo::soup::{SoupError, SoupReport, SoupSpec, repair};
use p3d_math::Trs;
use p3d_mesh::{EdgeFlags, MAX_UV_LAYERS, Mesh};

use crate::sandbox::{FileAccess, SandboxError};

/// Largest file an import reads.
pub const MAX_FILE_BYTES: u64 = 2 << 30;

/// A file format pixel3d imports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Gltf,
    Glb,
    Obj,
    Fbx,
}

impl Format {
    /// The format of a file name's extension.
    pub fn from_path(p: &Path) -> Option<Format> {
        match p.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "gltf" => Some(Format::Gltf),
            "glb" => Some(Format::Glb),
            "obj" => Some(Format::Obj),
            "fbx" => Some(Format::Fbx),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Format::Gltf => "glTF",
            Format::Glb => "GLB",
            Format::Obj => "OBJ",
            Format::Fbx => "FBX",
        }
    }
}

/// Options of an import.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ImportSpec {
    /// Vertices closer than this weld; 0 welds identical positions only.
    pub weld: f32,
}

/// A node of the file's hierarchy.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedNode {
    pub name: String,
    /// Index of the parent node; parents come before their children.
    pub parent: Option<usize>,
    pub transform: Trs,
    /// Index into [`Imported::meshes`]; several nodes may share a mesh.
    pub mesh: Option<usize>,
}

/// One mesh of the file.
#[derive(Debug, Clone, Default)]
pub struct ImportedMesh {
    pub name: String,
    pub mesh: Mesh,
    /// Material name per material slot (the faces' `face_material`).
    pub materials: Vec<String>,
    pub repair: SoupReport,
}

/// A file's contents.
#[derive(Debug, Clone, Default)]
pub struct Imported {
    pub nodes: Vec<ImportedNode>,
    pub meshes: Vec<ImportedMesh>,
    /// What was skipped or changed (points and lines, missing data).
    pub warnings: Vec<String>,
}

/// Why an import failed.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ImportError {
    #[error("{0}")]
    Sandbox(#[from] SandboxError),
    #[error("`{0}` is not a glTF, GLB, OBJ or FBX file")]
    UnknownFormat(String),
    #[error("cannot read the {format} file: {message}")]
    Parse { format: &'static str, message: String },
    #[error("the file is {0} bytes, more than an import reads")]
    TooLarge(u64),
    #[error("mesh `{mesh}`: {error}")]
    Repair { mesh: String, error: SoupError },
    #[error("the file has no meshes")]
    Empty,
}

/// Reads a file from inside the allowed folders; buffers and other files it
/// refers to must be inside them too. Returns the canonical path.
///
/// Complexity: O(file size) plus the repair, O(C) expected in corners.
pub fn read_file(access: &FileAccess, path: &str, spec: &ImportSpec) -> Result<(PathBuf, Imported), ImportError> {
    let file = access.readable(path)?;
    let format = Format::from_path(&file).ok_or_else(|| ImportError::UnknownFormat(path.to_owned()))?;
    let bytes = read_bytes(&file)?;
    let resources = |uri: &str| -> Result<Vec<u8>, ImportError> { read_bytes(&access.sibling(&file, uri)?) };
    let imported = read(&bytes, format, &resources, spec)?;
    Ok((file, imported))
}

fn read_bytes(p: &Path) -> Result<Vec<u8>, ImportError> {
    let io = |e: std::io::Error| ImportError::Sandbox(SandboxError::Io { path: p.display().to_string(), message: e.to_string() });
    let len = std::fs::metadata(p).map_err(io)?.len();
    if len > MAX_FILE_BYTES {
        return Err(ImportError::TooLarge(len));
    }
    std::fs::read(p).map_err(io)
}

/// Reads a file's bytes; `resources` loads other files it refers to by
/// relative URI (glTF buffers).
///
/// Complexity: O(file size) plus the repair, O(C) expected in corners.
pub fn read(
    bytes: &[u8],
    format: Format,
    resources: &dyn Fn(&str) -> Result<Vec<u8>, ImportError>,
    spec: &ImportSpec,
) -> Result<Imported, ImportError> {
    let out = match format {
        Format::Gltf | Format::Glb => gltf::read(bytes, format, resources, spec)?,
        Format::Obj | Format::Fbx => ufbx::read(bytes, format, spec)?,
    };
    if out.meshes.is_empty() {
        return Err(ImportError::Empty);
    }
    Ok(out)
}

/// A mesh as the file stores it: positions, polygons, and corner and face
/// attributes indexed by the input corner and face.
#[derive(Debug, Clone, Default)]
pub(crate) struct Soup {
    pub positions: Vec<Vec3>,
    pub sizes: Vec<u32>,
    pub corners: Vec<u32>,
    /// UV layers, one value per input corner.
    pub uv: Vec<Vec<Vec2>>,
    pub normals: Option<Vec<Vec3>>,
    pub colors: Option<Vec<Vec4>>,
    /// Material slot per input face.
    pub material: Vec<u16>,
}

impl Soup {
    /// Repairs the soup and moves the attributes onto the mesh.
    ///
    /// Complexity: O(C) expected in corners.
    pub fn finish(self, name: String, materials: Vec<String>, spec: &ImportSpec) -> Result<ImportedMesh, ImportError> {
        let r = repair(&self.positions, &self.sizes, &self.corners, &SoupSpec { weld: spec.weld })
            .map_err(|error| ImportError::Repair { mesh: name.clone(), error })?;
        let mut mesh = r.mesh;
        fn pick<T: Copy>(src: &[u32], col: &[T]) -> Vec<T> {
            src.iter().map(|&c| col[c as usize]).collect()
        }
        for layer in self.uv.iter().take(MAX_UV_LAYERS) {
            mesh.add_uv_layer_from_polygon_order(&pick(&r.corner_src, layer));
        }
        if let Some(n) = &self.normals {
            let vals: Vec<Vec3> = pick(&r.corner_src, n).into_iter().map(|v| v.normalize_or_zero()).collect();
            let mut column = vec![Vec3::ZERO; mesh.he_slots()];
            mesh.corner_values_from_polygon_order(&mut column, &vals);
            mesh.corner_normal = Some(column);
        }
        if let Some(c) = &self.colors {
            let mut column = vec![Vec4::ONE; mesh.he_slots()];
            mesh.corner_values_from_polygon_order(&mut column, &pick(&r.corner_src, c));
            mesh.corner_color = Some(column);
        }
        let faces: Vec<u32> = mesh.faces().collect();
        for (f, &src) in faces.iter().zip(&r.face_src) {
            mesh.face_material[*f as usize] = self.material.get(src as usize).copied().unwrap_or(0);
        }
        mark_splits(&mut mesh);
        Ok(ImportedMesh { name, mesh, materials, repair: r.report })
    }
}

/// Marks edges sharp where the normals on either side differ (the file's
/// hard edges) and as seams where UVs do.
///
/// Complexity: O(E).
fn mark_splits(mesh: &mut Mesh) {
    let cos = 1f32.to_radians().cos();
    let edges: Vec<u32> = mesh.edges().collect();
    for e in edges {
        let (h, t) = (2 * e, 2 * e + 1);
        if mesh.face(h).is_none() || mesh.face(t).is_none() {
            continue;
        }
        // Corners at the two ends on either side: (h, next(t)) at h's origin,
        // (next(h), t) at its end.
        let pairs = [(h, mesh.next(t)), (mesh.next(h), t)];
        let sharp = mesh.corner_normal.as_ref().is_some_and(|n| pairs.iter().any(|&(a, b)| n[a as usize].dot(n[b as usize]) < cos));
        let seam = (0..mesh.uv_layer_count().min(1))
            .any(|l| pairs.iter().any(|&(a, b)| mesh.corner_uv(l, a).zip(mesh.corner_uv(l, b)).is_some_and(|(x, y)| x.distance(y) > 1e-6)));
        if sharp {
            mesh.set_edge_flag(e, EdgeFlags::SHARP, true);
        }
        if seam {
            mesh.set_edge_flag(e, EdgeFlags::SEAM, true);
        }
    }
}

#[cfg(test)]
mod tests;
