#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use base64::Engine;
use glam::Vec3;
use p3d_mesh::EdgeFlags;
use serde_json::json;

use super::*;

fn none(_: &str) -> Result<Vec<u8>, ImportError> {
    Err(ImportError::Empty)
}

/// A unit cube as glTF stores it: 24 vertices (flat normals), 36 u32
/// indices, per-face UVs; as one buffer.
fn cube_buffer() -> (Vec<u8>, usize, usize, usize) {
    let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
        ([0.0, 0.0, 1.0], [[-0.5, -0.5, 0.5], [0.5, -0.5, 0.5], [0.5, 0.5, 0.5], [-0.5, 0.5, 0.5]]),
        ([0.0, 0.0, -1.0], [[0.5, -0.5, -0.5], [-0.5, -0.5, -0.5], [-0.5, 0.5, -0.5], [0.5, 0.5, -0.5]]),
        ([1.0, 0.0, 0.0], [[0.5, -0.5, 0.5], [0.5, -0.5, -0.5], [0.5, 0.5, -0.5], [0.5, 0.5, 0.5]]),
        ([-1.0, 0.0, 0.0], [[-0.5, -0.5, -0.5], [-0.5, -0.5, 0.5], [-0.5, 0.5, 0.5], [-0.5, 0.5, -0.5]]),
        ([0.0, 1.0, 0.0], [[-0.5, 0.5, 0.5], [0.5, 0.5, 0.5], [0.5, 0.5, -0.5], [-0.5, 0.5, -0.5]]),
        ([0.0, -1.0, 0.0], [[-0.5, -0.5, -0.5], [0.5, -0.5, -0.5], [0.5, -0.5, 0.5], [-0.5, -0.5, 0.5]]),
    ];
    let mut pos = Vec::new();
    let mut nrm = Vec::new();
    let mut uv = Vec::new();
    let mut idx = Vec::new();
    for (i, (n, quad)) in faces.iter().enumerate() {
        for (k, p) in quad.iter().enumerate() {
            pos.extend(p.iter().flat_map(|x| x.to_le_bytes()));
            nrm.extend(n.iter().flat_map(|x| x.to_le_bytes()));
            let t = [[0.0f32, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]][k];
            uv.extend(t.iter().flat_map(|x| x.to_le_bytes()));
        }
        let b = 4 * i as u32;
        idx.extend([b, b + 1, b + 2, b, b + 2, b + 3].iter().flat_map(|x| x.to_le_bytes()));
    }
    let (lp, ln, lu) = (pos.len(), nrm.len(), uv.len());
    let mut all = pos;
    all.extend(nrm);
    all.extend(uv);
    all.extend(idx);
    (all, lp, ln, lu)
}

fn cube_json(uri: Option<String>, len: usize, lp: usize, ln: usize, lu: usize) -> serde_json::Value {
    let mut buffer = json!({ "byteLength": len });
    if let Some(u) = uri {
        buffer["uri"] = json!(u);
    }
    json!({
        "asset": { "version": "2.0" },
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": [
            { "name": "crate", "mesh": 0, "translation": [0, 1, 0], "children": [1] },
            { "name": "lid", "mesh": 0, "scale": [1, 0.1, 1], "translation": [0, 0.6, 0] }
        ],
        "materials": [{ "name": "Wood" }],
        "meshes": [{ "name": "Cube", "primitives": [{
            "attributes": { "POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2 },
            "indices": 3,
            "material": 0
        }] }],
        "buffers": [buffer],
        "bufferViews": [
            { "buffer": 0, "byteOffset": 0, "byteLength": lp },
            { "buffer": 0, "byteOffset": lp, "byteLength": ln },
            { "buffer": 0, "byteOffset": lp + ln, "byteLength": lu },
            { "buffer": 0, "byteOffset": lp + ln + lu, "byteLength": 144 }
        ],
        "accessors": [
            { "bufferView": 0, "componentType": 5126, "count": 24, "type": "VEC3", "min": [-0.5, -0.5, -0.5], "max": [0.5, 0.5, 0.5] },
            { "bufferView": 1, "componentType": 5126, "count": 24, "type": "VEC3" },
            { "bufferView": 2, "componentType": 5126, "count": 24, "type": "VEC2" },
            { "bufferView": 3, "componentType": 5125, "count": 36, "type": "SCALAR" }
        ]
    })
}

fn check_cube(out: &Imported) {
    assert_eq!(out.meshes.len(), 1);
    let m = &out.meshes[0];
    assert_eq!(m.name, "Cube");
    assert_eq!(m.materials, vec!["Wood".to_owned()]);
    assert_eq!((m.mesh.vert_count(), m.mesh.face_count()), (8, 12));
    assert!(m.mesh.edges().all(|e| !m.mesh.is_boundary_edge(e)));
    assert_eq!(m.repair.welded, 16);
    // The 12 cube edges are hard; the quad diagonals are smooth.
    let sharp = m.mesh.edges().filter(|&e| m.mesh.edge_flags(e).contains(EdgeFlags::SHARP)).count();
    assert_eq!(sharp, 12);
    assert!(m.mesh.corner_normal.is_some());
    assert_eq!(m.mesh.uv_layer_count(), 1);
    // Both nodes use the one mesh; the lid is the crate's child.
    assert_eq!(out.nodes.len(), 2);
    assert_eq!(out.nodes[0].name, "crate");
    assert_eq!(out.nodes[1].parent, Some(0));
    assert_eq!(out.nodes[1].mesh, Some(0));
    assert_eq!(out.nodes[0].transform.translation, Vec3::Y);
    assert_eq!(out.nodes[1].transform.scale, Vec3::new(1.0, 0.1, 1.0));
}

#[test]
fn gltf_with_a_data_uri() {
    let (buf, lp, ln, lu) = cube_buffer();
    let uri = format!("data:application/octet-stream;base64,{}", base64::engine::general_purpose::STANDARD.encode(&buf));
    let text = cube_json(Some(uri), buf.len(), lp, ln, lu).to_string();
    let out = read(text.as_bytes(), Format::Gltf, &none, &ImportSpec::default()).unwrap();
    check_cube(&out);
}

#[test]
fn gltf_with_an_external_buffer_asks_for_it() {
    let (buf, lp, ln, lu) = cube_buffer();
    let text = cube_json(Some("cube%20data.bin".into()), buf.len(), lp, ln, lu).to_string();
    let load = |uri: &str| -> Result<Vec<u8>, ImportError> {
        assert_eq!(uri, "cube data.bin");
        Ok(buf.clone())
    };
    check_cube(&read(text.as_bytes(), Format::Gltf, &load, &ImportSpec::default()).unwrap());
    // A short buffer is an error, not a crash.
    let short = |_: &str| -> Result<Vec<u8>, ImportError> { Ok(vec![0; 10]) };
    assert!(matches!(read(text.as_bytes(), Format::Gltf, &short, &ImportSpec::default()), Err(ImportError::Parse { .. })));
}

#[test]
fn glb_reads_its_binary_chunk() {
    let (buf, lp, ln, lu) = cube_buffer();
    let mut text = cube_json(None, buf.len(), lp, ln, lu).to_string().into_bytes();
    while !text.len().is_multiple_of(4) {
        text.push(b' ');
    }
    let mut bin = buf.clone();
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let total = 12 + 8 + text.len() + 8 + bin.len();
    let mut glb = Vec::new();
    glb.extend(b"glTF");
    glb.extend(2u32.to_le_bytes());
    glb.extend((total as u32).to_le_bytes());
    glb.extend((text.len() as u32).to_le_bytes());
    glb.extend(b"JSON");
    glb.extend(&text);
    glb.extend((bin.len() as u32).to_le_bytes());
    glb.extend(b"BIN\0");
    glb.extend(&bin);
    check_cube(&read(&glb, Format::Glb, &none, &ImportSpec::default()).unwrap());
}

#[test]
fn obj_quads_with_two_materials() {
    let obj = "o box\nv -1 0 -1\nv 1 0 -1\nv 1 0 1\nv -1 0 1\nv -1 2 -1\nv 1 2 -1\nv 1 2 1\nv -1 2 1\n\
               vt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\n\
               usemtl red\nf 1/1 2/2 3/3 4/4\nf 5/1 8/2 7/3 6/4\n\
               usemtl blue\nf 1/1 5/2 6/3 2/4\nf 2/1 6/2 7/3 3/4\nf 3/1 7/2 8/3 4/4\nf 4/1 8/2 5/3 1/4\n";
    let out = read(obj.as_bytes(), Format::Obj, &none, &ImportSpec::default()).unwrap();
    assert_eq!(out.meshes.len(), 1);
    let m = &out.meshes[0];
    assert_eq!((m.mesh.vert_count(), m.mesh.face_count()), (8, 6));
    assert!(m.mesh.faces().all(|f| m.mesh.face_len(f) == 4));
    assert!(m.mesh.edges().all(|e| !m.mesh.is_boundary_edge(e)));
    assert_eq!(m.materials, vec!["red".to_owned(), "blue".to_owned()]);
    let (lo, hi) = m.mesh.bounds().unwrap();
    assert_eq!((lo, hi), (Vec3::new(-1.0, 0.0, -1.0), Vec3::new(1.0, 2.0, 1.0)));
    // V is flipped: the first corner's vt 0 0 becomes (0, 1).
    let f = m.mesh.faces().next().unwrap();
    let uvs: Vec<_> = m.mesh.face_halfedges(f).map(|h| m.mesh.corner_uv(0, h).unwrap()).collect();
    assert!(uvs.contains(&glam::Vec2::new(0.0, 1.0)), "{uvs:?}");
    assert!(out.nodes.iter().any(|n| n.mesh == Some(0)));
}

/// A cube in centimeters, its model moved 100 cm up, as ASCII FBX 7.4.
const FBX_CUBE: &str = "; FBX 7.4.0 project file
FBXHeaderExtension:  {
	FBXHeaderVersion: 1003
	FBXVersion: 7400
}
GlobalSettings:  {
	Version: 1000
	Properties70:  {
		P: \"UpAxis\", \"int\", \"Integer\", \"\",1
		P: \"UpAxisSign\", \"int\", \"Integer\", \"\",1
		P: \"FrontAxis\", \"int\", \"Integer\", \"\",2
		P: \"FrontAxisSign\", \"int\", \"Integer\", \"\",1
		P: \"CoordAxis\", \"int\", \"Integer\", \"\",0
		P: \"CoordAxisSign\", \"int\", \"Integer\", \"\",1
		P: \"UnitScaleFactor\", \"double\", \"Number\", \"\",1
	}
}
Objects:  {
	Geometry: 1000, \"Geometry::Cube\", \"Mesh\" {
		Vertices: *24 {
			a: -50,-50,50,50,-50,50,-50,50,50,50,50,50,-50,50,-50,50,50,-50,-50,-50,-50,50,-50,-50
		}
		PolygonVertexIndex: *24 {
			a: 0,1,3,-3,2,3,5,-5,4,5,7,-7,6,7,1,-1,1,7,5,-4,6,0,2,-5
		}
		GeometryVersion: 124
	}
	Model: 2000, \"Model::Cube\", \"Mesh\" {
		Version: 232
		Properties70:  {
			P: \"Lcl Translation\", \"Lcl Translation\", \"\", \"A\",0,100,0
		}
	}
}
Connections:  {
	C: \"OO\",2000,0
	C: \"OO\",1000,2000
}
";

#[test]
fn fbx_converts_centimeters_to_meters() {
    let out = read(FBX_CUBE.as_bytes(), Format::Fbx, &none, &ImportSpec::default()).unwrap();
    assert_eq!(out.meshes.len(), 1);
    let m = &out.meshes[0];
    assert_eq!((m.mesh.vert_count(), m.mesh.face_count()), (8, 6));
    assert!(m.mesh.edges().all(|e| !m.mesh.is_boundary_edge(e)));
    let node = out.nodes.iter().find(|n| n.name == "Cube").unwrap();
    assert_eq!(node.mesh, Some(0));
    // 50 cm and 100 cm in meters, wherever ufbx puts the scale.
    let (lo, hi) = m.mesh.bounds().unwrap();
    let size = (hi - lo) * node.transform.scale;
    assert!((size - Vec3::ONE).abs().max_element() < 1e-4, "{size:?}");
    assert!((node.transform.translation - Vec3::Y).length() < 1e-4, "{:?}", node.transform.translation);
}

#[test]
fn broken_files_fail_cleanly() {
    assert!(matches!(read(b"not json", Format::Gltf, &none, &ImportSpec::default()), Err(ImportError::Parse { .. })));
    assert!(matches!(read(b"\x00\x01garbage", Format::Fbx, &none, &ImportSpec::default()), Err(ImportError::Parse { .. })));
    assert_eq!(read(b"# nothing\n", Format::Obj, &none, &ImportSpec::default()).unwrap_err(), ImportError::Empty);
    assert_eq!(Format::from_path(Path::new("a/b.GLB")), Some(Format::Glb));
    assert_eq!(Format::from_path(Path::new("a/b.blend")), None);
}
