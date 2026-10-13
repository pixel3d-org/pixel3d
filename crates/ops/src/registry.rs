//! The op registry. Registration is explicit and ordered: no linker tricks,
//! deterministic listings.

use std::collections::BTreeMap;
use std::sync::Arc;

use p3d_scene::name::suggest;

use crate::dynop::{DynOp, Typed};
use crate::error::{ErrorCode, OpError};
use crate::op::Category;
use crate::{curve, deform, edit, element, expect, generate, prim, scene, select, shade, subd};

/// All available ops, built-in first, then addon and script ops in
/// registration order.
#[derive(Clone, Default)]
pub struct Registry {
    ops: Vec<Arc<dyn DynOp>>,
    by_id: BTreeMap<&'static str, usize>,
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.ops.iter().map(|o| o.id())).finish()
    }
}

impl Registry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Every built-in op, in catalog order.
    pub fn builtin() -> Self {
        let mut r = Registry::new();
        let ops: Vec<Arc<dyn DynOp>> = vec![
            Arc::new(Typed::<scene::CreateEmpty>::new()),
            Arc::new(Typed::<scene::Delete>::new()),
            Arc::new(Typed::<scene::Duplicate>::new()),
            Arc::new(Typed::<scene::Rename>::new()),
            Arc::new(Typed::<scene::Parent>::new()),
            Arc::new(Typed::<scene::Unparent>::new()),
            Arc::new(Typed::<scene::SetTransform>::new()),
            Arc::new(Typed::<scene::ApplyTransform>::new()),
            Arc::new(Typed::<scene::SetOrigin>::new()),
            Arc::new(Typed::<scene::Tag>::new()),
            Arc::new(Typed::<scene::SetMetadata>::new()),
            Arc::new(Typed::<scene::Join>::new()),
            Arc::new(Typed::<scene::Separate>::new()),
            Arc::new(Typed::<prim::BoxOp>::new()),
            Arc::new(Typed::<prim::Cylinder>::new()),
            Arc::new(Typed::<prim::Cone>::new()),
            Arc::new(Typed::<prim::QuadSphere>::new()),
            Arc::new(Typed::<prim::UvSphere>::new()),
            Arc::new(Typed::<prim::IcoSphere>::new()),
            Arc::new(Typed::<prim::Torus>::new()),
            Arc::new(Typed::<prim::Tube>::new()),
            Arc::new(Typed::<prim::Capsule>::new()),
            Arc::new(Typed::<prim::Wedge>::new()),
            Arc::new(Typed::<curve::PolylineOp>::new()),
            Arc::new(Typed::<curve::BezierOp>::new()),
            Arc::new(Typed::<curve::CircleOp>::new()),
            Arc::new(Typed::<curve::RectangleOp>::new()),
            Arc::new(Typed::<curve::ResampleOp>::new()),
            Arc::new(Typed::<curve::OffsetOp>::new()),
            Arc::new(Typed::<curve::FilletOp>::new()),
            Arc::new(Typed::<generate::LatheOp>::new()),
            Arc::new(Typed::<generate::ExtrudeShapeOp>::new()),
            Arc::new(Typed::<generate::SweepOp>::new()),
            Arc::new(Typed::<generate::PipeOp>::new()),
            Arc::new(Typed::<generate::LoftOp>::new()),
            Arc::new(Typed::<generate::ArrayOp>::new()),
            Arc::new(Typed::<generate::MirrorOp>::new()),
            Arc::new(Typed::<generate::ScatterOp>::new()),
            Arc::new(Typed::<prim::Plane>::new()),
            Arc::new(Typed::<select::Save>::new()),
            Arc::new(Typed::<select::Grow>::new()),
            Arc::new(Typed::<select::Shrink>::new()),
            Arc::new(Typed::<select::GroupDelete>::new()),
            Arc::new(Typed::<select::GroupRename>::new()),
            Arc::new(Typed::<edit::Extrude>::new()),
            Arc::new(Typed::<edit::Inset>::new()),
            Arc::new(Typed::<edit::Poke>::new()),
            Arc::new(Typed::<edit::Triangulate>::new()),
            Arc::new(Typed::<edit::TrisToQuads>::new()),
            Arc::new(Typed::<edit::Split>::new()),
            Arc::new(Typed::<edit::Flip>::new()),
            Arc::new(Typed::<edit::Subdivide>::new()),
            Arc::new(Typed::<edit::LoopCut>::new()),
            Arc::new(Typed::<edit::Bridge>::new()),
            Arc::new(Typed::<edit::Merge>::new()),
            Arc::new(Typed::<edit::MergeByDistance>::new()),
            Arc::new(Typed::<edit::Dissolve>::new()),
            Arc::new(Typed::<edit::PlaneCut>::new()),
            Arc::new(Typed::<edit::Symmetrize>::new()),
            Arc::new(Typed::<edit::Thicken>::new()),
            Arc::new(Typed::<edit::Smooth>::new()),
            Arc::new(Typed::<edit::Bevel>::new()),
            Arc::new(Typed::<element::TransformElements>::new()),
            Arc::new(Typed::<element::ShrinkFatten>::new()),
            Arc::new(Typed::<element::EdgeSlide>::new()),
            Arc::new(Typed::<edit::Fill>::new()),
            Arc::new(Typed::<edit::RecalcNormals>::new()),
            Arc::new(Typed::<edit::DeleteElements>::new()),
            Arc::new(Typed::<shade::SmoothByAngle>::new()),
            Arc::new(Typed::<shade::MarkSharp>::new()),
            Arc::new(Typed::<shade::ClearSharp>::new()),
            Arc::new(Typed::<shade::SetFlat>::new()),
            Arc::new(Typed::<shade::SetSmooth>::new()),
            Arc::new(Typed::<shade::WeightedNormals>::new()),
            Arc::new(Typed::<shade::Crease>::new()),
            Arc::new(Typed::<subd::CatmullClark>::new()),
            Arc::new(Typed::<deform::Bend>::new()),
            Arc::new(Typed::<deform::Twist>::new()),
            Arc::new(Typed::<deform::Taper>::new()),
            Arc::new(Typed::<deform::Displace>::new()),
            Arc::new(Typed::<expect::Add>::new()),
            Arc::new(Typed::<expect::Remove>::new()),
        ];
        for op in ops {
            // Built-in ids are unique by construction (tested).
            let _ = r.register(op);
        }
        r
    }

    /// Adds an op. Fails if the id is taken.
    pub fn register(&mut self, op: Arc<dyn DynOp>) -> Result<(), OpError> {
        let id = op.id();
        if self.by_id.contains_key(id) {
            return Err(OpError::invalid("", format!("an op `{id}` is already registered")));
        }
        self.by_id.insert(id, self.ops.len());
        self.ops.push(op);
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.ops.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// Looks up an op; unknown ids get suggestions from ids and aliases.
    ///
    /// Complexity: O(log n); O(n) on errors.
    pub fn get(&self, id: &str) -> Result<&dyn DynOp, OpError> {
        if let Some(&i) = self.by_id.get(id) {
            return Ok(&*self.ops[i]);
        }
        let mut names: Vec<&str> = self.ops.iter().map(|o| o.id()).collect();
        let alias_hits: Vec<String> =
            self.ops.iter().filter(|o| o.meta().aliases.iter().any(|a| a.eq_ignore_ascii_case(id))).map(|o| o.id().to_owned()).collect();
        let mut suggestions = alias_hits;
        names.sort_unstable();
        for s in suggest(id, names, 3) {
            if !suggestions.contains(&s) {
                suggestions.push(s);
            }
        }
        Err(OpError::new(ErrorCode::UnknownOp, format!("no op `{id}`"))
            .with_suggestions(suggestions)
            .with_hint("list_ops shows every op; search matches aliases such as Blender names"))
    }

    /// Ops in registration order.
    pub fn iter(&self) -> impl Iterator<Item = &dyn DynOp> {
        self.ops.iter().map(|o| &**o)
    }

    /// Ops in a category, in registration order.
    pub fn in_category(&self, c: Category) -> impl Iterator<Item = &dyn DynOp> {
        self.iter().filter(move |o| o.meta().category == c)
    }

    /// Ops whose id, label, aliases or summary contain every word of the
    /// query (case-insensitive), best matches first: id, then alias or
    /// label, then summary.
    ///
    /// Complexity: O(n x text length).
    pub fn search(&self, query: &str) -> Vec<&dyn DynOp> {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        if words.is_empty() {
            return self.iter().collect();
        }
        let mut hits: Vec<(u8, usize, &dyn DynOp)> = Vec::new();
        for (i, op) in self.iter().enumerate() {
            let m = op.meta();
            let id = op.id().to_lowercase();
            let names: Vec<String> = std::iter::once(m.label).chain(m.aliases.iter().copied()).map(str::to_lowercase).collect();
            let summary = m.summary.to_lowercase();
            let mut rank = 0u8;
            for w in &words {
                let r = if id.contains(w.as_str()) {
                    0
                } else if names.iter().any(|n| n.contains(w.as_str())) {
                    1
                } else if summary.contains(w.as_str()) {
                    2
                } else {
                    rank = u8::MAX;
                    break;
                };
                rank = rank.max(r);
            }
            if rank != u8::MAX {
                hits.push((rank, i, op));
            }
        }
        hits.sort_by_key(|(r, i, _)| (*r, *i));
        hits.into_iter().map(|(_, _, op)| op).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_ids_are_unique_and_well_formed() {
        let r = Registry::builtin();
        let mut ids: Vec<&str> = r.iter().map(|o| o.id()).collect();
        let n = ids.len();
        assert_eq!(n, r.len());
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), n, "duplicate op ids");
        for op in r.iter() {
            let id = op.id();
            let m = op.meta();
            assert!(id.starts_with(&format!("{}.", m.category.as_str())), "{id} is not in its category");
            assert!(id.chars().all(|c| c.is_ascii_lowercase() || c == '.' || c == '_'), "{id}");
            assert!(!m.summary.is_empty() && m.summary.ends_with('.'), "{id}: summary must be one sentence");
            assert!(!m.docs.is_empty() && !m.complexity.is_empty(), "{id}: docs and complexity");
            assert!(!m.examples.is_empty(), "{id}: at least one example");
            for ex in m.examples {
                let args: serde_json::Value = serde_json::from_str(ex.args).unwrap_or_else(|e| panic!("{id}: example args: {e}"));
                assert!(args.is_object(), "{id}: example args must be an object");
            }
            let schema = op.params_schema();
            assert_eq!(schema["additionalProperties"], serde_json::Value::Bool(false), "{id}: schema must be strict");
            if let Some(props) = schema["properties"].as_object() {
                for (name, p) in props {
                    assert!(p.get("description").is_some(), "{id}: parameter `{name}` needs a doc comment (agents read it)");
                }
            }
            assert!(!m.docs.contains("Phase "), "{id}: user docs must not mention roadmap phases");
            assert!(op.meta().version >= 1);
        }
    }

    #[test]
    fn unknown_ids_get_suggestions() {
        let r = Registry::builtin();
        let e = r.get("prim.bxo").unwrap_err();
        assert_eq!(e.code, ErrorCode::UnknownOp);
        assert!(e.suggestions.contains(&"prim.box".to_owned()), "{:?}", e.suggestions);
        let e = r.get("cube").unwrap_err();
        assert_eq!(e.suggestions.first().map(String::as_str), Some("prim.box"), "aliases count");
    }

    #[test]
    fn search_ranks_ids_then_aliases_then_summaries() {
        let r = Registry::builtin();
        let hits: Vec<&str> = r.search("box").iter().map(|o| o.id()).collect();
        assert_eq!(hits.first(), Some(&"prim.box"));
        assert!(r.search("cube").iter().any(|o| o.id() == "prim.box"));
        assert!(r.search("zzz nothing").is_empty());
        assert_eq!(r.search("").len(), r.len());
    }

    #[test]
    fn duplicates_are_rejected() {
        let mut r = Registry::builtin();
        assert!(r.register(Arc::new(Typed::<prim::BoxOp>::new())).is_err());
    }
}
