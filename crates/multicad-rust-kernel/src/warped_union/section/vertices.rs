//! Certify transverse section crossings at shared trim vertices.
use super::*;

pub(super) struct VertexContacts {
    /// Signed side on leaving each endpoint that lies on the section plane.
    sides: Vec<[Option<bool>; 2]>,
}

impl VertexContacts {
    pub(super) fn new(
        shell: &Shell,
        distances: &[f64],
        origin: Point,
        normal: Point,
        tolerance: f64,
    ) -> Result<Self, Error> {
        for (&point, &distance) in shell.vertices.iter().zip(distances) {
            // A nearby vertex is ambiguous at the requested resolution.
            // Reuse its position only within floating-point roundoff.
            let magnitude = point
                .into_iter()
                .chain(origin)
                .map(f64::abs)
                .fold(1., f64::max);
            let roundoff = (ANGULAR * magnitude).min(tolerance * 0.01);
            if distance.abs() <= tolerance && distance.abs() > roundoff {
                return Err(Error::Unsupported);
            }
        }
        let sides = shell
            .edges
            .iter()
            .map(|edge| {
                let mut sides = [None; 2];
                for (end, side) in sides.iter_mut().enumerate() {
                    if distances[edge.vertices[end]].abs() > tolerance {
                        continue;
                    }
                    let slope = match edge.curve {
                        Curve::Line => {
                            distances[edge.vertices[1 - end]] - distances[edge.vertices[end]]
                        }
                        Curve::Ellipse(arc) => {
                            let parameter = arc.start_rad() + end as f64 * arc.sweep_rad();
                            let tangent = arc.ellipse().tangent_at(parameter);
                            dot(tangent, normal)
                                * arc.sweep_rad().signum()
                                * if end == 0 { 1. } else { -1. }
                        }
                    };
                    if slope.abs() <= tolerance {
                        return Err(Error::Unsupported);
                    }
                    *side = Some(slope > 0.);
                }
                Ok(sides)
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { sides })
    }

    pub(super) fn endpoints(&self, shell: &Shell, index: usize) -> Option<[Option<Point>; 2]> {
        let sides = self.sides[index];
        sides.iter().any(Option::is_some).then(|| {
            std::array::from_fn(|end| {
                sides[end].map(|_| shell.vertices[shell.edges[index].vertices[end]])
            })
        })
    }

    pub(super) fn crossings<'a>(
        &'a self,
        shell: &'a Shell,
        wire: &'a [OrientedEdge],
    ) -> impl Iterator<Item = Hit> + 'a {
        wire.iter()
            .zip(wire.iter().cycle().skip(1))
            .filter_map(move |(before, after)| {
                let end = usize::from(before.forward);
                let start = usize::from(!after.forward);
                let a = self.sides[before.edge][end]?;
                let b = self.sides[after.edge][start]?;
                (a != b).then(|| {
                    let vertex = shell.edges[after.edge].vertices[start];
                    Hit {
                        port: [shell.edges.len(), vertex],
                        point: shell.vertices[vertex],
                    }
                })
            })
    }
}

pub(super) fn interior_arc_hits(
    arc: EllipseArc,
    plane: Plane,
    mut endpoints: [Option<Point>; 2],
    tolerance: f64,
) -> Result<Vec<Point>, Error> {
    let ellipse = arc.ellipse();
    let PlaneCrossings::Crossing(angles) = ellipse
        .plane_crossings(plane, tolerance)
        .map_err(|_| Error::Unsupported)?
    else {
        return Err(Error::Unsupported);
    };
    let mut hits = Vec::new();
    for angle in angles {
        let point = ellipse.point_at(angle);
        if let Some(endpoint) = endpoints
            .iter_mut()
            .find(|endpoint| endpoint.is_some_and(|end| length3(sub(point, end)) <= tolerance))
        {
            *endpoint = None;
        } else if travel(arc, angle) < arc.sweep_rad().abs() {
            hits.push(point);
        }
    }
    if endpoints.iter().any(Option::is_some) {
        return Err(Error::Unsupported);
    }
    Ok(hits)
}
