use super::*;
use crate::step::write_warped_union;
use std::f64::consts::PI;

const TOL: f64 = 1e-7;
const ORIGIN: Point = [13., -7., 42.];
const U: Point = [0.8, 0.6, 0.];
const V: Point = [-0.36, 0.48, 0.8];

fn source() -> String {
    stadium_source(&[[0.; 3], [0., 0., 20.], [20., 0., 20.], [20., 20., 20.]])
}

fn stadium_source(path: &[Point]) -> String {
    let profile = [
        Edge::Line {
            a: [-3., -1.],
            b: [3., -1.],
        },
        Edge::Arc {
            center: [3., 0.],
            a: [3., -1.],
            b: [3., 1.],
            clockwise: false,
        },
        Edge::Line {
            a: [3., 1.],
            b: [-3., 1.],
        },
        Edge::Arc {
            center: [-3., 0.],
            a: [-3., 1.],
            b: [-3., -1.],
            clockwise: false,
        },
    ];
    let shell = warped_union::mixed_miter(&profile, path, TOL).unwrap();
    write_warped_union(&shell, ORIGIN, U, V).unwrap()
}

#[test]
fn preserves_analytic_trims_after_placement_and_reordered_face_uses() {
    let source = source();
    let (readback, imported) = read_import_solid(&source, TOL).unwrap();
    let ImportProfile::WarpedUnion { shell } = imported.profile else {
        panic!("expected a line-and-arc miter sweep");
    };
    assert_eq!(readback.solid_count, 1);
    assert_eq!(readback.face_count, 20);
    assert_eq!(readback.ellipses.len(), 8);
    assert_eq!(readback.arcs.len(), 8);
    assert!((readback.volume_mm3 - 60. * (12. + PI)).abs() < TOL);
    let area: f64 = shell.faces().iter().map(|face| face.area_mm2).sum();
    assert!((area - (2. * (12. + PI) + 60. * (12. + 2. * PI))).abs() < TOL);

    let entities = parse_entities(&source, TOL).unwrap();
    let mut reader = Shell::new(&entities, TOL, true);
    let id = reader.represented_solids(true).unwrap()[0];
    let mut boundary = Boundary::parse(&mut reader, id).unwrap();
    let from_start = reconstruct(&reader, &boundary).unwrap().unwrap();
    boundary.faces.reverse();
    let len = boundary.faces.len();
    boundary.faces[1..len - 1].rotate_left(3);
    for (i, face) in boundary.faces.iter_mut().enumerate() {
        let len = face.loop_.len();
        face.loop_.rotate_left(i % len);
    }
    let from_end = reconstruct(&reader, &boundary).unwrap().unwrap();
    assert!(length(sub(from_start.origin_mm, from_end.origin_mm)) > 20.);
    let rewritten =
        write_warped_union(&from_end.shell, from_end.origin_mm, from_end.u, from_end.v).unwrap();
    let verified = read_analytic_solid(&rewritten, TOL).unwrap();
    assert_eq!(verified.face_count, readback.face_count);
    assert_eq!(verified.ellipses.len(), readback.ellipses.len());
    assert!((verified.volume_mm3 - readback.volume_mm3).abs() < TOL);
    for (actual, expected) in verified.bounds_mm.into_iter().zip(readback.bounds_mm) {
        assert!(length(sub(actual, expected)) < TOL);
    }
}

#[test]
fn rejects_altered_step_wall_support() {
    let source = source();
    let support = source
        .lines()
        .find(|line| line.contains("=CYLINDRICAL_SURFACE("))
        .unwrap();
    let changed = format!("{},1.1);", support.rsplit_once(',').unwrap().0);
    let damaged = source.replacen(support, &changed, 1);
    assert_eq!(
        read_warped_solid(&damaged, TOL).unwrap_err(),
        ReadError::Invalid
    );
    assert!(read_import_solid(&damaged, TOL).is_err());
}

#[test]
fn retains_authored_straight_stations_without_elliptical_rims() {
    let source = stadium_source(&[[0.; 3], [0., 0., 10.], [0., 0., 20.]]);
    let imported = read_warped_solid(&source, TOL).unwrap();
    assert_eq!(imported.shell.faces().len(), 14);
    let (proof, _) = read_import_solid(&source, TOL).unwrap();
    assert_eq!(proof.face_count, 14);
    assert!(proof.ellipses.is_empty());
    assert!((proof.volume_mm3 - 20. * (12. + PI)).abs() < TOL);
}

#[test]
fn distinguishes_a_four_sector_lens_from_one_common_circle() {
    let h = 3_f64.sqrt();
    let profile = [
        Edge::Arc {
            center: [-1., 0.],
            a: [0., -h],
            b: [0., h],
            clockwise: false,
        },
        Edge::Arc {
            center: [1., 0.],
            a: [0., h],
            b: [0., -h],
            clockwise: false,
        },
    ];
    let path = [[0.; 3], [0., 0., 20.], [20., 0., 20.]];
    let shell = warped_union::mixed_miter(&profile, &path, TOL).unwrap();
    let source = write_warped_union(&shell, ORIGIN, U, V).unwrap();
    let (proof, imported) = read_import_solid(&source, TOL).unwrap();
    assert!(matches!(
        imported.profile,
        ImportProfile::WarpedUnion { .. }
    ));
    assert_eq!(proof.face_count, 10);
    assert_eq!(proof.ellipses.len(), 4);
    assert_eq!(proof.arcs.len(), 8);
    assert!((proof.volume_mm3 - 40. * (8. * PI / 3. - 2. * h)).abs() < TOL);
}

#[test]
fn accepts_an_arc_and_chord_cap_and_rejects_the_same_loop_with_two_lines() {
    let profile = [
        Edge::Arc {
            center: [0., 2.],
            a: [-1., 0.],
            b: [1., 0.],
            clockwise: false,
        },
        Edge::Line {
            a: [1., 0.],
            b: [-1., 0.],
        },
    ];
    let path = [[0.; 3], [0., 0., 20.], [20., 0., 20.]];
    let shell = warped_union::mixed_miter(&profile, &path, TOL).unwrap();
    let source = write_warped_union(&shell, ORIGIN, U, V).unwrap();
    let (proof, _) = read_import_solid(&source, TOL).unwrap();
    let area = 2.5 * (2. * 0.5_f64.atan() - 0.8);
    assert_eq!(proof.face_count, 6);
    assert!((proof.volume_mm3 - 40. * area).abs() < TOL);

    let mut edges = source.lines().filter(|line| line.contains("=EDGE_CURVE("));
    let arc = edges.next().unwrap();
    let chord = edges.next().unwrap();
    let arc_endpoints = arc.rsplitn(3, ',').nth(2).unwrap();
    let chord_line = chord.rsplit(',').nth(1).unwrap();
    // Reuse the chord's line support for the arc's opposite traversal.
    let damaged = source.replacen(arc, &format!("{arc_endpoints},{chord_line},.F.);"), 1);
    assert_eq!(
        read_warped_solid(&damaged, TOL).unwrap_err(),
        ReadError::Invalid
    );
}

#[test]
fn rejects_broken_rim_counts_and_generator_links() {
    let source = source();
    let entities = parse_entities(&source, TOL).unwrap();
    let mut reader = Shell::new(&entities, TOL, true);
    let id = reader.represented_solids(true).unwrap()[0];
    let boundary = Boundary::parse(&mut reader, id).unwrap();
    let caps: Vec<_> = boundary
        .faces
        .iter()
        .enumerate()
        .filter_map(|(i, face)| (face.loop_.len() == 6).then_some(i))
        .collect();
    let &[start, end] = caps.as_slice() else {
        panic!("expected two six-edge caps");
    };
    let initial: Vec<_> = boundary.faces[start]
        .loop_
        .iter()
        .rev()
        .map(|&(edge, forward)| (edge, !forward))
        .collect();
    let (wall, slot) = boundary
        .faces
        .iter()
        .enumerate()
        .find_map(|(wall, face)| {
            let slot = face.loop_.iter().position(|&side| side == initial[0])?;
            Some((wall, slot))
        })
        .unwrap();
    let rail = boundary.faces[wall].loop_[(slot + 1) % 4].0;
    let opposite = boundary.faces[wall].loop_[(slot + 2) % 4].0;
    let circular = boundary.faces[start]
        .loop_
        .iter()
        .find_map(|&(edge, _)| match boundary.curves[&edge].1 {
            TrimCurve::Arc(arc) => Some(arc),
            TrimCurve::Line => None,
        })
        .unwrap();

    let mut changed = boundary.clone();
    changed.faces[end].loop_.pop();
    assert!(generator(&changed, &initial, end).is_none());
    let mut changed = boundary.clone();
    changed.faces[wall].loop_.push(initial[0]);
    assert!(generator(&changed, &initial, end).is_none());
    assert!(reconstruct(&reader, &changed).unwrap().is_none());
    let mut changed = boundary.clone();
    changed.curves.get_mut(&rail).unwrap().1 = TrimCurve::Arc(circular);
    assert!(generator(&changed, &initial, end).is_none());
    let mut changed = boundary.clone();
    changed.curves.get_mut(&opposite).unwrap().0.reverse();
    assert!(generator(&changed, &initial, end).is_none());

    let vertices = generator(&boundary, &initial, end).unwrap();
    let original = reader.points[&vertices[1]];
    reader
        .points
        .insert(vertices[1], std::array::from_fn(|i| original[i] + U[i]));
    assert!(matches!(
        reconstruct(&reader, &boundary),
        Err(ReadError::Invalid)
    ));
}

#[test]
fn recognizer_requires_two_circular_or_mixed_caps_without_holes() {
    let source = source();
    let entities = parse_entities(&source, TOL).unwrap();
    let mut reader = Shell::new(&entities, TOL, true);
    let id = reader.represented_solids(true).unwrap()[0];
    let mut boundary = Boundary::parse(&mut reader, id).unwrap();
    let cap = boundary
        .faces
        .iter_mut()
        .find(|face| face.loop_.len() == 6)
        .unwrap();
    cap.holes.push(cap.loop_.clone());
    assert!(reconstruct(&reader, &boundary).unwrap().is_none());
    let cap = boundary
        .faces
        .iter_mut()
        .find(|face| face.loop_.len() == 6)
        .unwrap();
    cap.holes.clear();
    let edge = cap
        .loop_
        .iter()
        .find(|&&(edge, _)| matches!(boundary.curves[&edge].1, TrimCurve::Arc(_)))
        .unwrap()
        .0;
    let other = boundary
        .curves
        .values()
        .find_map(|&(_, curve)| match curve {
            TrimCurve::Arc(arc) if !circle(arc) => Some(curve),
            _ => None,
        })
        .unwrap();
    boundary.curves.get_mut(&edge).unwrap().1 = other;
    assert!(reconstruct(&reader, &boundary).unwrap().is_none());
}

#[test]
fn rejects_off_plane_cap_points_and_arc_support() {
    let source = source();
    let entities = parse_entities(&source, TOL).unwrap();
    let mut reader = Shell::new(&entities, TOL, true);
    let id = reader.represented_solids(true).unwrap()[0];
    let mut boundary = Boundary::parse(&mut reader, id).unwrap();
    let up = cross(U, V);
    let chart = Chart {
        origin: ORIGIN,
        u: U,
        v: V,
        up,
        band: Band {
            center_z_mm: 0.,
            radius_mm: 1.,
        },
    };
    let (&id, &([a, _], TrimCurve::Arc(arc))) = boundary
        .curves
        .iter()
        .find(|(_, (ends, curve))| {
            matches!(curve, TrimCurve::Arc(_))
                && ends
                    .iter()
                    .all(|id| chart.local(reader.points[id])[2].abs() < TOL)
        })
        .unwrap()
    else {
        panic!("expected a start cap arc");
    };
    let original = reader.points[&a];
    reader
        .points
        .insert(a, std::array::from_fn(|i| original[i] + up[i]));
    assert!(matches!(
        profile_edge(&reader, &boundary, (id, true), chart),
        Err(ReadError::Invalid)
    ));
    reader.points.insert(a, original);

    let tilted = Ellipse::new(arc.ellipse().center_mm(), U, V, [1.; 2]).unwrap();
    boundary.curves.get_mut(&id).unwrap().1 =
        TrimCurve::Arc(EllipseArc::new(tilted, 0., FRAC_PI_2).unwrap());
    assert!(matches!(
        profile_edge(&reader, &boundary, (id, true), chart),
        Err(ReadError::Invalid)
    ));
}
