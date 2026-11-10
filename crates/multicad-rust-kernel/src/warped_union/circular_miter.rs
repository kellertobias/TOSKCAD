//! Exact miter joints between right-circular cylinder legs.
use super::{
    boss::{Rim, append_edge, oriented},
    *,
};
use std::f64::consts::PI;

const MAX_COORDINATE_MM: f64 = 1e12;
const MAX_PATH_POINTS: usize = 65;

/// Sweep a disk along two axes meeting at a miter.
pub fn circular_miter(path: [Point; 3], radius_mm: f64, tolerance_mm: f64) -> Result<Shell, Error> {
    circular_miter_chain(&path, radius_mm, tolerance_mm)
}

/// Sweep a disk along straight cylinder axes. The first and last path points
/// are the cap centers. Interior points are intersections of neighboring
/// axes. The cylinders share a rim on each joint's bisector plane. The shell
/// retains four sectors per leg and two exterior disk faces.
pub fn circular_miter_chain(
    path: &[Point],
    radius_mm: f64,
    tolerance_mm: f64,
) -> Result<Shell, Error> {
    build(path, radius_mm, tolerance_mm, None)
}

/// Restore a saved partition using the displacement from the start cap center
/// to one of its rim vertices.
pub(crate) fn circular_miter_chain_from_radial(
    path: &[Point],
    radius_mm: f64,
    tolerance_mm: f64,
    first_radial: Point,
) -> Result<Shell, Error> {
    build(path, radius_mm, tolerance_mm, Some(first_radial))
}

#[derive(Clone, Copy)]
struct Bend {
    normal: Point,
    bisector: Point,
    major: f64,
}

#[derive(Clone, Copy)]
struct PhasedRim {
    ellipse: Ellipse,
    phase: f64,
}

impl PhasedRim {
    fn offsets(self) -> [Point; 2] {
        let [major, minor] = self.ellipse.semiaxes_mm();
        let a = scale(self.ellipse.major_axis(), major);
        let b = scale(self.ellipse.minor_axis(), minor);
        let (sin, cos) = self.phase.sin_cos();
        [combine(a, cos, b, sin), combine(b, cos, a, -sin)]
    }

    fn point(self, parameter: f64) -> Point {
        self.ellipse.point_at(self.phase + parameter)
    }
}

fn build(
    path: &[Point],
    radius: f64,
    tolerance: f64,
    first_radial: Option<Point>,
) -> Result<Shell, Error> {
    if path.len() > MAX_PATH_POINTS {
        return Err(Error::Limit);
    }
    if path.len() < 3
        || !(1e-10..=1e-3).contains(&tolerance)
        || !radius.is_finite()
        || radius <= tolerance
        || path.iter().flatten().any(|value| !value.is_finite())
    {
        return Err(Error::Invalid);
    }
    if radius > MAX_COORDINATE_MM
        || path
            .iter()
            .flatten()
            .any(|value| value.abs() > MAX_COORDINATE_MM)
    {
        return Err(Error::Limit);
    }

    let legs: Vec<_> = path.windows(2).map(|pair| sub(pair[1], pair[0])).collect();
    let lengths: Vec<_> = legs.iter().copied().map(length3).collect();
    if lengths.iter().any(|&length| length <= tolerance) {
        return Err(Error::Invalid);
    }
    let axes: Vec<_> = legs
        .iter()
        .zip(&lengths)
        .map(|(&direction, &length)| scale(direction, length.recip()))
        .collect();
    let bends = axes
        .windows(2)
        .map(|pair| bend(pair[0], pair[1], radius))
        .collect::<Result<Vec<_>, _>>()?;
    let first_bend = bends.iter().flatten().next().ok_or(Error::Unsupported)?;
    let radial = if let Some(radial) = first_radial {
        let length = length3(radial);
        let station = dot(radial, axes[0]);
        if !length.is_finite() || (length - radius).abs() > tolerance || station.abs() > tolerance {
            return Err(Error::Invalid);
        }
        let projected = combine(radial, 1., axes[0], -station);
        scale(projected, length3(projected).recip())
    } else {
        cross(first_bend.normal, axes[0])
    };
    let trims = rims(path, &axes, &bends, radial, radius)?;
    let disk_area = PI * radius * radius;
    let rim_edge_count = 4 * trims.len();
    let mut shell = Shell {
        vertices: Vec::with_capacity(rim_edge_count),
        edges: Vec::with_capacity(rim_edge_count + 4 * legs.len()),
        faces: Vec::with_capacity(2 + 4 * legs.len()),
        bounds_mm: bounds(trims.iter().map(|rim| rim.ellipse)),
        volume_mm3: disk_area * lengths.iter().sum::<f64>(),
        solid_count: 1,
        shell_count: 1,
        band: None,
        chart_w: vec![],
        chart_edges: 0,
        matched_quarter_rims: Some((
            rim_edge_count,
            bends
                .iter()
                .flatten()
                .map(|bend| bend.major)
                .fold(radius, f64::max),
        )),
    };
    if shell
        .bounds_mm
        .iter()
        .flatten()
        .any(|value| value.abs() > MAX_COORDINATE_MM)
    {
        return Err(Error::Limit);
    }
    for (leg, &axis) in axes.iter().enumerate() {
        for (earlier, &earlier_axis) in axes.iter().take(leg.saturating_sub(1)).enumerate() {
            if !separated(
                [trims[earlier].ellipse, trims[earlier + 1].ellipse],
                [trims[leg].ellipse, trims[leg + 1].ellipse],
                [earlier_axis, axis],
                tolerance,
            ) {
                return Err(Error::Unsupported);
            }
        }
    }
    let rims = trims
        .iter()
        .copied()
        .map(|rim| append_rim(&mut shell, rim, tolerance))
        .collect::<Result<Vec<_>, _>>()?;
    let end = axes.len() - 1;
    shell.faces.push(Face {
        boundary: rims[0]
            .edges
            .iter()
            .rev()
            .map(|&edge| oriented(edge, false))
            .collect(),
        holes: Vec::new(),
        support: Support::Plane {
            normal: scale(axes[0], -1.),
        },
        point_mm: path[0],
        area_mm2: disk_area,
        source: 0,
        source_face: 0,
    });
    for (leg, &axis) in axes.iter().enumerate() {
        let [lower, upper] = [&rims[leg], &rims[leg + 1]];
        let [low, high] = [trims[leg], trims[leg + 1]];
        let [low_u, low_v] = low.offsets();
        let [high_u, high_v] = high.offsets();
        let [a, b] = [sub(high_u, low_u), sub(high_v, low_v)];
        let [cosine, sine] = [dot(axis, a), dot(axis, b)];
        // Generator lengths are affine in the shared radial angle. Their
        // global minimum is the center distance less the harmonic amplitude.
        let length = lengths[leg];
        if length - cosine.hypot(sine) <= tolerance
            || length3(cross(a, axis)).hypot(length3(cross(b, axis))) > tolerance
        {
            return Err(Error::Unsupported);
        }
        let rails: [usize; 4] = std::array::from_fn(|i| {
            append_edge(
                &mut shell,
                [lower.vertices[i], upper.vertices[i]],
                Curve::Line,
            )
        });
        if rails.iter().any(|&rail| {
            let [a, b] = shell.edges[rail].vertices.map(|i| shell.vertices[i]);
            let delta = sub(b, a);
            dot(delta, axis) <= tolerance || length3(cross(delta, axis)) > tolerance
        }) {
            return Err(Error::Unsupported);
        }
        let radial = [[1., 0.], [0., 1.], [-1., 0.], [0., -1.]];
        for sector in 0..4 {
            let next = (sector + 1) % 4;
            let [c0, s0] = radial[sector];
            let [c1, s1] = radial[next];
            let midpoint = (sector as f64 + 0.5) * FRAC_PI_2;
            let [p, q] = [low.point(midpoint), high.point(midpoint)];
            shell.faces.push(Face {
                boundary: vec![
                    oriented(lower.edges[sector], true),
                    oriented(rails[next], true),
                    oriented(upper.edges[sector], false),
                    oriented(rails[sector], false),
                ],
                holes: Vec::new(),
                support: Support::Cylinder {
                    axis_origin_mm: path[leg],
                    axis_direction: axis,
                    radius_mm: radius,
                    outward: true,
                },
                point_mm: std::array::from_fn(|i| p[i].midpoint(q[i])),
                area_mm2: radius * (FRAC_PI_2 * length + cosine * (s1 - s0) + sine * (c0 - c1)),
                source: leg,
                source_face: 2 + sector,
            });
        }
    }
    shell.faces.push(Face {
        boundary: rims[end + 1].edges.map(|edge| oriented(edge, true)).into(),
        holes: Vec::new(),
        support: Support::Plane { normal: axes[end] },
        point_mm: path[end + 1],
        area_mm2: disk_area,
        source: end,
        source_face: 1,
    });
    prove_incidence(&shell)?;
    Ok(shell)
}

fn bend(first: Point, second: Point, radius: f64) -> Result<Option<Bend>, Error> {
    let axis = cross(first, second);
    let turn = length3(axis);
    if turn <= 64. * f64::EPSILON && dot(first, second) > 0. {
        return Ok(None);
    }
    if turn <= FRAME_EPS {
        return Err(Error::Unsupported);
    }
    let sum = combine(first, 1., second, 1.);
    let bisector = scale(sum, length3(sum).recip());
    let cosine = 0.5 * length3(sum);
    let major = radius / cosine.min(1.);
    if major <= radius {
        return Err(Error::Unsupported);
    }
    if major > MAX_COORDINATE_MM {
        return Err(Error::Limit);
    }
    Ok(Some(Bend {
        normal: scale(axis, turn.recip()),
        bisector,
        major,
    }))
}

fn rims(
    path: &[Point],
    axes: &[Point],
    bends: &[Option<Bend>],
    mut radial: Point,
    radius: f64,
) -> Result<Vec<PhasedRim>, Error> {
    let circle = |point, axis, radial| {
        Ellipse::new(point, axis, radial, [radius; 2])
            .map(|ellipse| PhasedRim { ellipse, phase: 0. })
            .map_err(|_| Error::Invalid)
    };
    let mut rims = Vec::with_capacity(path.len());
    rims.push(circle(path[0], axes[0], radial)?);
    for (joint, bend) in bends.iter().enumerate() {
        let [incoming, outgoing] = [axes[joint], axes[joint + 1]];
        let point = path[joint + 1];
        let Some(bend) = bend else {
            rims.push(circle(point, incoming, radial)?);
            continue;
        };
        // The bend-normal component stays fixed. The in-plane radial
        // component rotates into the outgoing chart by the same angle as
        // the tangent. On the bisector it stretches by the secant.
        let incoming_radial = cross(bend.normal, incoming);
        let c = dot(radial, incoming_radial);
        let s = dot(radial, bend.normal);
        let ellipse = Ellipse::new(
            point,
            bend.bisector,
            cross(bend.normal, bend.bisector),
            [bend.major, radius],
        )
        .map_err(|_| Error::Invalid)?;
        rims.push(PhasedRim {
            ellipse,
            phase: s.atan2(c),
        });
        radial = combine(cross(bend.normal, outgoing), c, bend.normal, s);
    }
    let end = path.len() - 1;
    rims.push(circle(path[end], axes[end - 1], radial)?);
    Ok(rims)
}

fn separated(a: [Ellipse; 2], b: [Ellipse; 2], axes: [Point; 2], tolerance: f64) -> bool {
    let origin = a[0].center_mm();
    let directions = [
        [1., 0., 0.],
        [0., 1., 0.],
        [0., 0., 1.],
        axes[0],
        axes[1],
        cross(axes[0], axes[1]),
        a[0].normal(),
        a[1].normal(),
        b[0].normal(),
        b[1].normal(),
        sub(b[0].center_mm(), origin),
        sub(b[1].center_mm(), origin),
        sub(b[0].center_mm(), a[1].center_mm()),
        sub(b[1].center_mm(), a[1].center_mm()),
    ];
    directions.into_iter().any(|direction| {
        let length = length3(direction);
        if length <= FRAME_EPS {
            return false;
        }
        let axis = scale(direction, length.recip());
        let [a, b] = [a, b].map(|rims| {
            rims.into_iter()
                .fold([f64::INFINITY, f64::NEG_INFINITY], |[lo, hi], rim| {
                    let center = dot(sub(rim.center_mm(), origin), axis);
                    let [major, minor] = rim.semiaxes_mm();
                    let extent = (major * dot(rim.major_axis(), axis))
                        .hypot(minor * dot(rim.minor_axis(), axis));
                    [lo.min(center - extent), hi.max(center + extent)]
                })
        });
        // A positive-length leg is the convex hull of the two trimmed
        // disks. Its exact support interval is the hull of their intervals.
        let roundoff = 64. * f64::EPSILON * a.into_iter().chain(b).map(f64::abs).fold(1., f64::max);
        a[0] - b[1] > tolerance + roundoff || b[0] - a[1] > tolerance + roundoff
    })
}

impl Shell {
    /// Rotate the artificial quarter trims of an unmodified miter until no
    /// trim vertex lies on the cutting plane.
    pub(super) fn repartition_circular_miter(
        &self,
        signed_distance: impl Fn(Point) -> f64,
        tolerance: f64,
    ) -> Result<(Self, Vec<f64>), Error> {
        let (edges, _) = self.matched_quarter_rims.ok_or(Error::Unsupported)?;
        if edges < 12
            || self.vertices.len() != edges
            || self.edges.len() != 2 * edges - 4
            || self.faces.len() != edges - 2
        {
            return Err(Error::Unsupported);
        }
        let rims: Vec<_> = self.edges[..edges]
            .iter()
            .step_by(4)
            .map(|edge| match edge.curve {
                Curve::Ellipse(arc) => Ok(PhasedRim {
                    ellipse: arc.ellipse(),
                    phase: arc.start_rad(),
                }),
                Curve::Line => Err(Error::Unsupported),
            })
            .collect::<Result<_, _>>()?;
        // Each noncoincident ellipse has at most two plane hits, hence at
        // most two excluded phases modulo a quadrant per rim.
        let trials = 2 * rims.len() + 2;
        let phase = (1..trials)
            .map(|i| FRAC_PI_2 * i as f64 / trials as f64)
            .find(|&phase| {
                rims.iter().all(|rim| {
                    (0..4).all(|i| {
                        signed_distance(rim.point(phase + f64::from(i) * FRAC_PI_2)).abs()
                            > tolerance
                    })
                })
            })
            .ok_or(Error::Unsupported)?;
        let path: Vec<_> = rims.iter().map(|rim| rim.ellipse.center_mm()).collect();
        let first = rims[0];
        let [u, v] = first.offsets();
        let (s, c) = phase.sin_cos();
        let shifted = circular_miter_chain_from_radial(
            &path,
            first.ellipse.semiaxes_mm()[1],
            tolerance,
            combine(u, c, v, s),
        )?;
        let distances: Vec<_> = shifted
            .vertices
            .iter()
            .copied()
            .map(signed_distance)
            .collect();
        distances
            .iter()
            .all(|d| d.abs() > tolerance)
            .then_some((shifted, distances))
            .ok_or(Error::Unsupported)
    }
}

fn scale(point: Point, factor: f64) -> Point {
    point.map(|value| value * factor)
}

fn combine(a: Point, a_factor: f64, b: Point, b_factor: f64) -> Point {
    std::array::from_fn(|i| a[i] * a_factor + b[i] * b_factor)
}

fn append_rim(shell: &mut Shell, rim: PhasedRim, tolerance: f64) -> Result<Rim, Error> {
    let center = rim.ellipse.center_mm();
    let minor = rim.ellipse.semiaxes_mm()[1];
    let [u, v] = rim.offsets();
    let mut vertices = [0; 4];
    for (vertex, offset) in vertices
        .iter_mut()
        .zip([u, v, scale(u, -1.), scale(v, -1.)])
    {
        let point = combine(center, 1., offset, 1.);
        // A small rim can lose its physical vertices under rounding far
        // from the chart origin. Bound the actual offset error.
        if length3(sub(sub(point, center), offset)) > tolerance.min(minor * 0.125) {
            return Err(Error::Unsupported);
        }
        let index = shell.vertices.len();
        shell.vertices.push(point);
        *vertex = index;
    }
    let edges = std::array::from_fn(|i| {
        let arc = EllipseArc::new(rim.ellipse, rim.phase + i as f64 * FRAC_PI_2, FRAC_PI_2)
            .expect("constant quarter-ellipse trim");
        append_edge(
            shell,
            [vertices[i], vertices[(i + 1) % 4]],
            Curve::Ellipse(arc),
        )
    });
    Ok(Rim { vertices, edges })
}

fn bounds(ellipses: impl Iterator<Item = Ellipse>) -> [Point; 2] {
    let mut bounds = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    for ellipse in ellipses {
        let center = ellipse.center_mm();
        let [major, minor] = ellipse.semiaxes_mm();
        let u = ellipse.major_axis();
        let v = ellipse.minor_axis();
        for i in 0..3 {
            let extent = (major * u[i]).hypot(minor * v[i]);
            bounds[0][i] = bounds[0][i].min(center[i] - extent);
            bounds[1][i] = bounds[1][i].max(center[i] + extent);
        }
    }
    bounds
}

fn prove_incidence(shell: &Shell) -> Result<(), Error> {
    let mut uses = vec![[0; 2]; shell.edges.len()];
    for face in &shell.faces {
        for (&a, &b) in face
            .boundary
            .iter()
            .zip(face.boundary.iter().cycle().skip(1))
        {
            if shell.oriented_vertices(a)[1] != shell.oriented_vertices(b)[0] {
                return Err(Error::Invalid);
            }
            uses[a.edge][usize::from(a.forward)] += 1;
        }
    }
    if uses.iter().any(|&counts| counts != [1, 1]) {
        return Err(Error::Invalid);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
