use super::*;
use multicad_core::references::{AnalyticGeometry, TopologyKind};
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
    let (body, frame) = solid.rounded_cylinder_body()?;
    Some((|| {
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
        let (closest, distance_mm, inner_solution) =
            body.point_projection(frame, point, tolerance)?;
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
            witnesses_complete: true,
            total_solutions: 1,
            inner_solution,
        })
    })())
}
