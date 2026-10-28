use super::MeasurementQuery;
use crate::{
    document::{Document, EntityId},
    project_file::ContentHash,
    references::{AnalyticGeometry, TopologyKey},
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasurementStamp {
    pub document: EntityId,
    pub revision: u64,
    pub source_hash: ContentHash,
}
impl MeasurementStamp {
    pub fn capture(document: &Document, revision: u64) -> Result<Self, String> {
        Ok(Self {
            document: document.id(),
            revision,
            source_hash: ContentHash::document(document)?,
        })
    }
    /// Explicit action/publication boundary. Cache document/revision for UI paint.
    pub fn check(&self, document: &Document, revision: u64) -> Result<(), String> {
        if *self != Self::capture(document, revision)? {
            return Err("Measurement belongs to stale source; evaluate the current model".into());
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementFailureKind {
    Missing,
    Ambiguous,
    Stale,
    Unsupported,
    Invalid,
    Cancelled,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasurementFailure {
    pub kind: MeasurementFailureKind,
    pub reference: Option<EntityId>,
    pub message: String,
}
impl std::fmt::Display for MeasurementFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for MeasurementFailure {}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScalarMetric {
    Millimeters { value: f64 },
    Radians { value: f64 },
    Unsupported { reason: String },
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasurementWitness {
    pub a_mm: [f64; 3],
    pub b_mm: [f64; 3],
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasurementValues {
    pub minimum_mm: f64,
    pub witnesses: Vec<MeasurementWitness>,
    pub witnesses_complete: bool,
    pub total_solutions: u32,
    pub inner_solution: bool,
    pub axis_components_mm: Option<[f64; 3]>,
    pub axis_warning: Option<String>,
    /// Unoriented acute line/plane angle. Native edge orientation is not a stable
    /// authored direction, so an arbitrary directed angle is never reported.
    pub angle: ScalarMetric,
    /// Supporting-plane separation, distinct from the finite-face minimum.
    pub parallel_plane_separation: ScalarMetric,
    pub tolerance_mm: f64,
    pub boolean_fuzzy_mm: f64,
    pub key_a: TopologyKey,
    pub key_b: TopologyKey,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasurementEvaluation {
    pub stamp: MeasurementStamp,
    pub query: MeasurementQuery,
    pub result: Result<MeasurementValues, MeasurementFailure>,
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}
fn unit(a: [f64; 3]) -> [f64; 3] {
    let n = dot(a, a).sqrt();
    a.map(|x| x / n)
}
/// Supporting analytic metrics supplement an exact finite B-rep minimum. They
/// are never used as a substitute for that minimum.
pub fn analytic_metrics(
    a: Option<AnalyticGeometry>,
    b: Option<AnalyticGeometry>,
) -> Result<(ScalarMetric, ScalarMetric), String> {
    if let Some(a) = a {
        a.validate()?;
    }
    if let Some(b) = b {
        b.validate()?;
    }
    use AnalyticGeometry::{Line, Plane};
    let unsupported = |reason: &str| ScalarMetric::Unsupported {
        reason: reason.into(),
    };
    let direction = |a: [f64; 3], b: [f64; 3]| unit(std::array::from_fn(|i| b[i] - a[i]));
    let angle = match (a, b) {
        (Some(Line { a_mm: a, b_mm: b }), Some(Line { a_mm: c, b_mm: d })) => Some(
            dot(direction(a, b), direction(c, d))
                .abs()
                .clamp(0., 1.)
                .acos(),
        ),
        (Some(Plane { frame: a }), Some(Plane { frame: b })) => {
            Some(dot(a.normal(), b.normal()).abs().clamp(0., 1.).acos())
        }
        (Some(Line { a_mm: a, b_mm: b }), Some(Plane { frame }))
        | (Some(Plane { frame }), Some(Line { a_mm: a, b_mm: b })) => Some(
            dot(direction(a, b), frame.normal())
                .abs()
                .clamp(0., 1.)
                .asin(),
        ),
        _ => None,
    }
    .map_or_else(
        || unsupported("Angle requires straight edges or planar faces"),
        |value| ScalarMetric::Radians { value },
    );
    let separation = match (a, b) {
        (Some(Plane { frame: a }), Some(Plane { frame: b }))
            if dot(a.normal(), b.normal()).abs() > 1. - 1e-10 =>
        {
            ScalarMetric::Millimeters {
                value: dot(
                    std::array::from_fn(|i| b.origin_mm[i] - a.origin_mm[i]),
                    a.normal(),
                )
                .abs(),
            }
        }
        _ => unsupported("Supporting-plane separation requires two parallel planar faces"),
    };
    Ok((angle, separation))
}
/// A single owner-thread publication for one immutable document snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasurementBatch {
    pub stamp: MeasurementStamp,
    pub transient: Option<MeasurementEvaluation>,
    pub pinned: std::collections::BTreeMap<EntityId, MeasurementEvaluation>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasurementReply {
    pub request: u64,
    pub stamp: MeasurementStamp,
    pub result: Result<MeasurementBatch, String>,
}
