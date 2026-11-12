//! Complete coaxial circular edges, with a bounded representative witness.
use super::*;
use multicad_core::references::{AnalyticGeometry, ReferenceFrame, TopologyKind};
type Point = [f64; 3];
struct Ring {
    frame: ReferenceFrame,
    radius: f64,
}
fn dot(a: Point, b: Point) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}
fn norm(a: Point) -> f64 {
    a[0].hypot(a[1]).hypot(a[2])
}
fn sub(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] - b[i])
}
fn cross(a: Point, b: Point) -> Point {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn unit(a: Point) -> Result<Point, GeometryError> {
    let n = norm(a);
    if !n.is_finite() || n <= 0. {
        return Err(unproved());
    }
    Ok(a.map(|x| x / n))
}
fn unproved() -> GeometryError {
    error(
        GeometryErrorKind::Unsupported,
        "Circular edge distance requires complete coaxial circles within the numerical budget",
    )
}
fn ring(solid: &Solid, index: u32) -> Result<Option<Ring>, GeometryError> {
    let topology = solid.rust_topology().ok_or_else(unproved)??;
    let e = topology
        .elements
        .into_iter()
        .find(|e| e.key.kind == TopologyKind::Edge && e.key.ordinal == index)
        .ok_or_else(|| {
            error(
                GeometryErrorKind::Input,
                "Selected Rust B-rep edge is missing",
            )
        })?;
    Ok(match e.geometry {
        Some(AnalyticGeometry::CircularArc {
            frame,
            radius_mm,
            sweep_rad,
            ..
        }) if sweep_rad.abs() == std::f64::consts::TAU => Some(Ring {
            frame,
            radius: radius_mm,
        }),
        _ => None,
    })
}
pub(super) fn measure(
    a: &Solid,
    sa: BRepSelection,
    b: &Solid,
    sb: BRepSelection,
    tolerance: f64,
) -> Option<Result<MinimumDistance, GeometryError>> {
    let (BRepSelection::Edge(ai), BRepSelection::Edge(bi)) = (sa, sb) else {
        return None;
    };
    let rings = match (ring(a, ai), ring(b, bi)) {
        (Ok(Some(a)), Ok(Some(b))) => (a, b),
        (Err(e), _) | (_, Err(e)) => return Some(Err(e)),
        _ => return None,
    };
    Some(solve(rings.0, rings.1, tolerance))
}
fn solve(a: Ring, b: Ring, tolerance: f64) -> Result<MinimumDistance, GeometryError> {
    let basis = |ring: &Ring| -> Result<(Point, Point, f64), GeometryError> {
        let u = unit(ring.frame.u)?;
        let n = unit(cross(ring.frame.u, ring.frame.v))?;
        let v = cross(n, u);
        let drift = (norm(sub(ring.frame.u, u)) + norm(sub(ring.frame.v, v))) * ring.radius;
        Ok((u, n, drift))
    };
    let (u, na, da) = basis(&a)?;
    let (_, nb, db) = basis(&b)?;
    let magnitude = a
        .frame
        .origin_mm
        .into_iter()
        .chain(b.frame.origin_mm)
        .map(f64::abs)
        .fold(a.radius.max(b.radius).max(1.), f64::max);
    let arithmetic = 128. * f64::EPSILON * magnitude;
    let axis_error = norm(cross(na, nb)) * a.radius.max(b.radius);
    let delta = sub(b.frame.origin_mm, a.frame.origin_mm);
    let dz = dot(delta, na);
    let lateral = norm(std::array::from_fn(|i| delta[i] - na[i] * dz));
    if !arithmetic.is_finite()
        || !da.is_finite()
        || !db.is_finite()
        || da + db + arithmetic > tolerance / 32.
        || axis_error > tolerance / 64.
        || lateral > tolerance / 64.
    {
        return Err(unproved());
    }
    let local = multicad_rust_kernel::circular_distance::coaxial_circles(a.radius, b.radius, dz)
        .ok_or_else(unproved)?;
    let projected = unit(std::array::from_fn(|i| u[i] - nb[i] * dot(u, nb)))?;
    let pa = std::array::from_fn(|i| a.frame.origin_mm[i] + u[i] * a.radius);
    let pb = std::array::from_fn(|i| b.frame.origin_mm[i] + projected[i] * b.radius);
    let actual = norm(sub(pa, pb));
    if !actual.is_finite() || (actual - local.distance_mm).abs() > tolerance / 4. {
        return Err(unproved());
    }
    Ok(MinimumDistance {
        distance_mm: actual,
        witnesses: vec![DistanceWitness { a_mm: pa, b_mm: pb }],
        witnesses_complete: false,
        total_solutions: None,
        inner_solution: false,
    })
}
