//! Circular-target editing uses analytic circle intersections and finite curve
//! domains. Neither display samples nor inferred nearest contacts participate.
use super::*;
use crate::constraint_sketch::ConstraintKind;

struct Round {
    center_id: EntityId,
    center: Position,
    radius: f64,
    start_angle: f64,
    sweep: f64,
    clockwise: bool,
    endpoints: Option<[EntityId; 2]>,
}
impl Round {
    fn read(sketch: &Sketch, target: &Curve, tolerance: f64) -> Result<Self, TrimError> {
        let (center_id, radius, start_angle, sweep, clockwise, endpoints) = match target.kind {
            CurveKind::Circle { center, radius_mm } => (center, radius_mm, 0., TAU, false, None),
            CurveKind::Arc {
                center,
                start,
                end,
                clockwise,
            } => {
                validate_boundary(sketch, target, tolerance)?;
                let c = point(sketch, center);
                let a = sub(point(sketch, start), c);
                let b = sub(point(sketch, end), c);
                let first = a[1].atan2(a[0]);
                let last = b[1].atan2(b[0]);
                (
                    center,
                    length(a),
                    first,
                    (if clockwise {
                        first - last
                    } else {
                        last - first
                    })
                    .rem_euclid(TAU),
                    clockwise,
                    Some([start, end]),
                )
            }
            _ => {
                return Err(fail(
                    TrimErrorKind::UnsupportedTarget,
                    "Choose a circular arc or circle",
                    [target.id],
                ));
            }
        };
        if !radius.is_finite()
            || radius <= tolerance
            || radius > 1e9
            || !sweep.is_finite()
            || sweep * radius <= tolerance
        {
            return Err(fail(
                TrimErrorKind::InvalidGeometry,
                "Target circular geometry is degenerate or outside bounds",
                [target.id],
            ));
        }
        Ok(Self {
            center_id,
            center: point(sketch, center_id),
            radius,
            start_angle,
            sweep,
            clockwise,
            endpoints,
        })
    }
    fn point(&self, t: f64) -> Position {
        let angle = self.start_angle + if self.clockwise { -t } else { t };
        [
            self.center[0] + self.radius * angle.cos(),
            self.center[1] + self.radius * angle.sin(),
        ]
    }
    fn parameter(&self, p: Position) -> f64 {
        let d = sub(p, self.center);
        let a = d[1].atan2(d[0]);
        (if self.clockwise {
            self.start_angle - a
        } else {
            a - self.start_angle
        })
        .rem_euclid(TAU)
    }
    fn selected_parameter(&self, request: &TrimRequest, t: f64) -> Option<f64> {
        match request {
            TrimRequest::TrimCurve(_) => {
                if self.endpoints.is_none() || t <= self.sweep {
                    Some(t)
                } else {
                    None
                }
            }
            TrimRequest::ExtendArc(r) => {
                if t > self.sweep {
                    Some(match r.endpoint {
                        Endpoint::Start => t - TAU,
                        Endpoint::End => t,
                    })
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}
fn boundary_accept(
    sketch: &Sketch,
    curve: &Curve,
    p: Position,
    tolerance: f64,
) -> Result<bool, TrimError> {
    match curve.kind {
        CurveKind::Line { start, end } => {
            let a = point(sketch, start);
            let b = point(sketch, end);
            let d = sub(b, a);
            let size = length(d);
            if size <= tolerance {
                return Err(fail(
                    TrimErrorKind::InvalidGeometry,
                    "Boundary line is degenerate",
                    [curve.id],
                ));
            }
            let u = dot(sub(p, a), d) / (size * size);
            if !(0. ..=1.).contains(&u) {
                if u.abs().min((u - 1.).abs()) * size <= tolerance {
                    return Err(fail(
                        TrimErrorKind::AmbiguousIntersection,
                        "Circular contact lies indistinguishably close to a boundary endpoint",
                        [curve.id],
                    ));
                }
                return Ok(false);
            }
            if u != 0. && u != 1. && u.min(1. - u) * size <= tolerance {
                return Err(fail(
                    TrimErrorKind::AmbiguousIntersection,
                    "Circular contact is too close to a boundary endpoint",
                    [curve.id],
                ));
            }
            Ok(true)
        }
        CurveKind::Arc { .. } => boundary_contains(sketch, curve, p, tolerance),
        CurveKind::Circle { .. } => Ok(true),
        CurveKind::BSpline { .. } => Err(fail(
            TrimErrorKind::UnsupportedBoundary,
            "Spline trim boundaries require a dedicated exact curve editing operation",
            [curve.id],
        )),
    }
}
struct Intersection {
    point: Position,
    tangent: bool,
}
fn add_contact(
    out: &mut Vec<Contact>,
    round: &Round,
    sketch: &Sketch,
    curve: &Curve,
    request: &TrimRequest,
    intersection: Intersection,
    tolerance: f64,
) -> Result<(), TrimError> {
    let Intersection { point: p, tangent } = intersection;
    if p.iter().any(|v| !v.is_finite() || v.abs() > 1e9) {
        return Err(fail(
            TrimErrorKind::InvalidGeometry,
            "Circular intersection exceeds finite modeling bounds",
            [curve.id, request.target()],
        ));
    }
    let Some(t) = round.selected_parameter(request, round.parameter(p)) else {
        return Ok(());
    };
    if !boundary_accept(sketch, curve, p, tolerance)? {
        return Ok(());
    }
    if round.endpoints.is_some() {
        if t == 0. || t == round.sweep {
            return Ok(());
        }
        if t.abs().min((t - round.sweep).abs()) * round.radius <= tolerance {
            return Err(fail(
                TrimErrorKind::AmbiguousIntersection,
                "Circular contact is too close to a target endpoint",
                [curve.id, request.target()],
            ));
        }
    }
    if tangent {
        return Err(fail(
            TrimErrorKind::AmbiguousIntersection,
            "Tangent or near-tangent circular contact does not define an unambiguous cut",
            [curve.id, request.target()],
        ));
    }
    let residual = match curve.kind {
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
        CurveKind::BSpline { .. } => unreachable!("boundary type checked"),
    };
    if residual > tolerance || (length(sub(p, round.center)) - round.radius).abs() > tolerance {
        return Err(fail(
            TrimErrorKind::AmbiguousIntersection,
            "Circular intersection residual exceeds the geometry tolerance",
            [curve.id, request.target()],
        ));
    }
    out.push(Contact {
        point_mm: p,
        parameter: t,
        boundaries: BTreeSet::from([curve.id]),
    });
    Ok(())
}
fn contacts(
    round: &Round,
    sketch: &Sketch,
    request: &TrimRequest,
    tolerance: f64,
) -> Result<Vec<Contact>, TrimError> {
    let mut result = vec![];
    for id in request.boundaries() {
        let curve = sketch.curves.iter().find(|c| c.id == *id).ok_or_else(|| {
            fail(
                TrimErrorKind::InvalidRequest,
                "Boundary is not a current authored curve",
                [*id],
            )
        })?;
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
                let d = sub(point(sketch, end), a);
                let size = length(d);
                if size <= tolerance {
                    return Err(fail(
                        TrimErrorKind::InvalidGeometry,
                        "Boundary line is degenerate",
                        [*id],
                    ));
                }
                let unit = d.map(|v| v / size);
                let delta = sub(round.center, a);
                let projection = dot(delta, unit);
                let distance = cross(delta, unit).abs();
                if distance > round.radius + tolerance {
                    continue;
                }
                if (distance - round.radius).abs() <= tolerance {
                    add_contact(
                        &mut result,
                        round,
                        sketch,
                        curve,
                        request,
                        Intersection {
                            point: along(a, unit, projection),
                            tangent: true,
                        },
                        tolerance,
                    )?;
                    continue;
                }
                let h2 = (round.radius - distance) * (round.radius + distance);
                if h2 < 0. {
                    continue;
                }
                for sign in [-1., 1.] {
                    add_contact(
                        &mut result,
                        round,
                        sketch,
                        curve,
                        request,
                        Intersection {
                            point: along(a, unit, projection + sign * h2.sqrt()),
                            tangent: false,
                        },
                        tolerance,
                    )?;
                }
            }
            CurveKind::Circle { center, .. } | CurveKind::Arc { center, .. } => {
                let c = point(sketch, center);
                let radius = match curve.kind {
                    CurveKind::Circle { radius_mm, .. } => radius_mm,
                    CurveKind::Arc { start, .. } => length(sub(point(sketch, start), c)),
                    _ => unreachable!(),
                };
                if !radius.is_finite() || radius <= tolerance {
                    return Err(fail(
                        TrimErrorKind::InvalidGeometry,
                        "Boundary circle is degenerate",
                        [*id],
                    ));
                }
                let delta = sub(c, round.center);
                let distance = length(delta);
                if distance <= tolerance && (radius - round.radius).abs() <= tolerance {
                    return Err(fail(
                        TrimErrorKind::Overlap,
                        "Co-circular boundaries cannot define a unique cut",
                        [request.target(), *id],
                    ));
                }
                if distance <= tolerance
                    || distance > round.radius + radius + tolerance
                    || distance < (round.radius - radius).abs() - tolerance
                {
                    continue;
                }
                let unit = delta.map(|v| v / distance);
                let x = (round.radius * round.radius - radius * radius + distance * distance)
                    / (2. * distance);
                let middle = along(round.center, unit, x);
                let tangent = (distance - (round.radius + radius)).abs() <= tolerance
                    || (distance - (round.radius - radius).abs()).abs() <= tolerance;
                if tangent {
                    add_contact(
                        &mut result,
                        round,
                        sketch,
                        curve,
                        request,
                        Intersection {
                            point: middle,
                            tangent: true,
                        },
                        tolerance,
                    )?;
                    continue;
                }
                let h2 = (round.radius - x) * (round.radius + x);
                if h2 < 0. {
                    return Err(fail(
                        TrimErrorKind::AmbiguousIntersection,
                        "Circular intersection could not be separated from tangency",
                        [request.target(), *id],
                    ));
                }
                for sign in [-1., 1.] {
                    let h = sign * h2.sqrt();
                    let p = [middle[0] - unit[1] * h, middle[1] + unit[0] * h];
                    add_contact(
                        &mut result,
                        round,
                        sketch,
                        curve,
                        request,
                        Intersection {
                            point: p,
                            tangent: false,
                        },
                        tolerance,
                    )?;
                }
            }
        }
    }
    result.sort_by(|a, b| a.parameter.total_cmp(&b.parameter));
    for pair in result.windows(2) {
        if (pair[1].parameter - pair[0].parameter) * round.radius <= tolerance {
            return Err(fail(
                TrimErrorKind::AmbiguousIntersection,
                "Multiple circular contacts coincide or are too close; choose one boundary",
                pair[0].boundaries.union(&pair[1].boundaries).copied(),
            ));
        }
    }
    if round.endpoints.is_none()
        && result.len() > 1
        && (TAU - result.last().expect("contact").parameter + result[0].parameter) * round.radius
            <= tolerance
    {
        return Err(fail(
            TrimErrorKind::AmbiguousIntersection,
            "Circular contacts coincide across the parameter seam",
            result.iter().flat_map(|c| c.boundaries.iter().copied()),
        ));
    }
    Ok(result)
}
fn spans(
    round: &Round,
    request: &TrimRequest,
    contacts: &[Contact],
    tolerance: f64,
) -> Result<(Vec<[f64; 2]>, Action), TrimError> {
    match request {
        TrimRequest::TrimCurve(r) => {
            if r.pick_mm.iter().any(|v| !v.is_finite() || v.abs() > 1e9)
                || (length(sub(r.pick_mm, round.center)) - round.radius).abs() > tolerance
            {
                return Err(fail(
                    TrimErrorKind::InvalidPick,
                    "Pick must lie on the current circular curve",
                    [r.target],
                ));
            }
            let t = round.parameter(r.pick_mm);
            if round.endpoints.is_some()
                && (t > round.sweep || t.min(round.sweep - t) * round.radius <= tolerance)
            {
                return Err(fail(
                    TrimErrorKind::InvalidPick,
                    "Pick must lie inside the actual arc, away from its endpoints",
                    [r.target],
                ));
            }
            if contacts.iter().any(|c| {
                let delta = (c.parameter - t).abs();
                delta.min(TAU - delta) * round.radius <= tolerance
            }) {
                return Err(fail(
                    TrimErrorKind::InvalidPick,
                    "Pick is too close to a cut; choose an open circular span",
                    [r.target],
                ));
            }
            let (low, high, spans) = if round.endpoints.is_none() {
                if contacts.len() < 2 {
                    return Err(fail(
                        TrimErrorKind::NoIntersection,
                        "Trimming a circle requires at least two distinct boundary intersections",
                        [r.target],
                    ));
                }
                let low = contacts
                    .iter()
                    .rfind(|c| c.parameter < t)
                    .unwrap_or(contacts.last().expect("contacts"))
                    .parameter;
                let high = contacts
                    .iter()
                    .find(|c| c.parameter > t)
                    .unwrap_or(&contacts[0])
                    .parameter;
                let removed_high = if high > low { high } else { high + TAU };
                let retained_high = if low > high { low } else { low + TAU };
                (low, removed_high, vec![[high, retained_high]])
            } else {
                let low = contacts
                    .iter()
                    .rfind(|c| c.parameter < t)
                    .map_or(0., |c| c.parameter);
                let high = contacts
                    .iter()
                    .find(|c| c.parameter > t)
                    .map_or(round.sweep, |c| c.parameter);
                if low == 0. && high == round.sweep {
                    return Err(fail(
                        TrimErrorKind::NoIntersection,
                        "No interior boundary defines an arc trim span",
                        [r.target],
                    ));
                }
                let mut spans = vec![];
                if low > 0. {
                    spans.push([0., low]);
                }
                if high < round.sweep {
                    spans.push([high, round.sweep]);
                }
                (low, high, spans)
            };
            Ok((
                spans,
                Action::TrimArc {
                    removed_mm: [round.point(low), round.point(high)],
                    center_mm: round.center,
                    radius_mm: round.radius,
                    clockwise: round.clockwise,
                    removed_sweep_rad: high - low,
                },
            ))
        }
        TrimRequest::ExtendArc(r) => {
            if round.endpoints.is_none() {
                return Err(fail(
                    TrimErrorKind::UnsupportedTarget,
                    "A closed circle has no endpoint to extend",
                    [r.target],
                ));
            }
            let contact = match r.endpoint {
                Endpoint::Start => contacts.last(),
                Endpoint::End => contacts.first(),
            }
            .ok_or_else(|| {
                fail(
                    TrimErrorKind::NoIntersection,
                    "No actual boundary lies beyond the selected arc endpoint",
                    [r.target],
                )
            })?;
            let (span, from, added) = match r.endpoint {
                Endpoint::Start => (
                    [contact.parameter, round.sweep],
                    round.point(0.),
                    -contact.parameter,
                ),
                Endpoint::End => (
                    [0., contact.parameter],
                    round.point(round.sweep),
                    contact.parameter - round.sweep,
                ),
            };
            Ok((
                vec![span],
                Action::ExtendArc {
                    endpoint: r.endpoint,
                    from_mm: from,
                    to_mm: contact.point_mm,
                    center_mm: round.center,
                    radius_mm: round.radius,
                    clockwise: round.clockwise,
                    added_sweep_rad: added,
                },
            ))
        }
        _ => Err(fail(
            TrimErrorKind::UnsupportedTarget,
            "Use the circular trim or arc extension request",
            [request.target()],
        )),
    }
}
pub(super) fn draft(
    source: &Source,
    solved: &Sketch,
    expected: GeometryStamp,
    hash: ContentHash,
    target: &Curve,
    request: &TrimRequest,
    tolerance: f64,
) -> Result<TrimDraft, TrimError> {
    let evaluated = solved
        .curves
        .iter()
        .find(|c| c.id == target.id)
        .expect("checked structure");
    let round = Round::read(solved, evaluated, tolerance)?;
    if round.endpoints.is_none() && matches!(request, TrimRequest::ExtendArc(_)) {
        return Err(fail(
            TrimErrorKind::UnsupportedTarget,
            "A closed circle has no endpoint to extend",
            [target.id],
        ));
    }
    let contacts = contacts(&round, solved, request, tolerance)?;
    let (spans, action) = spans(&round, request, &contacts, tolerance)?;
    for [low, high] in &spans {
        if (high - low) * round.radius <= tolerance
            || *high - *low >= TAU
            || length(sub(round.point(*high), round.point(*low))) <= tolerance
        {
            return Err(fail(
                TrimErrorKind::AmbiguousIntersection,
                "Result would be a zero, tolerance-sized or full-turn arc",
                [target.id],
            ));
        }
    }
    let split = spans.len() > 1;
    let detached: BTreeSet<_> = round
        .endpoints
        .into_iter()
        .flat_map(|ids| [(ids[0], 0.), (ids[1], round.sweep)])
        .filter(|(_, t)| !spans.iter().any(|s| s.contains(t)))
        .map(|(id, _)| id)
        .collect();
    let mut conflicting = BTreeSet::new();
    for constraint in &source.sketch.constraints {
        let refs = constraint.kind.references();
        let curve = refs.contains(&target.id);
        let radius_compatible = !split
            && matches!(constraint.kind,ConstraintKind::Radius{curve,..}|ConstraintKind::Diameter{curve,..} if curve==target.id);
        if refs.iter().any(|id| detached.contains(id)) || (curve && !radius_compatible) {
            conflicting.insert(constraint.id);
        }
    }
    if !conflicting.is_empty() {
        return Err(fail(
            TrimErrorKind::ConstraintConflict,
            "Circular edit would change an incompatible driving/reference constraint or detach a constrained endpoint; constraints were preserved",
            conflicting
                .into_iter()
                .chain(std::iter::once(target.id))
                .chain(detached.iter().copied()),
        ));
    }
    let mut candidate = source.clone();
    let mut seed = solved.clone();
    let mut created_points = BTreeSet::new();
    let mut created_curves = BTreeSet::new();
    let mut replacements = vec![];
    let mut curves = vec![];
    for [low, high] in spans {
        let mut ids = [round.center_id; 2];
        for (index, t) in [low, high].into_iter().enumerate() {
            let preserved = round.endpoints.and_then(|ids| {
                if index == 0 && t == 0. {
                    Some(ids[0])
                } else if index == 1 && t == round.sweep {
                    Some(ids[1])
                } else {
                    None
                }
            });
            ids[index] = if let Some(id) = preserved {
                id
            } else {
                let id = EntityId::new();
                // These parameters come from identified cuts, including a single
                // explicit turn offset. Preserve the computed intersection point
                // rather than regenerating it through atan2/sin/cos roundoff.
                let contact = contacts
                    .iter()
                    .find(|c| c.parameter == t || c.parameter + TAU == t || c.parameter - TAU == t)
                    .ok_or_else(|| {
                        fail(
                            TrimErrorKind::InvalidGeometry,
                            "Circular endpoint has no identified cut contact",
                            [target.id],
                        )
                    })?;
                let p = Point {
                    id,
                    position_mm: contact.point_mm,
                };
                candidate.sketch.points.push(p.clone());
                seed.points.push(p);
                created_points.insert(id);
                id
            };
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
            kind: CurveKind::Arc {
                center: round.center_id,
                start: ids[0],
                end: ids[1],
                clockwise: round.clockwise,
            },
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
            parameterization: Parameterization::ArcRadians,
            type_changes: if round.endpoints.is_none() {
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
