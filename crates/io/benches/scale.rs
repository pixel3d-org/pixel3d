//! Scaling check inputs for `cargo xtask bench-scale`: importing a GLB of a
//! sphere stored as glTF stores it (a vertex per corner with flat normals,
//! u32 indices) at n, 4n and 16n triangles.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use p3d_io::import::{Format, ImportError, ImportSpec, read};
use p3d_testdata::{scale_sizes, sphere};

/// A GLB with one triangle mesh: positions, normals and indices.
fn glb(tris: usize) -> Vec<u8> {
    let m = sphere(tris);
    let (mut pos, mut nrm, mut idx) = (Vec::new(), Vec::new(), Vec::new());
    let mut count = 0u32;
    for f in m.faces() {
        let n = m.face_normal(f);
        for t in m.face_triangles(f) {
            for h in t {
                let p = m.positions[m.origin(h) as usize];
                pos.extend(p.to_array().iter().flat_map(|x| x.to_le_bytes()));
                nrm.extend(n.to_array().iter().flat_map(|x| x.to_le_bytes()));
                idx.extend(count.to_le_bytes());
                count += 1;
            }
        }
    }
    let (lp, ln, li) = (pos.len(), nrm.len(), idx.len());
    let mut bin = pos;
    bin.extend(nrm);
    bin.extend(idx);
    let json = serde_json::json!({
        "asset": { "version": "2.0" },
        "meshes": [{ "primitives": [{ "attributes": { "POSITION": 0, "NORMAL": 1 }, "indices": 2 }] }],
        "nodes": [{ "mesh": 0 }],
        "scenes": [{ "nodes": [0] }],
        "buffers": [{ "byteLength": bin.len() }],
        "bufferViews": [
            { "buffer": 0, "byteOffset": 0, "byteLength": lp },
            { "buffer": 0, "byteOffset": lp, "byteLength": ln },
            { "buffer": 0, "byteOffset": lp + ln, "byteLength": li }
        ],
        "accessors": [
            { "bufferView": 0, "componentType": 5126, "count": count, "type": "VEC3", "min": [-1, -1, -1], "max": [1, 1, 1] },
            { "bufferView": 1, "componentType": 5126, "count": count, "type": "VEC3" },
            { "bufferView": 2, "componentType": 5125, "count": count, "type": "SCALAR" }
        ]
    });
    let mut text = json.to_string().into_bytes();
    while !text.len().is_multiple_of(4) {
        text.push(b' ');
    }
    let total = 12 + 8 + text.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend(b"glTF");
    out.extend(2u32.to_le_bytes());
    out.extend((total as u32).to_le_bytes());
    out.extend((text.len() as u32).to_le_bytes());
    out.extend(b"JSON");
    out.extend(&text);
    out.extend((bin.len() as u32).to_le_bytes());
    out.extend(b"BIN\0");
    out.extend(&bin);
    out
}

fn none(_: &str) -> Result<Vec<u8>, ImportError> {
    Err(ImportError::Empty)
}

fn scale(c: &mut Criterion) {
    let sizes = scale_sizes(4_000, 100_000);
    let mut g = c.benchmark_group("scale_import_glb");
    for &t in &sizes {
        let bytes = glb(t);
        g.bench_with_input(BenchmarkId::from_parameter(t), &bytes, |b, bytes| {
            b.iter(|| read(black_box(bytes), Format::Glb, &none, &ImportSpec::default()))
        });
    }
    g.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(20).warm_up_time(std::time::Duration::from_millis(500)).measurement_time(std::time::Duration::from_secs(2));
    targets = scale
}
criterion_main!(benches);
