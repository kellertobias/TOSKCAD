use super::*;
use crate::step::{Writer, write_warped_union};
use std::f64::consts::PI;

const TOL: f64 = 1e-7;
const ORIGIN: Point = [13., -7., 42.];
const U: Point = [0.8, 0.6, 0.];
const V: Point = [-0.36, 0.48, 0.8];

fn fixture() -> warped_union::Shell {
    warped_union::circular_miter([[0.; 3], [0., 0., 20.], [16., 0., 32.]], 2., TOL).unwrap()
}

#[test]
fn replays_a_placed_nonplanar_chain_from_either_cap_and_shuffled_faces() {
    let path = [
        [0.; 3],
        [0., 0., 9.],
        [0., 0., 20.],
        [16., 0., 32.],
        [16., 18., 32.],
        [30., 18., 32.],
    ];
    let expected = warped_union::circular_miter_chain(&path, 2., TOL).unwrap();
    let source = write_warped_union(&expected, ORIGIN, U, V).unwrap();
    let (readback, _) = read_import_solid(&source, TOL).unwrap();
    assert_eq!(readback.solid_count, 1);
    assert_eq!(readback.face_count, 22);
    assert_eq!(readback.ellipses.len(), 12);
    assert_eq!(readback.arcs.len(), 12);
    assert_eq!(readback.edges_mm.len(), 20);
    assert!((readback.volume_mm3 - 288. * PI).abs() < TOL);

    let entities = parse_entities(&source, TOL).unwrap();
    let mut reader = Shell::new(&entities, TOL, true);
    let id = reader.represented_solids(true).unwrap()[0];
    let boundary = Boundary::parse(&mut reader, id).unwrap();
    let forward = reconstruct(&reader, &boundary).unwrap().unwrap();
    let mut reordered = boundary.clone();
    reordered.faces.reverse();
    reordered.faces[1..21].rotate_left(6);
    for (i, face) in reordered.faces.iter_mut().enumerate() {
        face.loop_.rotate_left(i % 4);
    }
    let reverse = reconstruct(&reader, &reordered).unwrap().unwrap();
    assert!(length(sub(forward.origin_mm, reverse.origin_mm)) > 20.);
    let rewritten =
        write_warped_union(&reverse.shell, reverse.origin_mm, reverse.u, reverse.v).unwrap();
    let verified = read_analytic_solid(&rewritten, TOL).unwrap();
    assert_eq!(verified.face_count, 22);
    assert_eq!(verified.ellipses.len(), 12);
    assert!((verified.volume_mm3 - readback.volume_mm3).abs() < TOL);
    for (a, b) in verified.bounds_mm.into_iter().zip(readback.bounds_mm) {
        assert!(length(sub(a, b)) < TOL);
    }

    let (rims, edge_rim) = rims(&boundary, path.len()).unwrap();
    let (chain, _) = centerline(&boundary, &rims, &edge_rim).unwrap();
    let [a, b, _, _, y, z] = <[_; 6]>::try_from(chain).unwrap();
    let reroute = |changed: &mut Boundary, fixed, from, to| {
        let target_edge = *edge_rim.iter().find(|&(_, &rim)| rim == to).unwrap().0;
        for face in &mut changed.faces {
            if matches!(face.surface, Surface::Cylinder { .. })
                && face
                    .loop_
                    .iter()
                    .any(|(edge, _)| edge_rim.get(edge) == Some(&fixed))
                && let Some(usage) = face
                    .loop_
                    .iter_mut()
                    .find(|(edge, _)| edge_rim.get(edge) == Some(&from))
            {
                usage.0 = target_edge;
            }
        }
    };
    let mut changed = boundary.clone();
    reroute(&mut changed, a, b, z);
    assert!(reconstruct(&reader, &changed).unwrap().is_none());
    // Close the inner rims into a cycle disconnected from the two caps.
    // Every individual rim now has the right degree, but the shell's
    // cylinder incidence must still form one connected path.
    reroute(&mut changed, y, z, b);
    assert!(reconstruct(&reader, &changed).unwrap().is_none());
}

#[test]
fn replays_a_rotated_miter_as_one_solid_with_analytic_trims() {
    let expected = fixture();
    let source = write_warped_union(&expected, ORIGIN, U, V).unwrap();
    let (readback, imported) = read_import_solid(&source, TOL).unwrap();
    let ImportProfile::WarpedUnion { shell } = imported.profile else {
        panic!("expected an analytic miter")
    };
    assert_eq!(readback.solid_count, 1);
    assert_eq!(readback.face_count, 10);
    assert_eq!(readback.ellipses.len(), 4);
    assert_eq!(readback.arcs.len(), 8);
    assert_eq!(readback.edges_mm.len(), 8);
    assert!((readback.volume_mm3 - 160. * PI).abs() < TOL);
    let area = shell.faces().iter().map(|face| face.area_mm2).sum::<f64>();
    assert!((area - 168. * PI).abs() < TOL);
    for arc in &readback.ellipses {
        let [major, minor] = arc.ellipse().semiaxes_mm();
        // Incoming and outgoing axes have cosine 3/5. The bisector makes
        // cos(half turn) = sqrt(4/5), so the major semiaxis is sqrt(5).
        assert!((major - 5_f64.sqrt()).abs() < TOL);
        assert!((minor - 2.).abs() < TOL);
    }

    let rewritten = write_warped_union(&shell, imported.origin_mm, imported.u, imported.v).unwrap();
    let verified = read_analytic_solid(&rewritten, TOL).unwrap();
    assert_eq!(verified.solid_count, 1);
    assert!((verified.volume_mm3 - readback.volume_mm3).abs() < TOL);
    for (a, b) in verified.bounds_mm.into_iter().zip(readback.bounds_mm) {
        assert!(length(sub(a, b)) < TOL);
    }

    let mut writer = Writer::new();
    writer.append_warped_union(&expected, ORIGIN, U, V).unwrap();
    writer
        .append_box([crate::Rational::new(4, 1).unwrap(); 3], [2., 3., 4.], U, V)
        .unwrap();
    let compound = writer.finish().unwrap();
    let members = read_import_compound(&compound, TOL).unwrap();
    assert_eq!(members.len(), 2);
    assert!(matches!(
        members[0].1.profile,
        ImportProfile::WarpedUnion { .. }
    ));
    assert!(matches!(
        members[1].1.profile,
        ImportProfile::Polygon { .. }
    ));
}

#[test]
fn rejects_a_cylindrical_support_that_no_longer_matches_the_miter() {
    let source = write_warped_union(&fixture(), ORIGIN, U, V).unwrap();
    let support = source
        .lines()
        .find(|line| line.contains("=CYLINDRICAL_SURFACE("))
        .unwrap();
    let changed = format!("{},2.25);", support.rsplit_once(',').unwrap().0);
    let damaged = source.replacen(support, &changed, 1);
    assert_eq!(
        read_warped_solid(&damaged, TOL).unwrap_err(),
        ReadError::Invalid
    );
    assert!(read_import_solid(&damaged, TOL).is_err());
}

#[test]
fn only_recognizes_the_two_disk_and_ellipse_boundary_recipe() {
    let source = write_warped_union(&fixture(), ORIGIN, U, V).unwrap();
    let entities = parse_entities(&source, TOL).unwrap();
    let mut reader = Shell::new(&entities, TOL, true);
    let id = reader.represented_solids(true).unwrap()[0];
    let boundary = Boundary::parse(&mut reader, id).unwrap();
    let mut reversed = boundary.clone();
    reversed.faces.reverse();
    let from_start = reconstruct(&reader, &boundary).unwrap().unwrap();
    let from_end = reconstruct(&reader, &reversed).unwrap().unwrap();
    assert!(length(sub(from_start.origin_mm, from_end.origin_mm)) > 20.);
    assert!((from_start.shell.volume_mm3() - from_end.shell.volume_mm3()).abs() < TOL);
    let cap = boundary
        .faces
        .iter()
        .position(|face| matches!(face.surface, Surface::Plane { .. }))
        .unwrap();
    let cap_edge = boundary.faces[cap].loop_[0].0;
    let join_edges: Vec<_> = boundary
        .curves
        .iter()
        .filter_map(|(&id, &(_, curve))| {
            let TrimCurve::Arc(arc) = curve else {
                return None;
            };
            let [major, minor] = arc.ellipse().semiaxes_mm();
            (major > minor).then_some(id)
        })
        .collect();

    let mut changed = boundary.clone();
    changed.faces[cap].loop_.pop();
    assert!(reconstruct(&reader, &changed).unwrap().is_none());
    let mut changed = boundary.clone();
    changed.faces[cap].surface = Surface::Cylinder {
        origin: ORIGIN,
        axis: U,
        radius: 2.,
        outward: true,
    };
    assert!(reconstruct(&reader, &changed).unwrap().is_none());
    let mut changed = boundary.clone();
    changed.curves.get_mut(&cap_edge).unwrap().1 = TrimCurve::Line;
    assert!(reconstruct(&reader, &changed).unwrap().is_none());
    let mut changed = boundary.clone();
    changed.curves.get_mut(&join_edges[0]).unwrap().1 = TrimCurve::Line;
    assert!(reconstruct(&reader, &changed).unwrap().is_none());
    for edge in join_edges {
        changed.curves.get_mut(&edge).unwrap().1 = TrimCurve::Line;
    }
    assert!(reconstruct(&reader, &changed).unwrap().is_none());
}

#[test]
fn refuses_broken_rim_and_leg_incidence_even_with_plausible_record_counts() {
    let source = write_warped_union(&fixture(), ORIGIN, U, V).unwrap();
    let entities = parse_entities(&source, TOL).unwrap();
    let mut reader = Shell::new(&entities, TOL, true);
    let id = reader.represented_solids(true).unwrap()[0];
    let boundary = Boundary::parse(&mut reader, id).unwrap();
    let caps: Vec<_> = boundary
        .faces
        .iter()
        .enumerate()
        .filter_map(|(index, face)| matches!(face.surface, Surface::Plane { .. }).then_some(index))
        .collect();
    let [start, end] = caps.as_slice() else {
        panic!("expected two caps");
    };
    let [start, end] = [*start, *end];
    let cylinder = boundary
        .faces
        .iter()
        .position(|face| matches!(face.surface, Surface::Cylinder { .. }))
        .unwrap();
    let cap_loop = &boundary.faces[start].loop_;
    let cap_edge = cap_loop[0].0;
    let (rims, edge_rim) = rims(&boundary, 3).unwrap();
    let cap_rim = edge_rim[&cap_edge];
    let joint = rims.iter().position(|&rim| !is_circle(rim)).unwrap();
    let joint_edges: Vec<_> = edge_rim
        .iter()
        .filter_map(|(&edge, &rim)| (rim == joint).then_some(edge))
        .collect();
    let rail = boundary
        .curves
        .iter()
        .find_map(|(&id, &(_, curve))| matches!(curve, TrimCurve::Line).then_some(id))
        .unwrap();

    let mut changed = boundary.clone();
    changed.curves.get_mut(&cap_edge).unwrap().0[0] = u32::MAX;
    assert!(reconstruct(&reader, &changed).unwrap().is_none());
    let mut changed = boundary.clone();
    changed.curves.get_mut(&cap_edge).unwrap().1 = boundary.curves[&joint_edges[0]].1;
    assert!(reconstruct(&reader, &changed).unwrap().is_none());
    let mut changed = boundary.clone();
    let vertices: Vec<_> = cap_loop
        .iter()
        .map(|&(edge, forward)| boundary.curves[&edge].0[usize::from(!forward)])
        .collect();
    for (i, &(edge, _)) in cap_loop.iter().enumerate() {
        changed.curves.get_mut(&edge).unwrap().0 = [vertices[i / 2 * 2], vertices[i / 2 * 2 + 1]];
    }
    assert!(reconstruct(&reader, &changed).unwrap().is_none());

    let mut changed = boundary.clone();
    changed.faces[start].loop_[0].0 = rail;
    assert!(reconstruct(&reader, &changed).unwrap().is_none());
    let mut changed = boundary.clone();
    changed.faces[start].loop_[1].0 = joint_edges[0];
    assert!(reconstruct(&reader, &changed).unwrap().is_none());
    let mut changed = boundary.clone();
    for &(edge, _) in cap_loop {
        changed.curves.get_mut(&edge).unwrap().1 = boundary.curves[&joint_edges[0]].1;
    }
    assert!(reconstruct(&reader, &changed).unwrap().is_none());
    let mut changed = boundary.clone();
    changed.faces[end].loop_ = cap_loop.clone();
    assert!(reconstruct(&reader, &changed).unwrap().is_none());
    let mut changed = boundary.clone();
    changed.faces[cylinder].surface = boundary.faces[start].surface;
    changed.faces[cylinder].loop_ = cap_loop.clone();
    assert!(reconstruct(&reader, &changed).unwrap().is_none());

    let mut changed = boundary.clone();
    let sector = &mut changed.faces[cylinder].loop_;
    let arcs: Vec<_> = sector
        .iter()
        .enumerate()
        .filter_map(|(index, (edge, _))| edge_rim.contains_key(edge).then_some(index))
        .collect();
    sector[arcs[1]].0 = sector[arcs[0]].0;
    assert!(reconstruct(&reader, &changed).unwrap().is_none());

    let mut changed = boundary.clone();
    let sector = &mut changed.faces[cylinder].loop_;
    let start_edge = sector
        .iter_mut()
        .find(|(edge, _)| edge_rim.get(edge) == Some(&cap_rim))
        .unwrap();
    start_edge.0 = boundary.faces[end].loop_[0].0;
    assert!(reconstruct(&reader, &changed).unwrap().is_none());

    let mut changed = boundary.clone();
    for edge in joint_edges {
        changed.curves.get_mut(&edge).unwrap().1 = boundary.curves[&cap_edge].1;
    }
    assert!(reconstruct(&reader, &changed).unwrap().is_none());
}
