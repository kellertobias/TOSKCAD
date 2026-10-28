//! Exact finite curve edits. Query results contain no source identity decisions.
use super::*;
use crate::{
    constraint_sketch::ConstraintKind,
    sketch_regions::{CurveProjection, ExactCurve, RegionQueries, SplineContinuation},
};

const MAX_CONTACTS: usize = 256;
const PARAMETER_EPSILON: f64 = 1e-12;
fn query<T>(
    result: Result<T, crate::sketch_regions::QueryError>,
    id: EntityId,
) -> Result<T, TrimError> {
    result.map_err(|e| fail(TrimErrorKind::InvalidGeometry, e.to_string(), [id]))
}
fn exact(sketch: &Sketch, curve: &Curve) -> ExactCurve {
    match &curve.kind {
        CurveKind::Line { start, end } => ExactCurve::Line {
            a: point(sketch, *start),
            b: point(sketch, *end),
        },
        CurveKind::Arc {
            center,
            start,
            end,
            clockwise,
        } => ExactCurve::Arc {
            center: point(sketch, *center),
            a: point(sketch, *start),
            b: point(sketch, *end),
            clockwise: *clockwise,
        },
        CurveKind::Circle { center, radius_mm } => ExactCurve::Circle {
            center: point(sketch, *center),
            radius_mm: *radius_mm,
        },
        CurveKind::BSpline { poles, basis } => ExactCurve::BSpline {
            poles: poles.iter().map(|id| point(sketch, *id)).collect(),
            basis: basis.clone(),
        },
    }
}
fn ends(curve: &ExactCurve) -> (Position, Position) {
    match curve {
        ExactCurve::Line { a, b } | ExactCurve::Arc { a, b, .. } => (*a, *b),
        ExactCurve::BSpline { poles, .. } => (poles[0], *poles.last().expect("validated poles")),
        ExactCurve::Circle { center, radius_mm } => {
            let p = [center[0] + radius_mm, center[1]];
            (p, p)
        }
    }
}
fn checked_curve(
    curve: &ExactCurve,
    id: EntityId,
    tol: f64,
    queries: &dyn RegionQueries,
) -> Result<(), TrimError> {
    let positions: Vec<_> = match curve {
        ExactCurve::BSpline { poles, basis } => {
            basis
                .validate(poles.len())
                .map_err(|e| fail(TrimErrorKind::InvalidGeometry, e.to_string(), [id]))?;
            poles.clone()
        }
        ExactCurve::Line { a, b } => vec![*a, *b],
        ExactCurve::Arc { center, a, b, .. } => vec![*center, *a, *b],
        ExactCurve::Circle { center, radius_mm } => {
            if !radius_mm.is_finite() || *radius_mm <= tol || *radius_mm > 1e9 {
                return Err(fail(
                    TrimErrorKind::InvalidGeometry,
                    "Invalid exact circle radius",
                    [id],
                ));
            }
            vec![*center]
        }
    };
    if positions
        .iter()
        .flatten()
        .any(|x| !x.is_finite() || x.abs() > 1e9)
    {
        return Err(fail(
            TrimErrorKind::InvalidGeometry,
            "Exact curve exceeds finite modeling bounds",
            [id],
        ));
    }
    let hits = query(queries.self_intersections(curve, tol), id)?;
    if hits.overlap || !hits.points.is_empty() {
        return Err(fail(
            TrimErrorKind::AmbiguousIntersection,
            "Self-intersecting or overlapping curves cannot define an unambiguous edit",
            [id],
        ));
    }
    Ok(())
}
fn project(
    curve: &ExactCurve,
    p: Position,
    tol: f64,
    queries: &dyn RegionQueries,
    id: EntityId,
) -> Result<CurveProjection, TrimError> {
    let projections = query(queries.project_parameters(curve, p, tol), id)?;
    if projections.len() > MAX_CONTACTS {
        return Err(fail(
            TrimErrorKind::Budget,
            "Projection candidate budget exceeded",
            [id],
        ));
    }
    let mut valid = Vec::<CurveProjection>::new();
    for h in projections {
        if !h.parameter.is_finite()
            || !(0. ..=1.).contains(&h.parameter)
            || !h.distance_mm.is_finite()
            || h.distance_mm < 0.
            || h.point_mm
                .iter()
                .chain(&h.tangent)
                .any(|x| !x.is_finite() || x.abs() > 1e9)
            || length(h.tangent) <= 1e-15
        {
            return Err(fail(
                TrimErrorKind::InvalidGeometry,
                "Native projection is nonfinite, singular or outside the finite curve domain",
                [id],
            ));
        }
        if h.distance_mm <= tol
            && length(sub(h.point_mm, p)) <= tol
            && !valid
                .iter()
                .any(|a| (a.parameter - h.parameter).abs() <= PARAMETER_EPSILON)
        {
            valid.push(h);
        }
    }
    if valid.len() != 1 {
        return Err(fail(
            if valid.is_empty() {
                TrimErrorKind::InvalidPick
            } else {
                TrimErrorKind::AmbiguousIntersection
            },
            "Point must identify one exact current curve parameter within tolerance",
            [id],
        ));
    }
    Ok(valid.remove(0))
}
fn validate_request(
    request: &TrimRequest,
    tol: f64,
    queries: &dyn RegionQueries,
) -> Result<(), TrimError> {
    if !tol.is_finite()
        || !(1e-9..=1e-3).contains(&tol)
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
    if request.boundaries().len() + 1 > queries.maximum_curves(true) {
        return Err(fail(
            TrimErrorKind::Budget,
            "Exact spline edit exceeds the bounded curve-query budget",
            [request.target()],
        ));
    }
    Ok(())
}
fn continuation(
    sketch: &Sketch,
    target: &Curve,
    r: &ExtendSpline,
    tol: f64,
    queries: &dyn RegionQueries,
) -> Result<SplineContinuation, TrimError> {
    validate_request(&TrimRequest::ExtendSpline(r.clone()), tol, queries)?;
    if !matches!(target.kind, CurveKind::BSpline { .. }) {
        return Err(fail(
            TrimErrorKind::UnsupportedTarget,
            "Tangent continuation requires a B-spline endpoint",
            [target.id],
        ));
    }
    if !r.max_length_mm.is_finite() || r.max_length_mm <= tol || r.max_length_mm > 1e6 {
        return Err(fail(
            TrimErrorKind::InvalidRequest,
            "Tangent continuation length must exceed tolerance and be at most 1000000 mm",
            [target.id],
        ));
    }
    let curve = exact(sketch, target);
    checked_curve(&curve, target.id, tol, queries)?;
    let (a, b) = ends(&curve);
    if length(sub(a, b)) <= tol {
        return Err(fail(
            TrimErrorKind::UnsupportedTarget,
            "A closed spline has no distinct endpoint to continue",
            [target.id],
        ));
    }
    let result = query(
        queries.spline_continuation(&curve, r.endpoint == Endpoint::Start, r.max_length_mm, tol),
        target.id,
    )?;
    let [low, high] = result.original_range;
    let correct = match r.endpoint {
        Endpoint::Start => low > 0. && high == 1.,
        Endpoint::End => low == 0. && high < 1.,
    };
    if !low.is_finite()
        || !high.is_finite()
        || low >= high
        || low < 0.
        || high > 1.
        || !correct
        || !matches!(result.curve, ExactCurve::BSpline { .. })
    {
        return Err(fail(
            TrimErrorKind::InvalidGeometry,
            "Native tangent continuation did not retain an identifiable original interval",
            [target.id],
        ));
    }
    checked_curve(&result.curve, target.id, tol, queries)?;
    Ok(result)
}
fn contacts(
    sketch: &Sketch,
    target: &Curve,
    curve: &ExactCurve,
    request: &TrimRequest,
    tol: f64,
    queries: &dyn RegionQueries,
    original: [f64; 2],
) -> Result<Vec<Contact>, TrimError> {
    let mut result = Vec::<Contact>::new();
    let (a, b) = ends(curve);
    for id in request.boundaries() {
        let boundary = sketch.curves.iter().find(|c| c.id == *id).ok_or_else(|| {
            fail(
                TrimErrorKind::InvalidRequest,
                "Boundary is not a current authored curve",
                [*id],
            )
        })?;
        let boundary = exact(sketch, boundary);
        checked_curve(&boundary, *id, tol, queries)?;
        let hits = query(queries.intersections(curve, &boundary, tol), *id)?;
        if hits.overlap {
            return Err(fail(
                TrimErrorKind::Overlap,
                "Overlapping curves cannot identify a unique edit contact",
                [target.id, *id],
            ));
        }
        if hits.points.len() > MAX_CONTACTS {
            return Err(fail(
                TrimErrorKind::Budget,
                "Exact intersection budget exceeded",
                [target.id, *id],
            ));
        }
        for p in hits.points {
            let h = project(curve, p, tol, queries, target.id)?;
            if let TrimRequest::ExtendSpline(r) = request {
                let is_extension = match r.endpoint {
                    Endpoint::Start => h.parameter < original[0],
                    Endpoint::End => h.parameter > original[1],
                };
                if !is_extension {
                    continue;
                }
                let ids = points(target);
                let endpoint = match r.endpoint {
                    Endpoint::Start => ids[0],
                    Endpoint::End => *ids.last().expect("validated spline"),
                };
                if length(sub(p, point(sketch, endpoint))) <= tol {
                    return Err(fail(
                        TrimErrorKind::AmbiguousIntersection,
                        "Continuation contact is too close to the original endpoint",
                        [target.id, *id],
                    ));
                }
            }
            let bh = project(&boundary, p, tol, queries, *id)?;
            let sine =
                cross(h.tangent, bh.tangent).abs() / (length(h.tangent) * length(bh.tangent));
            if !sine.is_finite() || sine <= 1e-7 {
                return Err(fail(
                    TrimErrorKind::AmbiguousIntersection,
                    "Tangent or ill-conditioned exact curve contact requires another boundary",
                    [target.id, *id],
                ));
            }
            if !matches!(request, TrimRequest::ExtendSpline(_)) {
                if h.parameter == 0. || h.parameter == 1. {
                    continue;
                }
                if length(sub(p, a)).min(length(sub(p, b))) <= tol {
                    return Err(fail(
                        TrimErrorKind::AmbiguousIntersection,
                        "Intersection is too close to a curve endpoint",
                        [target.id, *id],
                    ));
                }
            }
            if result.iter().any(|old| {
                length(sub(old.point_mm, p)) <= tol
                    || (old.parameter - h.parameter).abs() <= PARAMETER_EPSILON
            }) {
                return Err(fail(
                    TrimErrorKind::AmbiguousIntersection,
                    "Coincident boundary contacts require selecting one explicit boundary",
                    [target.id, *id],
                ));
            }
            result.push(Contact {
                point_mm: h.point_mm,
                parameter: h.parameter,
                boundaries: BTreeSet::from([*id]),
            });
            if result.len() > MAX_CONTACTS {
                return Err(fail(
                    TrimErrorKind::Budget,
                    "Exact edit contact budget exceeded",
                    [target.id],
                ));
            }
        }
    }
    result.sort_by(|a, b| a.parameter.total_cmp(&b.parameter));
    Ok(result)
}
pub(super) fn extension_contacts(
    sketch: &Sketch,
    target: &Curve,
    r: &ExtendSpline,
    tol: f64,
    queries: &dyn RegionQueries,
) -> Result<Vec<Contact>, TrimError> {
    let extended = continuation(sketch, target, r, tol, queries)?;
    contacts(
        sketch,
        target,
        &extended.curve,
        &TrimRequest::ExtendSpline(r.clone()),
        tol,
        queries,
        extended.original_range,
    )
}
#[allow(clippy::too_many_arguments)]
pub(super) fn draft(
    source: &Source,
    solved: &Sketch,
    expected: GeometryStamp,
    hash: ContentHash,
    target: &Curve,
    request: &TrimRequest,
    tol: f64,
    queries: &dyn RegionQueries,
) -> Result<TrimDraft, TrimError> {
    validate_request(request, tol, queries)?;
    let evaluated = solved
        .curves
        .iter()
        .find(|c| c.id == target.id)
        .expect("validated identity");
    let (curve, original) = if let TrimRequest::ExtendSpline(r) = request {
        let c = continuation(solved, evaluated, r, tol, queries)?;
        (c.curve, c.original_range)
    } else {
        if matches!(request, TrimRequest::Extend(_) | TrimRequest::ExtendArc(_)) {
            return Err(fail(
                if matches!(target.kind, CurveKind::BSpline { .. }) {
                    TrimErrorKind::UnsupportedTarget
                } else {
                    TrimErrorKind::UnsupportedBoundary
                },
                "Use bounded spline tangent continuation for a spline target; analytic extension against spline boundaries is unsupported",
                [target.id],
            ));
        }
        if matches!(request, TrimRequest::Trim(_)) && !matches!(target.kind, CurveKind::Line { .. })
        {
            return Err(fail(
                TrimErrorKind::UnsupportedTarget,
                "Line trim requires a line target; use Trim curve for splines",
                [target.id],
            ));
        }
        let c = exact(solved, evaluated);
        checked_curve(&c, target.id, tol, queries)?;
        (c, [0., 1.])
    };
    let (a, b) = ends(&curve);
    if !matches!(curve, ExactCurve::Circle { .. }) && length(sub(a, b)) <= tol {
        return Err(fail(
            TrimErrorKind::UnsupportedTarget,
            "Closed spline trimming requires an explicit seam-aware edit",
            [target.id],
        ));
    }
    let hits = contacts(solved, target, &curve, request, tol, queries, original)?;
    let (spans, action) = match request {
        TrimRequest::ExtendSpline(r) => {
            let index = match (r.contact_index, hits.len()) {
                (Some(i), _) => i,
                (None, 1) => 0,
                (None, 0) => {
                    return Err(fail(
                        TrimErrorKind::NoIntersection,
                        "No boundary lies within the bounded tangent continuation",
                        [target.id],
                    ));
                }
                (None, _) => {
                    return Err(fail(
                        TrimErrorKind::AmbiguousIntersection,
                        "Several tangent-continuation contacts exist; explicitly choose one",
                        [target.id],
                    ));
                }
            };
            let contact = hits.get(index).ok_or_else(|| {
                fail(
                    TrimErrorKind::InvalidRequest,
                    "Selected continuation contact is unavailable; refresh the choices",
                    [target.id],
                )
            })?;
            let (span, from) = match r.endpoint {
                Endpoint::Start => ([contact.parameter, 1.], point(solved, points(evaluated)[0])),
                Endpoint::End => (
                    [0., contact.parameter],
                    point(solved, *points(evaluated).last().expect("poles")),
                ),
            };
            (
                vec![span],
                Action::Extend {
                    endpoint: r.endpoint,
                    from_mm: from,
                    to_mm: contact.point_mm,
                },
            )
        }
        TrimRequest::Trim(r) | TrimRequest::TrimCurve(r) => {
            if r.pick_mm.iter().any(|x| !x.is_finite() || x.abs() > 1e9) {
                return Err(fail(
                    TrimErrorKind::InvalidPick,
                    "Pick must be finite and on the exact current curve",
                    [target.id],
                ));
            }
            let pick = project(&curve, r.pick_mm, tol, queries, target.id)?;
            if length(sub(pick.point_mm, a)).min(length(sub(pick.point_mm, b))) <= tol
                || hits
                    .iter()
                    .any(|c| length(sub(c.point_mm, pick.point_mm)) <= tol)
            {
                return Err(fail(
                    TrimErrorKind::InvalidPick,
                    "Pick an open span away from intersections and endpoints",
                    [target.id],
                ));
            }
            if matches!(curve, ExactCurve::Circle { .. }) {
                return Err(fail(
                    TrimErrorKind::UnsupportedTarget,
                    "Circle trim against spline boundaries requires an explicit seam-aware edit",
                    [target.id],
                ));
            }
            let low = hits
                .iter()
                .rfind(|h| h.parameter < pick.parameter)
                .map_or(0., |h| h.parameter);
            let high = hits
                .iter()
                .find(|h| h.parameter > pick.parameter)
                .map_or(1., |h| h.parameter);
            if low == 0. && high == 1. {
                return Err(fail(
                    TrimErrorKind::NoIntersection,
                    "No interior boundary defines the picked trim span",
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
            let cut_point = |t: f64| {
                if t == 0. {
                    a
                } else if t == 1. {
                    b
                } else {
                    hits.iter()
                        .find(|h| h.parameter == t)
                        .expect("cut contact")
                        .point_mm
                }
            };
            (
                spans,
                Action::Trim {
                    removed_mm: [cut_point(low), cut_point(high)],
                },
            )
        }
        _ => {
            return Err(fail(
                TrimErrorKind::UnsupportedTarget,
                "Choose exact Trim curve or bounded spline continuation",
                [target.id],
            ));
        }
    };
    apply_segments(
        source, solved, expected, hash, target, request, &curve, original, spans, action, hits,
        tol, queries,
    )
}
#[allow(clippy::too_many_arguments)]
fn apply_segments(
    source: &Source,
    solved: &Sketch,
    expected: GeometryStamp,
    hash: ContentHash,
    target: &Curve,
    request: &TrimRequest,
    curve: &ExactCurve,
    original: [f64; 2],
    spans: Vec<[f64; 2]>,
    action: Action,
    contacts: Vec<Contact>,
    tol: f64,
    queries: &dyn RegionQueries,
) -> Result<TrimDraft, TrimError> {
    let old = points(target);
    let endpoint_ids = match &target.kind {
        CurveKind::Line { start, end } | CurveKind::Arc { start, end, .. } => Some([*start, *end]),
        CurveKind::BSpline { poles, .. } => {
            Some([poles[0], *poles.last().expect("validated poles")])
        }
        CurveKind::Circle { .. } => None,
    };
    let mut preserved = BTreeSet::new();
    if let Some(ids) = endpoint_ids {
        for span in &spans {
            if span[0] == original[0] {
                preserved.insert(ids[0]);
            }
            if span[1] == original[1] {
                preserved.insert(ids[1]);
            }
        }
    }
    if let CurveKind::Arc { center, .. } | CurveKind::Circle { center, .. } = target.kind {
        preserved.insert(center);
    }
    let detached: BTreeSet<_> = old
        .iter()
        .filter(|id| !preserved.contains(id))
        .copied()
        .collect();
    let conflicts:BTreeSet<_>=source.sketch.constraints.iter().filter(|c|{
        let refs=c.kind.references();let radius_compatible=spans.len()==1&&matches!(target.kind,CurveKind::Arc{..}|CurveKind::Circle{..})&&matches!(c.kind,ConstraintKind::Radius{curve,..}|ConstraintKind::Diameter{curve,..} if curve==target.id);
        refs.iter().any(|id|detached.contains(id))||(refs.contains(&target.id)&&!radius_compatible)
    }).map(|c|c.id).collect();
    if !conflicts.is_empty() {
        return Err(fail(
            TrimErrorKind::ConstraintConflict,
            "Spline edit would replace constrained control poles or curve intent; constraints and source were preserved",
            conflicts
                .into_iter()
                .chain(detached.iter().copied())
                .chain([target.id]),
        ));
    }
    let mut candidate = source.clone();
    let mut seed = solved.clone();
    let split = spans.len() > 1;
    let mut created_points = BTreeSet::new();
    let mut created_curves = BTreeSet::new();
    let mut replacements = vec![];
    let mut curves = vec![];
    for span in spans {
        let segment = query(
            queries.curve_segment(curve, span[0], span[1], tol),
            target.id,
        )?;
        checked_curve(&segment, target.id, tol, queries)?;
        let (a, b) = ends(&segment);
        if length(sub(a, b)) <= tol {
            return Err(fail(
                TrimErrorKind::AmbiguousIntersection,
                "Retained exact span is zero-sized or closed",
                [target.id],
            ));
        }
        let mut add =
            |position: Position, retained: Option<EntityId>| -> Result<EntityId, TrimError> {
                if let Some(id) = retained {
                    if length(sub(position, point(solved, id))) > tol {
                        return Err(fail(
                            TrimErrorKind::InvalidGeometry,
                            "Exact edit moved a retained authored endpoint",
                            [target.id, id],
                        ));
                    }
                    return Ok(id);
                }
                let id = EntityId::new();
                let p = Point {
                    id,
                    position_mm: position,
                };
                candidate.sketch.points.push(p.clone());
                seed.points.push(p);
                created_points.insert(id);
                Ok(id)
            };
        let first = endpoint_ids.and_then(|ids| (span[0] == original[0]).then_some(ids[0]));
        let last = endpoint_ids.and_then(|ids| (span[1] == original[1]).then_some(ids[1]));
        let kind = match segment {
            ExactCurve::BSpline { poles, basis } => {
                let count = poles.len();
                let ids = poles
                    .into_iter()
                    .enumerate()
                    .map(|(i, p)| {
                        add(
                            p,
                            if i == 0 {
                                first
                            } else if i + 1 == count {
                                last
                            } else {
                                None
                            },
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                CurveKind::BSpline { poles: ids, basis }
            }
            ExactCurve::Line { a, b } => CurveKind::Line {
                start: add(a, first)?,
                end: add(b, last)?,
            },
            ExactCurve::Arc {
                center,
                a,
                b,
                clockwise,
            } => {
                let center_id = match target.kind {
                    CurveKind::Arc { center, .. } | CurveKind::Circle { center, .. } => center,
                    _ => {
                        return Err(fail(
                            TrimErrorKind::InvalidGeometry,
                            "Native segmentation unexpectedly changed the authored curve kind",
                            [target.id],
                        ));
                    }
                };
                if length(sub(center, point(solved, center_id))) > tol {
                    return Err(fail(
                        TrimErrorKind::InvalidGeometry,
                        "Exact segmentation changed the circular center",
                        [target.id],
                    ));
                }
                CurveKind::Arc {
                    center: center_id,
                    start: add(a, first)?,
                    end: add(b, last)?,
                    clockwise,
                }
            }
            ExactCurve::Circle { .. } => {
                return Err(fail(
                    TrimErrorKind::InvalidGeometry,
                    "Segmentation returned a closed whole circle",
                    [target.id],
                ));
            }
        };
        if matches!(target.kind, CurveKind::BSpline { .. })
            && !matches!(kind, CurveKind::BSpline { .. })
        {
            return Err(fail(
                TrimErrorKind::InvalidGeometry,
                "Spline segmentation must retain an exact spline basis",
                [target.id],
            ));
        }
        let id = if split {
            let id = EntityId::new();
            created_curves.insert(id);
            id
        } else {
            target.id
        };
        replacements.push(id);
        curves.push(Curve {
            id,
            construction: target.construction,
            kind,
        });
    }
    for sketch in [&mut candidate.sketch, &mut seed] {
        let index = sketch
            .curves
            .iter()
            .position(|c| c.id == target.id)
            .expect("target exists");
        sketch.curves.splice(index..index + 1, curves.clone());
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
            parameterization: Parameterization::CurveFraction,
            type_changes: if matches!(target.kind, CurveKind::Circle { .. }) {
                BTreeMap::from([(target.id, CurveTypeChange::CircleToArc)])
            } else {
                BTreeMap::new()
            },
            replacements: BTreeMap::from([(target.id, replacements)]),
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
