//! Source-neutral analytic trim/extend drafts using line/arc/circle
//! boundaries. Numerical currentness is established by the solver wrapper;
//! structural checks here never constitute proof of a successful solve.
use crate::{
    constraint_sketch::{Curve, CurveKind, Point, Sketch, source::Source},
    document::EntityId,
    project_file::ContentHash,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    f64::consts::TAU,
};
type Position = [f64; 2];
mod circular;
mod exact;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GeometryStamp {
    pub document: EntityId,
    pub sketch: EntityId,
    pub revision: u64,
    pub document_hash: ContentHash,
}
/// Derived input from an adapter which has verified the current document,
/// revision, plane/report and successful solve. Core checks stamps and payload
/// identity, but cannot independently establish solver currentness.
pub struct DerivedGeometry<'a> {
    pub stamp: GeometryStamp,
    pub canonical_hash: ContentHash,
    pub solved: Option<&'a Sketch>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Endpoint {
    Start,
    End,
}
#[derive(Clone, Debug, PartialEq)]
pub struct TrimLine {
    pub target: EntityId,
    pub pick_mm: Position,
    pub boundaries: BTreeSet<EntityId>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ExtendLine {
    pub target: EntityId,
    pub endpoint: Endpoint,
    pub boundaries: BTreeSet<EntityId>,
}
pub type TrimCurve = TrimLine;
pub type ExtendArc = ExtendLine;
/// Finite exact tangent continuation. Multiple contacts require an explicit sorted contact index.
#[derive(Clone, Debug, PartialEq)]
pub struct ExtendSpline {
    pub target: EntityId,
    pub endpoint: Endpoint,
    pub boundaries: BTreeSet<EntityId>,
    pub max_length_mm: f64,
    pub contact_index: Option<usize>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum TrimRequest {
    Trim(TrimLine),
    Extend(ExtendLine),
    /// Trim a supported line, circular arc or circle by its picked span.
    TrimCurve(TrimCurve),
    ExtendArc(ExtendArc),
    ExtendSpline(ExtendSpline),
}
impl TrimRequest {
    pub fn target(&self) -> EntityId {
        match self {
            Self::Trim(r) | Self::TrimCurve(r) => r.target,
            Self::Extend(r) | Self::ExtendArc(r) => r.target,
            Self::ExtendSpline(r) => r.target,
        }
    }
    pub fn boundaries(&self) -> &BTreeSet<EntityId> {
        match self {
            Self::Trim(r) | Self::TrimCurve(r) => &r.boundaries,
            Self::Extend(r) | Self::ExtendArc(r) => &r.boundaries,
            Self::ExtendSpline(r) => &r.boundaries,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrimPolicy {
    pub tolerance_mm: f64,
}
impl Default for TrimPolicy {
    fn default() -> Self {
        Self { tolerance_mm: 1e-7 }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Contact {
    pub point_mm: Position,
    /// Directed angle in radians for circular targets, normalized line fraction
    /// otherwise. Arc-start extension contacts have negative angles.
    pub parameter: f64,
    pub boundaries: BTreeSet<EntityId>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Trim {
        removed_mm: [Position; 2],
    },
    Extend {
        endpoint: Endpoint,
        from_mm: Position,
        to_mm: Position,
    },
    TrimArc {
        removed_mm: [Position; 2],
        center_mm: Position,
        radius_mm: f64,
        clockwise: bool,
        removed_sweep_rad: f64,
    },
    ExtendArc {
        endpoint: Endpoint,
        from_mm: Position,
        to_mm: Position,
        center_mm: Position,
        radius_mm: f64,
        clockwise: bool,
        added_sweep_rad: f64,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Parameterization {
    LineFraction,
    ArcRadians,
    /// Normalized parameter of the exact queried curve, including continuation.
    CurveFraction,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CurveTypeChange {
    CircleToArc,
}
#[derive(Clone, Debug, PartialEq)]
pub struct TrimReport {
    pub required: GeometryStamp,
    pub canonical_hash: ContentHash,
    pub target: EntityId,
    pub boundaries: BTreeSet<EntityId>,
    pub action: Action,
    pub parameterization: Parameterization,
    pub type_changes: BTreeMap<EntityId, CurveTypeChange>,
    /// An interior split removes the old curve and creates both fragments.
    /// A single surviving curve keeps the old identity. Type changes are explicit.
    pub replacements: BTreeMap<EntityId, Vec<EntityId>>,
    pub created_points: BTreeSet<EntityId>,
    pub created_curves: BTreeSet<EntityId>,
    pub removed_curves: BTreeSet<EntityId>,
    /// Old endpoints remain untouched, including every listed neighboring curve.
    pub detached_points: BTreeMap<EntityId, BTreeSet<EntityId>>,
    pub preserved_constraints: BTreeSet<EntityId>,
    pub contacts: Vec<Contact>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct TrimDraft {
    pub source: Source,
    /// Solver seed only. Existing solved coordinates are never copied into the
    /// canonical source by this operation.
    pub seed: Sketch,
    pub report: TrimReport,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrimErrorKind {
    InvalidSource,
    InvalidContext,
    Unsolved,
    UnsupportedTarget,
    UnsupportedBoundary,
    InvalidRequest,
    InvalidGeometry,
    InvalidPick,
    AmbiguousIntersection,
    Overlap,
    NoIntersection,
    ConstraintConflict,
    Budget,
}
#[derive(Clone, Debug, PartialEq)]
pub struct TrimError {
    pub kind: TrimErrorKind,
    pub message: String,
    pub involved: BTreeSet<EntityId>,
}
impl std::fmt::Display for TrimError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(f)
    }
}
impl std::error::Error for TrimError {}
fn fail(
    kind: TrimErrorKind,
    message: impl Into<String>,
    ids: impl IntoIterator<Item = EntityId>,
) -> TrimError {
    TrimError {
        kind,
        message: message.into(),
        involved: ids.into_iter().collect(),
    }
}
pub fn canonical_hash(source: &Source) -> Result<ContentHash, TrimError> {
    source
        .validate()
        .map_err(|e| fail(TrimErrorKind::InvalidSource, e.to_string(), []))?;
    serde_json::to_vec(source)
        .map(|bytes| ContentHash::of(&bytes))
        .map_err(|e| fail(TrimErrorKind::InvalidSource, e.to_string(), []))
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
fn sub(a: Position, b: Position) -> Position {
    [a[0] - b[0], a[1] - b[1]]
}
fn dot(a: Position, b: Position) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
fn cross(a: Position, b: Position) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
fn length(a: Position) -> f64 {
    a[0].hypot(a[1])
}
fn along(a: Position, d: Position, t: f64) -> Position {
    [a[0] + d[0] * t, a[1] + d[1] * t]
}
fn point(sketch: &Sketch, id: EntityId) -> Position {
    sketch
        .points
        .iter()
        .find(|p| p.id == id)
        .expect("validated point")
        .position_mm
}
fn structure(source: &Source, solved: &Sketch) -> bool {
    source.sketch.points.len() == solved.points.len()
        && source.sketch.curves.len() == solved.curves.len()
        && source.sketch.constraints == solved.constraints
        && source
            .sketch
            .points
            .iter()
            .zip(&solved.points)
            .all(|(a, b)| a.id == b.id)
        && source
            .sketch
            .curves
            .iter()
            .zip(&solved.curves)
            .all(|(a, b)| {
                a.id == b.id
                    && a.construction == b.construction
                    && match (&a.kind, &b.kind) {
                        (
                            CurveKind::Circle { center: a, .. },
                            CurveKind::Circle { center: b, .. },
                        ) => a == b,
                        (a, b) => a == b,
                    }
            })
}
struct Axis {
    a: Position,
    d: Position,
    length: f64,
}
impl Axis {
    fn point(&self, t: f64) -> Position {
        along(self.a, self.d, t)
    }
}
fn relevant(request: &TrimRequest, t: f64) -> bool {
    match request {
        TrimRequest::Trim(_) | TrimRequest::TrimCurve(_) => (0. ..=1.).contains(&t),
        TrimRequest::ExtendSpline(_) => false,
        TrimRequest::Extend(r) | TrimRequest::ExtendArc(r) => match r.endpoint {
            Endpoint::Start => t < 0.,
            Endpoint::End => t > 1.,
        },
    }
}
fn validate_boundary(sketch: &Sketch, curve: &Curve, tolerance: f64) -> Result<(), TrimError> {
    if let CurveKind::Arc {
        center, start, end, ..
    } = curve.kind
    {
        let c = point(sketch, center);
        let a = point(sketch, start);
        let b = point(sketch, end);
        let radius = length(sub(a, c));
        if radius <= tolerance
            || (radius - length(sub(b, c))).abs() > tolerance
            || length(sub(a, b)) <= tolerance
        {
            return Err(fail(
                TrimErrorKind::InvalidGeometry,
                "Boundary arc must be nondegenerate and circular",
                [curve.id],
            ));
        }
    }
    Ok(())
}
fn boundary_contains(
    sketch: &Sketch,
    curve: &Curve,
    p: Position,
    tolerance: f64,
) -> Result<bool, TrimError> {
    let CurveKind::Arc {
        center,
        start,
        end,
        clockwise,
    } = curve.kind
    else {
        return Ok(true);
    };
    let c = point(sketch, center);
    let a = point(sketch, start);
    let b = point(sketch, end);
    for endpoint in [a, b] {
        if p == endpoint {
            return Ok(true);
        }
        if length(sub(p, endpoint)) <= tolerance {
            return Err(fail(
                TrimErrorKind::AmbiguousIntersection,
                "Intersection is too close to a boundary arc endpoint",
                [curve.id],
            ));
        }
    }
    let radius = length(sub(a, c));
    let angle = |p: Position| {
        let d = sub(p, c);
        d[1].atan2(d[0])
    };
    let sweep = if clockwise {
        (angle(a) - angle(b)).rem_euclid(TAU)
    } else {
        (angle(b) - angle(a)).rem_euclid(TAU)
    };
    let travel = if clockwise {
        (angle(a) - angle(p)).rem_euclid(TAU)
    } else {
        (angle(p) - angle(a)).rem_euclid(TAU)
    };
    if !radius.is_finite()
        || radius <= tolerance
        || (radius - length(sub(b, c))).abs() > tolerance
        || length(sub(a, b)) <= tolerance
    {
        return Err(fail(
            TrimErrorKind::InvalidGeometry,
            "Boundary arc must be nondegenerate and circular",
            [curve.id],
        ));
    }
    Ok(travel < sweep)
}
fn contacts(
    sketch: &Sketch,
    axis: &Axis,
    request: &TrimRequest,
    tolerance: f64,
) -> Result<Vec<Contact>, TrimError> {
    let mut contacts = vec![];
    for id in request.boundaries() {
        let curve = sketch.curves.iter().find(|c| c.id == *id).ok_or_else(|| {
            fail(
                TrimErrorKind::InvalidRequest,
                "Boundary is not a current authored curve",
                [*id],
            )
        })?;
        let mut candidates = vec![];
        validate_boundary(sketch, curve, tolerance)?;
        match curve.kind {
            CurveKind::BSpline { .. } => {
                return Err(fail(
                    TrimErrorKind::UnsupportedBoundary,
                    "Spline trim boundaries require a dedicated exact curve editing operation",
                    [request.target(), *id],
                ));
            }
            CurveKind::Line { start, end } => {
                let a = point(sketch, start);
                let b = point(sketch, end);
                let d = sub(b, a);
                let size = length(d);
                if size <= tolerance {
                    return Err(fail(
                        TrimErrorKind::InvalidGeometry,
                        "Boundary line is degenerate",
                        [*id],
                    ));
                }
                let det = cross(axis.d, d);
                let delta = sub(a, axis.a);
                if det == 0. {
                    if cross(delta, axis.d).abs() / axis.length <= tolerance {
                        let t0 = dot(delta, axis.d) / (axis.length * axis.length);
                        let t1 = dot(sub(b, axis.a), axis.d) / (axis.length * axis.length);
                        let (low, high) = (t0.min(t1), t0.max(t1));
                        let overlaps = match request {
                            TrimRequest::ExtendSpline(_) => false,
                            TrimRequest::Trim(_) | TrimRequest::TrimCurve(_) => {
                                high >= 0. && low <= 1.
                            }
                            TrimRequest::Extend(r) | TrimRequest::ExtendArc(r) => {
                                match r.endpoint {
                                    Endpoint::Start => low < 0.,
                                    Endpoint::End => high > 1.,
                                }
                            }
                        };
                        if overlaps {
                            return Err(fail(
                                TrimErrorKind::Overlap,
                                "Collinear or indistinguishably close boundary does not define a unique cut",
                                [request.target(), *id],
                            ));
                        }
                    }
                    continue;
                }
                let t = cross(delta, d) / det;
                let u = cross(delta, axis.d) / det;
                if !relevant(request, t) {
                    continue;
                }
                if !(0. ..=1.).contains(&u) {
                    if u.is_finite() && (u.abs().min((u - 1.).abs())) * size <= tolerance {
                        return Err(fail(
                            TrimErrorKind::AmbiguousIntersection,
                            "Intersection lies just outside a boundary endpoint",
                            [*id],
                        ));
                    }
                    continue;
                }
                if u != 0. && u != 1. && u.min(1. - u) * size <= tolerance {
                    return Err(fail(
                        TrimErrorKind::AmbiguousIntersection,
                        "Intersection is too close to a boundary endpoint",
                        [*id],
                    ));
                }
                candidates.push(t);
            }
            CurveKind::Circle { center, .. } | CurveKind::Arc { center, .. } => {
                let c = point(sketch, center);
                let radius = match curve.kind {
                    CurveKind::Circle { radius_mm, .. } => radius_mm,
                    CurveKind::Arc { start, .. } => length(sub(point(sketch, start), c)),
                    _ => unreachable!(),
                };
                let unit = axis.d.map(|v| v / axis.length);
                let delta = sub(c, axis.a);
                let projection = dot(delta, unit);
                let distance = cross(delta, unit).abs();
                if !radius.is_finite() || radius <= tolerance {
                    return Err(fail(
                        TrimErrorKind::InvalidGeometry,
                        "Boundary radius is degenerate",
                        [*id],
                    ));
                }
                if distance > radius + tolerance {
                    continue;
                }
                let h2 = (radius - distance) * (radius + distance);
                if (radius - distance).abs() <= tolerance {
                    let t = projection / axis.length;
                    if relevant(request, t)
                        && boundary_contains(sketch, curve, axis.point(t), tolerance)?
                    {
                        return Err(fail(
                            TrimErrorKind::AmbiguousIntersection,
                            "Tangent or near-tangent boundary contact is ambiguous",
                            [*id],
                        ));
                    }
                    continue;
                }
                if h2 < 0. {
                    continue;
                }
                let h = h2.sqrt();
                for t in [
                    (projection - h) / axis.length,
                    (projection + h) / axis.length,
                ] {
                    if relevant(request, t)
                        && boundary_contains(sketch, curve, axis.point(t), tolerance)?
                    {
                        candidates.push(t);
                    }
                }
            }
        }
        for t in candidates {
            let p = axis.point(t);
            if !t.is_finite() || p.iter().any(|v| !v.is_finite() || v.abs() > 1e9) {
                return Err(fail(
                    TrimErrorKind::InvalidGeometry,
                    "Intersection exceeds finite modeling bounds",
                    [*id],
                ));
            }
            let residual = match curve.kind {
                CurveKind::BSpline { .. } => {
                    return Err(fail(
                        TrimErrorKind::UnsupportedBoundary,
                        "Spline trim boundaries are not supported by the analytic line trim increment",
                        [*id],
                    ));
                }
                CurveKind::Line { start, end } => {
                    let a = point(sketch, start);
                    let d = sub(point(sketch, end), a);
                    cross(sub(p, a), d).abs() / length(d)
                }
                CurveKind::Circle { center, radius_mm } => {
                    (length(sub(p, point(sketch, center))) - radius_mm).abs()
                }
                CurveKind::Arc { center, start, .. } => {
                    let c = point(sketch, center);
                    (length(sub(p, c)) - length(sub(point(sketch, start), c))).abs()
                }
            };
            if residual > tolerance {
                return Err(fail(
                    TrimErrorKind::AmbiguousIntersection,
                    "Intersection residual exceeds the explicit geometry tolerance",
                    [*id, request.target()],
                ));
            }
            let endpoint_distance = t.abs().min((t - 1.).abs()) * axis.length;
            if t == 0. || t == 1. {
                continue;
            }
            if endpoint_distance <= tolerance {
                return Err(fail(
                    TrimErrorKind::AmbiguousIntersection,
                    "Cut would create a zero or tolerance-sized span",
                    [request.target(), *id],
                ));
            }
            contacts.push(Contact {
                point_mm: p,
                parameter: t,
                boundaries: BTreeSet::from([*id]),
            });
        }
    }
    contacts.sort_by(|a, b| a.parameter.total_cmp(&b.parameter));
    let mut unique: Vec<Contact> = vec![];
    for contact in contacts {
        if let Some(previous) = unique.last()
            && (previous.parameter - contact.parameter).abs() * axis.length <= tolerance
        {
            return Err(fail(
                TrimErrorKind::AmbiguousIntersection,
                "Multiple boundary intersections coincide or are too close; choose a single boundary",
                previous.boundaries.union(&contact.boundaries).copied(),
            ));
        }
        unique.push(contact);
    }
    Ok(unique)
}
/// The line-specific requests retain their original target restriction. Generic
/// trim and arc extension use exact circular geometry; spline targets, tangencies
/// and ambiguous near-endpoint cuts remain explicit failures.
pub fn draft(
    source: &Source,
    expected: GeometryStamp,
    input: DerivedGeometry<'_>,
    request: &TrimRequest,
    policy: TrimPolicy,
) -> Result<TrimDraft, TrimError> {
    draft_with_queries(
        source,
        expected,
        input,
        request,
        policy,
        &crate::sketch_regions::AnalyticQueries,
    )
}
/// Exact worker-side editing. Native query capabilities never enter canonical source.
pub fn draft_with_queries(
    source: &Source,
    expected: GeometryStamp,
    input: DerivedGeometry<'_>,
    request: &TrimRequest,
    policy: TrimPolicy,
    queries: &dyn crate::sketch_regions::RegionQueries,
) -> Result<TrimDraft, TrimError> {
    let hash = canonical_hash(source)?;
    if input.stamp != expected || input.canonical_hash != hash {
        return Err(fail(
            TrimErrorKind::InvalidContext,
            "Derived geometry belongs to stale or foreign source",
            [expected.sketch],
        ));
    }
    let solved = input.solved.ok_or_else(|| {
        fail(
            TrimErrorKind::Unsolved,
            "A current successful sketch solve is required",
            [expected.sketch],
        )
    })?;
    if !structure(source, solved) {
        return Err(fail(
            TrimErrorKind::InvalidContext,
            "Solved geometry does not preserve canonical sketch identity",
            [expected.sketch],
        ));
    }
    Source {
        u: source.u,
        sketch: solved.clone(),
    }
    .validate()
    .map_err(|e| {
        fail(
            TrimErrorKind::InvalidContext,
            e.to_string(),
            [expected.sketch],
        )
    })?;
    let tolerance = policy.tolerance_mm;
    if !tolerance.is_finite()
        || !(1e-9..=1e-3).contains(&tolerance)
        || request.boundaries().is_empty()
        || request.boundaries().len() > 128
        || request.boundaries().contains(&request.target())
    {
        return Err(fail(
            TrimErrorKind::InvalidRequest,
            "Choose 1–128 other boundaries and a valid explicit tolerance",
            [request.target()],
        ));
    }
    let target = source
        .sketch
        .curves
        .iter()
        .find(|c| c.id == request.target())
        .ok_or_else(|| {
            fail(
                TrimErrorKind::InvalidRequest,
                "Target is not a current authored curve",
                [request.target()],
            )
        })?;
    let spline_boundary = solved.curves.iter().any(|c| {
        request.boundaries().contains(&c.id) && matches!(c.kind, CurveKind::BSpline { .. })
    });
    if matches!(target.kind, CurveKind::BSpline { .. })
        || spline_boundary
        || matches!(request, TrimRequest::ExtendSpline(_))
    {
        return exact::draft(
            source, solved, expected, hash, target, request, tolerance, queries,
        );
    }
    if matches!(
        request,
        TrimRequest::TrimCurve(_) | TrimRequest::ExtendArc(_)
    ) && matches!(
        target.kind,
        CurveKind::Arc { .. } | CurveKind::Circle { .. }
    ) {
        return circular::draft(source, solved, expected, hash, target, request, tolerance);
    }
    if matches!(request, TrimRequest::ExtendArc(_)) {
        return Err(fail(
            TrimErrorKind::UnsupportedTarget,
            "Arc extension requires an actual circular arc",
            [target.id],
        ));
    }
    let CurveKind::Line { start, end } = target.kind else {
        return Err(fail(
            TrimErrorKind::UnsupportedTarget,
            "This trim/extend increment supports line targets only",
            [target.id],
        ));
    };
    let a = point(solved, start);
    let b = point(solved, end);
    let d = sub(b, a);
    let size = length(d);
    if size <= tolerance {
        return Err(fail(
            TrimErrorKind::InvalidGeometry,
            "Target line is degenerate",
            [target.id, start, end],
        ));
    }
    let axis = Axis { a, d, length: size };
    let contacts = contacts(solved, &axis, request, tolerance)?;
    let (spans, action): (Vec<[f64; 2]>, Action) = match request {
        TrimRequest::ExtendSpline(_) => {
            return Err(fail(
                TrimErrorKind::UnsupportedTarget,
                "Spline extension requires the exact query service",
                [target.id],
            ));
        }
        TrimRequest::Trim(r) | TrimRequest::TrimCurve(r) => {
            if r.pick_mm.iter().any(|v| !v.is_finite() || v.abs() > 1e9) {
                return Err(fail(
                    TrimErrorKind::InvalidPick,
                    "Pick must be finite and on the current line",
                    [target.id],
                ));
            }
            let delta = sub(r.pick_mm, a);
            let t = dot(delta, d) / (size * size);
            if cross(delta, d).abs() / size > tolerance || !(0. ..=1.).contains(&t) {
                return Err(fail(
                    TrimErrorKind::InvalidPick,
                    "Pick must lie on the actual current line segment",
                    [target.id],
                ));
            }
            if t.min(1. - t) * size <= tolerance
                || contacts
                    .iter()
                    .any(|c| (c.parameter - t).abs() * size <= tolerance)
            {
                return Err(fail(
                    TrimErrorKind::InvalidPick,
                    "Pick is on or too close to a cut or endpoint; choose a span",
                    [target.id],
                ));
            }
            let low = contacts
                .iter()
                .rfind(|c| c.parameter < t)
                .map_or(0., |c| c.parameter);
            let high = contacts
                .iter()
                .find(|c| c.parameter > t)
                .map_or(1., |c| c.parameter);
            if low == 0. && high == 1. {
                return Err(fail(
                    TrimErrorKind::NoIntersection,
                    "No interior boundary defines a trim span",
                    [target.id],
                ));
            }
            let mut spans = vec![];
            if low > 0. {
                spans.push([0., low]);
            }
            if high < 1. {
                spans.push([high, 1.]);
            }
            (
                spans,
                Action::Trim {
                    removed_mm: [axis.point(low), axis.point(high)],
                },
            )
        }
        TrimRequest::Extend(r) | TrimRequest::ExtendArc(r) => {
            let contact = match r.endpoint {
                Endpoint::Start => contacts.last(),
                Endpoint::End => contacts.first(),
            }
            .ok_or_else(|| {
                fail(
                    TrimErrorKind::NoIntersection,
                    "No actual boundary lies beyond the selected endpoint",
                    [target.id],
                )
            })?;
            let span = match r.endpoint {
                Endpoint::Start => [contact.parameter, 1.],
                Endpoint::End => [0., contact.parameter],
            };
            (
                vec![span],
                Action::Extend {
                    endpoint: r.endpoint,
                    from_mm: match r.endpoint {
                        Endpoint::Start => a,
                        Endpoint::End => b,
                    },
                    to_mm: contact.point_mm,
                },
            )
        }
    };
    let detached: BTreeSet<_> = [(start, 0.), (end, 1.)]
        .into_iter()
        .filter(|(_, t)| !spans.iter().any(|s| s.contains(t)))
        .map(|(id, _)| id)
        .collect();
    let affected: BTreeSet<_> = std::iter::once(target.id)
        .chain(detached.iter().copied())
        .collect();
    let conflicting: Vec<_> = source
        .sketch
        .constraints
        .iter()
        .filter(|c| c.kind.references().iter().any(|id| affected.contains(id)))
        .collect();
    if !conflicting.is_empty() {
        return Err(fail(
            TrimErrorKind::ConstraintConflict,
            "Trim/extend would change a constrained curve or detach a constrained endpoint; driving and reference constraints were preserved",
            affected.into_iter().chain(conflicting.iter().map(|c| c.id)),
        ));
    }
    let mut candidate = source.clone();
    let mut seed = solved.clone();
    let split = spans.len() == 2;
    let mut replacement = vec![];
    let mut created_points = BTreeSet::new();
    let mut created_curves = BTreeSet::new();
    let mut new_curves = vec![];
    for [low, high] in spans {
        let mut ids = [start, end];
        for (index, t) in [low, high].into_iter().enumerate() {
            if (index == 0 && t == 0.) || (index == 1 && t == 1.) {
                continue;
            }
            let id = EntityId::new();
            let p = Point {
                id,
                position_mm: axis.point(t),
            };
            candidate.sketch.points.push(p.clone());
            seed.points.push(p);
            ids[index] = id;
            created_points.insert(id);
        }
        let id = if split {
            let id = EntityId::new();
            created_curves.insert(id);
            id
        } else {
            target.id
        };
        replacement.push(id);
        new_curves.push(Curve {
            id,
            construction: target.construction,
            kind: CurveKind::Line {
                start: ids[0],
                end: ids[1],
            },
        });
    }
    for sketch in [&mut candidate.sketch, &mut seed] {
        let index = sketch
            .curves
            .iter()
            .position(|c| c.id == target.id)
            .expect("target exists");
        sketch.curves.splice(index..index + 1, new_curves.clone());
    }
    candidate
        .validate()
        .map_err(|e| fail(TrimErrorKind::Budget, e.to_string(), [target.id]))?;
    Source {
        u: source.u,
        sketch: seed.clone(),
    }
    .validate()
    .map_err(|e| fail(TrimErrorKind::InvalidGeometry, e.to_string(), [target.id]))?;
    let detached_points = detached
        .into_iter()
        .map(|id| {
            (
                id,
                source
                    .sketch
                    .curves
                    .iter()
                    .filter(|c| c.id != target.id && points(c).contains(&id))
                    .map(|c| c.id)
                    .collect(),
            )
        })
        .collect();
    Ok(TrimDraft {
        source: candidate,
        seed,
        report: TrimReport {
            required: expected,
            canonical_hash: hash,
            target: target.id,
            boundaries: request.boundaries().clone(),
            action,
            parameterization: Parameterization::LineFraction,
            type_changes: BTreeMap::new(),
            replacements: BTreeMap::from([(target.id, replacement)]),
            created_points,
            created_curves,
            removed_curves: if split {
                BTreeSet::from([target.id])
            } else {
                BTreeSet::new()
            },
            detached_points,
            preserved_constraints: source.sketch.constraints.iter().map(|c| c.id).collect(),
            contacts,
        },
    })
}

/// Discover bounded continuation contacts without creating a commit-capable edit.
pub fn spline_extension_contacts(
    source: &Source,
    expected: GeometryStamp,
    input: DerivedGeometry<'_>,
    request: &ExtendSpline,
    policy: TrimPolicy,
    queries: &dyn crate::sketch_regions::RegionQueries,
) -> Result<Vec<Contact>, TrimError> {
    let hash = canonical_hash(source)?;
    if input.stamp != expected || input.canonical_hash != hash {
        return Err(fail(
            TrimErrorKind::InvalidContext,
            "Derived geometry belongs to stale or foreign source",
            [expected.sketch],
        ));
    }
    let solved = input.solved.ok_or_else(|| {
        fail(
            TrimErrorKind::Unsolved,
            "A current successful sketch solve is required",
            [expected.sketch],
        )
    })?;
    if !structure(source, solved) {
        return Err(fail(
            TrimErrorKind::InvalidContext,
            "Solved geometry does not preserve canonical sketch identity",
            [expected.sketch],
        ));
    }
    Source {
        u: source.u,
        sketch: solved.clone(),
    }
    .validate()
    .map_err(|e| {
        fail(
            TrimErrorKind::InvalidContext,
            e.to_string(),
            [expected.sketch],
        )
    })?;
    let target = source
        .sketch
        .curves
        .iter()
        .find(|c| c.id == request.target)
        .ok_or_else(|| {
            fail(
                TrimErrorKind::InvalidRequest,
                "Target is not a current authored curve",
                [request.target],
            )
        })?;
    exact::extension_contacts(solved, target, request, policy.tolerance_mm, queries)
}
