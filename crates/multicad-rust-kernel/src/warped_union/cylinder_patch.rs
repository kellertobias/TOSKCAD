//! Check the angular and axial chart of a bounded circular cylinder patch.
use super::*;

pub(super) const ANGULAR: f64 = 64. * f64::EPSILON;

pub(super) fn axis_offset(point: Point, origin: Point, axis: Point) -> Point {
    let delta = sub(point, origin);
    let axial = dot(delta, axis);
    std::array::from_fn(|i| (-axis[i]).mul_add(axial, delta[i]))
}

pub(super) struct CylinderPatch {
    pub(super) arcs: [(usize, EllipseArc); 2],
    pub(super) rails: [usize; 2],
}

impl CylinderPatch {
    pub(super) fn new(
        shell: &Shell,
        face: &Face,
        origin: Point,
        axis: Point,
        radius: f64,
        tolerance: f64,
    ) -> Result<Self, Error> {
        if !face.holes.is_empty() {
            return Err(Error::Unsupported);
        }
        let [a, b_rail, c, d] = face.boundary.as_slice() else {
            return Err(Error::Unsupported);
        };
        let arcs = [a.edge, c.edge].map(|edge| {
            let Curve::Ellipse(arc) = shell.edges[edge].curve else {
                return Err(Error::Unsupported);
            };
            let ellipse = arc.ellipse();
            let [major, minor] = ellipse.semiaxes_mm();
            if length3(axis_offset(ellipse.center_mm(), origin, axis)) > tolerance
                || (minor - radius).abs() > tolerance
                || arc.sweep_rad().abs() > FRAC_PI_2 + ANGULAR
            {
                return Err(Error::Unsupported);
            }
            if tilted(arc, axis) {
                let cosine = dot(ellipse.normal(), axis).abs();
                // A tilted ellipse must project along the cylinder axis
                // onto the supporting circle, at the same radial phase.
                if shell.matched_quarter_rims.is_none()
                    || cosine <= ANGULAR
                    || (major * cosine - radius).abs() > tolerance
                    || dot(ellipse.minor_axis(), axis).abs() > ANGULAR
                {
                    return Err(Error::Unsupported);
                }
            } else if major != minor {
                return Err(Error::Unsupported);
            }
            Ok((edge, arc))
        });
        let [a, b] = arcs;
        let mut arcs = [a?, b?];
        if tilted(arcs[0].1, axis) {
            arcs.swap(0, 1);
            if tilted(arcs[0].1, axis) {
                return Err(Error::Unsupported);
            }
        }
        let rails = [b_rail.edge, d.edge];
        // Each rail connects matching angular endpoints on the two trims.
        // Their short sweeps uniquely bound the wall between those generators.
        for edge in rails.map(|edge| shell.edges[edge]) {
            let [a, b] = edge.vertices.map(|i| shell.vertices[i]);
            let span = sub(b, a);
            if edge.curve != Curve::Line
                || length3(cross(span, axis)) > tolerance
                || dot(span, axis).abs() <= tolerance
            {
                return Err(Error::Unsupported);
            }
        }
        Ok(Self { arcs, rails })
    }
}

pub(super) fn tilted(arc: EllipseArc, axis: Point) -> bool {
    length3(cross(arc.ellipse().normal(), axis)) > ANGULAR
}
