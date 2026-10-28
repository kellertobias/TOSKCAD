//! Detached exact native query contracts. Canonical source stores selectors and
//! output identities; evaluated geometry is carried only by current receipts.
use super::{Behavior, source::invalid};
use crate::{
    constraint_sketch::SplineBasis,
    document::{DocumentError, EntityId},
    model::{OccurrencePath, RigidPose},
    project_file::ContentHash,
    references::{GeometryIntent, Origin},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
pub const OCCURRENCE_SCHEMA: u32 = 2;
pub const OCCURRENCE_CAPABILITY: &str = "sketch.external.2";
pub const MAX_EXTERNAL_RESULTS: usize = 256;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OccurrenceSource {
    pub component: EntityId,
    pub source: EntityId,
    pub path: OccurrencePath,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OccurrenceContext {
    pub component: EntityId,
    pub path: OccurrencePath,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryMethod {
    Coplanar,
    Project,
    Intersect,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalQuery {
    pub reference: EntityId,
    pub sketch: EntityId,
    pub source: OccurrenceSource,
    /// None means the sketch plane is in world coordinates; otherwise its frame
    /// is in the explicitly selected receiving Part definition's coordinates.
    pub sketch_context: Option<OccurrenceContext>,
    pub intent: GeometryIntent,
    pub method: QueryMethod,
}
fn path_valid(path: &OccurrencePath) -> bool {
    !path.root.is_nil()
        && path.members.len() <= 32
        && path.members.iter().all(|id| !id.is_nil())
        && path.members.iter().collect::<BTreeSet<_>>().len() == path.members.len()
}
impl ExternalQuery {
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.reference.is_nil()
            || self.sketch.is_nil()
            || self.source.component.is_nil()
            || self.source.source.is_nil()
            || !path_valid(&self.source.path)
            || self
                .sketch_context
                .as_ref()
                .is_some_and(|c| c.component.is_nil() || !path_valid(&c.path))
        {
            return Err(invalid(
                "Occurrence references need explicit non-nil source/sketch identities and bounded canonical paths",
            ));
        }
        self.intent.validate().map_err(invalid)?;
        if self.intent.attachment.is_some() {
            return Err(invalid(
                "Exact external queries use the selected finite geometry; attachment offsets and rolls are unsupported",
            ));
        }
        Ok(())
    }
    pub fn hash(&self) -> Result<ContentHash, DocumentError> {
        self.validate()?;
        serde_json::to_vec(self)
            .map(|v| ContentHash::of(&v))
            .map_err(invalid)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalGeometryFamily {
    Point,
    Line,
    Circle,
    Arc,
    BSpline,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExternalGeometry {
    Point {
        position_mm: [f64; 2],
    },
    Line {
        a_mm: [f64; 2],
        b_mm: [f64; 2],
    },
    Circle {
        center_mm: [f64; 2],
        radius_mm: f64,
    },
    Arc {
        center_mm: [f64; 2],
        a_mm: [f64; 2],
        b_mm: [f64; 2],
        clockwise: bool,
    },
    BSpline {
        poles_mm: Vec<[f64; 2]>,
        basis: SplineBasis,
    },
}
impl ExternalGeometry {
    pub fn family(&self) -> ExternalGeometryFamily {
        match self {
            Self::Point { .. } => ExternalGeometryFamily::Point,
            Self::Line { .. } => ExternalGeometryFamily::Line,
            Self::Circle { .. } => ExternalGeometryFamily::Circle,
            Self::Arc { .. } => ExternalGeometryFamily::Arc,
            Self::BSpline { .. } => ExternalGeometryFamily::BSpline,
        }
    }
    pub fn points(&self) -> Vec<[f64; 2]> {
        match self {
            Self::Point { position_mm } => vec![*position_mm],
            Self::Line { a_mm, b_mm } => vec![*a_mm, *b_mm],
            Self::Circle { center_mm, .. } => vec![*center_mm],
            Self::Arc {
                center_mm,
                a_mm,
                b_mm,
                ..
            } => vec![*center_mm, *a_mm, *b_mm],
            Self::BSpline { poles_mm, .. } => poles_mm.clone(),
        }
    }
    pub fn validate(&self) -> Result<(), DocumentError> {
        let points = self.points();
        if points
            .iter()
            .flatten()
            .any(|v| !v.is_finite() || v.abs() > 1e9)
        {
            return Err(invalid(
                "External native geometry exceeds finite modeling bounds",
            ));
        }
        match self {
            Self::Circle { radius_mm, .. }
                if !radius_mm.is_finite() || *radius_mm <= 1e-9 || *radius_mm > 1e9 =>
            {
                Err(invalid("External circle has an invalid radius"))
            }
            Self::BSpline { poles_mm, basis } => basis.validate(poles_mm.len()),
            Self::Line { a_mm, b_mm } if (a_mm[0] - b_mm[0]).hypot(a_mm[1] - b_mm[1]) <= 1e-9 => {
                Err(invalid("External line is degenerate"))
            }
            Self::Arc {
                center_mm,
                a_mm,
                b_mm,
                ..
            } => {
                let radius = (a_mm[0] - center_mm[0]).hypot(a_mm[1] - center_mm[1]);
                let other = (b_mm[0] - center_mm[0]).hypot(b_mm[1] - center_mm[1]);
                if radius <= 1e-9
                    || (radius - other).abs() > 1e-7
                    || (a_mm[0] - b_mm[0]).hypot(a_mm[1] - b_mm[1]) <= 1e-9
                {
                    Err(invalid(
                        "External arc is degenerate or has inconsistent radii",
                    ))
                } else {
                    Ok(())
                }
            }
            _ => Ok(()),
        }
    }
}
/// A finite input-edge parameter band, not an output spline's parameterization.
/// Native crossing sign distinguishes transverse branches. Tangencies have no
/// persistent crossing selector and remain explicit diagnostic choices.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalBranch {
    pub source_interval: [f64; 2],
    pub crossing: Option<i8>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResultSelector {
    /// Sorted unique actual source ancestry; never a topology ordinal or position.
    pub support: Vec<Origin>,
    pub family: ExternalGeometryFamily,
    pub branch: Option<ExternalBranch>,
    /// A changed catalogue requires explicit repair, rather than silently choosing
    /// an ordinal after a branch has disappeared or appeared.
    pub expected_count: usize,
}
impl ResultSelector {
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.expected_count == 0
            || self.expected_count > MAX_EXTERNAL_RESULTS
            || self.support.is_empty()
            || self.support.len() > 1024
            || self.support.windows(2).any(|p| p[0] >= p[1])
            || self
                .support
                .iter()
                .any(|o| o.source.is_nil() || o.role.is_empty() || o.role.len() > 1024)
        {
            return Err(invalid(
                "External selector requires bounded unique named source support and an explicit result count",
            ));
        }
        if let Some(b) = &self.branch
            && (b
                .source_interval
                .iter()
                .any(|p| !p.is_finite() || p.abs() > 1e9)
                || b.source_interval[0] >= b.source_interval[1]
                || b.crossing.is_some_and(|v| v != 1 && v != -1))
        {
            return Err(invalid(
                "External branch needs a finite increasing source interval and a transverse crossing sign",
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalChoice {
    pub selector: ResultSelector,
    pub geometry: ExternalGeometry,
    pub source_parameter: Option<f64>,
    pub source_interval: Option<[f64; 2]>,
    pub tangent: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalCatalog {
    pub document: EntityId,
    pub revision: u64,
    pub source_hash: ContentHash,
    pub query_hash: ContentHash,
    pub query: ExternalQuery,
    pub sketch_world_pose: RigidPose,
    pub results: Vec<ExternalChoice>,
    pub coincident: bool,
    pub diagnostics: Vec<String>,
}
impl ExternalCatalog {
    pub fn validate(&self) -> Result<(), DocumentError> {
        self.query.validate()?;
        self.sketch_world_pose.validate()?;
        if self.document.is_nil()
            || self.query.hash()? != self.query_hash
            || self.results.len() > MAX_EXTERNAL_RESULTS
            || self.diagnostics.len() > MAX_EXTERNAL_RESULTS
        {
            return Err(invalid(
                "External catalogue source/query stamp or result budget is invalid",
            ));
        }
        for (c, i) in self.results.iter().zip(0..) {
            c.selector.validate()?;
            c.geometry.validate()?;
            if c.selector.family != c.geometry.family()
                || c.selector.expected_count != self.results.len()
                || self.results[..i].iter().any(|p| p.selector == c.selector)
                || c.source_parameter
                    .is_some_and(|p| !p.is_finite() || p.abs() > 1e9)
                || c.source_interval.is_some_and(|p| {
                    p.iter().any(|x| !x.is_finite() || x.abs() > 1e9) || p[0] >= p[1]
                })
            {
                return Err(invalid(
                    "Native catalogue has duplicate/invalid semantic choices or source parameters",
                ));
            }
        }
        Ok(())
    }
}
/// Resolve a persisted semantic branch against a current catalogue. Recomputed
/// midpoint bands may move; only the actual current source parameter is tested
/// against the saved interval. No nearest, output index or coordinate fallback.
pub fn select_external_choice<'a>(
    catalog: &'a ExternalCatalog,
    selector: &ResultSelector,
) -> Result<&'a ExternalChoice, DocumentError> {
    catalog.validate()?;
    selector.validate()?;
    if selector.expected_count != catalog.results.len() {
        return Err(invalid(
            "External intersection count changed; explicitly repair the saved result selection",
        ));
    }
    let candidates = catalog
        .results
        .iter()
        .filter(|choice| {
            if choice.tangent
                || choice.selector.support != selector.support
                || choice.selector.family != selector.family
            {
                return false;
            }
            match &selector.branch {
                None => choice.selector.branch.is_none(),
                Some(branch) => {
                    choice.source_parameter.is_some_and(|p| {
                        p >= branch.source_interval[0] && p <= branch.source_interval[1]
                    }) && choice
                        .selector
                        .branch
                        .as_ref()
                        .is_some_and(|current| current.crossing == branch.crossing)
                }
            }
        })
        .collect::<Vec<_>>();
    if candidates.len() != 1
        || (catalog.coincident && catalog.query.method == QueryMethod::Intersect)
    {
        return Err(invalid(
            "Saved external support/branch is missing, tangent or ambiguous; explicitly repair without nearest-result rebinding",
        ));
    }
    Ok(candidates[0])
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalOutputIds {
    pub curve: Option<EntityId>,
    pub points: Vec<EntityId>,
    pub locks: Vec<EntityId>,
}
impl ExternalOutputIds {
    pub fn allocate(geometry: &ExternalGeometry) -> Self {
        let points = geometry.points();
        Self {
            curve: (!matches!(geometry, ExternalGeometry::Point { .. })).then(EntityId::new),
            points: points.iter().map(|_| EntityId::new()).collect(),
            locks: (0..if matches!(geometry, ExternalGeometry::Circle { .. }) {
                2
            } else {
                points.len()
            })
                .map(|_| EntityId::new())
                .collect(),
        }
    }
    pub fn all(&self) -> Vec<EntityId> {
        self.points
            .iter()
            .chain(&self.locks)
            .copied()
            .chain(self.curve)
            .collect()
    }
    pub fn validate_for(&self, geometry: &ExternalGeometry) -> Result<(), DocumentError> {
        geometry.validate()?;
        let all = self.all();
        if self.points.len() != geometry.points().len()
            || self.locks.len()
                != if matches!(geometry, ExternalGeometry::Circle { .. }) {
                    2
                } else {
                    self.points.len()
                }
            || self.curve.is_some() == matches!(geometry, ExternalGeometry::Point { .. })
            || all.iter().any(|id| id.is_nil())
            || all.iter().collect::<BTreeSet<_>>().len() != all.len()
        {
            return Err(invalid(
                "External output identities do not match the exact current geometry family and pole count; explicitly repair",
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OccurrenceCaptureRequest {
    pub query: ExternalQuery,
    pub selector: ResultSelector,
    pub behavior: Behavior,
    pub output: ExternalOutputIds,
}
impl OccurrenceCaptureRequest {
    pub fn new(
        catalog: &ExternalCatalog,
        selector: &ResultSelector,
        behavior: Behavior,
    ) -> Result<Self, DocumentError> {
        catalog.validate()?;
        let choice = catalog
            .results
            .iter()
            .find(|c| &c.selector == selector)
            .ok_or_else(|| invalid("Explicitly select one current semantic native result"))?;
        if choice.tangent || (catalog.coincident && catalog.query.method == QueryMethod::Intersect)
        {
            return Err(invalid(
                "Tangent/coincident native results require an unambiguous alternative selection",
            ));
        }
        Ok(Self {
            query: catalog.query.clone(),
            selector: selector.clone(),
            behavior,
            output: ExternalOutputIds::allocate(&choice.geometry),
        })
    }
}
/// A native result bound to the exact candidate document and canonical binding.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalResultReceipt {
    pub document: EntityId,
    pub revision: u64,
    pub source_hash: ContentHash,
    pub query_hash: ContentHash,
    pub dependency_hash: ContentHash,
    pub plane_hash: ContentHash,
    pub pose_hash: ContentHash,
    pub selector: ResultSelector,
    pub geometry: ExternalGeometry,
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detached_geometry_and_output_identity_validation_preserve_exact_weighted_basis() {
        let mut basis = SplineBasis::open_uniform(2, 3).unwrap();
        basis.weights = Some(vec![1., 0.5, 1.]);
        let geometry = ExternalGeometry::BSpline {
            poles_mm: vec![[0., 0.], [10., 10.], [20., 0.]],
            basis,
        };
        geometry.validate().unwrap();
        let output = ExternalOutputIds::allocate(&geometry);
        output.validate_for(&geometry).unwrap();
        let roundtrip: ExternalGeometry =
            serde_json::from_slice(&serde_json::to_vec(&geometry).unwrap()).unwrap();
        assert_eq!(geometry, roundtrip);
        let mut invalid = output.clone();
        invalid.points[1] = invalid.points[0];
        assert!(invalid.validate_for(&geometry).is_err());
        assert_eq!(output.points.len(), 3);
        assert_eq!(output.locks.len(), 3);
        assert!(output.curve.is_some());
    }
    #[test]
    fn semantic_selectors_reject_ordinal_only_ambiguous_or_unbounded_branches() {
        let origin = Origin {
            source: EntityId::new(),
            role: "source.edge".into(),
        };
        let selector = ResultSelector {
            support: vec![origin.clone()],
            family: ExternalGeometryFamily::Point,
            branch: Some(ExternalBranch {
                source_interval: [0., 1.],
                crossing: Some(1),
            }),
            expected_count: 2,
        };
        selector.validate().unwrap();
        let mut bad = selector.clone();
        bad.support.clear();
        assert!(bad.validate().is_err());
        let mut bad = selector.clone();
        bad.support.push(origin);
        assert!(bad.validate().is_err());
        let mut bad = selector.clone();
        bad.branch.as_mut().unwrap().source_interval = [1., 0.];
        assert!(bad.validate().is_err());
        let mut bad = selector;
        bad.branch.as_mut().unwrap().crossing = Some(0);
        assert!(bad.validate().is_err());
    }
}
