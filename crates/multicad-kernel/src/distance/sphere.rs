//! Closed sphere and complete spherical face to a finite B-rep point.
use super::*;
use multicad_core::references::{AnalyticGeometry, TopologyKind};

pub(super) fn measure(
    a: &Solid,
    sa: BRepSelection,
    b: &Solid,
    sb: BRepSelection,
    tolerance: f64,
) -> Option<Result<MinimumDistance, GeometryError>> {
    let (solid, selection, probe, index, reverse) = match (sa, sb) {
        (BRepSelection::Whole | BRepSelection::Face(_), BRepSelection::Vertex(i)) => {
            (a, sa, b, i, false)
        }
        (BRepSelection::Vertex(i), BRepSelection::Whole | BRepSelection::Face(_)) => {
            (b, sb, a, i, true)
        }
        _ => return None,
    };
    let (body, frame) = solid.sphere_body()?;
    Some((|| {
        if matches!(selection, BRepSelection::Face(i) if i != 0) {
            return Err(error(
                GeometryErrorKind::Input,
                "Selected spherical face is missing",
            ));
        }
        let topology = probe.rust_topology().ok_or_else(|| {
            error(
                GeometryErrorKind::Unsupported,
                "Point selection has no exact Rust topology",
            )
        })??;
        let point = topology
            .elements
            .iter()
            .find(|e| e.key.kind == TopologyKind::Vertex && e.key.ordinal == index)
            .and_then(|e| match e.geometry {
                Some(AnalyticGeometry::Point { position_mm }) => Some(position_mm),
                _ => None,
            })
            .ok_or_else(|| {
                error(
                    GeometryErrorKind::Input,
                    "Selected Rust B-rep vertex is missing",
                )
            })?;
        let sphere = body.geometry();
        let bounds = probe.bounds()?;
        let magnitude = point
            .into_iter()
            .chain(frame.origin)
            .chain(bounds.min)
            .chain(bounds.max)
            .map(f64::abs)
            .fold(sphere.radius_mm().max(1.), f64::max);
        let uncertainty = 64. * f64::EPSILON * magnitude;
        if !uncertainty.is_finite() || uncertainty > tolerance / 8. {
            return Err(error(
                GeometryErrorKind::Unsupported,
                "Sphere distance placement exceeds the numerical budget",
            ));
        }
        let local = std::array::from_fn(|i| point[i] - frame.origin[i]);
        let surface = matches!(selection, BRepSelection::Face(_));
        let projection = if surface {
            sphere.closest_surface_point(local)
        } else {
            sphere.closest_point(local)
        }
        .ok_or_else(|| {
            error(
                GeometryErrorKind::InvalidResult,
                "Sphere point projection is not finite",
            )
        })?;
        let closest = if !surface && projection.distance_mm == 0. {
            point
        } else {
            std::array::from_fn(|i| frame.origin[i] + projection.point_mm[i])
        };
        let delta: [f64; 3] = std::array::from_fn(|i| point[i] - closest[i]);
        let distance_mm = delta[0].hypot(delta[1]).hypot(delta[2]);
        if !distance_mm.is_finite() || (distance_mm - projection.distance_mm).abs() > tolerance / 4.
        {
            return Err(error(
                GeometryErrorKind::Unsupported,
                "Sphere distance witness exceeds the numerical budget",
            ));
        }
        let radial = local[0].hypot(local[1]).hypot(local[2]);
        let unique = projection.unique && (!surface || radial > uncertainty);
        Ok(MinimumDistance {
            distance_mm,
            witnesses: vec![if reverse {
                DistanceWitness {
                    a_mm: point,
                    b_mm: closest,
                }
            } else {
                DistanceWitness {
                    a_mm: closest,
                    b_mm: point,
                }
            }],
            witnesses_complete: unique,
            total_solutions: unique.then_some(1),
            inner_solution: !surface
                && projection.inside
                && sphere.radius_mm() - radial > uncertainty,
        })
    })())
}
