//! Source-neutral rigid drafts. The solver validates a draft before a document
//! transaction publishes it. Constraints and stable IDs are retained.
use crate::{
    constraint_sketch::{Axis, Constraint, ConstraintKind, Curve, CurveKind, source::Source},
    document::EntityId,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Transform {
    Translate {
        delta_mm: [f64; 2],
    },
    Rotate {
        center_mm: [f64; 2],
        angle_rad: f64,
    },
    Mirror {
        point_mm: [f64; 2],
        direction: [f64; 2],
    },
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TransformOptions {
    /// Explicit caller consent to include curves connected through moved points.
    pub expand_connected: bool,
    /// Caller-owned read-only metadata. Source has no external-curve or
    /// parameter-binding fields; these IDs do not create those source semantics.
    pub protected_geometry: BTreeSet<EntityId>,
    pub protected_constraints: BTreeSet<EntityId>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransformRequest {
    pub selection: BTreeSet<EntityId>,
    pub transform: Transform,
    #[serde(default)]
    pub options: TransformOptions,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TransformReport {
    pub requested: BTreeSet<EntityId>,
    pub expanded_selection: BTreeSet<EntityId>,
    pub points: BTreeSet<EntityId>,
    pub curves: BTreeSet<EntityId>,
    pub constraints: BTreeSet<EntityId>,
    /// The solver refreshes measurements. Stored reference dimensions remain
    /// reference-only and their literal values/units are not rewritten.
    pub reference_dimensions: BTreeSet<EntityId>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct TransformDraft {
    pub source: Source,
    pub report: TransformReport,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransformErrorKind {
    InvalidSource,
    InvalidTransform,
    EmptySelection,
    UnknownSelection,
    NonGeometrySelection,
    SharedGeometry,
    ProtectedEntity,
    LockedGeometry,
    ExternalConstraint,
    OrientationConstraint,
    InvalidGeometry,
}
#[derive(Clone, Debug, PartialEq)]
pub struct TransformError {
    pub kind: TransformErrorKind,
    pub message: String,
    pub involved: BTreeSet<EntityId>,
    pub required_selection: BTreeSet<EntityId>,
}
impl std::fmt::Display for TransformError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(f)
    }
}
impl std::error::Error for TransformError {}
fn error(
    kind: TransformErrorKind,
    message: impl Into<String>,
    ids: impl IntoIterator<Item = EntityId>,
) -> TransformError {
    TransformError {
        kind,
        message: message.into(),
        involved: ids.into_iter().collect(),
        required_selection: BTreeSet::new(),
    }
}
fn curve_points(curve: &Curve) -> Vec<EntityId> {
    match curve.kind {
        CurveKind::BSpline { ref poles, .. } => poles.clone(),
        CurveKind::Line { start, end } => vec![start, end],
        CurveKind::Circle { center, .. } => vec![center],
        CurveKind::Arc {
            center, start, end, ..
        } => vec![center, start, end],
    }
}
#[derive(Clone, Copy)]
struct Mapping {
    matrix: [[f64; 2]; 2],
    center: [f64; 2],
    shift: [f64; 2],
    mirror: bool,
}
impl Mapping {
    fn new(transform: Transform) -> Result<Self, TransformError> {
        let invalid = || {
            error(
                TransformErrorKind::InvalidTransform,
                "Transform needs finite bounded coordinates and a valid angle or mirror axis",
                [],
            )
        };
        let (matrix, center, shift, mirror) = match transform {
            Transform::Translate { delta_mm } => ([[1., 0.], [0., 1.]], [0.; 2], delta_mm, false),
            Transform::Rotate {
                center_mm,
                angle_rad,
            } => {
                if !angle_rad.is_finite() || angle_rad.abs() > 1e6 {
                    return Err(invalid());
                }
                let angle = angle_rad.rem_euclid(std::f64::consts::TAU);
                // Exact cardinal inputs avoid trig noise without rounding tiny
                // arbitrary rotations to identity at large model coordinates.
                let (s, c) = if angle == 0. {
                    (0., 1.)
                } else if angle == std::f64::consts::FRAC_PI_2 {
                    (1., 0.)
                } else if angle == std::f64::consts::PI {
                    (0., -1.)
                } else if angle == 3. * std::f64::consts::FRAC_PI_2 {
                    (-1., 0.)
                } else {
                    angle.sin_cos()
                };
                ([[c, -s], [s, c]], center_mm, [0.; 2], false)
            }
            Transform::Mirror {
                point_mm,
                direction,
            } => {
                let length = direction[0].hypot(direction[1]);
                if direction.iter().any(|v| !v.is_finite()) || !length.is_finite() || length < 1e-12
                {
                    return Err(invalid());
                }
                let [x, y] = direction.map(|v| v / length);
                (
                    [[2. * x * x - 1., 2. * x * y], [2. * x * y, 2. * y * y - 1.]],
                    point_mm,
                    [0.; 2],
                    true,
                )
            }
        };
        if center
            .iter()
            .chain(&shift)
            .any(|v| !v.is_finite() || v.abs() > 1e9)
        {
            return Err(invalid());
        }
        Ok(Self {
            matrix,
            center,
            shift,
            mirror,
        })
    }
    fn point(self, point: [f64; 2]) -> [f64; 2] {
        let d = [point[0] - self.center[0], point[1] - self.center[1]];
        std::array::from_fn(|i| {
            self.center[i] + self.matrix[i][0] * d[0] + self.matrix[i][1] * d[1] + self.shift[i]
        })
    }
}
fn changed_points(
    points: &BTreeSet<EntityId>,
    positions: &BTreeMap<EntityId, [f64; 2]>,
    mapping: Mapping,
) -> BTreeSet<EntityId> {
    points
        .iter()
        .copied()
        .filter(|id| mapping.point(positions[id]) != positions[id])
        .collect()
}
struct SelectionClosure {
    points: BTreeSet<EntityId>,
    curves: BTreeSet<EntityId>,
    expanded: BTreeSet<EntityId>,
}
fn closure(
    source: &Source,
    selection: &BTreeSet<EntityId>,
    positions: &BTreeMap<EntityId, [f64; 2]>,
    mapping: Mapping,
    expand: bool,
) -> Result<SelectionClosure, TransformError> {
    let explicit_points: BTreeSet<_> = selection
        .iter()
        .copied()
        .filter(|id| positions.contains_key(id))
        .collect();
    let mut curves: BTreeSet<_> = source
        .sketch
        .curves
        .iter()
        .filter(|c| {
            selection.contains(&c.id)
                || curve_points(c)
                    .iter()
                    .all(|id| explicit_points.contains(id))
        })
        .map(|c| c.id)
        .collect();
    let mut points = explicit_points;
    for curve in &source.sketch.curves {
        if curves.contains(&curve.id) {
            points.extend(curve_points(curve));
        }
    }
    let mut expanded = BTreeSet::new();
    loop {
        let changed = changed_points(&points, positions, mapping);
        let blocked: Vec<_> = source
            .sketch
            .curves
            .iter()
            .filter(|c| {
                !curves.contains(&c.id) && curve_points(c).iter().any(|id| changed.contains(id))
            })
            .collect();
        if blocked.is_empty() {
            return Ok(SelectionClosure {
                points,
                curves,
                expanded,
            });
        }
        if !expand {
            let required: BTreeSet<_> = blocked
                .iter()
                .flat_map(|c| std::iter::once(c.id).chain(curve_points(c)))
                .collect();
            let mut failure = error(
                TransformErrorKind::SharedGeometry,
                "Moved points also belong to unselected curves; explicitly include connected geometry",
                changed.into_iter().chain(required.iter().copied()),
            );
            failure.required_selection = required;
            return Err(failure);
        }
        for curve in blocked {
            curves.insert(curve.id);
            expanded.insert(curve.id);
            for point in curve_points(curve) {
                if points.insert(point) {
                    expanded.insert(point);
                }
            }
        }
    }
}
fn line(source: &Source, id: EntityId) -> Option<[[f64; 2]; 2]> {
    let curve = source.sketch.curves.iter().find(|c| c.id == id)?;
    let CurveKind::Line { start, end } = curve.kind else {
        return None;
    };
    Some([start, end].map(|id| {
        source
            .sketch
            .points
            .iter()
            .find(|p| p.id == id)
            .expect("validated point")
            .position_mm
    }))
}
fn direction(source: &Source, id: EntityId) -> Option<[f64; 2]> {
    let [a, b] = line(source, id)?;
    Some([b[0] - a[0], b[1] - a[1]])
}
fn near(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-8
}
fn check_orientation(
    before: &Source,
    after: &Source,
    constraint: &Constraint,
) -> Result<(), TransformError> {
    let fail = || {
        error(
            TransformErrorKind::OrientationConstraint,
            "Transform would change an orientation or signed dimension; its constraint and value were preserved",
            std::iter::once(constraint.id).chain(constraint.kind.references()),
        )
    };
    match &constraint.kind {
        ConstraintKind::Horizontal { line } => {
            if !direction(after, *line).is_some_and(|d| near(d[1], 0.)) {
                return Err(fail());
            }
        }
        ConstraintKind::Vertical { line } => {
            if !direction(after, *line).is_some_and(|d| near(d[0], 0.)) {
                return Err(fail());
            }
        }
        ConstraintKind::Distance {
            a,
            b,
            axis: axis @ (Axis::X | Axis::Y),
            ..
        } => {
            let point = |source: &Source, id| {
                source
                    .sketch
                    .points
                    .iter()
                    .find(|p| p.id == id)
                    .map(|p| p.position_mm)
            };
            if let (Some(p), Some(q), Some(r), Some(s)) = (
                point(before, *a),
                point(before, *b),
                point(after, *a),
                point(after, *b),
            ) {
                let index = usize::from(*axis == Axis::Y);
                if !near(q[index] - p[index], s[index] - r[index]) {
                    return Err(fail());
                }
            }
        }
        ConstraintKind::Angle { from, to, .. } => {
            let angle = |source: &Source| {
                let a = direction(source, *from)?;
                let b = direction(source, *to)?;
                Some((a[0] * b[1] - a[1] * b[0]).atan2(a[0] * b[0] + a[1] * b[1]))
            };
            let (Some(a), Some(b)) = (angle(before), angle(after)) else {
                return Err(fail());
            };
            let difference = (b - a + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU)
                - std::f64::consts::PI;
            if !near(difference, 0.) {
                return Err(fail());
            }
        }
        _ => {}
    }
    Ok(())
}
fn validate_geometry(source: &Source) -> Result<(), TransformError> {
    let point = |id| {
        source
            .sketch
            .points
            .iter()
            .find(|p| p.id == id)
            .expect("validated point")
            .position_mm
    };
    for curve in &source.sketch.curves {
        let invalid = match curve.kind {
            CurveKind::BSpline { ref poles, .. } => !poles.windows(2).any(|p| {
                let a = point(p[0]);
                let b = point(p[1]);
                (a[0] - b[0]).hypot(a[1] - b[1]) >= 1e-7
            }),
            CurveKind::Line { start, end } => {
                let a = point(start);
                let b = point(end);
                (a[0] - b[0]).hypot(a[1] - b[1]) < 1e-7
            }
            CurveKind::Arc {
                center, start, end, ..
            } => {
                let c = point(center);
                let a = point(start);
                let b = point(end);
                let r = (a[0] - c[0]).hypot(a[1] - c[1]);
                let s = (b[0] - c[0]).hypot(b[1] - c[1]);
                r < 1e-7 || !near(r, s) || (a[0] - b[0]).hypot(a[1] - b[1]) < 1e-7
            }
            CurveKind::Circle { .. } => false,
        };
        if invalid {
            return Err(error(
                TransformErrorKind::InvalidGeometry,
                "Transform requires nondegenerate lines and consistent circular arcs",
                std::iter::once(curve.id).chain(curve_points(curve)),
            ));
        }
    }
    Ok(())
}
/// Select real point/curve IDs. Point-only selection includes every point of an
/// affected curve, or explicitly opts into connected expansion. Numerical
/// consistency and reference measurements still require solver validation.
pub fn draft(
    source: &Source,
    selection: &BTreeSet<EntityId>,
    transform: Transform,
    options: TransformOptions,
) -> Result<TransformDraft, TransformError> {
    source
        .validate()
        .map_err(|e| error(TransformErrorKind::InvalidSource, e.to_string(), []))?;
    validate_geometry(source)?;
    let mapping = Mapping::new(transform)?;
    if selection.is_empty() {
        return Err(error(
            TransformErrorKind::EmptySelection,
            "Select points or curves to transform",
            [],
        ));
    }
    let positions: BTreeMap<_, _> = source
        .sketch
        .points
        .iter()
        .map(|p| (p.id, p.position_mm))
        .collect();
    let geometry: BTreeSet<_> = positions
        .keys()
        .copied()
        .chain(source.sketch.curves.iter().map(|c| c.id))
        .collect();
    let constraints: BTreeSet<_> = source.sketch.constraints.iter().map(|c| c.id).collect();
    let non_geometry: BTreeSet<_> = selection.intersection(&constraints).copied().collect();
    if !non_geometry.is_empty() {
        return Err(error(
            TransformErrorKind::NonGeometrySelection,
            "Dimensions and constraints are not transformable geometry; select their points or curves",
            non_geometry,
        ));
    }
    let unknown: BTreeSet<_> = selection.difference(&geometry).copied().collect();
    if !unknown.is_empty() {
        return Err(error(
            TransformErrorKind::UnknownSelection,
            "Selection contains geometry absent from this sketch",
            unknown,
        ));
    }
    let invalid_protection: BTreeSet<_> = options
        .protected_geometry
        .difference(&geometry)
        .chain(options.protected_constraints.difference(&constraints))
        .copied()
        .collect();
    if !invalid_protection.is_empty() {
        return Err(error(
            TransformErrorKind::UnknownSelection,
            "Protection metadata contains IDs absent from this sketch",
            invalid_protection,
        ));
    }
    let selection_closure = closure(
        source,
        selection,
        &positions,
        mapping,
        options.expand_connected,
    )?;
    let mut candidate = source.clone();
    let mut report = TransformReport {
        requested: selection.clone(),
        expanded_selection: selection_closure.expanded,
        points: changed_points(&selection_closure.points, &positions, mapping),
        ..Default::default()
    };
    for point in &mut candidate.sketch.points {
        if selection_closure.points.contains(&point.id) {
            point.position_mm = mapping.point(point.position_mm);
        }
    }
    for curve in &mut candidate.sketch.curves {
        let moved = curve_points(curve)
            .iter()
            .any(|id| report.points.contains(id));
        let mirrored_arc = mapping.mirror
            && selection_closure.curves.contains(&curve.id)
            && matches!(curve.kind, CurveKind::Arc { .. });
        if mirrored_arc && let CurveKind::Arc { clockwise, .. } = &mut curve.kind {
            *clockwise = !*clockwise;
        }
        if moved || mirrored_arc {
            report.curves.insert(curve.id);
        }
    }
    candidate.validate().map_err(|e| {
        error(
            TransformErrorKind::InvalidGeometry,
            e.to_string(),
            report.points.iter().copied(),
        )
    })?;
    validate_geometry(&candidate)?;
    let changed: BTreeSet<_> = report.points.union(&report.curves).copied().collect();
    let protected: BTreeSet<_> = changed
        .intersection(&options.protected_geometry)
        .copied()
        .collect();
    if !protected.is_empty() {
        return Err(error(
            TransformErrorKind::ProtectedEntity,
            "Transform would move protected or external geometry",
            protected,
        ));
    }
    let selected: BTreeSet<_> = selection_closure
        .points
        .union(&selection_closure.curves)
        .copied()
        .collect();
    for constraint in &source.sketch.constraints {
        let refs: BTreeSet<_> = constraint.kind.references().into_iter().collect();
        if refs.is_disjoint(&changed) {
            continue;
        }
        report.constraints.insert(constraint.id);
        if options.protected_constraints.contains(&constraint.id) {
            return Err(error(
                TransformErrorKind::ProtectedEntity,
                "Transform would change a protected constraint; no draft was produced",
                std::iter::once(constraint.id).chain(refs),
            ));
        }
        if matches!(constraint.kind, ConstraintKind::Fix { .. }) {
            return Err(error(
                TransformErrorKind::LockedGeometry,
                "Transform would move fixed geometry; its lock was preserved",
                std::iter::once(constraint.id).chain(refs),
            ));
        }
        if constraint.reference {
            report.reference_dimensions.insert(constraint.id);
            continue;
        }
        if !refs.is_subset(&selected) {
            return Err(error(
                TransformErrorKind::ExternalConstraint,
                "A driving constraint crosses the transform selection; explicitly include its geometry or revise the constraint",
                std::iter::once(constraint.id).chain(refs),
            ));
        }
        check_orientation(source, &candidate, constraint)?;
    }
    Ok(TransformDraft {
        source: candidate,
        report,
    })
}
