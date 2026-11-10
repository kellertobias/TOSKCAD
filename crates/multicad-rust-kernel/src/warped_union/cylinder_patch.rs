//! Check the rectangular angular/axial chart of a circular cylinder patch.
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
            if length3(cross(ellipse.normal(), axis)) > ANGULAR
                || length3(axis_offset(ellipse.center_mm(), origin, axis)) > tolerance
                || major != minor
                || (major - radius).abs() > tolerance
                || arc.sweep_rad().abs() > FRAC_PI_2 + ANGULAR
            {
                return Err(Error::Unsupported);
            }
            Ok((edge, arc))
        });
        let [a, b] = arcs;
        let arcs = [a?, b?];
        let rails = [b_rail.edge, d.edge];
        // The existing closed four-edge ring connects each circular endpoint
        // to the matching radial endpoint at the other cap. The short arc
        // between the pair is unique, so the wall is a rectangular chart.
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
