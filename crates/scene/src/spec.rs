//! Model Specs as stored in the document (project spec 6.7, ADR 0010): the
//! source exactly as the author wrote it, and the record of the last
//! compile. Parsing, validation and compilation live in `p3d-spec`; this
//! crate only keeps the data.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// A Model Spec on its root object.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SpecRecord {
    /// The Model Spec as written (JSON).
    pub source: serde_json::Value,
    /// What the last compile made, by part name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub parts: BTreeMap<String, PartRecord>,
}

/// One compiled part.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PartRecord {
    /// Content hash of everything that shaped the part's mesh.
    pub hash: u64,
    /// Public id of the part's object; none when the part is blended into
    /// its parent's mesh.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object: Option<u32>,
    /// Fingerprint of the mesh as compiled, so edits made outside the spec
    /// are noticed.
    #[serde(default)]
    pub fingerprint: u64,
    #[serde(default)]
    pub provenance: Provenance,
}

/// Which spec node made each face of a compiled mesh.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Provenance {
    pub entries: Vec<ProvEntry>,
    /// Entry index per live face in slot order, run-length encoded as
    /// `[entry, count]`.
    pub runs: Vec<[u32; 2]>,
}

/// A spec node and the op that built it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ProvEntry {
    /// JSON pointer into the spec, for example `/parts/head/sdf/1`.
    pub path: String,
    /// The op that produced the faces, for example `sdf.mesh`.
    pub op: String,
}

impl Provenance {
    /// Run-length encodes one entry index per face.
    ///
    /// Complexity: O(faces).
    pub fn from_faces(entries: Vec<ProvEntry>, per_face: &[u32]) -> Provenance {
        let mut runs: Vec<[u32; 2]> = Vec::new();
        for &e in per_face {
            match runs.last_mut() {
                Some(r) if r[0] == e => r[1] += 1,
                _ => runs.push([e, 1]),
            }
        }
        Provenance { entries, runs }
    }

    /// The entry index of every face, in order.
    ///
    /// Complexity: O(faces).
    pub fn per_face(&self) -> Vec<u32> {
        self.runs.iter().flat_map(|&[e, n]| std::iter::repeat_n(e, n as usize)).collect()
    }

    /// Number of faces covered.
    ///
    /// Complexity: O(runs).
    pub fn face_count(&self) -> usize {
        self.runs.iter().map(|r| r[1] as usize).sum()
    }

    /// The entry of the `i`-th live face.
    ///
    /// Complexity: O(runs).
    pub fn entry_of(&self, i: usize) -> Option<&ProvEntry> {
        let mut seen = 0usize;
        for &[e, n] in &self.runs {
            seen += n as usize;
            if i < seen {
                return self.entries.get(e as usize);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provenance_round_trips_through_runs() {
        let entries = vec![
            ProvEntry { path: "/parts/a/sdf/0".into(), op: "sdf.mesh".into() },
            ProvEntry { path: "/parts/a/sdf/1".into(), op: "sdf.mesh".into() },
        ];
        let faces = [0, 0, 0, 1, 1, 0];
        let p = Provenance::from_faces(entries, &faces);
        assert_eq!(p.runs, vec![[0, 3], [1, 2], [0, 1]]);
        assert_eq!(p.per_face(), faces);
        assert_eq!(p.face_count(), 6);
        assert_eq!(p.entry_of(4).map(|e| e.path.as_str()), Some("/parts/a/sdf/1"));
        assert_eq!(p.entry_of(6), None);
    }
}
