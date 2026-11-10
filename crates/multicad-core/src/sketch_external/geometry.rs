use super::{
    Binding, Method,
    source::{install, invalid, locks},
};
use crate::{
    constraint_sketch::Sketch,
    document::{Document, DocumentError, EntityId},
    references::{AnalyticGeometry, ReferenceAddress, ReferenceReport, Resolution},
    sketch::{Frame, dot},
};
use std::collections::BTreeSet;
pub const PLANE_TOLERANCE_MM: f64 = 1e-7;
#[derive(Clone, Debug, PartialEq)]
pub enum LocalGeometry {
    Point([f64; 2]),
    Line([f64; 2], [f64; 2]),
    Circle {
        center: [f64; 2],
        radius: f64,
    },
    Arc {
        center: [f64; 2],
        start: [f64; 2],
        end: [f64; 2],
        clockwise: bool,
    },
}
pub fn project(
    geometry: AnalyticGeometry,
    frame: Frame,
    method: Method,
) -> Result<LocalGeometry, DocumentError> {
    geometry.validate().map_err(invalid)?;
    let height = |p: [f64; 3]| dot(std::array::from_fn(|i| p[i] - frame.origin[i]), frame.n);
    let coplanar = |p: [f64; 3]| {
        if height(p).abs() <= PLANE_TOLERANCE_MM {
            Ok(frame.coordinates(p))
        } else {
            Err(invalid(
                "Selected geometry is not coplanar with this sketch",
            ))
        }
    };
    if method == Method::Intersect {
        return match geometry {
            AnalyticGeometry::Line { a_mm, b_mm } => {
                let a = height(a_mm);
                let b = height(b_mm);
                if a.abs() <= PLANE_TOLERANCE_MM && b.abs() <= PLANE_TOLERANCE_MM {
                    return Err(invalid(
                        "The edge lies in the sketch plane; choose coplanar projection instead of an ambiguous intersection",
                    ));
                }
                let delta = b - a;
                if delta.abs() < 1e-12 {
                    return Err(invalid("Finite edge does not cross the sketch plane"));
                }
                let t = -a / delta;
                if !(-1e-9..=1. + 1e-9).contains(&t) {
                    return Err(invalid("The finite edge does not reach this sketch plane"));
                }
                Ok(LocalGeometry::Point(frame.coordinates(
                    std::array::from_fn(|i| a_mm[i] + (b_mm[i] - a_mm[i]) * t.clamp(0., 1.)),
                )))
            }
            _ => Err(invalid(
                "Intersection supports finite straight edges crossing the plane. Face extents and multiple circular/spline intersections require an explicit supported selector; no unbounded line or nearest-point guess is generated",
            )),
        };
    }
    match geometry {
        AnalyticGeometry::Point { position_mm } => Ok(LocalGeometry::Point(coplanar(position_mm)?)),
        AnalyticGeometry::Line { a_mm, b_mm } => {
            Ok(LocalGeometry::Line(coplanar(a_mm)?, coplanar(b_mm)?))
        }
        AnalyticGeometry::CircularArc {
            frame: source,
            radius_mm,
            start_rad,
            sweep_rad,
        } => {
            let center = coplanar(source.origin_mm)?;
            let alignment = dot(source.normal(), frame.n);
            if (alignment.abs() - 1.).abs() > 1e-9 {
                return Err(invalid(
                    "Selected circular edge is not coplanar with this sketch",
                ));
            }
            if (sweep_rad.abs() - std::f64::consts::TAU).abs() <= 1e-9 {
                return Ok(LocalGeometry::Circle {
                    center,
                    radius: radius_mm,
                });
            }
            let at = |angle: f64| {
                frame.coordinates(std::array::from_fn(|i| {
                    source.origin_mm[i]
                        + radius_mm * (source.u[i] * angle.cos() + source.v[i] * angle.sin())
                }))
            };
            Ok(LocalGeometry::Arc {
                center,
                start: at(start_rad),
                end: at(start_rad + sweep_rad),
                clockwise: sweep_rad * alignment < 0.,
            })
        }
        AnalyticGeometry::EllipticArc { .. } => Err(invalid(
            "Sketch external geometry currently requires straight or circular edges",
        )),
        AnalyticGeometry::Plane { .. } => Err(invalid(
            "Select an exact point or finite analytic edge. A face plane alone does not describe its finite boundary",
        )),
    }
}
pub fn resolved_geometry(
    document: &Document,
    revision: u64,
    binding: &Binding,
    frame: Frame,
    report: &ReferenceReport,
) -> Result<LocalGeometry, DocumentError> {
    if report.revision != revision {
        return Err(invalid("External reference report is stale"));
    }
    let diagnostic = report
        .get(ReferenceAddress {
            owner: binding.reference,
            reference: binding.intent.id,
        })
        .ok_or_else(|| invalid("External reference has no current exact evaluation"))?;
    if diagnostic.source != binding.source || diagnostic.intent != binding.intent {
        return Err(invalid(
            "External evaluation does not match the canonical source intent",
        ));
    }
    let hash = crate::references::dependency_hash(document, binding.source)?;
    if diagnostic.dependency_hash != Some(hash) {
        return Err(invalid(
            "External source changed after this geometry report; rebuild its native geometry",
        ));
    }
    let geometry = match diagnostic.resolution {
        Resolution::Resolved {
            geometry: Some(geometry),
            ..
        } => geometry,
        Resolution::Resolved { geometry: None, .. } => {
            return Err(invalid(
                "Native source does not provide an exact supported analytic geometry descriptor",
            ));
        }
        ref other => {
            return Err(invalid(format!(
                "External reference is unresolved: {other:?}"
            )));
        }
    };
    project(geometry, frame, binding.definition.method)
}
/// Overwrite numerical seeds only, then inject source-owned immutable geometry
/// equations. Returned IDs are transient constraints which must not be saved.
pub fn apply_to_solver(
    document: &Document,
    revision: u64,
    sketch: EntityId,
    frame: Frame,
    report: Option<&ReferenceReport>,
    solver: &mut Sketch,
) -> Result<BTreeSet<EntityId>, DocumentError> {
    let bindings = super::bindings(document, sketch)?;
    let occurrences = super::occurrence_bindings(document, sketch)?;
    if bindings.is_empty() && occurrences.is_empty() {
        return Ok(BTreeSet::new());
    }
    let report=report.ok_or_else(||invalid("External sketch geometry requires a current native reference report; authored guesses are unavailable"))?;
    validate_plane_report(document, sketch, report)?;
    let mut ids = BTreeSet::new();
    for binding in bindings {
        let shape = resolved_geometry(document, revision, &binding, frame, report)?;
        install(solver, &binding.definition, &shape, false)?;
        for constraint in locks(&binding.definition, &shape) {
            ids.insert(constraint.id);
            solver.constraints.push(constraint);
        }
    }
    ids.extend(super::occurrence_source::apply_occurrences(
        document, revision, sketch, frame, report, solver,
    )?);
    if solver.constraints.len() > 1024 {
        return Err(invalid(
            "External reference locks exceed the sketch solver constraint budget",
        ));
    }
    Ok(ids)
}
fn validate_plane_report(
    document: &Document,
    sketch: EntityId,
    report: &ReferenceReport,
) -> Result<(), DocumentError> {
    use crate::document::EntityData;
    let Some(EntityData::Sketch { plane, .. }) = document.get(sketch).map(|entity| &entity.data)
    else {
        return Err(invalid("Sketch is missing"));
    };
    let mut pending = Some(*plane);
    let mut visited = BTreeSet::new();
    while let Some(plane) = pending.take() {
        if !visited.insert(plane) || visited.len() > 256 {
            return Err(invalid(
                "External sketch plane dependency chain is cyclic or too deep",
            ));
        }
        let Some(EntityData::Plane {
            anchor: Some(link), ..
        }) = document.get(plane).map(|entity| &entity.data)
        else {
            continue;
        };
        if let Some(intent) = &link.geometry {
            let diagnostic = report
                .get(ReferenceAddress {
                    owner: plane,
                    reference: intent.id,
                })
                .ok_or_else(|| {
                    invalid("External sketch plane has no current exact reference report")
                })?;
            if diagnostic.source != link.target
                || diagnostic.intent != *intent
                || diagnostic.dependency_hash
                    != Some(crate::references::dependency_hash(document, link.target)?)
            {
                return Err(invalid(
                    "The attached sketch plane source changed after this report; rebuild its native geometry",
                ));
            }
        }
        if link.role == crate::sketch::OFFSET_PLANE_ROLE {
            pending = Some(link.target);
        }
    }
    Ok(())
}
