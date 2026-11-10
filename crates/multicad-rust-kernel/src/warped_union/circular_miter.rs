//! One exact miter between two right-circular cylinder legs.
use super::{
    boss::{Rim, append_edge, oriented},
    *,
};
use std::f64::consts::PI;

const MAX_COORDINATE_MM: f64 = 1e12;

/// Sweep a disk between two caps normal to their respective straight axes.
/// `path` gives the two cap centers and their common axis intersection.
/// The cylinders share one ellipse in the angle bisector plane, with no
/// internal cap. Cylindrical sectors have source `leg` and source face `2..6`.
/// The start cap has source face `0` and the end cap has source face `1`.
pub fn circular_miter(path: [Point; 3], radius_mm: f64, tolerance_mm: f64) -> Result<Shell, Error> {
    circular_miter_with_phase(path, radius_mm, tolerance_mm, 0.)
}

fn circular_miter_with_phase(
    path: [Point; 3],
    radius_mm: f64,
    tolerance_mm: f64,
    phase: f64,
) -> Result<Shell, Error> {
    if !(1e-10..=1e-3).contains(&tolerance_mm)
        || !radius_mm.is_finite()
        || radius_mm <= tolerance_mm
        || path.iter().flatten().any(|value| !value.is_finite())
    {
        return Err(Error::Invalid);
    }
    if radius_mm > MAX_COORDINATE_MM
        || path
            .iter()
            .flatten()
            .any(|value| value.abs() > MAX_COORDINATE_MM)
    {
        return Err(Error::Limit);
    }

    let [start, corner, end] = path;
    let vectors = [sub(corner, start), sub(end, corner)];
    let lengths = vectors.map(length3);
    if lengths.into_iter().any(|length| length <= tolerance_mm) {
        return Err(Error::Invalid);
    }
    let [first, second] = std::array::from_fn(|i| scale(vectors[i], lengths[i].recip()));
    let bend = cross(first, second);
    let turn = length3(bend);
    if turn <= FRAME_EPS {
        return Err(Error::Unsupported);
    }
    let bend_normal = scale(bend, turn.recip());
    let sum = std::array::from_fn(|i| first[i] + second[i]);
    let bisector = scale(sum, length3(sum).recip());
    let cosine = 0.5 * length3(sum);
    let sine = 0.5 * length3(sub(first, second));
    let reach = radius_mm * sine / cosine;
    let major = radius_mm / cosine.min(1.);
    if major <= radius_mm {
        return Err(Error::Unsupported);
    }
    if major > MAX_COORDINATE_MM {
        return Err(Error::Limit);
    }
    // The miter cuts each generator at an axial station whose deviation
    // from the axis intersection is bounded by reach. Beyond this check
    // the two legs occupy strictly opposite halfspaces of the bisector.
    if lengths
        .into_iter()
        .any(|length| length - reach <= tolerance_mm)
    {
        return Err(Error::Unsupported);
    }

    let axes = [first, bisector, second];
    let ellipse = |i: usize| {
        let axis = axes[i];
        let major = if i == 1 { major } else { radius_mm };
        Ellipse::new(path[i], axis, cross(bend_normal, axis), [major, radius_mm])
            .ok()
            .ok_or(Error::Invalid)
    };
    let ellipses = [ellipse(0)?, ellipse(1)?, ellipse(2)?];
    let disk_area = PI * radius_mm * radius_mm;
    let mut shell = Shell {
        vertices: Vec::with_capacity(12),
        edges: Vec::with_capacity(20),
        faces: Vec::with_capacity(10),
        bounds_mm: bounds(&ellipses),
        volume_mm3: disk_area * (lengths[0] + lengths[1]),
        solid_count: 1,
        shell_count: 1,
        band: None,
        chart_w: vec![],
        chart_edges: 0,
        matched_quarter_rims: Some((12, major)),
    };
    if shell
        .bounds_mm
        .iter()
        .flatten()
        .any(|value| value.abs() > MAX_COORDINATE_MM)
    {
        return Err(Error::Limit);
    }
    let [a, joint, b] = ellipses;
    let rims = [
        append_rim(&mut shell, a, tolerance_mm, phase)?,
        append_rim(&mut shell, joint, tolerance_mm, phase)?,
        append_rim(&mut shell, b, tolerance_mm, phase)?,
    ];
    let (phase_sine, phase_cosine) = phase.sin_cos();

    shell.faces.push(Face {
        boundary: rims[0]
            .edges
            .iter()
            .rev()
            .map(|&edge| oriented(edge, false))
            .collect(),
        holes: Vec::new(),
        support: Support::Plane {
            normal: scale(first, -1.),
        },
        point_mm: start,
        area_mm2: disk_area,
        source: 0,
        source_face: 0,
    });
    for (leg, axis) in [first, second].into_iter().enumerate() {
        let [lower, upper] = [&rims[leg], &rims[leg + 1]];
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
            dot(delta, axis) <= tolerance_mm || length3(cross(delta, axis)) > tolerance_mm
        }) {
            return Err(Error::Unsupported);
        }
        // The generator lengths are L − reach·cos(θ). Each sector integrates
        // this affine harmonic exactly over one quadrant.
        let sine_deltas = [
            phase_cosine - phase_sine,
            -phase_cosine - phase_sine,
            phase_sine - phase_cosine,
            phase_sine + phase_cosine,
        ];
        for (sector, delta_sine) in sine_deltas.into_iter().enumerate() {
            let next = (sector + 1) % 4;
            let midpoint = phase + (sector as f64 + 0.5) * FRAC_PI_2;
            let [a, b] =
                [ellipses[leg], ellipses[leg + 1]].map(|ellipse| ellipse.point_at(midpoint));
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
                    radius_mm,
                    outward: true,
                },
                point_mm: std::array::from_fn(|i| a[i].midpoint(b[i])),
                area_mm2: radius_mm * (FRAC_PI_2 * lengths[leg] - reach * delta_sine),
                source: leg,
                source_face: 2 + sector,
            });
        }
    }
    shell.faces.push(Face {
        boundary: rims[2].edges.map(|edge| oriented(edge, true)).into(),
        holes: Vec::new(),
        support: Support::Plane { normal: second },
        point_mm: end,
        area_mm2: disk_area,
        source: 1,
        source_face: 1,
    });
    // Every circumferential edge is shared by adjacent legs or a cap,
    // and adjacent sectors traverse each generator in opposite directions.
    prove_incidence(&shell)?;
    Ok(shell)
}

impl Shell {
    /// Rotate the artificial quarter trims of an unmodified miter until no
    /// trim vertex lies on the cutting plane. The three rim supports and
    /// the two cylinder supports stay the same.
    pub(super) fn repartition_circular_miter(
        &self,
        signed_distance: impl Fn(Point) -> f64,
        tolerance_mm: f64,
    ) -> Result<(Self, Vec<f64>), Error> {
        if !matches!(self.matched_quarter_rims, Some((12, _)))
            || self.vertices.len() != 12
            || self.edges.len() != 20
            || self.faces.len() != 10
        {
            return Err(Error::Unsupported);
        }
        let rims = [0, 4, 8].map(|index| match self.edges[index].curve {
            Curve::Ellipse(arc) => Some(arc.ellipse()),
            Curve::Line => None,
        });
        let [Some(first), Some(joint), Some(last)] = rims else {
            return Err(Error::Unsupported);
        };
        let ellipses = [first, joint, last];
        // Each noncoincident ellipse has at most two plane hits, hence at
        // most six excluded phases modulo a quadrant. A clearance check
        // also rejects numerically indistinguishable hits and coincidences.
        let phase = (1..8)
            .map(|i| f64::from(i) * FRAC_PI_2 / 8.)
            .find(|&phase| {
                ellipses.iter().all(|ellipse| {
                    (0..4).all(|i| {
                        let point = ellipse.point_at(phase + f64::from(i) * FRAC_PI_2);
                        signed_distance(point).abs() > tolerance_mm
                    })
                })
            })
            .ok_or(Error::Unsupported)?;
        let shifted = circular_miter_with_phase(
            ellipses.map(Ellipse::center_mm),
            first.semiaxes_mm()[1],
            tolerance_mm,
            phase,
        )?;
        let distances: Vec<_> = shifted
            .vertices
            .iter()
            .copied()
            .map(signed_distance)
            .collect();
        distances
            .iter()
            .all(|d| d.abs() > tolerance_mm)
            .then_some((shifted, distances))
            .ok_or(Error::Unsupported)
    }
}

fn scale(point: Point, factor: f64) -> Point {
    point.map(|value| value * factor)
}

fn append_rim(
    shell: &mut Shell,
    ellipse: Ellipse,
    tolerance_mm: f64,
    phase: f64,
) -> Result<Rim, Error> {
    let center = ellipse.center_mm();
    let [major, minor] = ellipse.semiaxes_mm();
    let a = scale(ellipse.major_axis(), major);
    let b = scale(ellipse.minor_axis(), minor);
    let (sin, cos) = phase.sin_cos();
    let u = std::array::from_fn(|i| a[i] * cos + b[i] * sin);
    let v = std::array::from_fn(|i| b[i] * cos - a[i] * sin);
    let mut vertices = [0; 4];
    for (vertex, offset) in vertices
        .iter_mut()
        .zip([u, v, scale(u, -1.), scale(v, -1.)])
    {
        let point = std::array::from_fn(|i| center[i] + offset[i]);
        // A small but analytically valid rim can lose its physical vertices
        // under rounding far from the chart origin. Bound the actual offset
        // error and keep adjacent vertices separated at the model tolerance.
        if length3(sub(sub(point, center), offset)) > tolerance_mm.min(minor * 0.125) {
            return Err(Error::Unsupported);
        }
        let index = shell.vertices.len();
        shell.vertices.push(point);
        *vertex = index;
    }
    let edges = std::array::from_fn(|i| {
        let arc = EllipseArc::new(ellipse, phase + i as f64 * FRAC_PI_2, FRAC_PI_2)
            .expect("constant quarter-ellipse trim");
        append_edge(
            shell,
            [vertices[i], vertices[(i + 1) % 4]],
            Curve::Ellipse(arc),
        )
    });
    Ok(Rim { vertices, edges })
}

fn bounds(ellipses: &[Ellipse; 3]) -> [Point; 2] {
    // Every point of a cylinder generator lies between the paired rim
    // points, so its coordinate extrema are achieved on these ellipses.
    let mut bounds = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    for &ellipse in ellipses {
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
    let mut uses = [[0; 2]; 20];
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
