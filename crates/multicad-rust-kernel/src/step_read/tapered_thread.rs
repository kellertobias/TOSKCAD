//! Full finite tensor and shared trim proof for the Custom transport.
use super::helical::{face, ring};
use super::*;
use crate::tapered_thread::interval::I;
use crate::tapered_thread::{MANUFACTURING_DEGREE as D, Thread};

pub fn verify_tapered_thread(
    input: &str,
    thread: &Thread,
    origin: Point,
    u: Point,
    v: Point,
    tolerance: f64,
) -> Result<()> {
    let expected = crate::step::tapered_thread::source_nets(thread, [origin, u, v], tolerance)
        .map_err(|e| {
            if e == crate::step::StepError::Limit {
                ReadError::Limit
            } else {
                ReadError::Unsupported
            }
        })?;
    verify_prepared(input, thread, [origin, u, v], tolerance, &expected)
}
pub(crate) fn verify_prepared(
    input: &str,
    thread: &Thread,
    frame: [Point; 3],
    tolerance: f64,
    expected: &crate::step::tapered_thread::Prepared,
) -> Result<()> {
    compound::read_members_by(input, tolerance, true, true, |shell, id, _| {
        verify_member(shell, id, thread, frame, expected)
    })?;
    Ok(())
}
fn require(ok: bool) -> Result<()> {
    if ok { Ok(()) } else { Err(ReadError::Invalid) }
}
struct EdgeDefinition {
    ends: [u32; 2],
    curve: u32,
    forward: bool,
}
fn edge_definition(shell: &Shell<'_, '_>, id: u32) -> Result<EdgeDefinition> {
    let [Value::String(_), a, b, curve, forward] = shell.args(id, "EDGE_CURVE")? else {
        return Err(ReadError::Invalid);
    };
    Ok(EdgeDefinition {
        ends: [reference(a)?, reference(b)?],
        curve: reference(curve)?,
        forward: sense(forward)?,
    })
}
pub(super) fn knots(mult: &Value<'_>, knots: &Value<'_>, spans: usize) -> Result<()> {
    let (Value::List(mult), Value::List(knots)) = (mult, knots) else {
        return Err(ReadError::Invalid);
    };
    require(mult.len() == spans + 1 && knots.len() == spans + 1)?;
    for i in 0..=spans {
        require(
            mult[i]
                == Value::Number(if i == 0 || i == spans {
                    (D + 1) as f64
                } else {
                    D as f64
                })
                && knots[i] == Value::Number(i as f64 / spans as f64),
        )?;
    }
    Ok(())
}
pub(super) fn net(shell: &Shell<'_, '_>, surface: u32) -> Result<(Vec<Vec<Point>>, [usize; 2])> {
    let [
        Value::String(_),
        Value::Number(ud),
        Value::Number(vd),
        Value::List(rows),
        Value::Enum("UNSPECIFIED"),
        Value::Enum("F"),
        Value::Enum("F"),
        Value::Enum("F"),
        um,
        vm,
        uk,
        vk,
        Value::Enum("UNSPECIFIED"),
    ] = shell.args(surface, "B_SPLINE_SURFACE_WITH_KNOTS")?
    else {
        return Err(ReadError::Invalid);
    };
    require(*ud == D as f64 && *vd == D as f64 && rows.len() > D && (rows.len() - 1) % D == 0)?;
    let points = rows
        .iter()
        .map(|row| {
            refs(row)?
                .into_iter()
                .map(|id| shell.point(id))
                .collect::<Result<Vec<_>>>()
        })
        .collect::<Result<Vec<_>>>()?;
    let width = points[0].len();
    require(width > D && (width - 1) % D == 0 && points.iter().all(|row| row.len() == width))?;
    let spans = [(rows.len() - 1) / D, (width - 1) / D];
    knots(um, uk, spans[0])?;
    knots(vm, vk, spans[1])?;
    Ok((points, spans))
}
fn curve(shell: &mut Shell<'_, '_>, edge: u32, expected: &[Point], spans: usize) -> Result<()> {
    let edge = edge_definition(shell, edge)?;
    require(edge.forward)?;
    let [a, b] = edge.ends;
    let c = edge.curve;
    let [
        Value::String(_),
        Value::Number(d),
        points,
        Value::Enum("UNSPECIFIED"),
        Value::Enum("F"),
        Value::Enum("F"),
        mult,
        k,
        Value::Enum("UNSPECIFIED"),
    ] = shell.args(c, "B_SPLINE_CURVE_WITH_KNOTS")?
    else {
        return Err(ReadError::Invalid);
    };
    require(*d == D as f64)?;
    knots(mult, k, spans)?;
    let points = refs(points)?
        .into_iter()
        .map(|id| shell.point(id))
        .collect::<Result<Vec<_>>>()?;
    require(points == expected && a != b)?;
    require(shell.vertex(a)? == expected[0] && shell.vertex(b)? == expected[expected.len() - 1])
}
fn verify_member(
    shell: &mut Shell<'_, '_>,
    id: u32,
    thread: &Thread,
    frame: [Point; 3],
    expected: &crate::step::tapered_thread::Prepared,
) -> Result<()> {
    let faces = shell.member_faces(id)?;
    let n = thread.spec().starts as usize;
    require(faces.len() == 2 + 4 * n)?;
    let normal = unit(cross(frame[1], frame[2]))?;
    let mut matched = BTreeSet::new();
    let mut uses = BTreeMap::<u32, Vec<(usize, bool)>>::new();
    let mut ends = BTreeMap::<u32, [u32; 2]>::new();
    let mut wall_edges = vec![None::<[u32; 4]>; 3 * n];
    let mut planar = [false; 2];
    let mut cones = 0;
    let mut all_rings = Vec::new();
    for (fi, &face_id) in faces.iter().enumerate() {
        let (surface, forward, bounds) = face(shell, face_id)?;
        require(bounds.len() == 1)?;
        let edges = ring(shell, bounds[0], true)?;
        for &(edge, sense) in &edges {
            uses.entry(edge).or_default().push((fi, sense));
            ends.insert(edge, edge_definition(shell, edge)?.ends);
        }
        let name = shell.simple(surface)?.name;
        let mut cap_end = None;
        match name {
            "B_SPLINE_SURFACE_WITH_KNOTS" => {
                let (points, spans) = net(shell, surface)?;
                let flat = points.iter().flatten().copied().collect::<Vec<_>>();
                let candidates = expected
                    .points
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| **p == flat)
                    .map(|(i, _)| i)
                    .collect::<Vec<_>>();
                let [index] = candidates[..] else {
                    return Err(ReadError::Invalid);
                };
                require(
                    matched.insert(index)
                        && spans == expected.tensors.walls()[index % 3].spans()
                        && forward == thread.spec().left_hand
                        && edges.len() == 4,
                )?;
                let h = points[0].len();
                let w = points.len();
                let boundaries = [
                    (0..w).map(|i| points[i][0]).collect::<Vec<_>>(),
                    points[w - 1].clone(),
                    (0..w).map(|i| points[i][h - 1]).collect(),
                    points[0].clone(),
                ];
                let original = if thread.spec().left_hand {
                    edges.clone()
                } else {
                    edges.iter().rev().map(|&(id, s)| (id, !s)).collect()
                };
                wall_edges[index] = Some(std::array::from_fn(|i| original[i].0));
                for (i, &(edge, sense)) in original.iter().enumerate() {
                    require(sense == (i < 2))?;
                    curve(
                        shell,
                        edge,
                        &boundaries[i],
                        if i % 2 == 0 { spans[0] } else { spans[1] },
                    )?;
                }
            }
            "PLANE" => {
                require(forward)?;
                let [Value::String(_), axis] = shell.args(surface, "PLANE")? else {
                    return Err(ReadError::Invalid);
                };
                let plane = shell.placement(reference(axis)?)?;
                let end = usize::from(dot(plane.normal, normal) > 0.);
                require(!planar[end])?;
                planar[end] = true;
                cap_end = Some(end);
                let origin = std::array::from_fn(|i| {
                    frame[0][i]
                        + cross(frame[1], frame[2])[i]
                            * if end == 0 {
                                0.
                            } else {
                                thread.spec().length_mm
                            }
                });
                require(
                    length(sub(plane.origin, origin)) <= shell.tolerance / 16.
                        && length(sub(
                            plane.normal,
                            normal.map(|x| if end == 0 { -x } else { x }),
                        )) <= 1e-12,
                )?;
            }
            "CONICAL_SURFACE" | "CYLINDRICAL_SURFACE" => {
                require(forward)?;
                cones += 1;
            }
            _ => return Err(ReadError::Unsupported),
        }
        all_rings.push((name, surface, edges, cap_end));
    }
    require(matched.len() == 3 * n && planar == [true, true] && cones == n)?;
    for use_ in uses.values() {
        require(use_.len() == 2 && use_[0].0 != use_[1].0 && use_[0].1 != use_[1].1)?;
    }
    for (_, _, ring, _) in &all_rings {
        for (a, b) in ring
            .iter()
            .zip(ring.iter().cycle().skip(1))
            .take(ring.len())
        {
            let ae = ends[&a.0];
            let be = ends[&b.0];
            require(ae[usize::from(a.1)] == be[usize::from(!b.1)])?;
        }
    }
    let wall_edges = wall_edges
        .into_iter()
        .map(|e| e.ok_or(ReadError::Invalid))
        .collect::<Result<Vec<_>>>()?;
    let mut gaps = vec![None::<[[u32; 2]; 2]>; n];
    // Match each finite root sector to the actual adjacent source starts,
    // rather than accepting an arbitrary pair from the global rail set.
    for &(name, surface, ref edges, _) in &all_rings {
        if name != "CONICAL_SURFACE" && name != "CYLINDRICAL_SURFACE" {
            continue;
        }
        check_cone(shell, surface, thread, frame)?;
        require(edges.len() == 6)?;
        let edges = orient(edges, !thread.spec().left_hand);
        let candidates = (0..n)
            .filter_map(|start| {
                let a = wall_edges[start * 3 + 2][1];
                let b = wall_edges[((start + 1) % n) * 3][3];
                edges
                    .iter()
                    .position(|e| *e == (b, true))
                    .filter(|&i| edges[(i + 3) % 6] == (a, false))
                    .map(|i| (start, i))
            })
            .collect::<Vec<_>>();
        let [(start, i)] = candidates[..] else {
            return Err(ReadError::Invalid);
        };
        require(gaps[start].is_none())?;
        let bottom = [edges[(i + 4) % 6], edges[(i + 5) % 6]];
        let top = [edges[(i + 2) % 6], edges[(i + 1) % 6]];
        for (end, arcs) in [(0, bottom), (1, top)] {
            for (edge, sense) in arcs {
                require(sense == (end == 0) && check_circle(shell, edge, thread, frame)? == end)?;
            }
        }
        gaps[start] = Some([bottom.map(|e| e.0), top.map(|e| e.0)]);
    }
    let gaps = gaps
        .into_iter()
        .map(|gap| gap.ok_or(ReadError::Invalid))
        .collect::<Result<Vec<_>>>()?;
    for &(name, _, ref edges, end) in &all_rings {
        if name != "PLANE" {
            continue;
        }
        let end = end.ok_or(ReadError::Invalid)?;
        let mut expected = Vec::new();
        for start in 0..n {
            expected.extend((0..3).map(|band| (wall_edges[start * 3 + band][end * 2], true)));
            expected.extend(gaps[start][end].map(|edge| (edge, true)));
        }
        let actual = orient(edges, thread.spec().left_hand != (end == 1));
        require(cyclic_equal(&actual, &expected))?;
    }
    Ok(())
}
fn orient(edges: &[(u32, bool)], reverse: bool) -> Vec<(u32, bool)> {
    if reverse {
        edges.iter().rev().map(|&(id, s)| (id, !s)).collect()
    } else {
        edges.to_vec()
    }
}
fn cyclic_equal(a: &[(u32, bool)], b: &[(u32, bool)]) -> bool {
    a.len() == b.len()
        && !a.is_empty()
        && a.iter()
            .position(|e| *e == b[0])
            .is_some_and(|offset| (0..b.len()).all(|i| a[(offset + i) % a.len()] == b[i]))
}
fn check_cone(
    shell: &Shell<'_, '_>,
    surface: u32,
    thread: &Thread,
    frame: [Point; 3],
) -> Result<()> {
    let k = thread.radial_taper();
    require(
        shell.simple(surface)?.name
            == if k == 0. {
                "CYLINDRICAL_SURFACE"
            } else {
                "CONICAL_SURFACE"
            },
    )?;
    let z = if k < 0. { thread.spec().length_mm } else { 0. };
    let args = &shell.simple(surface)?.args;
    let (axis, radius, angle) = match args.as_slice() {
        [Value::String(_), axis, Value::Number(r), Value::Number(a)] if k != 0. => (axis, *r, *a),
        [Value::String(_), axis, Value::Number(r)] if k == 0. => (axis, *r, 0.),
        _ => return Err(ReadError::Invalid),
    };
    let actual = shell.placement(reference(axis)?)?;
    let normal = cross(frame[1], frame[2]);
    let origin = std::array::from_fn(|i| frame[0][i] + normal[i] * z);
    let r = thread.root_radius_at(z).map_err(|_| ReadError::Invalid)?;
    require(
        radius >= r[0]
            && radius <= r[1]
            && angle == k.abs().atan()
            && length(sub(actual.origin, origin)) <= shell.tolerance / 16.
            && length(sub(
                actual.normal,
                unit(normal)?.map(|v| if k < 0. { -v } else { v }),
            )) <= 1e-12,
    )
}
fn check_circle(
    shell: &mut Shell<'_, '_>,
    edge: u32,
    thread: &Thread,
    frame: [Point; 3],
) -> Result<usize> {
    let edge = edge_definition(shell, edge)?;
    let [a, b] = edge.ends;
    let c = edge.curve;
    let sense = edge.forward;
    require(sense == thread.spec().left_hand)?;
    let [Value::String(_), axis, Value::Number(radius)] = shell.args(c, "CIRCLE")? else {
        return Err(ReadError::Invalid);
    };
    let radius = *radius;
    let axis = reference(axis)?;
    let [Value::String(_), _, raw_normal, _] = shell.args(axis, "AXIS2_PLACEMENT_3D")? else {
        return Err(ReadError::Invalid);
    };
    let [Value::String(_), raw_normal] = shell.args(reference(raw_normal)?, "DIRECTION")? else {
        return Err(ReadError::Invalid);
    };
    let raw_normal = numbers(raw_normal)?.map(I::point);
    let plane = shell.placement(axis)?;
    let normal = unit(cross(frame[1], frame[2]))?;
    require(length(sub(plane.normal, normal)) <= 1e-12)?;
    let z = dot(sub(plane.origin, frame[0]), normal);
    let end = if z.abs() <= shell.tolerance / 16. {
        0.
    } else if (z - thread.spec().length_mm).abs() <= shell.tolerance / 16. {
        thread.spec().length_mm
    } else {
        return Err(ReadError::Invalid);
    };
    let expected = thread.root_radius_at(end).map_err(|_| ReadError::Invalid)?;
    let center = std::array::from_fn(|i| frame[0][i] + cross(frame[1], frame[2])[i] * end);
    require(length(sub(plane.origin, center)) <= shell.tolerance / 16.)?;
    require(radius >= expected[0] && radius <= expected[1])?;
    let mut points = Vec::new();
    for vertex in [a, b] {
        let p = shell.vertex(vertex)?;
        points.push(p);
        let d = sub(p, plane.origin);
        require(
            dot(d, normal).abs() <= shell.tolerance / 16.
                && (length(d) - radius).abs() <= shell.tolerance / 16.,
        )?;
    }
    let radial = points
        .iter()
        .map(|&p| std::array::from_fn::<_, 3, _>(|i| I::point(p[i]) - I::point(plane.origin[i])))
        .collect::<Vec<_>>();
    let [a, b] = [radial[0], radial[1]];
    let cross = [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ];
    let sine = cross
        .into_iter()
        .zip(raw_normal)
        .fold(I::point(0.), |s, (a, b)| s + a * b)
        * I::point(if sense { 1. } else { -1. });
    // Every root gap consists of two short arcs, never a long-arc guess.
    require(sine.valid() && sine.lo > 0.)?;
    Ok(usize::from(end != 0.))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Thread {
        Thread::new(
            crate::helical_thread::Spec {
                major_radius_mm: 4.,
                pitch_mm: 1.25,
                length_mm: 8.,
                depth_mm: 0.5,
                crest_width_mm: 0.25,
                lower_flank_deg: 15.,
                upper_flank_deg: 15.,
                starts: 1,
                left_hand: false,
                root_radius_mm: None,
            },
            0.1,
            1e-7,
            1e-6,
        )
        .unwrap()
    }
    fn entity(input: &str, kind: &str, change: impl FnOnce(&str) -> String) -> String {
        let line = input
            .lines()
            .find(|s| s.contains(&format!("={kind}(")))
            .unwrap();
        input.replacen(line, &change(line), 1)
    }
    fn argument(line: &str, index: usize) -> std::ops::Range<usize> {
        let start = line.find('(').unwrap() + 1;
        let end = line.rfind(')').unwrap();
        let mut starts = vec![start];
        let mut depth = 0;
        let mut quoted = false;
        for (i, c) in line[start..end].char_indices() {
            match c {
                '\'' => quoted = !quoted,
                '(' if !quoted => depth += 1,
                ')' if !quoted => depth -= 1,
                ',' if !quoted && depth == 0 => starts.push(start + i + 1),
                _ => {}
            }
        }
        let finish = starts.get(index + 1).map_or(end, |i| i - 1);
        starts[index]..finish
    }
    fn replace_argument(line: &str, index: usize, value: &str) -> String {
        let r = argument(line, index);
        format!("{}{}{}", &line[..r.start], value, &line[r.end..])
    }
    fn by_id(input: &str, id: u32, change: impl FnOnce(&str) -> String) -> String {
        let line = input
            .lines()
            .find(|line| line.starts_with(&format!("#{id}=")))
            .unwrap();
        input.replacen(line, &change(line), 1)
    }
    #[test]
    fn known_source_readback_requires_finite_nets_supports_and_a_closed_oriented_incidence_graph() {
        let thread = fixture();
        let o = [0.; 3];
        let u = [1., 0., 0.];
        let v = [0., 1., 0.];
        let text = crate::step::write_tapered_thread(&thread, o, u, v, 1e-7).unwrap();
        let proof = crate::step::tapered_thread::source_nets(&thread, [o, u, v], 1e-7).unwrap();
        let verify = |input: &str| verify_prepared(input, &thread, [o, u, v], 1e-7, &proof);
        verify(&text).unwrap();
        let entities = parse_entities(&text, 1e-7).unwrap();
        let shell = Shell::new(&entities, 1e-7, true);
        let ids = shell
            .member_faces(shell.represented_solids(true).unwrap()[0])
            .unwrap();
        let a = face(&shell, ids[0]).unwrap().0;
        let b = face(&shell, ids[1]).unwrap().0;
        let mut swapped = text.clone();
        for (id, from, to) in [(ids[0], a, b), (ids[1], b, a)] {
            let line = swapped
                .lines()
                .find(|line| line.starts_with(&format!("#{id}=")))
                .unwrap()
                .to_string();
            swapped = swapped.replacen(
                &line,
                &line.replacen(&format!(",#{from},"), &format!(",#{to},"), 1),
                1,
            );
        }
        assert_ne!(swapped, text);
        let changed = parse_entities(&swapped, 1e-7).unwrap();
        let changed = Shell::new(&changed, 1e-7, true);
        for id in ids {
            let before = face(&shell, id).unwrap();
            let after = face(&changed, id).unwrap();
            assert_eq!(before.1, after.1);
            assert_eq!(before.2, after.2);
            assert_eq!(
                ring(&shell, before.2[0], true).unwrap(),
                ring(&changed, after.2[0], true).unwrap()
            );
        }
        assert_eq!(verify(&swapped), Err(ReadError::Invalid));
        let changed = entity(&text, "CLOSED_SHELL", |s| {
            let begin = s.find(",(").unwrap() + 2;
            let end = s.rfind(")").unwrap() - 1;
            let list = s[begin..end].split(',').rev().collect::<Vec<_>>().join(",");
            format!("{}{}{}", &s[..begin], list, &s[end..])
        });
        verify(&changed).unwrap();
        let changed = entity(&text, "CLOSED_SHELL", |s| {
            let first = s.find(",(").unwrap() + 2;
            let end = first + s[first..].find(',').unwrap() + 1;
            format!("{}{}", &s[..first], &s[end..])
        });
        assert!(verify(&changed).is_err());
        for (old, new) in [
            (
                "B_SPLINE_SURFACE_WITH_KNOTS('',9,9",
                "B_SPLINE_SURFACE_WITH_KNOTS('',9,8",
            ),
            ("B_SPLINE_SURFACE_WITH_KNOTS", "TOROIDAL_SURFACE"),
            (
                "B_SPLINE_CURVE_WITH_KNOTS('',9",
                "B_SPLINE_CURVE_WITH_KNOTS('',8",
            ),
            ("B_SPLINE_CURVE_WITH_KNOTS", "LINE"),
            ("FACE_OUTER_BOUND", "FACE_BOUND"),
            ("CONICAL_SURFACE", "CYLINDRICAL_SURFACE"),
            ("CIRCLE", "ELLIPSE"),
        ] {
            let changed = text.replacen(old, new, 1);
            assert_ne!(changed, text);
            assert!(verify(&changed).is_err(), "accepted {old}→{new}");
        }
        for kind in [
            "B_SPLINE_SURFACE_WITH_KNOTS",
            "B_SPLINE_CURVE_WITH_KNOTS",
            "CIRCLE",
            "CONICAL_SURFACE",
            "PLANE",
            "EDGE_CURVE",
        ] {
            let changed = entity(&text, kind, |s| s.replacen("('',", "('',(),", 1));
            assert!(verify(&changed).is_err(), "accepted malformed {kind}");
        }
        let changed = entity(&text, "ADVANCED_FACE", |s| s.replacen(".T.)", ".F.)", 1));
        assert!(verify(&changed).is_err());
        let changed = entity(&text, "EDGE_CURVE", |s| s.replacen(".T.)", ".F.)", 1));
        assert!(verify(&changed).is_err());
        let changed = entity(&text, "CARTESIAN_POINT", |s| s.replacen("E0", "E1", 1));
        assert_ne!(changed, text);
        assert!(verify(&changed).is_err());
        assert_eq!(
            verify_tapered_thread(&text, &thread, [1e7; 3], u, v, 1e-7),
            Err(ReadError::Unsupported)
        );
        assert_eq!(
            verify_tapered_thread(&text, &thread, o, u, v, 1e-90),
            Err(ReadError::Limit)
        );
        verify(&text).unwrap();
    }
    #[test]
    fn malformed_finite_cap_supports_knots_and_long_circle_arcs_require_repair() {
        let thread = fixture();
        let frame = [[0.; 3], [1., 0., 0.], [0., 1., 0.]];
        let proof = crate::step::tapered_thread::source_nets(&thread, frame, 1e-7).unwrap();
        let text =
            crate::step::write_tapered_thread(&thread, frame[0], frame[1], frame[2], 1e-7).unwrap();
        let verify = |input: &str| verify_prepared(input, &thread, frame, 1e-7, &proof);
        let changed = entity(&text, "B_SPLINE_SURFACE_WITH_KNOTS", |line| {
            replace_argument(line, 8, ".F.")
        });
        assert!(verify(&changed).is_err());
        let changed = entity(&text, "B_SPLINE_SURFACE_WITH_KNOTS", |line| {
            let r = argument(line, 10);
            let old = &line[r];
            let changed = old.replacen("0.0000000000000000E0", "2.5000000000000000E-1", 1);
            assert_ne!(changed, old);
            replace_argument(line, 10, &changed)
        });
        assert!(verify(&changed).is_err());
        let entities = parse_entities(&text, 1e-7).unwrap();
        let mut shell = Shell::new(&entities, 1e-7, true);
        let ids = shell
            .member_faces(shell.represented_solids(true).unwrap()[0])
            .unwrap();
        let plane_surface = face(&shell, ids[0]).unwrap().0;
        let plane_axis = reference(&shell.args(plane_surface, "PLANE").unwrap()[1]).unwrap();
        let normal = reference(&shell.args(plane_axis, "AXIS2_PLACEMENT_3D").unwrap()[2]).unwrap();
        let changed = by_id(&text, normal, |line| {
            replace_argument(line, 1, "(0.,0.1,-0.99498743710662)")
        });
        assert!(verify(&changed).is_err());
        let wall = ids
            .iter()
            .copied()
            .find(|&id| {
                shell.simple(face(&shell, id).unwrap().0).unwrap().name
                    == "B_SPLINE_SURFACE_WITH_KNOTS"
            })
            .unwrap();
        let bound = face(&shell, wall).unwrap().2[0];
        let wire = reference(&shell.args(bound, "FACE_OUTER_BOUND").unwrap()[1]).unwrap();
        let changed = by_id(&text, wire, |line| {
            let r = argument(line, 1);
            let old = &line[r];
            let first = old[1..].split(',').next().unwrap();
            let new = format!("{},{}{}", &old[..old.len() - 1], first, ")");
            replace_argument(line, 1, &new)
        });
        assert!(verify(&changed).is_err());
        let circle = entities
            .iter()
            .find(|(_, v)| v.len() == 1 && v[0].name == "CIRCLE")
            .map(|(&id, _)| id)
            .unwrap();
        let axis = reference(&shell.args(circle, "CIRCLE").unwrap()[1]).unwrap();
        let direction = reference(&shell.args(axis, "AXIS2_PLACEMENT_3D").unwrap()[2]).unwrap();
        for id in [axis, direction] {
            let changed = by_id(&text, id, |line| line.replacen("('',", "('',(),", 1));
            assert!(verify(&changed).is_err());
        }
        let changed = by_id(&text, axis, |line| replace_argument(line, 1, "#900000"));
        let changed = changed.replacen(
            "ENDSEC;\nEND-ISO",
            "#900000=CARTESIAN_POINT('',(0.,0.,4.));\nENDSEC;\nEND-ISO",
            1,
        );
        assert_ne!(changed, text);
        assert!(verify(&changed).is_err());
        let edge = entities
            .iter()
            .filter(|(_, v)| v.len() == 1 && v[0].name == "EDGE_CURVE")
            .find(|(_, v)| reference(&v[0].args[3]).ok() == Some(circle))
            .map(|(&id, _)| id)
            .unwrap();
        let edge = edge_definition(&shell, edge).unwrap();
        let middle = edge.ends[1];
        let point = shell.vertex(middle).unwrap();
        let cartesian = reference(&shell.args(middle, "VERTEX_POINT").unwrap()[1]).unwrap();
        for p in [
            [point[0], point[1], point[2] + 0.01],
            [point[0] * 1.01, point[1] * 1.01, point[2]],
            [-point[0], -point[1], point[2]],
        ] {
            let changed = by_id(&text, cartesian, |line| {
                replace_argument(
                    line,
                    1,
                    &format!("({:.16E},{:.16E},{:.16E})", p[0], p[1], p[2]),
                )
            });
            assert!(verify(&changed).is_err());
        }
        verify(&text).unwrap();
    }
}
