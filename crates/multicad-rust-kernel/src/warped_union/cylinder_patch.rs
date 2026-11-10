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
        // An imported mixed rim retains its partition. Roundoff when the
        // saved endpoints are placed back in the cap chart can put a quarter
        // just past 90 degrees. Admit only a small, linear-tolerance excess.
        let quarter_slack = if shell
            .matched_quarter_rims
            .is_some_and(|rims| !rims.circular)
        {
            ANGULAR.max((tolerance / radius).min(1e-6))
        } else {
            ANGULAR
        };
        let arcs = [a.edge, c.edge].map(|edge| {
            let Curve::Ellipse(arc) = shell.edges[edge].curve else {
                return Err(Error::Unsupported);
            };
            let ellipse = arc.ellipse();
            let [major, minor] = ellipse.semiaxes_mm();
            if length3(axis_offset(ellipse.center_mm(), origin, axis)) > tolerance
                || (minor - radius).abs() > tolerance
                || arc.sweep_rad().abs() > FRAC_PI_2 + quarter_slack
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
                    || shell.edges[edge]
                        .vertices
                        .into_iter()
                        .zip(arc.endpoints())
                        .any(|(id, point)| length3(sub(shell.vertices[id], point)) > tolerance)
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
        if tilted(arcs[0].1, axis) && !tilted(arcs[1].1, axis) {
            arcs.swap(0, 1);
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

    /// Radial phase is the phase of the first rim projected along the axis
    /// onto the perpendicular disk. Its major and minor vectors project to
    /// perpendicular vectors of equal length, so the parameter is unchanged.
    pub(super) fn radial_chart(&self, axis: Point) -> Result<EllipseArc, Error> {
        let (_, arc) = self.arcs[0];
        if !tilted(arc, axis) {
            return Ok(arc);
        }
        let rim = arc.ellipse();
        let alignment = dot(rim.normal(), axis);
        let normal = axis.map(|value| value * alignment.signum());
        let major = axis_offset(rim.major_axis(), [0.; 3], axis);
        let magnitude = length3(major);
        let radius = rim.semiaxes_mm()[1];
        Ellipse::new(
            rim.center_mm(),
            normal,
            major.map(|value| value / magnitude),
            [radius; 2],
        )
        .and_then(|disk| EllipseArc::new(disk, arc.start_rad(), arc.sweep_rad()))
        .map_err(|_| Error::Unsupported)
    }
}

pub(super) fn tilted(arc: EllipseArc, axis: Point) -> bool {
    length3(cross(arc.ellipse().normal(), axis)) > ANGULAR
}

pub(super) fn trim_station(arc: EllipseArc, point: Point, axis: Point) -> f64 {
    let rim = arc.ellipse();
    dot(sub(rim.center_mm(), point), rim.normal()) / dot(rim.normal(), axis)
}
