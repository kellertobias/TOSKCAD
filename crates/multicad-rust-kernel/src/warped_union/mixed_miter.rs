//! Exact line and arc profiles swept along mitered straight legs.
use super::{
    boss::{append_edge, oriented},
    circular_miter::prove_incidence,
    cylinder_patch::{ANGULAR, CylinderPatch},
    *,
};
use crate::normal_sweep::{self, Edge as ProfileEdge, Sweep};

const MAX_COORDINATE_MM: f64 = 1e12;
const MAX_PATH_POINTS: usize = 65;
const MAX_RIM_EDGES: usize = 256;
const CARTESIAN_AXES: [Point; 3] = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
type RimBounds = [[f64; 2]; 3];

struct Piece {
    edge: ProfileEdge,
    source: usize,
}

#[derive(Clone, Copy)]
struct Rim {
    origin: Point,
    u: Point,
    v: Point,
    normal: Point,
    major: Point,
    stretch: f64,
}

impl Rim {
    fn perpendicular(origin: Point, u: Point, v: Point, axis: Point) -> Self {
        Self {
            origin,
            u,
            v,
            normal: axis,
            major: u,
            stretch: 1.,
        }
    }

    fn offset(self, [x, y]: [f64; 2]) -> Point {
        std::array::from_fn(|i| self.u[i].mul_add(x, self.v[i] * y))
    }

    fn map(self, point: [f64; 2]) -> Point {
        let offset = self.offset(point);
        std::array::from_fn(|i| self.origin[i] + offset[i])
    }

    fn interval(self, ring: &[ProfileEdge], direction: Point, origin: Point) -> [f64; 2] {
        let center = dot(sub(self.origin, origin), direction);
        let coefficients = [dot(self.u, direction), dot(self.v, direction)];
        [
            center - support(ring, coefficients.map(|c| -c)),
            center + support(ring, coefficients),
        ]
    }

    fn curve(
        self,
        edge: ProfileEdge,
        endpoints: [Point; 2],
        tolerance: f64,
    ) -> Result<Curve, Error> {
        let ProfileEdge::Arc { center, a, .. } = edge else {
            return Ok(Curve::Line);
        };
        let radius = edge.radius_mm().ok_or(Error::Invalid)?;
        let semiaxes = [radius * self.stretch, radius];
        let ellipse = Ellipse::new(self.map(center), self.normal, self.major, semiaxes)
            .map_err(|_| Error::Unsupported)?;
        // Use the local radius vector to recover the phase before adding the
        // world coordinates of the station. Those coordinates can be large.
        let radial = self.offset([a[0] - center[0], a[1] - center[1]]);
        let x = dot(radial, ellipse.major_axis()) / semiaxes[0];
        let y = dot(radial, ellipse.minor_axis()) / semiaxes[1];
        let arc = EllipseArc::new(
            ellipse,
            y.atan2(x),
            edge.signed_sweep_rad().ok_or(Error::Invalid)?,
        )
        .map_err(|_| Error::Unsupported)?;
        if arc
            .endpoints()
            .into_iter()
            .zip(endpoints)
            .any(|(a, b)| length3(sub(a, b)) > tolerance)
        {
            return Err(Error::Unsupported);
        }
        Ok(Curve::Ellipse(arc))
    }
}

/// Sweep one closed line and arc ring. The profile is in XY and the path
/// starts at the origin along +Z. All wall sectors retain their authored edge
/// identity. The result contains the two exterior caps and no internal caps.
pub fn mixed_miter(
    profile: &[ProfileEdge],
    path: &[Point],
    tolerance_mm: f64,
) -> Result<Shell, Error> {
    mixed_miter_with_end_frame(profile, path, tolerance_mm).map(|(shell, _)| shell)
}

/// Sweep a ring and return its transported X and Y directions on the end cap.
/// The end cap frame is relative to the last point in the path.
pub fn mixed_miter_with_end_frame(
    profile: &[ProfileEdge],
    path: &[Point],
    tolerance_mm: f64,
) -> Result<(Shell, [Point; 2]), Error> {
    build(profile, path, tolerance_mm, true)
}

/// Replay the rim partitions stored in a STEP boundary.
pub(crate) fn mixed_miter_from_rim(
    profile: &[ProfileEdge],
    path: &[Point],
    tolerance_mm: f64,
) -> Result<Shell, Error> {
    build(profile, path, tolerance_mm, false).map(|(shell, _)| shell)
}

fn build(
    profile: &[ProfileEdge],
    path: &[Point],
    tolerance_mm: f64,
    split_arcs: bool,
) -> Result<(Shell, [Point; 2]), Error> {
    if path.len() > MAX_PATH_POINTS || profile.len() > MAX_RIM_EDGES {
        return Err(Error::Limit);
    }
    if path.len() < 3
        || !(1e-10..=1e-3).contains(&tolerance_mm)
        || path[0] != [0.; 3]
        || path[1][..2] != [0.; 2]
        || path[1][2] <= tolerance_mm
        || path.iter().flatten().any(|value| !value.is_finite())
    {
        return Err(Error::Invalid);
    }
    if path
        .iter()
        .flatten()
        .any(|value| value.abs() > MAX_COORDINATE_MM)
    {
        return Err(Error::Limit);
    }

    let (sweep, pieces) = certify_profile(profile, tolerance_mm, split_arcs)?;
    let ring = &sweep.rings()[0];
    let legs = path
        .windows(2)
        .map(|pair| {
            let delta = sub(pair[1], pair[0]);
            let length = length3(delta);
            if length <= tolerance_mm {
                return Err(Error::Invalid);
            }
            Ok((scale(delta, length.recip()), length))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let rims = transport(path, &legs)?;
    let rim_bounds: Vec<_> = rims
        .iter()
        .map(|rim| CARTESIAN_AXES.map(|axis| rim.interval(ring, axis, [0.; 3])))
        .collect();
    let bounds_mm = std::array::from_fn(|end| {
        std::array::from_fn(|i| {
            rim_bounds.iter().map(|bounds| bounds[i][end]).fold(
                if end == 0 {
                    f64::INFINITY
                } else {
                    f64::NEG_INFINITY
                },
                if end == 0 { f64::min } else { f64::max },
            )
        })
    });
    if bounds_mm
        .iter()
        .flatten()
        .any(|v| v.abs() > MAX_COORDINATE_MM)
    {
        return Err(Error::Limit);
    }

    let edge_count = pieces.len();
    let rim_edges = edge_count * rims.len();
    let mut shell = Shell {
        vertices: Vec::with_capacity(rim_edges),
        edges: Vec::with_capacity(rim_edges + edge_count * legs.len()),
        faces: Vec::with_capacity(2 + edge_count * legs.len()),
        bounds_mm,
        volume_mm3: 0.,
        solid_count: 1,
        shell_count: 1,
        band: None,
        chart_w: vec![],
        chart_edges: 0,
        matched_quarter_rims: None,
    };
    for rim in &rims {
        let base = shell.vertices.len();
        shell
            .vertices
            .extend(pieces.iter().map(|piece| rim.map(piece.edge.start())));
        for (i, &Piece { edge, .. }) in pieces.iter().enumerate() {
            let vertices = [base + i, base + (i + 1) % edge_count];
            let endpoints = vertices.map(|id| shell.vertices[id]);
            if length3(sub(endpoints[1], endpoints[0])) <= tolerance_mm {
                return Err(Error::Unsupported);
            }
            let curve = rim.curve(edge, endpoints, tolerance_mm)?;
            append_edge(&mut shell, vertices, curve);
        }
    }
    shell.matched_quarter_rims = Some(MatchedRims {
        edges: rim_edges,
        pieces: edge_count,
        circular: false,
    });
    let area = sweep.area_mm2();
    let centroid = sweep.centroid();
    let last = legs.len() - 1;
    shell
        .faces
        .push(cap(rims[0], 0, edge_count, centroid, area, true));
    let profile_size = [0, 1].map(|i| {
        let mut axis = [0.; 2];
        axis[i] = 1.;
        support(ring, axis).max(support(ring, axis.map(|v| -v)))
    });
    for (leg, &(axis, length)) in legs.iter().enumerate() {
        let [low, high] = [rims[leg], rims[leg + 1]];
        let [du, dv] = [sub(high.u, low.u), sub(high.v, low.v)];
        let gradient = [dot(du, axis), dot(dv, axis)];
        // The length along each generator is affine in the profile point.
        // Exact support over the curved boundary bounds every interior point.
        if length - support(ring, gradient.map(|g| -g)) <= tolerance_mm
            || length3(cross(du, axis)) * profile_size[0]
                + length3(cross(dv, axis)) * profile_size[1]
                > tolerance_mm
        {
            return Err(Error::Unsupported);
        }
        for (earlier, &(earlier_axis, _)) in legs.iter().take(leg.saturating_sub(1)).enumerate() {
            if !separated(
                ring,
                [rims[earlier], rims[earlier + 1]],
                [low, high],
                [earlier_axis, axis],
                [
                    [rim_bounds[earlier], rim_bounds[earlier + 1]],
                    [rim_bounds[leg], rim_bounds[leg + 1]],
                ],
                tolerance_mm,
            ) {
                return Err(Error::Unsupported);
            }
        }
        let gap = [gradient[0], gradient[1], length];
        shell.volume_mm3 += area * generator_length(gap, centroid);
        append_leg(
            &mut shell,
            &pieces,
            [low, high],
            leg,
            axis,
            gap,
            tolerance_mm,
        )?;
    }
    let end = rims[last + 1];
    shell
        .faces
        .push(cap(end, last, edge_count, centroid, area, false));
    prove_incidence(&shell)?;
    Ok((shell, [end.u, end.v]))
}

fn certify_profile(
    profile: &[ProfileEdge],
    tolerance: f64,
    split_arcs: bool,
) -> Result<(Sweep, Vec<Piece>), Error> {
    if !profile
        .iter()
        .any(|edge| matches!(edge, ProfileEdge::Arc { .. }))
        || profile
            .iter()
            .any(|edge| matches!(edge, ProfileEdge::Circle { .. }))
    {
        return Err(Error::Unsupported);
    }
    let reversed = normal_sweep::signed_area(profile) < 0.;
    let mut ring = profile.to_vec();
    if reversed {
        ring.reverse();
        ring.iter_mut().for_each(|edge| *edge = edge.reversed());
    }
    let sweep =
        Sweep::new(vec![ring], 1., tolerance, tolerance.next_up()).map_err(
            |error| match error {
                normal_sweep::Error::Limit => Error::Limit,
                _ => Error::Invalid,
            },
        )?;
    let mut pieces = Vec::new();
    for (i, &edge) in sweep.rings()[0].iter().enumerate() {
        let source = if reversed { profile.len() - 1 - i } else { i };
        if split_arcs
            && let ProfileEdge::Arc {
                center, clockwise, ..
            } = edge
        {
            let span = edge.signed_sweep_rad().ok_or(Error::Invalid)?.abs();
            // Keep an authored quarter arc intact despite phase roundoff.
            let count = ((span - ANGULAR / 2.) / FRAC_PI_2).ceil().max(1.) as usize;
            for part in 0..count {
                pieces.push(Piece {
                    edge: ProfileEdge::Arc {
                        center,
                        a: edge.point(part as f64 / count as f64),
                        b: edge.point((part + 1) as f64 / count as f64),
                        clockwise,
                    },
                    source,
                });
            }
        } else {
            pieces.push(Piece { edge, source });
        }
        if pieces.len() > MAX_RIM_EDGES {
            return Err(Error::Limit);
        }
    }
    Ok((sweep, pieces))
}

fn transport(path: &[Point], legs: &[(Point, f64)]) -> Result<Vec<Rim>, Error> {
    let [mut u, mut v, first] = CARTESIAN_AXES;
    let mut rims = vec![Rim::perpendicular(path[0], u, v, first)];
    for (i, pair) in legs.windows(2).enumerate() {
        let [incoming, outgoing] = [pair[0].0, pair[1].0];
        let origin = path[i + 1];
        let bend_axis = cross(incoming, outgoing);
        let turn = length3(bend_axis);
        if turn <= ANGULAR && dot(incoming, outgoing) > 0. {
            rims.push(Rim::perpendicular(origin, u, v, incoming));
            u = unit(sub(u, scale(outgoing, dot(u, outgoing))));
            v = cross(outgoing, u);
            continue;
        }
        if turn <= FRAME_EPS {
            return Err(Error::Unsupported);
        }
        let side = scale(bend_axis, turn.recip());
        let across_incoming = cross(side, incoming);
        let across_outgoing = cross(side, outgoing);
        let sum = std::array::from_fn(|i| incoming[i] + outgoing[i]);
        let cosine = (0.5 * length3(sum)).min(1.);
        let sine = 0.5 * length3(sub(incoming, outgoing));
        let normal = unit(combine(incoming, cosine, across_incoming, sine));
        let major = cross(side, normal);
        let project = |d| combine(side, dot(side, d), major, dot(across_incoming, d) / cosine);
        rims.push(Rim {
            origin,
            u: project(u),
            v: project(v),
            normal,
            major,
            stretch: cosine.recip(),
        });
        // The incoming and outgoing legs meet the bisector at equal angles.
        // Projecting its ellipse along the outgoing axis restores a circle.
        u = unit(combine(
            side,
            dot(side, u),
            across_outgoing,
            dot(across_incoming, u),
        ));
        v = cross(outgoing, u);
    }
    rims.push(Rim::perpendicular(
        path[path.len() - 1],
        u,
        v,
        legs[legs.len() - 1].0,
    ));
    Ok(rims)
}

fn append_leg(
    shell: &mut Shell,
    pieces: &[Piece],
    [low, high]: [Rim; 2],
    leg: usize,
    axis: Point,
    gap: [f64; 3],
    tolerance: f64,
) -> Result<(), Error> {
    let n = pieces.len();
    let bottom = leg * n;
    let top = bottom + n;
    let rail = shell.edges.len();
    for i in 0..n {
        let vertices = [bottom + i, top + i];
        let [a, b] = vertices.map(|id| shell.vertices[id]);
        let delta = sub(b, a);
        if dot(delta, axis) <= tolerance || length3(cross(delta, axis)) > tolerance {
            return Err(Error::Unsupported);
        }
        append_edge(shell, vertices, Curve::Line);
    }
    for (i, &Piece { edge, source }) in pieces.iter().enumerate() {
        let next = (i + 1) % n;
        let support = if let ProfileEdge::Arc {
            center, clockwise, ..
        } = edge
        {
            Support::Cylinder {
                axis_origin_mm: low.map(center),
                axis_direction: axis,
                radius_mm: edge.radius_mm().ok_or(Error::Invalid)?,
                outward: !clockwise,
            }
        } else {
            let delta = sub(shell.vertices[bottom + next], shell.vertices[bottom + i]);
            Support::Plane {
                normal: unit(cross(delta, axis)),
            }
        };
        let middle = edge.point(0.5);
        let [a, b] = [low.map(middle), high.map(middle)];
        let face = Face {
            boundary: vec![
                oriented(bottom + i, true),
                oriented(rail + next, true),
                oriented(top + i, false),
                oriented(rail + i, false),
            ],
            holes: Vec::new(),
            support,
            point_mm: std::array::from_fn(|i| a[i].midpoint(b[i])),
            area_mm2: edge.length_mm() * generator_length(gap, edge.centroid()),
            source: leg,
            source_face: source + 2,
        };
        if let Support::Cylinder {
            axis_origin_mm,
            radius_mm,
            ..
        } = face.support
        {
            CylinderPatch::new(shell, &face, axis_origin_mm, axis, radius_mm, tolerance)?;
        }
        shell.faces.push(face);
    }
    Ok(())
}

fn cap(rim: Rim, leg: usize, n: usize, centroid: [f64; 2], area: f64, start: bool) -> Face {
    let boundary = if start {
        (0..n).rev().map(|edge| oriented(edge, false)).collect()
    } else {
        let base = (leg + 1) * n;
        (base..base + n).map(|edge| oriented(edge, true)).collect()
    };
    Face {
        boundary,
        holes: Vec::new(),
        support: Support::Plane {
            normal: scale(rim.normal, if start { -1. } else { 1. }),
        },
        point_mm: rim.map(centroid),
        area_mm2: area,
        source: leg,
        source_face: usize::from(!start),
    }
}

fn separated(
    ring: &[ProfileEdge],
    a: [Rim; 2],
    b: [Rim; 2],
    [u, v]: [Point; 2],
    [aabb, babb]: [[RimBounds; 2]; 2],
    tolerance: f64,
) -> bool {
    let intervals = |[a, b]: [[f64; 2]; 2]| [a[0].min(b[0]), a[1].max(b[1])];
    let disjoint = |a: [f64; 2], b: [f64; 2]| {
        let roundoff = ANGULAR * a.into_iter().chain(b).map(f64::abs).fold(1., f64::max);
        a[0] - b[1] > tolerance + roundoff || b[0] - a[1] > tolerance + roundoff
    };
    if (0..3).any(|i| disjoint(intervals(aabb.map(|r| r[i])), intervals(babb.map(|r| r[i])))) {
        return true;
    }
    [
        u,
        v,
        cross(u, v),
        a[0].normal,
        a[1].normal,
        b[0].normal,
        b[1].normal,
        sub(b[0].origin, a[1].origin),
    ]
    .into_iter()
    .filter(|&direction| length3(direction) > ANGULAR)
    .map(unit)
    .any(|direction| {
        // Every generator lies in the convex hull of its two rim points.
        let range =
            |rims: [Rim; 2]| intervals(rims.map(|r| r.interval(ring, direction, a[0].origin)));
        disjoint(range(a), range(b))
    })
}

fn support(ring: &[ProfileEdge], direction: [f64; 2]) -> f64 {
    ring.iter()
        .map(|edge| edge.support(direction))
        .fold(f64::NEG_INFINITY, f64::max)
}

fn generator_length([dx, dy, length]: [f64; 3], [x, y]: [f64; 2]) -> f64 {
    dx.mul_add(x, dy.mul_add(y, length))
}

fn unit(v: Point) -> Point {
    scale(v, length3(v).recip())
}

fn scale(v: Point, by: f64) -> Point {
    v.map(|coordinate| coordinate * by)
}

fn combine(a: Point, x: f64, b: Point, y: f64) -> Point {
    std::array::from_fn(|i| a[i].mul_add(x, b[i] * y))
}

#[cfg(test)]
mod tests;
