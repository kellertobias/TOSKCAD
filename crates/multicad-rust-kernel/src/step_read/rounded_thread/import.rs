//! Candidate parameters come from actual finite jets, then the complete
//! independently parsed graph must certify their physical image.
use super::*;
use crate::rounded_thread::Spec;
use std::f64::consts::{FRAC_PI_2, TAU};

#[derive(Clone, Copy)]
struct Grid {
    id: u32,
    start: Point,
    radial_next: Point,
    axial_next: Point,
    radial_domain: [f64; 2],
    radial_span: f64,
    axial_domain: [f64; 2],
}
fn domain(value: &Value<'_>) -> Result<([f64; 2], f64)> {
    let Value::List(values) = value else {
        return Err(ReadError::Invalid);
    };
    if values.len() > 1025 {
        return Err(ReadError::Limit);
    }
    require(values.len() >= 2)?;
    let mut knots = Vec::with_capacity(values.len());
    for value in values {
        let Value::Number(v) = value else {
            return Err(ReadError::Invalid);
        };
        require(v.is_finite() && knots.last().is_none_or(|p| *p < *v))?;
        knots.push(*v);
    }
    Ok((
        [knots[0], *knots.last().expect("knots")],
        knots[1] - knots[0],
    ))
}
fn grid(shell: &Shell<'_, '_>, id: u32) -> Result<Grid> {
    let (surface, _, _) = face(shell, id)?;
    let args = shell.args(surface, "B_SPLINE_SURFACE_WITH_KNOTS")?;
    let [
        Value::String(_),
        Value::Number(a),
        Value::Number(b),
        Value::List(rows),
        _,
        _,
        _,
        _,
        _,
        _,
        r,
        z,
        _,
    ] = args
    else {
        return Err(ReadError::Invalid);
    };
    require(
        *a == DEGREE as f64
            && *b == DEGREE as f64
            && rows.len() > DEGREE
            && (rows.len() - 1).is_multiple_of(DEGREE),
    )?;
    if rows.len() > 1024 * DEGREE + 1 {
        return Err(ReadError::Limit);
    }
    let first = refs(&rows[0])?;
    let next = refs(&rows[1])?;
    require(
        first.len() > DEGREE
            && (first.len() - 1).is_multiple_of(DEGREE)
            && first.len() == next.len(),
    )?;
    if first.len() > (MAX_STEP_ENTITIES / 2) / rows.len() {
        return Err(ReadError::Limit);
    }
    let (radial_domain, radial_span) = domain(r)?;
    let (axial_domain, _) = domain(z)?;
    Ok(Grid {
        id,
        start: shell.point(first[0])?,
        radial_next: shell.point(next[0])?,
        axial_next: shell.point(first[1])?,
        radial_domain,
        radial_span,
        axial_domain,
    })
}
struct Chart {
    origin: Point,
    rows: [Point; 3],
    det: f64,
}
impl Chart {
    fn new([origin, u, v]: [Point; 3]) -> Result<Self> {
        let n = cross(u, v);
        let rows = [cross(v, n), cross(n, u), n];
        let det = dot(u, rows[0]);
        require(det.is_finite() && det > 0.)?;
        Ok(Self { origin, rows, det })
    }
    fn local(&self, p: Point) -> Point {
        self.rows
            .map(|row| dot(row, sub(p, self.origin)) / self.det)
    }
}
fn plane(shell: &Shell<'_, '_>, id: u32) -> Result<Plane> {
    let (surface, forward, _) = face(shell, id)?;
    require(forward)?;
    let [Value::String(_), axis] = shell.args(surface, "PLANE")? else {
        return Err(ReadError::Invalid);
    };
    shell.placement(reference(axis)?)
}

pub(super) fn read_member(
    shell: &mut Shell<'_, '_>,
    id: u32,
) -> Result<(Readback, ImportedRoundedThread)> {
    let faces = shell.member_faces(id)?;
    if faces.len() != 7 {
        return Err(ReadError::Unsupported);
    }
    let mut grids = Vec::with_capacity(5);
    let mut caps = Vec::with_capacity(2);
    for face_id in faces {
        let (surface, _, _) = face(shell, face_id)?;
        match shell.simple(surface)?.name {
            "B_SPLINE_SURFACE_WITH_KNOTS" => grids.push(grid(shell, face_id)?),
            "PLANE" => caps.push(plane(shell, face_id)?),
            _ => return Err(ReadError::Unsupported),
        }
    }
    require(grids.len() == 5 && caps.len() == 2)?;
    let tolerance = shell.tolerance.min(1e-7);
    let error = tolerance / 16.;
    let bottoms = caps
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            grids
                .iter()
                .all(|g| dot(sub(g.start, p.origin), p.normal).abs() <= error)
        })
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    let [bottom] = bottoms[..] else {
        return Err(ReadError::Invalid);
    };
    let p = &caps[bottom];
    let far = &caps[1 - bottom];
    let origin = p.origin;
    let n = p.normal.map(|x| -x);
    let length = dot(sub(far.origin, origin), n);
    require(
        length > tolerance
            && length.is_finite()
            && length_diff(
                far.origin,
                std::array::from_fn(|i| origin[i] + n[i] * length),
            ) <= error
            && length_diff(far.normal, n) <= 1e-10,
    )?;
    let radius = |g: &Grid| crate::step_read::length(sub(g.start, origin));
    grids.sort_by(|a, b| radius(b).total_cmp(&radius(a)));
    let major = radius(&grids[0]);
    require(major - radius(&grids[1]) > error)?;
    let crest = grids[0];
    let delta = sub(crest.start, origin);
    let dz = dot(delta, n);
    let u = unit(std::array::from_fn(|i| delta[i] - n[i] * dz))?;
    let v = cross(n, u);
    let frame = [origin, u, v];
    let chart = Chart::new(frame)?;
    let a = chart.local(crest.start);
    let b = chart.local(crest.axial_next);
    let handed = a[0] * (b[1] - a[1]) - a[1] * (b[0] - a[0]);
    require(handed.is_finite() && handed != 0.)?;
    let hand = handed.signum();
    let spin = crest.axial_domain;
    require(spin[0] == 0. && spin[1] > 0. && grids.iter().all(|g| g.axial_domain == spin))?;
    let rate = length / spin[1];
    let pitch = rate * TAU;
    let phase = |g: &Grid| {
        let p = chart.local(g.start);
        (-hand * p[1].atan2(p[0])).rem_euclid(TAU)
    };
    grids[1..].sort_by(|a, b| phase(a).total_cmp(&phase(b)));
    require(grids[0].id == crest.id && grids[0].radial_domain[0] == 0.)?;
    let angle = FRAC_PI_2 - grids[0].radial_domain[1];
    let circle_radius = |g: &Grid| {
        let p = chart.local(g.start);
        let q = chart.local(g.radial_next);
        let factor = DEGREE as f64 / g.radial_span;
        let angular =
            (p[0] * (q[1] - p[1]) - p[1] * (q[0] - p[0])) * factor / (p[0] * p[0] + p[1] * p[1]);
        rate * angular.abs() / g.radial_domain[0].cos()
    };
    let rc = circle_radius(&grids[0]);
    let rr = circle_radius(&grids[2]);
    let depth = pitch / (2. * angle.tan()) - (rc + rr) * (1. / angle.sin() - 1.);
    let thread = Thread::new(Spec {
        major_radius_mm: major,
        pitch_mm: pitch,
        length_mm: length,
        depth_mm: depth,
        crest_radius_mm: rc,
        root_radius_mm: rr,
        flank_angle_deg: angle.to_degrees(),
        starts: 1,
        left_hand: hand < 0.,
    })
    .map_err(|e| match e {
        crate::rounded_thread::Error::Limit => ReadError::Limit,
        _ => ReadError::Invalid,
    })?;
    let volume = verify_member(shell, id, &thread, frame, Some(error))?;
    let mut bounds = [[0.; 3]; 2];
    let normal = cross(u, v);
    for (end, sign) in [-1., 1.].into_iter().enumerate() {
        for axis in 0..3 {
            let direction = [u[axis] * sign, v[axis] * sign, normal[axis] * sign];
            let support = thread
                .support_bounds(direction, tolerance / 8.)
                .map_err(|_| ReadError::Limit)?[1];
            let value = origin[axis] + sign * (support + tolerance / 4.);
            bounds[end][axis] = if end == 0 {
                value.next_down()
            } else {
                value.next_up()
            };
        }
    }
    Ok((
        Readback {
            bounds_mm: bounds,
            volume_mm3: volume,
            face_count: 7,
            edges_mm: Vec::new(),
            circles: Vec::new(),
            semicircles: Vec::new(),
            arcs: Vec::new(),
            ellipses: Vec::new(),
            splines: Vec::new(),
            solid_count: 1,
        },
        ImportedRoundedThread {
            thread,
            origin_mm: origin,
            u,
            v,
        },
    ))
}
fn length_diff(a: Point, b: Point) -> f64 {
    length(sub(a, b))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parameter_domains_keep_scalar_order_and_resource_failures_distinct() {
        assert_eq!(domain(&Value::Number(1.)), Err(ReadError::Invalid));
        assert_eq!(domain(&Value::List(vec![])), Err(ReadError::Invalid));
        assert_eq!(
            domain(&Value::List(vec![Value::Enum("T"), Value::Number(1.)])),
            Err(ReadError::Invalid)
        );
        assert_eq!(
            domain(&Value::List(vec![Value::Number(1.), Value::Number(1.)])),
            Err(ReadError::Invalid)
        );
        assert_eq!(
            domain(&Value::List(
                (0..1026).map(|i| Value::Number(i as f64)).collect()
            )),
            Err(ReadError::Limit)
        );
        assert_eq!(
            domain(&Value::List(vec![Value::Number(0.), Value::Number(1.)])),
            Ok(([0., 1.], 1.))
        );
    }
}
