//! Bounded, source-neutral arrays of independent authored geometry. Originals
//! remain unchanged; every new point, curve and constraint has a fresh identity.
use crate::{
    constraint_sketch::{Constraint, ConstraintKind, Curve, CurveKind, source::Source},
    document::EntityId,
    sketch_transform::{self, Transform, TransformErrorKind, TransformOptions},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
pub const MAX_PATTERN_INSTANCES: usize = 256;
pub const PLACEMENT_TOLERANCE_MM: f64 = 1e-7;
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Pattern {
    /// Count includes the original; instance i uses i * step_mm.
    Linear { step_mm: [f64; 2], count: usize },
    /// Count includes the original; angles are increments, not a total span.
    Circular {
        center_mm: [f64; 2],
        step_angle_rad: f64,
        count: usize,
    },
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PatternOptions {
    pub expand_connected: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatternRequest {
    pub selection: BTreeSet<EntityId>,
    pub pattern: Pattern,
    #[serde(default)]
    pub options: PatternOptions,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatternInstance {
    /// Zero is the unchanged original and is not present in this list.
    pub index: usize,
    pub points: BTreeMap<EntityId, EntityId>,
    pub curves: BTreeMap<EntityId, EntityId>,
    pub constraints: BTreeMap<EntityId, EntityId>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PatternReport {
    pub requested: BTreeSet<EntityId>,
    pub expanded_selection: BTreeSet<EntityId>,
    pub points: BTreeSet<EntityId>,
    pub curves: BTreeSet<EntityId>,
    pub constraints: BTreeSet<EntityId>,
    /// Copied locks apply at the new transformed authored positions.
    pub fixed_constraints: BTreeSet<EntityId>,
    /// The solver refreshes these read-only measurements; source RHS is kept.
    pub reference_dimensions: BTreeSet<EntityId>,
    pub instances: Vec<PatternInstance>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct PatternDraft {
    pub source: Source,
    pub report: PatternReport,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PatternErrorKind {
    InvalidSource,
    InvalidPattern,
    EmptySelection,
    UnknownSelection,
    NonGeometrySelection,
    SharedGeometry,
    ExternalConstraint,
    BudgetExceeded,
    DuplicatePlacement,
    Transform(TransformErrorKind),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatternError {
    pub kind: PatternErrorKind,
    pub message: String,
    pub involved: BTreeSet<EntityId>,
    pub required_selection: BTreeSet<EntityId>,
    pub instance: Option<usize>,
}
impl std::fmt::Display for PatternError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(f)
    }
}
impl std::error::Error for PatternError {}
fn error(
    kind: PatternErrorKind,
    message: impl Into<String>,
    ids: impl IntoIterator<Item = EntityId>,
) -> PatternError {
    PatternError {
        kind,
        message: message.into(),
        involved: ids.into_iter().collect(),
        required_selection: BTreeSet::new(),
        instance: None,
    }
}
fn points(curve: &Curve) -> Vec<EntityId> {
    match curve.kind {
        CurveKind::BSpline { ref poles, .. } => poles.clone(),
        CurveKind::Line { start, end } => vec![start, end],
        CurveKind::Circle { center, .. } => vec![center],
        CurveKind::Arc {
            center, start, end, ..
        } => vec![center, start, end],
    }
}
impl Pattern {
    fn count(self) -> usize {
        match self {
            Self::Linear { count, .. } | Self::Circular { count, .. } => count,
        }
    }
    fn transform(self, index: usize) -> Result<Transform, PatternError> {
        let bad = || {
            error(
                PatternErrorKind::InvalidPattern,
                "Pattern needs finite bounded placements, a nonzero step and 2–256 total instances",
                [],
            )
        };
        if !(2..=MAX_PATTERN_INSTANCES).contains(&self.count()) {
            return Err(bad());
        }
        match self {
            Self::Linear { step_mm, .. } => {
                let delta_mm = step_mm.map(|v| v * index as f64);
                if step_mm.iter().any(|v| !v.is_finite())
                    || step_mm[0].hypot(step_mm[1]) <= PLACEMENT_TOLERANCE_MM
                    || delta_mm.iter().any(|v| !v.is_finite() || v.abs() > 1e9)
                {
                    return Err(bad());
                }
                Ok(Transform::Translate { delta_mm })
            }
            Self::Circular {
                center_mm,
                step_angle_rad,
                ..
            } => {
                let angle_rad = step_angle_rad * index as f64;
                if center_mm.iter().any(|v| !v.is_finite() || v.abs() > 1e9)
                    || !step_angle_rad.is_finite()
                    || !angle_rad.is_finite()
                    || angle_rad.abs() > 1e6
                {
                    return Err(bad());
                }
                if angle_rad.rem_euclid(std::f64::consts::TAU) == 0. {
                    return Err(error(
                        PatternErrorKind::DuplicatePlacement,
                        "Circular pattern repeats the original placement after a full turn",
                        [],
                    ));
                }
                Ok(Transform::Rotate {
                    center_mm,
                    angle_rad,
                })
            }
        }
    }
}
fn selection(
    source: &Source,
    requested: &BTreeSet<EntityId>,
    expand: bool,
) -> Result<PatternReport, PatternError> {
    if requested.is_empty() {
        return Err(error(
            PatternErrorKind::EmptySelection,
            "Select complete geometry to repeat",
            [],
        ));
    }
    let point_ids: BTreeSet<_> = source.sketch.points.iter().map(|p| p.id).collect();
    let curve_ids: BTreeSet<_> = source.sketch.curves.iter().map(|c| c.id).collect();
    let constraint_ids: BTreeSet<_> = source.sketch.constraints.iter().map(|c| c.id).collect();
    let non_geometry: BTreeSet<_> = requested.intersection(&constraint_ids).copied().collect();
    if !non_geometry.is_empty() {
        return Err(error(
            PatternErrorKind::NonGeometrySelection,
            "Select geometry rather than dimensional or constraint IDs",
            non_geometry,
        ));
    }
    let all: BTreeSet<_> = point_ids.union(&curve_ids).copied().collect();
    let unknown: BTreeSet<_> = requested.difference(&all).copied().collect();
    if !unknown.is_empty() {
        return Err(error(
            PatternErrorKind::UnknownSelection,
            "Pattern selection contains IDs absent from this sketch",
            unknown,
        ));
    }
    let explicit_points: BTreeSet<_> = requested.intersection(&point_ids).copied().collect();
    let mut report = PatternReport {
        requested: requested.clone(),
        points: explicit_points.clone(),
        ..Default::default()
    };
    for curve in &source.sketch.curves {
        if requested.contains(&curve.id)
            || points(curve).iter().all(|p| explicit_points.contains(p))
        {
            report.curves.insert(curve.id);
            report.points.extend(points(curve));
        }
    }
    let initial: BTreeSet<_> = report.points.union(&report.curves).copied().collect();
    loop {
        let blocked: Vec<_> = source
            .sketch
            .curves
            .iter()
            .filter(|c| {
                !report.curves.contains(&c.id)
                    && points(c).iter().any(|p| report.points.contains(p))
            })
            .collect();
        if blocked.is_empty() {
            break;
        }
        if !expand {
            let required: BTreeSet<_> = blocked
                .iter()
                .flat_map(|c| std::iter::once(c.id).chain(points(c)))
                .collect();
            let mut failure = error(
                PatternErrorKind::SharedGeometry,
                "Selected points also belong to unselected curves; explicitly include connected geometry",
                report
                    .points
                    .iter()
                    .copied()
                    .chain(required.iter().copied()),
            );
            failure.required_selection = required;
            return Err(failure);
        }
        for curve in blocked {
            report.curves.insert(curve.id);
            report.points.extend(points(curve));
        }
    }
    report.expanded_selection = report
        .points
        .union(&report.curves)
        .copied()
        .filter(|id| !initial.contains(id))
        .collect();
    let geometry: BTreeSet<_> = report.points.union(&report.curves).copied().collect();
    for constraint in &source.sketch.constraints {
        let refs: BTreeSet<_> = constraint.kind.references().into_iter().collect();
        if refs.is_disjoint(&geometry) {
            continue;
        }
        if !refs.is_subset(&geometry) {
            let mut failure = error(
                PatternErrorKind::ExternalConstraint,
                if constraint.reference {
                    "A reference dimension crosses the pattern selection; include its geometry before copying it"
                } else {
                    "A driving constraint crosses the pattern selection; include its geometry or revise the constraint"
                },
                std::iter::once(constraint.id).chain(refs.iter().copied()),
            );
            failure.required_selection = refs.difference(&geometry).copied().collect();
            return Err(failure);
        }
        report.constraints.insert(constraint.id);
        if constraint.reference {
            report.reference_dimensions.insert(constraint.id);
        }
        if matches!(constraint.kind, ConstraintKind::Fix { .. }) {
            report.fixed_constraints.insert(constraint.id);
        }
    }
    Ok(report)
}
fn remap(kind: &ConstraintKind, ids: &BTreeMap<EntityId, EntityId>) -> ConstraintKind {
    let mut kind = kind.clone();
    let map = |id: &mut EntityId| {
        *id = ids[id];
    };
    match &mut kind {
        ConstraintKind::Fix { point } => map(point),
        ConstraintKind::Horizontal { line }
        | ConstraintKind::Vertical { line }
        | ConstraintKind::Length { line, .. } => map(line),
        ConstraintKind::Radius { curve, .. } | ConstraintKind::Diameter { curve, .. } => map(curve),
        ConstraintKind::Coincident { a, b }
        | ConstraintKind::Parallel { a, b }
        | ConstraintKind::Perpendicular { a, b }
        | ConstraintKind::Tangent { a, b }
        | ConstraintKind::Equal { a, b }
        | ConstraintKind::Distance { a, b, .. } => {
            map(a);
            map(b);
        }
        ConstraintKind::Symmetric { a, b, about } => {
            map(a);
            map(b);
            map(about);
        }
        ConstraintKind::Angle { from, to, .. } => {
            map(from);
            map(to);
        }
    }
    kind
}
fn new_id(used: &mut BTreeSet<EntityId>) -> EntityId {
    loop {
        let id = EntityId::new();
        if used.insert(id) {
            return id;
        }
    }
}
fn coincides(a: &Source, b: &Source) -> bool {
    a.sketch.points.iter().zip(&b.sketch.points).all(|(a, b)| {
        (a.position_mm[0] - b.position_mm[0]).hypot(a.position_mm[1] - b.position_mm[1])
            <= PLACEMENT_TOLERANCE_MM
    })
}
/// Copy all internal constraints, including expression bindings. Cross-selection
/// constraints are rejected, not omitted. Rigid orientation restrictions use the
/// same rules as direct transforms; fixed copies lock their new positions.
/// Numerical solve and a canonical document transaction are subsequent steps.
pub fn draft(
    source: &Source,
    requested: &BTreeSet<EntityId>,
    pattern: Pattern,
    options: PatternOptions,
) -> Result<PatternDraft, PatternError> {
    source
        .validate()
        .map_err(|e| error(PatternErrorKind::InvalidSource, e.to_string(), []))?;
    pattern.transform(1)?;
    pattern.transform(pattern.count() - 1).map_err(|mut e| {
        e.instance = Some(pattern.count() - 1);
        e
    })?;
    let mut report = selection(source, requested, options.expand_connected)?;
    let copies = pattern.count() - 1;
    for (existing, selected, limit) in [
        (source.sketch.points.len(), report.points.len(), 256),
        (source.sketch.curves.len(), report.curves.len(), 512),
        (
            source.sketch.constraints.len(),
            report.constraints.len(),
            1024,
        ),
    ] {
        if selected
            .checked_mul(copies)
            .and_then(|n| n.checked_add(existing))
            .is_none_or(|n| n > limit)
        {
            return Err(error(
                PatternErrorKind::BudgetExceeded,
                "Pattern exceeds the sketch point, curve or constraint budget",
                requested.iter().copied(),
            ));
        }
    }
    let template = Source {
        u: source.u,
        sketch: crate::constraint_sketch::Sketch {
            points: source
                .sketch
                .points
                .iter()
                .filter(|p| report.points.contains(&p.id))
                .cloned()
                .collect(),
            curves: source
                .sketch
                .curves
                .iter()
                .filter(|c| report.curves.contains(&c.id))
                .cloned()
                .collect(),
            constraints: source
                .sketch
                .constraints
                .iter()
                .filter(|c| {
                    report.constraints.contains(&c.id)
                        && !matches!(c.kind, ConstraintKind::Fix { .. })
                })
                .cloned()
                .collect(),
            texts: vec![],
        },
    };
    let selected: BTreeSet<_> = report.points.union(&report.curves).copied().collect();
    let mut placements = vec![template.clone()];
    for index in 1..pattern.count() {
        let mut transformed = sketch_transform::draft(
            &template,
            &selected,
            pattern.transform(index).map_err(|mut e| {
                e.instance = Some(index);
                e
            })?,
            TransformOptions::default(),
        )
        .map_err(|e| PatternError {
            kind: PatternErrorKind::Transform(e.kind),
            message: format!("Pattern instance {}: {}", index + 1, e.message),
            involved: e.involved,
            required_selection: e.required_selection,
            instance: Some(index),
        })?
        .source;
        if placements.iter().any(|p| coincides(p, &transformed)) {
            let mut failure = error(
                PatternErrorKind::DuplicatePlacement,
                "Pattern placement repeats or moves less than the 1e-7 mm tolerance",
                selected.iter().copied(),
            );
            failure.instance = Some(index);
            return Err(failure);
        }
        // Keep the complete original constraint set for cloning. Fix was only
        // excluded from the transformation check because originals never move.
        transformed.sketch.constraints = source
            .sketch
            .constraints
            .iter()
            .filter(|c| report.constraints.contains(&c.id))
            .cloned()
            .collect();
        placements.push(transformed);
    }
    let mut candidate = source.clone();
    let mut used: BTreeSet<_> = source
        .sketch
        .points
        .iter()
        .map(|p| p.id)
        .chain(source.sketch.curves.iter().map(|c| c.id))
        .chain(source.sketch.constraints.iter().map(|c| c.id))
        .chain(
            source
                .sketch
                .constraints
                .iter()
                .filter_map(|c| c.kind.dimension()?.expression())
                .flat_map(|e| e.bindings.iter().map(|b| b.target)),
        )
        .collect();
    for (index, placement) in placements.into_iter().enumerate().skip(1) {
        let mut instance = PatternInstance {
            index,
            points: BTreeMap::new(),
            curves: BTreeMap::new(),
            constraints: BTreeMap::new(),
        };
        for point in &placement.sketch.points {
            instance.points.insert(point.id, new_id(&mut used));
        }
        for curve in &placement.sketch.curves {
            instance.curves.insert(curve.id, new_id(&mut used));
        }
        for constraint in &placement.sketch.constraints {
            instance
                .constraints
                .insert(constraint.id, new_id(&mut used));
        }
        for mut point in placement.sketch.points {
            point.id = instance.points[&point.id];
            candidate.sketch.points.push(point);
        }
        for curve in placement.sketch.curves {
            let kind = match curve.kind {
                CurveKind::BSpline { poles, basis } => CurveKind::BSpline {
                    poles: poles.into_iter().map(|p| instance.points[&p]).collect(),
                    basis,
                },
                CurveKind::Line { start, end } => CurveKind::Line {
                    start: instance.points[&start],
                    end: instance.points[&end],
                },
                CurveKind::Circle { center, radius_mm } => CurveKind::Circle {
                    center: instance.points[&center],
                    radius_mm,
                },
                CurveKind::Arc {
                    center,
                    start,
                    end,
                    clockwise,
                } => CurveKind::Arc {
                    center: instance.points[&center],
                    start: instance.points[&start],
                    end: instance.points[&end],
                    clockwise,
                },
            };
            candidate.sketch.curves.push(Curve {
                id: instance.curves[&curve.id],
                construction: curve.construction,
                kind,
            });
        }
        let ids: BTreeMap<_, _> = instance
            .points
            .iter()
            .chain(&instance.curves)
            .map(|(a, b)| (*a, *b))
            .collect();
        for constraint in placement.sketch.constraints {
            candidate.sketch.constraints.push(Constraint {
                id: instance.constraints[&constraint.id],
                reference: constraint.reference,
                kind: remap(&constraint.kind, &ids),
            });
        }
        report.instances.push(instance);
    }
    candidate
        .validate()
        .map_err(|e| error(PatternErrorKind::InvalidSource, e.to_string(), []))?;
    Ok(PatternDraft {
        source: candidate,
        report,
    })
}
