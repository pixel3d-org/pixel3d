//! `expect.*`: tests for models, stored in the document (project spec 17.11).

use p3d_scene::{Check, Document, Expectation, ObjectRef, Range};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{ErrorCode, OpError};
use crate::op::{Category, Op, OpCtx, OpExample, OpFlags, OpMeta, OpOutput};
use crate::params::Length;
use crate::tx::Tx;

/// A range of lengths; either end may be left open.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LengthRange {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<Length>,
}

impl LengthRange {
    fn range(&self) -> Range {
        Range { min: self.min.map(Length::meters), max: self.max.map(Length::meters) }
    }
}

/// A mirror axis.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MirrorAxis {
    /// Left-right (across the YZ plane).
    #[default]
    X,
    /// Up-down.
    Y,
    /// Front-back.
    Z,
}

/// What to test.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ExpectArgs {
    /// World size of an object together with its children.
    Dims {
        /// Object (name, path or id).
        target: String,
        /// Size along X.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        width: Option<LengthRange>,
        /// Size along Y.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        height: Option<LengthRange>,
        /// Size along Z.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        depth: Option<LengthRange>,
    },
    /// `a` and `b` are mirror images across the world plane `axis = at`;
    /// without `b`, `a` (with its children) mirrors itself across its
    /// center plane.
    Symmetric {
        /// First object.
        a: String,
        /// Its mirror partner.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        b: Option<String>,
        /// Mirror axis (default x: left-right).
        #[serde(default)]
        axis: MirrorAxis,
        /// Plane position; 0 for pairs, the center for a single object.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at: Option<Length>,
        /// Largest allowed deviation (default 1 mm).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tolerance: Option<Length>,
    },
    /// `part` touches `to`.
    Attached {
        /// The attached object.
        part: String,
        /// What it attaches to.
        to: String,
        /// Largest allowed gap (default 1 mm).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_gap: Option<Length>,
        /// Deepest it may sink in (no limit by default).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_penetration: Option<Length>,
    },
    /// No two of the objects pass through or sit inside each other.
    NotIntersecting {
        /// 2 to 64 objects.
        parts: Vec<String>,
    },
    /// Triangle count of an object with its children, or of the scene.
    Budget {
        /// Object; the whole scene when omitted.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<String>,
        /// Upper limit (default: the asset class budget).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_tris: Option<u64>,
        /// Lower limit.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min_tris: Option<u64>,
    },
    /// The object's meshes are closed and face outward.
    Watertight {
        /// Object (name, path or id).
        target: String,
    },
    /// A tree or plant (an object with a `foliage` or `trunk` group): crown size, silhouette coverage and overdraw.
    Vegetation {
        /// Object (name, path or id).
        target: String,
        /// Crown width (the larger of its X and Z extents).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        crown_width: Option<LengthRange>,
        /// Crown height.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        crown_height: Option<LengthRange>,
        /// Covered share of the crown silhouette (front and side mean), for example `{"min": 0.85}`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        coverage: Option<ShareRange>,
        /// Overdraw budget: most card layers per covered pixel on average.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_overdraw: Option<f64>,
    },
}

/// A range of shares from 0 to 1; either end may be left open.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShareRange {
    /// Lowest share.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    /// Highest share.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
}

/// Parameters of `expect.add`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddParams {
    /// Name; `E1`, `E2`, ... when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The test: one of dims, symmetric, attached, not_intersecting, budget, watertight, vegetation.
    pub expect: ExpectArgs,
}

impl Default for AddParams {
    fn default() -> Self {
        AddParams { name: None, expect: ExpectArgs::Watertight { target: String::new() } }
    }
}

fn object_ref(doc: &Document, name: &str, path: &str) -> Result<ObjectRef, OpError> {
    let k = doc.resolve(name).map_err(|e| OpError::from_scene(e, path))?;
    let id = doc.object(k).map(|o| o.id()).ok_or_else(|| OpError::internal("object vanished"))?;
    Ok(ObjectRef { name: name.to_owned(), id })
}

fn non_negative(l: Option<Length>, path: &str) -> Result<(), OpError> {
    match l {
        Some(x) if !(x.0.is_finite() && x.0 >= 0.0) => Err(OpError::out_of_range(path, "must be zero or more")),
        _ => Ok(()),
    }
}

/// The canonical check with objects resolved.
fn to_check(doc: &Document, a: &ExpectArgs) -> Result<Check, OpError> {
    Ok(match a {
        ExpectArgs::Dims { target, width, height, depth } => {
            if width.is_none() && height.is_none() && depth.is_none() {
                return Err(OpError::invalid("/expect/dims", "give at least one of width, height, depth"));
            }
            Check::Dims {
                target: object_ref(doc, target, "/expect/dims/target")?,
                width: width.as_ref().map(LengthRange::range),
                height: height.as_ref().map(LengthRange::range),
                depth: depth.as_ref().map(LengthRange::range),
            }
        }
        ExpectArgs::Symmetric { a, b, axis, at, tolerance } => {
            non_negative(*tolerance, "/expect/symmetric/tolerance")?;
            Check::Symmetric {
                a: object_ref(doc, a, "/expect/symmetric/a")?,
                b: b.as_deref().map(|b| object_ref(doc, b, "/expect/symmetric/b")).transpose()?,
                axis: match axis {
                    MirrorAxis::X => 'x',
                    MirrorAxis::Y => 'y',
                    MirrorAxis::Z => 'z',
                },
                at: at.map(Length::meters),
                tolerance: tolerance.map_or(1e-3, Length::meters),
            }
        }
        ExpectArgs::Attached { part, to, max_gap, max_penetration } => {
            non_negative(*max_gap, "/expect/attached/max_gap")?;
            non_negative(*max_penetration, "/expect/attached/max_penetration")?;
            Check::Attached {
                part: object_ref(doc, part, "/expect/attached/part")?,
                to: object_ref(doc, to, "/expect/attached/to")?,
                max_gap: max_gap.map_or(1e-3, Length::meters),
                max_penetration: max_penetration.map(Length::meters),
            }
        }
        ExpectArgs::NotIntersecting { parts } => {
            if !(2..=p3d_perceive::expect::MAX_NOT_INTERSECTING).contains(&parts.len()) {
                return Err(OpError::out_of_range("/expect/not_intersecting/parts", format!("{} objects; give 2 to 64", parts.len())));
            }
            let refs: Result<Vec<ObjectRef>, OpError> =
                parts.iter().enumerate().map(|(i, p)| object_ref(doc, p, &format!("/expect/not_intersecting/parts/{i}"))).collect();
            Check::NotIntersecting { parts: refs? }
        }
        ExpectArgs::Budget { target, max_tris, min_tris } => {
            let max = max_tris.unwrap_or(u64::from(doc.settings.tri_budget()));
            if min_tris.is_some_and(|m| m > max) {
                return Err(OpError::out_of_range("/expect/budget/min_tris", format!("{} is above max_tris {max}", min_tris.unwrap_or(0))));
            }
            Check::Budget {
                target: target.as_deref().map(|t| object_ref(doc, t, "/expect/budget/target")).transpose()?,
                min_tris: *min_tris,
                max_tris: max,
            }
        }
        ExpectArgs::Watertight { target } => Check::Watertight { target: object_ref(doc, target, "/expect/watertight/target")? },
        ExpectArgs::Vegetation { target, crown_width, crown_height, coverage, max_overdraw } => {
            if let Some(c) = coverage
                && [c.min, c.max].iter().flatten().any(|x| !(0.0..=1.0).contains(x))
            {
                return Err(OpError::out_of_range("/expect/vegetation/coverage", "shares are between 0 and 1"));
            }
            if max_overdraw.is_some_and(|x| !(x.is_finite() && x > 0.0)) {
                return Err(OpError::out_of_range("/expect/vegetation/max_overdraw", "must be above 0"));
            }
            if crown_width.is_none() && crown_height.is_none() && coverage.is_none() && max_overdraw.is_none() {
                return Err(OpError::invalid("/expect/vegetation", "give at least one of crown_width, crown_height, coverage, max_overdraw"));
            }
            Check::Vegetation {
                target: object_ref(doc, target, "/expect/vegetation/target")?,
                crown_width: crown_width.as_ref().map(LengthRange::range),
                crown_height: crown_height.as_ref().map(LengthRange::range),
                coverage: coverage.as_ref().map(|c| Range { min: c.min, max: c.max }),
                max_overdraw: *max_overdraw,
            }
        }
    })
}

/// `expect.add`.
pub struct Add;

impl Op for Add {
    const ID: &'static str = "expect.add";
    type Params = AddParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Add expectation",
            category: Category::Expect,
            summary: "Adds a test for the model (size, symmetry, attachment, no intersections, budget, watertight) that runs after every change.",
            docs: "Expectations are tests for models: every response that changes the scene ends with a test line such as \
                   `tests 3/4 pass; failing: E2 attached(lid, crate)`. Write them first and model until they pass.\n\n\
                   - `dims`: size of an object with its children, each of width (X), height (Y), depth (Z) as `{min, max}`.\n\
                   - `symmetric`: `a` and `b` mirror each other across `axis = at` (default the left-right plane x = 0); \
                   without `b`, `a` mirrors itself across its center.\n\
                   - `attached`: gap at most `max_gap` (1 mm) and, if given, sinking in at most `max_penetration`.\n\
                   - `not_intersecting`: none of the listed objects pass through or sit inside each other.\n\
                   - `budget`: triangle count, by default at most the asset class budget.\n\
                   - `watertight`: closed meshes facing outward (needed for 3D printing, bakes and volume).\n\n\
                   Objects are remembered by id, so renaming them keeps the test.",
            examples: &[OpExample {
                args: r#"{"expect": {"attached": {"part": "lid", "to": "crate"}}}"#,
                result: "added E1 attached(lid, crate): passes now (gap 0 mm)",
            }],
            aliases: &["test", "assert", "constraint check", "requirement"],
            complexity: "O(1) to add; the check it runs once is that check's cost",
            flags: OpFlags::UNDOABLE,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &AddParams) -> Result<(), OpError> {
        to_check(doc, &p.expect)?;
        if let Some(n) = &p.name {
            if n.is_empty() || n.len() > 64 || n.trim() != n || n.chars().any(char::is_control) {
                return Err(OpError::invalid("/name", "names are 1 to 64 characters without surrounding spaces"));
            }
            if doc.expectations().iter().any(|e| &e.name == n) {
                return Err(OpError::invalid("/name", format!("an expectation `{n}` exists")).with_suggestions(vec![doc.next_expectation_name()]));
            }
        }
        Ok(())
    }

    fn apply(tx: &mut Tx, p: &AddParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let check = to_check(tx.doc(), &p.expect)?;
        let name = p.name.clone().unwrap_or_else(|| tx.doc().next_expectation_name());
        let e = Expectation { name: name.clone(), check };
        let outcome = p3d_perceive::expect::check(tx.doc(), &e);
        tx.doc_mut().expectations_mut().push(e);
        let state = if outcome.pass { "passes now" } else { "fails now" };
        Ok(OpOutput::summary(format!("added {name} {}: {state} ({})", outcome.label, outcome.message)))
    }
}

/// Parameters of `expect.remove`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoveParams {
    /// Name of the expectation, for example `E2`.
    pub name: String,
}

/// `expect.remove`.
pub struct Remove;

impl Op for Remove {
    const ID: &'static str = "expect.remove";
    type Params = RemoveParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Remove expectation",
            category: Category::Expect,
            summary: "Removes an expectation by name.",
            docs: "Removes one test. `expect list` shows the names.",
            examples: &[OpExample { args: r#"{"name": "size"}"#, result: "removed size dims(lid); 0 left" }],
            aliases: &["delete test", "drop expectation"],
            complexity: "O(expectations)",
            flags: OpFlags::UNDOABLE,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &RemoveParams) -> Result<(), OpError> {
        if doc.expectations().iter().any(|e| e.name == p.name) {
            return Ok(());
        }
        Err(OpError::new(ErrorCode::NotFound, format!("no expectation `{}`", p.name)).at("/name").with_suggestions(p3d_scene::name::suggest(
            &p.name,
            doc.expectations().iter().map(|e| e.name.as_str()),
            3,
        )))
    }

    fn apply(tx: &mut Tx, p: &RemoveParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let list = tx.doc_mut().expectations_mut();
        let i =
            list.iter().position(|e| e.name == p.name).ok_or_else(|| OpError::new(ErrorCode::NotFound, format!("no expectation `{}`", p.name)))?;
        let e = list.remove(i);
        let left = list.len();
        Ok(OpOutput::summary(format!("removed {} {}; {left} left", e.name, e.check.label())))
    }
}
