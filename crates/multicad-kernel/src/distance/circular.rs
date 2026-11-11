//! Closed finite cylinder to point projection, without display tessellation.
use super::*;
use multicad_core::references::{AnalyticGeometry, TopologyKind};

type Point = [f64; 3];
fn dot(a: Point, b: Point) -> f64 {
    (0..3).map(|i| a[i] * b[i]).sum()
}
fn norm(a: Point) -> f64 {
    a[0].hypot(a[1]).hypot(a[2])
}
fn sub(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] - b[i])
}

pub(super) fn measure(
    a: &Solid,
    sa: BRepSelection,
    b: &Solid,
    sb: BRepSelection,
    tolerance: f64,
) -> Option<Result<MinimumDistance, GeometryError>> {
    let (solid, probe, index, reverse) = match (sa, sb) {
        (BRepSelection::Whole, BRepSelection::Vertex(i)) => (a, b, i, false),
        (BRepSelection::Vertex(i), BRepSelection::Whole) => (b, a, i, true),
        _ => return None,
    };
    let (cylinder, frame) = solid.axial_cylinder_frame()?;
    Some((|| {
        let topology = probe.rust_topology().ok_or_else(|| {
            error(
                GeometryErrorKind::Unsupported,
                "Point selection has no exact Rust topology",
            )
        })??;
        let point = match topology
            .elements
            .iter()
            .find(|e| e.key.kind == TopologyKind::Vertex && e.key.ordinal == index)
            .and_then(|e| e.geometry)
        {
            Some(AnalyticGeometry::Point { position_mm }) => position_mm,
            _ => {
                return Err(error(
                    GeometryErrorKind::Input,
                    "Selected Rust B-rep vertex is missing",
                ));
            }
        };
        let unit = |v: Point| {
            let n = norm(v);
            v.map(|x| x / n)
        };
        let u = unit(frame.u);
        let n = unit(frame.normal());
        let v = [
            n[1] * u[2] - n[2] * u[1],
            n[2] * u[0] - n[0] * u[2],
            n[0] * u[1] - n[1] * u[0],
        ];
        let radius = cylinder.circle().radius().as_f64();
        let height = cylinder.height().as_f64();
        let magnitude = point
            .into_iter()
            .chain(frame.origin)
            .map(f64::abs)
            .fold(radius.max(height).max(1.), f64::max);
        let uncertainty = norm(sub(frame.u, u)) * radius
            + norm(sub(frame.v, v)) * radius
            + norm(sub(frame.normal(), n)) * height
            + 64. * f64::EPSILON * magnitude;
        if !uncertainty.is_finite() || uncertainty > tolerance / 8. {
            return Err(error(
                GeometryErrorKind::Unsupported,
                "Cylinder distance placement cannot be certified within model tolerance",
            ));
        }
        let delta = sub(point, frame.origin);
        let local = [dot(delta, u), dot(delta, v), dot(delta, n)];
        let projected = cylinder.closest_point_in_frame(local).map_err(|_| {
            error(
                GeometryErrorKind::InvalidResult,
                "Cylinder point projection is not finite",
            )
        })?;
        let closest = if projected.distance_mm == 0. {
            point
        } else {
            std::array::from_fn(|i| {
                frame.origin[i]
                    + u[i] * projected.point_mm[0]
                    + v[i] * projected.point_mm[1]
                    + n[i] * projected.point_mm[2]
            })
        };
        let distance = norm(sub(point, closest));
        if !distance.is_finite() || (distance - projected.distance_mm).abs() > tolerance / 4. {
            return Err(error(
                GeometryErrorKind::Unsupported,
                "Cylinder distance witness exceeds the numerical error budget",
            ));
        }
        let clearance = (radius - local[0].hypot(local[1]))
            .min(local[2])
            .min(height - local[2]);
        Ok(MinimumDistance {
            distance_mm: distance,
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
            witnesses_complete: true,
            total_solutions: Some(1),
            inner_solution: projected.strictly_inside && clearance > uncertainty,
        })
    })())
}
