//! glTF 2.0 and GLB through the `gltf` crate. glTF already uses pixel3d's
//! units and axes, so nothing is converted.

use base64::Engine;
use glam::{Quat, Vec2, Vec3, Vec4};
use gltf::buffer::Source;
use gltf::mesh::Mode;
use p3d_math::Trs;

use super::{Format, ImportError, ImportSpec, Imported, ImportedNode, Soup};

fn parse(message: impl Into<String>) -> ImportError {
    ImportError::Parse { format: "glTF", message: message.into() }
}

/// Decodes `%XX` escapes in a relative URI.
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(v) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The bytes of every buffer: the GLB chunk, data URIs, or files next to
/// the main file.
///
/// Complexity: O(buffer bytes).
fn buffers(g: &gltf::Gltf, resources: &dyn Fn(&str) -> Result<Vec<u8>, ImportError>) -> Result<Vec<Vec<u8>>, ImportError> {
    let mut out = Vec::new();
    for b in g.document.buffers() {
        let data = match b.source() {
            Source::Bin => g.blob.clone().ok_or_else(|| parse("the binary chunk is missing"))?,
            Source::Uri(uri) => match uri.strip_prefix("data:") {
                Some(rest) => {
                    let (_, payload) = rest.split_once(";base64,").ok_or_else(|| parse("only base64 data URIs are supported"))?;
                    base64::engine::general_purpose::STANDARD.decode(payload).map_err(|e| parse(format!("bad data URI: {e}")))?
                }
                None => resources(&percent_decode(uri))?,
            },
        };
        if data.len() < b.length() {
            return Err(parse(format!("buffer {} has {} bytes but needs {}", b.index(), data.len(), b.length())));
        }
        out.push(data);
    }
    Ok(out)
}

/// Triangles of a primitive's index list in its drawing mode.
///
/// Complexity: O(indices).
fn triangles(mode: Mode, idx: &[u32]) -> Vec<[u32; 3]> {
    match mode {
        Mode::Triangles => idx.as_chunks::<3>().0.to_vec(),
        Mode::TriangleStrip => (0..idx.len().saturating_sub(2))
            .map(|i| if i % 2 == 0 { [idx[i], idx[i + 1], idx[i + 2]] } else { [idx[i + 1], idx[i], idx[i + 2]] })
            .collect(),
        Mode::TriangleFan => (1..idx.len().saturating_sub(1)).map(|i| [idx[0], idx[i], idx[i + 1]]).collect(),
        _ => Vec::new(),
    }
}

/// Reads a glTF or GLB file.
///
/// Complexity: O(file size) plus the repair of every mesh.
pub(crate) fn read(
    bytes: &[u8],
    _format: Format,
    resources: &dyn Fn(&str) -> Result<Vec<u8>, ImportError>,
    spec: &ImportSpec,
) -> Result<Imported, ImportError> {
    let g = gltf::Gltf::from_slice(bytes).map_err(|e| parse(e.to_string()))?;
    let data = buffers(&g, resources)?;
    let mut out = Imported::default();
    // Our mesh index for each file mesh (meshes without triangles are skipped).
    let mut by_file: Vec<Option<usize>> = vec![None; g.document.meshes().len()];
    for m in g.document.meshes() {
        let name = m.name().map_or_else(|| format!("mesh_{}", m.index()), str::to_owned);
        let mut soup = Soup::default();
        let mut slots: Vec<Option<usize>> = Vec::new();
        let mut materials: Vec<String> = Vec::new();
        let (mut has_normals, mut has_colors, mut uv_sets) = (true, true, 2usize);
        let mut normals = Vec::new();
        let mut colors = Vec::new();
        let mut uvs: [Vec<Vec2>; 2] = [Vec::new(), Vec::new()];
        for prim in m.primitives() {
            let tris = |idx: &[u32]| triangles(prim.mode(), idx);
            if !matches!(prim.mode(), Mode::Triangles | Mode::TriangleStrip | Mode::TriangleFan) {
                out.warnings.push(format!("{name}: skipped a primitive of points or lines"));
                continue;
            }
            let reader = prim.reader(|b| data.get(b.index()).map(Vec::as_slice));
            let Some(pos) = reader.read_positions() else {
                out.warnings.push(format!("{name}: skipped a primitive without positions"));
                continue;
            };
            let pos: Vec<Vec3> = pos.map(Vec3::from).collect();
            let n = pos.len() as u32;
            let idx: Vec<u32> = reader.read_indices().map_or_else(|| (0..n).collect(), |i| i.into_u32().collect());
            if let Some(&bad) = idx.iter().find(|&&i| i >= n) {
                return Err(parse(format!("{name}: index {bad} is past the {n} vertices")));
            }
            let vn: Option<Vec<Vec3>> = reader.read_normals().map(|it| it.map(Vec3::from).collect());
            let vc: Option<Vec<Vec4>> = reader.read_colors(0).map(|it| it.into_rgba_f32().map(Vec4::from).collect());
            let vt: [Option<Vec<Vec2>>; 2] = [0, 1].map(|s| reader.read_tex_coords(s).map(|it| it.into_f32().map(Vec2::from).collect()));
            has_normals &= vn.is_some();
            has_colors &= vc.is_some();
            uv_sets = uv_sets.min(vt.iter().take_while(|t| t.is_some()).count());
            let key = prim.material().index();
            let slot = match slots.iter().position(|s| *s == key) {
                Some(s) => s,
                None => {
                    slots.push(key);
                    materials.push(
                        prim.material().name().map_or_else(|| key.map_or_else(|| "default".to_owned(), |k| format!("material_{k}")), str::to_owned),
                    );
                    slots.len() - 1
                }
            };
            let base = soup.positions.len() as u32;
            soup.positions.extend(pos);
            for t in tris(&idx) {
                soup.sizes.push(3);
                soup.material.push(slot.min(u16::MAX as usize) as u16);
                for v in t {
                    soup.corners.push(base + v);
                    normals.push(vn.as_ref().map_or(Vec3::ZERO, |x| x[v as usize]));
                    colors.push(vc.as_ref().map_or(Vec4::ONE, |x| x[v as usize]));
                    for (k, layer) in uvs.iter_mut().enumerate() {
                        layer.push(vt[k].as_ref().map_or(Vec2::ZERO, |x| x[v as usize]));
                    }
                }
            }
        }
        if soup.sizes.is_empty() {
            continue;
        }
        let [uv0, uv1] = uvs;
        soup.uv = [uv0, uv1].into_iter().take(uv_sets).collect();
        soup.normals = has_normals.then_some(normals);
        soup.colors = has_colors.then_some(colors);
        out.meshes.push(soup.finish(name, materials, spec)?);
        by_file[m.index()] = Some(out.meshes.len() - 1);
    }
    let scene = g.document.default_scene().or_else(|| g.document.scenes().next());
    match scene {
        Some(s) => {
            let mut stack: Vec<(gltf::Node, Option<usize>)> = s.nodes().map(|n| (n, None)).collect();
            stack.reverse();
            let mut seen = vec![false; g.document.nodes().len()];
            while let Some((node, parent)) = stack.pop() {
                // A node listed twice (a cycle in a broken file) is read once.
                if std::mem::replace(&mut seen[node.index()], true) {
                    out.warnings.push(format!("node {} appears more than once; read once", node.index()));
                    continue;
                }
                let (t, r, sc) = node.transform().decomposed();
                let mesh = node.mesh().and_then(|m| by_file[m.index()]);
                let name = node
                    .name()
                    .map(str::to_owned)
                    .or_else(|| node.mesh().and_then(|m| m.name().map(str::to_owned)))
                    .unwrap_or_else(|| format!("node_{}", node.index()));
                let transform = Trs { translation: Vec3::from(t), rotation: Quat::from_array(r).normalize(), scale: Vec3::from(sc) };
                out.nodes.push(ImportedNode { name, parent, transform, mesh });
                let me = out.nodes.len() - 1;
                stack.extend(node.children().collect::<Vec<_>>().into_iter().rev().map(|c| (c, Some(me))));
            }
        }
        None => {
            for (i, m) in out.meshes.iter().enumerate() {
                out.nodes.push(ImportedNode { name: m.name.clone(), parent: None, transform: Trs::IDENTITY, mesh: Some(i) });
            }
        }
    }
    Ok(out)
}
