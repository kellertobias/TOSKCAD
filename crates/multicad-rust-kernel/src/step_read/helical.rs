//! Readback proof for certified helical tensor surfaces in an authored solid.
use super::*;
use crate::{
    headed_screw::Screw,
    helical_thread::Thread,
    step::{
        StepError,
        helical::{Closure, DEGREE, Spline, prepare, product},
    },
};

mod hex_head;
mod socket_head;
mod tangent_shank;
mod volume;

type EdgeRef = (u32, bool);

/// Verify the serialized B-rep of a known analytic thread. The verifier
/// requires its complete shell, shared edge incidence and exact decoded
/// control net of the certified approximation. Returns the volume in mm³
/// integrated from the decoded spline surfaces and any cylinder wall.
pub fn verify_helical_thread(
    input: &str,
    thread: &Thread,
    stock_radius_mm: Option<f64>,
    origin: Point,
    u: Point,
    v: Point,
    tolerance: f64,
) -> Result<f64> {
    verify(
        input,
        thread,
        Closure::stock(stock_radius_mm),
        [origin, u, v],
        tolerance,
    )
}

/// Verify and integrate an authored analytic headed screw STEP.
pub fn verify_helical_screw(
    input: &str,
    screw: &Screw,
    origin: Point,
    u: Point,
    v: Point,
    tolerance: f64,
) -> Result<f64> {
    let closure = Closure::screw(screw).map_err(step_error)?;
    verify(input, screw.thread(), closure, [origin, u, v], tolerance)
}

pub(super) fn step_error(error: StepError) -> ReadError {
    match error {
        StepError::Limit => ReadError::Limit,
        StepError::Unsupported => ReadError::Unsupported,
        _ => ReadError::Invalid,
    }
}

pub(super) fn verify(
    input: &str,
    thread: &Thread,
    closure: Closure,
    [origin, u, v]: [Point; 3],
    tolerance: f64,
) -> Result<f64> {
    let splines = prepare(thread, closure, origin, tolerance).map_err(step_error)?;
    let normal = cross(u, v);
    let axis = unit(normal)?;
    let quadrature = volume::Quadrature::new();
    let world = |[x, y]: Point2, z: f64| {
        std::array::from_fn(|i| origin[i] + u[i] * x + v[i] * y + normal[i] * z)
    };
    let height = thread.spec().length_mm;
    let shank_length = closure.tangent_shank();
    let forward = thread.spec().left_hand != closure.inside();
    let mut volumes = compound::read_members_by(input, tolerance, true, true, |shell, id, _| {
        let faces = shell.member_faces(id)?;
        require(faces.len() == closure.face_count())?;
        let mut rings = [[0; 4]; 2];
        let mut rails = [[0; 2]; 4];
        let mut corners = [[[0; 2]; 2]; 4];
        let mut flux = 0.;
        for (i, radial) in splines.bands.iter().enumerate() {
            let (surface, outward, bounds) = face(shell, faces[i])?;
            let [bound] = bounds[..] else {
                return Err(ReadError::Invalid);
            };
            require(outward == forward)?;
            let [
                Value::String(_),
                Value::Number(d1),
                Value::Number(d2),
                grid,
                Value::Enum("UNSPECIFIED"),
                Value::Enum("F"),
                Value::Enum("F"),
                Value::Enum("F"),
                radial_mults,
                axial_mults,
                radial_knots,
                axial_knots,
                Value::Enum("UNSPECIFIED"),
            ] = shell.args(surface, "B_SPLINE_SURFACE_WITH_KNOTS")?
            else {
                return Err(ReadError::Invalid);
            };
            require(*d1 == DEGREE as f64 && *d2 == DEGREE as f64)?;
            knots(radial_mults, radial_knots, radial)?;
            knots(axial_mults, axial_knots, &splines.axial)?;
            let Value::List(rows) = grid else {
                return Err(ReadError::Invalid);
            };
            require(rows.len() == radial.poles.len())?;
            let controls = rows
                .iter()
                .zip(&radial.poles)
                .map(|(row, &radial)| {
                    poles(
                        shell,
                        row,
                        splines
                            .axial
                            .poles
                            .iter()
                            .zip(&splines.heights)
                            .map(|(&axial, &z)| world(product(radial, axial), z)),
                    )
                })
                .collect::<Result<Vec<_>>>()?;
            flux += quadrature.spline_flux(&controls, origin, axis);

            let uses = ring(shell, bound, true)?;
            let uses = orient(&uses, forward);
            let [(start, true), (next, true), (end, false), (previous, false)] = uses[..] else {
                return Err(ReadError::Invalid);
            };
            rings[0][i] = start;
            rings[1][i] = end;
            rails[i] = [previous, next];
            let [a, b] = [0, radial.poles.len() - 1];
            for (j, (rail, p)) in [(previous, radial.poles[a]), (next, radial.poles[b])]
                .into_iter()
                .enumerate()
            {
                corners[i][j] = spline_edge(
                    shell,
                    rail,
                    &splines.axial,
                    splines
                        .axial
                        .poles
                        .iter()
                        .zip(&splines.heights)
                        .map(|(&axial, &z)| world(product(p, axial), z)),
                )?;
            }
            for (cap, &rim) in [start, end].iter().enumerate() {
                if cap == 0 && i == 3 && shank_length.is_some() {
                    // The circular crest is verified with the shank wall.
                    continue;
                }
                let z = [0., height][cap];
                let axial = splines.axial.poles[if cap == 0 {
                    0
                } else {
                    splines.axial.poles.len() - 1
                }];
                let endpoints = spline_edge(
                    shell,
                    rim,
                    radial,
                    radial.poles.iter().map(|&p| world(product(p, axial), z)),
                )?;
                require(endpoints == [corners[i][0][cap], corners[i][1][cap]])?;
            }
        }
        require((0..4).all(|i| rails[i][1] == rails[(i + 1) % 4][0]))?;

        let mut cylinder_rims = [0; 2];
        let mut caps = [[0.; 3]; 2];
        let mut shoulder = None;
        let mut tangent = None;
        for (end, (face_id, z)) in faces[4..6].iter().zip([0., height]).enumerate() {
            let top = end == 1;
            let (surface, outward, bounds) = face(shell, *face_id)?;
            let hole = closure.cap_hole(top);
            let inner = matches!(closure, Closure::Hollow(_));
            require(outward && bounds.len() == 1 + usize::from(hole || inner))?;
            let [Value::String(_), axis] = shell.args(surface, "PLANE")? else {
                return Err(ReadError::Invalid);
            };
            let positive = top || closure.is_screw();
            let cap_axis = normal.map(|n| if positive { n } else { -n });
            caps[end] = check_axis(shell, axis, world([0.; 2], z), cap_axis, u)?.origin;
            let mut expected = rings[end].map(|edge| (edge, true));
            let actual = orient(
                &ring(shell, bounds[usize::from(hole || inner)], !hole)?,
                top == forward,
            );
            if !top && shank_length.is_some() {
                let [_, _, _, (complement, false)] = actual[..] else {
                    return Err(ReadError::Invalid);
                };
                expected[3] = (complement, false);
                tangent = Some(tangent_shank::Junction {
                    crest: rings[0][3],
                    complement,
                    corners: [corners[0][0][0], corners[3][0][0]],
                });
            } else if closure.is_screw() && !top {
                shoulder = Some(bounds[0]);
            }
            require(actual == expected)?;
            if let Some((radius, outer)) = closure.cylinder() {
                let rim = ring(shell, bounds[0], outer)?;
                let [(edge, sense)] = rim[..] else {
                    return Err(ReadError::Invalid);
                };
                require(sense == (top == outer))?;
                shell.edge(edge)?;
                let Curve::Circle(circle) = shell.edges[&edge].curve else {
                    return Err(ReadError::Invalid);
                };
                require(
                    circle.center_mm == world([0.; 2], z)
                        && length(sub(circle.normal, normal)) < 1e-10
                        && circle.radius_mm == radius,
                )?;
                cylinder_rims[end] = edge;
            }
        }
        let outside_volume = match closure {
            Closure::Shaft => 0.,
            Closure::Bore(radius) | Closure::Hollow(radius) => {
                let outer = matches!(closure, Closure::Bore(_));
                let (radius, axis) = check_cylinder(
                    shell,
                    faces[6],
                    cylinder_rims,
                    (radius, outer),
                    [origin, normal, u],
                )?;
                let sign = if outer { 1. } else { -1. };
                sign * std::f64::consts::PI * radius * radius * dot(sub(caps[1], caps[0]), axis)
            }
            Closure::HexHead {
                across_flats_mm,
                height_mm,
            } => hex_head::verify(
                shell,
                &faces[6..],
                shoulder.ok_or(ReadError::Invalid)?,
                caps[0],
                [u, normal],
                [across_flats_mm, height_mm],
                world,
            )?,
            Closure::SocketHead(head) => {
                let (rim, seat, shank_volume, offset) = if let Some(length) = shank_length {
                    let joint = tangent.ok_or(ReadError::Invalid)?;
                    let (rim, seat, volume) = tangent_shank::verify(
                        shell,
                        [faces[6], faces[7]],
                        joint,
                        thread,
                        length,
                        [origin, u, v],
                    )?;
                    (rim, seat, volume, 8)
                } else {
                    (shoulder.ok_or(ReadError::Invalid)?, caps[0], 0., 6)
                };
                let head_volume = socket_head::verify(
                    shell,
                    &faces[offset..],
                    rim,
                    seat,
                    [u, normal],
                    head,
                    world,
                )?;
                head_volume + shank_volume
            }
        };
        let volume = outside_volume + if forward { flux } else { -flux };
        require(volume.is_finite() && volume > 0.)?;
        Ok(volume)
    })?;
    volumes.pop().ok_or(ReadError::Invalid)
}

pub(super) fn require(condition: bool) -> Result<()> {
    condition.then_some(()).ok_or(ReadError::Invalid)
}

pub(super) fn orient(edges: &[EdgeRef], forward: bool) -> Vec<EdgeRef> {
    if forward {
        edges.to_vec()
    } else {
        edges.iter().rev().map(|&(id, same)| (id, !same)).collect()
    }
}

pub(super) fn face(shell: &Shell<'_, '_>, id: u32) -> Result<(u32, bool, Vec<u32>)> {
    let [Value::String(_), bounds, surface, forward] = shell.args(id, "ADVANCED_FACE")? else {
        return Err(ReadError::Invalid);
    };
    Ok((reference(surface)?, sense(forward)?, refs(bounds)?))
}

pub(super) fn ring(shell: &Shell<'_, '_>, id: u32, exterior: bool) -> Result<Vec<EdgeRef>> {
    let (outside, wire, forward) = shell.bound(id, false)?;
    require(outside == exterior)?;
    let [Value::String(_), edges] = shell.args(wire, "EDGE_LOOP")? else {
        return Err(ReadError::Invalid);
    };
    let edges = refs(edges)?
        .into_iter()
        .map(|id| {
            let [
                Value::String(_),
                Value::Derived,
                Value::Derived,
                edge,
                forward,
            ] = shell.args(id, "ORIENTED_EDGE")?
            else {
                return Err(ReadError::Invalid);
            };
            Ok((reference(edge)?, sense(forward)?))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(orient(&edges, forward))
}

fn knots(mult: &Value<'_>, knots: &Value<'_>, expected: &Spline) -> Result<()> {
    let (Value::List(mult), Value::List(knots)) = (mult, knots) else {
        return Err(ReadError::Invalid);
    };
    let len = expected.knots.len();
    require(mult.len() == len && knots.len() == len)?;
    for (i, ((mult, knot), &expected)) in mult.iter().zip(knots).zip(&expected.knots).enumerate() {
        let degree = DEGREE + usize::from(i == 0 || i + 1 == len);
        require(*mult == Value::Number(degree as f64) && *knot == Value::Number(expected))?;
    }
    Ok(())
}

pub(super) fn poles(
    shell: &Shell<'_, '_>,
    actual: &Value<'_>,
    expected: impl Iterator<Item = Point>,
) -> Result<Vec<Point>> {
    let actual = refs(actual)?;
    let mut points = Vec::with_capacity(actual.len());
    for (index, expected) in expected.enumerate() {
        let point = shell.point(*actual.get(index).ok_or(ReadError::Invalid)?)?;
        require(point == expected)?;
        points.push(point);
    }
    require(points.len() == actual.len())?;
    Ok(points)
}

fn spline_edge(
    shell: &mut Shell<'_, '_>,
    id: u32,
    spline: &Spline,
    points: impl Iterator<Item = Point>,
) -> Result<[u32; 2]> {
    let [Value::String(_), a, b, curve, Value::Enum("T")] = shell.args(id, "EDGE_CURVE")? else {
        return Err(ReadError::Invalid);
    };
    let [a, b] = [reference(a)?, reference(b)?];
    let [
        Value::String(_),
        Value::Number(d),
        controls,
        Value::Enum("UNSPECIFIED"),
        Value::Enum("F"),
        Value::Enum("F"),
        mult,
        segments,
        Value::Enum("UNSPECIFIED"),
    ] = shell.args(reference(curve)?, "B_SPLINE_CURVE_WITH_KNOTS")?
    else {
        return Err(ReadError::Invalid);
    };
    require(*d == DEGREE as f64 && a != b)?;
    knots(mult, segments, spline)?;
    let points = poles(shell, controls, points)?;
    let [start, end] = [points[0], points[points.len() - 1]];
    require(start == shell.vertex(a)? && end == shell.vertex(b)? && start != end)?;
    Ok([a, b])
}

pub(super) fn check_axis(
    shell: &Shell<'_, '_>,
    id: &Value<'_>,
    origin: Point,
    normal: Point,
    u: Point,
) -> Result<Plane> {
    let frame = shell.placement(reference(id)?)?;
    require(
        frame.origin == origin
            && length(sub(frame.normal, normal)) < 1e-10
            && length(sub(frame.u, u)) < 1e-10,
    )?;
    Ok(frame)
}

fn check_cylinder(
    shell: &mut Shell<'_, '_>,
    id: u32,
    rims: [u32; 2],
    (radius, outward): (f64, bool),
    [origin, normal, u]: [Point; 3],
) -> Result<(f64, Point)> {
    let (surface, forward, bounds) = face(shell, id)?;
    let [bound] = bounds[..] else {
        return Err(ReadError::Invalid);
    };
    require(forward == outward)?;
    let [Value::String(_), axis, Value::Number(r)] = shell.args(surface, "CYLINDRICAL_SURFACE")?
    else {
        return Err(ReadError::Invalid);
    };
    let measured_radius = *r;
    require(measured_radius == radius)?;
    let axis = check_axis(shell, axis, origin, normal, u)?.normal;
    let uses = ring(shell, bound, true)?;
    let [
        (bottom, bottom_sense),
        (seam, true),
        (top, top_sense),
        (repeated, false),
    ] = uses[..]
    else {
        return Err(ReadError::Invalid);
    };
    require(
        [bottom, top] == rims
            && seam == repeated
            && bottom_sense == outward
            && top_sense != outward,
    )?;
    let seam_ends = shell.edge(seam)?;
    require(
        matches!(shell.edges[&seam].curve, Curve::Line)
            && seam_ends
                == [
                    shell.edges[&bottom].vertices[0],
                    shell.edges[&top].vertices[0],
                ],
    )?;
    Ok((measured_radius, axis))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{helical_thread::Spec, step::write_helical_thread};

    fn rewrite(
        input: &str,
        kind: &str,
        ordinal: usize,
        change: impl FnOnce(&str) -> String,
    ) -> String {
        let (line, (id, body)) = input
            .lines()
            .filter_map(|line| {
                let (id, body) = line.split_once('=')?;
                body.starts_with(kind).then_some((line, (id, body)))
            })
            .nth(ordinal)
            .unwrap();
        input.replacen(line, &format!("{id}={}", change(body)), 1)
    }

    #[test]
    fn rejects_altered_advanced_shell_records() {
        let thread = Thread::new(Spec {
            major_radius_mm: 3.,
            pitch_mm: 1.,
            length_mm: 3.7,
            depth_mm: 5. * 3_f64.sqrt() / 16.,
            crest_width_mm: 0.125,
            lower_flank_deg: 30.,
            upper_flank_deg: 30.,
            starts: 1,
            left_hand: true,
            root_radius_mm: None,
        })
        .unwrap();
        let [origin, u, v] = [[0.; 3], [1., 0., 0.], [0., 1., 0.]];
        let radius = Some(4.5);
        let step = write_helical_thread(&thread, radius, origin, u, v, 1e-7).unwrap();
        let verify = |text: &str| verify_helical_thread(text, &thread, radius, origin, u, v, 1e-7);
        assert!(verify(&step).unwrap() > 0.);
        let many = Thread::new(Spec {
            starts: 2,
            ..thread.spec()
        })
        .unwrap();
        assert_eq!(
            verify_helical_thread(&step, &many, radius, origin, u, v, 1e-7),
            Err(ReadError::Unsupported)
        );
        assert_eq!(
            verify_helical_thread(&step, &thread, radius, origin, u, v, 1e-16),
            Err(ReadError::Limit)
        );
        assert_eq!(
            verify_helical_thread(&step, &thread, Some(1.), origin, u, v, 1e-7),
            Err(ReadError::Invalid)
        );

        let wrong_bounds = |body: &str| {
            let right = body.rsplit_once("),").unwrap().1;
            format!("ADVANCED_FACE('',(),{right}")
        };
        let wrong_surface_grid = |body: &str| {
            let suffix = body.split_once(",.UNSPECIFIED.").unwrap().1;
            format!("B_SPLINE_SURFACE_WITH_KNOTS('',11,11,$,.UNSPECIFIED.{suffix}")
        };
        let wrong_mult = |body: &str| {
            let (before, after) = body.split_once(",(12,").unwrap();
            let after = after.split_once("),").unwrap().1;
            format!("{before},$,{after}")
        };
        let wrong_loop_direction = |body: &str| {
            let (before, sense) = body.rsplit_once(',').unwrap();
            let opposite = if sense == ".T.);" { ".F." } else { ".T." };
            format!("{before},{opposite});")
        };
        let wrong_ring_type = |body: &str| body.replace("FACE_OUTER_BOUND", "FACE_BOUND");
        let only_seam_in_first_round_cap = |body: &str| {
            let seam = step
                .lines()
                .filter_map(|line| {
                    let (id, body) = line.split_once('=')?;
                    body.starts_with("EDGE_CURVE").then_some(id)
                })
                .nth(14)
                .unwrap();
            let direction = body.rsplit_once(',').unwrap().1;
            format!("ORIENTED_EDGE('',*,*,{seam},{direction}")
        };
        let surface_control = |text: &str| {
            let surface = text
                .lines()
                .find(|line| line.contains("=B_SPLINE_SURFACE_WITH_KNOTS"))
                .unwrap();
            let control = surface
                .split_once("((")
                .unwrap()
                .1
                .split_once(',')
                .unwrap()
                .0;
            let start = format!("{control}=CARTESIAN_POINT");
            let line = text.lines().find(|line| line.starts_with(&start)).unwrap();
            text.replacen(line, &format!("{start}('',(0.,0.,0.));"), 1)
        };
        let change_support_normal = |text: &str, kind: &str, axis_z: f64| {
            let support = text.lines().find(|line| line.contains(kind)).unwrap();
            let placement = support.split(',').nth(1).unwrap().trim_end_matches(");");
            let placement = text
                .lines()
                .find(|line| line.starts_with(&format!("{placement}=")))
                .unwrap();
            let normal = placement.split(',').nth(2).unwrap();
            let normal = format!("{normal}=DIRECTION");
            let line = text.lines().find(|line| line.starts_with(&normal)).unwrap();
            text.replacen(line, &format!("{normal}('',(0.,0.,{axis_z:.1}));"), 1)
        };
        for changed in [
            rewrite(&step, "ADVANCED_FACE", 0, wrong_bounds),
            rewrite(&step, "ADVANCED_FACE", 6, wrong_bounds),
            rewrite(&step, "ADVANCED_FACE", 0, |_| "ADVANCED_FACE('');".into()),
            rewrite(&step, "B_SPLINE_SURFACE_WITH_KNOTS", 0, wrong_surface_grid),
            rewrite(&step, "B_SPLINE_SURFACE_WITH_KNOTS", 0, wrong_mult),
            rewrite(&step, "B_SPLINE_SURFACE_WITH_KNOTS", 0, |body| {
                body.replacen(".F.,.F.,.F.", ".T.,.F.,.F.", 1)
            }),
            rewrite(&step, "ORIENTED_EDGE", 0, wrong_loop_direction),
            rewrite(&step, "ORIENTED_EDGE", 0, |_| "ORIENTED_EDGE('');".into()),
            rewrite(&step, "ORIENTED_EDGE", 26, wrong_loop_direction),
            rewrite(&step, "ORIENTED_EDGE", 20, only_seam_in_first_round_cap),
            rewrite(&step, "EDGE_LOOP", 0, |_| "EDGE_LOOP('');".into()),
            rewrite(&step, "EDGE_LOOP", 5, |_| "EDGE_LOOP('',());".into()),
            rewrite(&step, "EDGE_CURVE", 0, |_| "EDGE_CURVE('');".into()),
            rewrite(&step, "B_SPLINE_CURVE_WITH_KNOTS", 0, |_| {
                "B_SPLINE_CURVE_WITH_KNOTS('');".into()
            }),
            rewrite(&step, "B_SPLINE_CURVE_WITH_KNOTS", 4, |_| {
                "B_SPLINE_CURVE_WITH_KNOTS('');".into()
            }),
            rewrite(&step, "CYLINDRICAL_SURFACE", 0, |_| {
                "CYLINDRICAL_SURFACE('');".into()
            }),
            rewrite(&step, "CIRCLE", 0, |body| {
                let (left, _) = body.rsplit_once(',').unwrap();
                format!("{left},4.6);")
            }),
            rewrite(&step, "PLANE", 0, |_| "PLANE('');".into()),
            rewrite(&step, "FACE_OUTER_BOUND", 0, wrong_ring_type),
            rewrite(&step, "FACE_BOUND", 0, wrong_loop_direction),
            surface_control(&step),
            change_support_normal(&step, "=CIRCLE", -1.),
            change_support_normal(&step, "=PLANE", 1.),
            rewrite(&step, "EDGE_CURVE", 14, |body| {
                let fields = body.split(',').collect::<Vec<_>>();
                format!(
                    "EDGE_CURVE('',{},{},{},.F.);",
                    fields[2], fields[1], fields[3]
                )
            }),
        ] {
            assert_ne!(changed, step);
            assert!(verify(&changed).is_err());
        }
    }
}
