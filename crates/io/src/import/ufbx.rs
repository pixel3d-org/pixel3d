//! FBX and OBJ through ufbx (a vendored C library, approved in project spec
//! 3.5). ufbx converts axes and units to pixel3d's (+Y up, +Z front,
//! meters) by changing geometry and transforms, and folds geometry
//! transforms into the meshes. UVs are flipped vertically: FBX and OBJ
//! start V at the bottom of the image, glTF and pixel3d at the top.

use glam::{DQuat, DVec3, Vec2, Vec3, Vec4};
use p3d_math::Trs;
use ufbx::{CoordinateAxes, CoordinateAxis, FileFormat, GeometryTransformHandling, LoadOpts, SpaceConversion};

use super::{Format, ImportError, ImportSpec, Imported, ImportedNode, Soup};

fn axes() -> CoordinateAxes {
    CoordinateAxes { right: CoordinateAxis::PositiveX, up: CoordinateAxis::PositiveY, front: CoordinateAxis::PositiveZ }
}

fn v3(v: ufbx::Vec3) -> Vec3 {
    DVec3::new(v.x, v.y, v.z).as_vec3()
}

/// Reads an FBX or OBJ file.
///
/// Complexity: O(file size) plus the repair of every mesh.
pub(crate) fn read(bytes: &[u8], format: Format, spec: &ImportSpec) -> Result<Imported, ImportError> {
    let opts = LoadOpts {
        target_axes: axes(),
        target_unit_meters: 1.0,
        space_conversion: SpaceConversion::ModifyGeometry,
        geometry_transform_handling: GeometryTransformHandling::ModifyGeometry,
        file_format: if format == Format::Obj { FileFormat::Obj } else { FileFormat::Fbx },
        obj_axes: axes(),
        obj_unit_meters: 1.0,
        ignore_animation: true,
        ignore_embedded: true,
        load_external_files: false,
        ..LoadOpts::default()
    };
    let scene = ufbx::load_memory(bytes, opts).map_err(|e| ImportError::Parse { format: format.name(), message: format!("{e:?}") })?;
    let mut out = Imported::default();
    let mut by_mesh: Vec<Option<usize>> = vec![None; scene.meshes.len()];
    for (mi, m) in scene.meshes.iter().enumerate() {
        let name = if m.element.name.is_empty() { format!("mesh_{mi}") } else { m.element.name.to_string() };
        let mut soup = Soup { positions: m.vertices.iter().map(|&p| v3(p)).collect(), ..Soup::default() };
        let uv_layers: Vec<&ufbx::VertexVec2> = m.uv_sets.iter().map(|s| &s.vertex_uv).filter(|u| u.exists).take(2).collect();
        let mut uvs: Vec<Vec<Vec2>> = vec![Vec::new(); uv_layers.len()];
        let normals = m.vertex_normal.exists.then_some(&m.vertex_normal);
        let colors = m.vertex_color.exists.then_some(&m.vertex_color);
        let mut ns = Vec::new();
        let mut cs = Vec::new();
        let mut skipped = 0usize;
        for (fi, face) in m.faces.iter().enumerate() {
            if face.num_indices < 3 {
                skipped += 1;
                continue;
            }
            soup.sizes.push(face.num_indices);
            soup.material.push(m.face_material.get(fi).copied().unwrap_or(0).min(u32::from(u16::MAX)) as u16);
            for ix in face.index_begin as usize..(face.index_begin + face.num_indices) as usize {
                soup.corners.push(m.vertex_indices[ix]);
                for (layer, out) in uv_layers.iter().zip(uvs.iter_mut()) {
                    let uv = layer.values[layer.indices[ix] as usize];
                    out.push(Vec2::new(uv.x as f32, 1.0 - uv.y as f32));
                }
                if let Some(n) = normals {
                    ns.push(v3(n.values[n.indices[ix] as usize]));
                }
                if let Some(c) = colors {
                    let v = c.values[c.indices[ix] as usize];
                    cs.push(Vec4::new(v.x as f32, v.y as f32, v.z as f32, v.w as f32));
                }
            }
        }
        if skipped > 0 {
            out.warnings.push(format!("{name}: skipped {skipped} point or line face{}", if skipped == 1 { "" } else { "s" }));
        }
        if soup.sizes.is_empty() {
            continue;
        }
        soup.uv = uvs;
        soup.normals = normals.map(|_| ns);
        soup.colors = colors.map(|_| cs);
        let materials: Vec<String> = m.materials.iter().map(|mat| mat.element.name.to_string()).collect();
        let materials = if materials.is_empty() { vec!["default".to_owned()] } else { materials };
        out.meshes.push(soup.finish(name, materials, spec)?);
        by_mesh[mi] = Some(out.meshes.len() - 1);
    }
    // Nodes by depth so parents come first; the file's root node is not kept.
    let mut nodes: Vec<&ufbx::Node> = scene.nodes.iter().map(|n| &**n).filter(|n| !n.is_root).collect();
    nodes.sort_by_key(|n| (n.node_depth, n.element.typed_id));
    let mut ours: Vec<Option<usize>> = vec![None; scene.nodes.len()];
    for n in nodes {
        let parent = n.parent.as_ref().filter(|p| !p.is_root).and_then(|p| ours.get(p.element.typed_id as usize).copied().flatten());
        let mesh = n.mesh.as_ref().and_then(|m| by_mesh.get(m.element.typed_id as usize).copied().flatten());
        let t = &n.local_transform;
        let transform = Trs {
            translation: v3(t.translation),
            rotation: DQuat::from_xyzw(t.rotation.x, t.rotation.y, t.rotation.z, t.rotation.w).as_quat().normalize(),
            scale: v3(t.scale),
        };
        let name = if n.element.name.is_empty() { format!("node_{}", n.element.typed_id) } else { n.element.name.to_string() };
        out.nodes.push(ImportedNode { name, parent, transform, mesh });
        if let Some(slot) = ours.get_mut(n.element.typed_id as usize) {
            *slot = Some(out.nodes.len() - 1);
        }
    }
    Ok(out)
}
