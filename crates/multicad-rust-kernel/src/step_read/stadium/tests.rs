use super::*;
use crate::step::write_stadium_prism;

const TOLERANCE: f64 = 1e-7;
const U: Point = [0.8, 0.6, 0.];
const V: Point = [-0.36, 0.48, 0.8];

fn near(a: f64, b: f64) {
    assert!((a - b).abs() < TOLERANCE, "{a} != {b}");
}

fn fixture(reverse: bool) -> (String, Stadium, Point) {
    let origin = [12_300., -7_800., 200.];
    let profile = Stadium::new([[2., 3.], [6., 9.]], 1.5).unwrap();
    let text = write_stadium_prism(&profile, 4., origin, U, V, reverse).unwrap();
    (text, profile, origin)
}

#[test]
fn standalone_stadium_readback_proves_exact_oblique_bounds_and_volume() {
    for reverse in [false, true] {
        let (text, profile, origin) = fixture(reverse);
        let (readback, solid) = read_import_solid(&text, TOLERANCE).unwrap();
        assert_eq!(readback.face_count, 6);
        assert_eq!(readback.edges_mm.len(), 8);
        assert_eq!(readback.semicircles.len(), 4);
        assert!(readback.circles.is_empty());
        near(readback.volume_mm3, profile.area_mm2() * 4.);
        let normal = cross(U, V);
        let travel = if reverse { -4. } else { 4. };
        for i in 0..3 {
            let sweep = normal[i] * travel;
            let planar = [U[i], V[i]];
            near(
                readback.bounds_mm[0][i],
                origin[i] + sweep.min(0.) - profile.support(planar.map(|x| -x)),
            );
            near(
                readback.bounds_mm[1][i],
                origin[i] + sweep.max(0.) + profile.support(planar),
            );
        }
        let ImportProfile::Stadium { profile: rebuilt } = solid.profile else {
            panic!("standalone stadium should reconstruct its exact profile");
        };
        near(rebuilt.area_mm2(), profile.area_mm2());
        near(rebuilt.radius_mm(), profile.radius_mm());
        near(rebuilt.spine_length_mm(), profile.spine_length_mm());
        near(solid.height_mm, 4.);
        assert!(read_planar_solid(&text, TOLERANCE).is_err());
    }
}

fn change_record(
    text: &str,
    entity: &str,
    index: usize,
    change: impl FnOnce(&str) -> String,
) -> String {
    let record = text
        .lines()
        .filter(|line| {
            line.split_once('=')
                .is_some_and(|(_, body)| body.starts_with(entity))
        })
        .nth(index)
        .unwrap();
    text.replacen(record, &change(record), 1)
}

#[test]
fn damaged_half_arc_and_half_cylinder_fail_independent_shell_verification() {
    let (text, _, _) = fixture(false);
    let circle = text.lines().find(|line| line.contains("=CIRCLE(")).unwrap();
    let (id, _) = circle.split_once('=').unwrap();
    let edge = text
        .lines()
        .find(|line| line.contains("=EDGE_CURVE(") && line.ends_with(&format!(",{id},.T.);")))
        .unwrap();
    let reverse_arc = text.replace(edge, &edge.replace(".T.", ".F."));
    let broken_radius = change_record(&text, "CYLINDRICAL_SURFACE", 0, |line| {
        let (args, _) = line.rsplit_once(',').unwrap();
        format!("{args},2.);")
    });
    let broken_wall_sense =
        change_record(&text, "ADVANCED_FACE", 3, |line| line.replace(".T.", ".F."));
    let broken_cap = change_record(&text, "EDGE_LOOP", 0, |line| {
        let (args, _) = line.rsplit_once(',').unwrap();
        format!("{args}));")
    });
    for invalid in [reverse_arc, broken_radius, broken_wall_sense, broken_cap] {
        assert_eq!(
            read_analytic_solid(&invalid, TOLERANCE),
            Err(ReadError::Invalid)
        );
    }
}

#[test]
fn stadium_planar_trim_uses_exact_capsule_clearance() {
    let stadium = |centers, radius| Trim::Stadium {
        profile: Stadium::new(centers, radius).unwrap(),
        ccw: true,
    };
    let disk = |center, radius| Trim::Circle {
        center,
        radius,
        ccw: true,
    };
    let polygon = |x, y, size| {
        Trim::Polygon(vec![
            [x, y],
            [x + size, y],
            [x + size, y + size],
            [x, y + size],
        ])
    };
    let outer = stadium([[2., 2.], [8., 2.]], 1.);
    let near_side = stadium([[4., 2.5], [6., 2.5]], 0.8);
    let inside = stadium([[3., 2.], [6., 2.]], 0.4);
    let clear = stadium([[4., 4.], [6., 4.]], 0.8);
    assert!(outer.strictly_contains(&inside, TOLERANCE));
    assert!(!outer.strictly_contains(&near_side, TOLERANCE));
    assert!(!outer.disjoint(&near_side, TOLERANCE));
    assert!(outer.disjoint(&clear, TOLERANCE));
    assert!(outer.strictly_contains(&disk([4., 2.], 0.5), TOLERANCE));
    assert!(!outer.strictly_contains(&disk([4., 2.5], 0.5), TOLERANCE));
    assert!(outer.disjoint(&disk([2., 4.], 0.9), TOLERANCE));
    assert!(!outer.disjoint(&disk([2., 4.], 1.), TOLERANCE));
    assert!(disk([5., 2.], 5.).strictly_contains(&outer, TOLERANCE));
    assert!(!disk([5., 2.], 4.).strictly_contains(&outer, TOLERANCE));
    assert!(outer.strictly_contains(&polygon(3., 2., 0.4), TOLERANCE));
    assert!(!outer.strictly_contains(&polygon(3., 2., 1.), TOLERANCE));
    assert!(polygon(0., 0., 10.).strictly_contains(&outer, TOLERANCE));
    assert!(!polygon(1., 1., 9.).strictly_contains(&outer, TOLERANCE));
    assert!(!outer.disjoint(&polygon(8.9, 1.9, 0.3), TOLERANCE));
    assert!(outer.disjoint(&polygon(9.1, 1.9, 0.3), TOLERANCE));
    assert!(polygon(9.1, 1.9, 0.3).disjoint(&outer, TOLERANCE));
}

#[test]
fn non_half_arcs_are_outside_the_verified_subset() {
    let entities = BTreeMap::new();
    let shell = Shell::new(&entities, TOLERANCE, true);
    let circle = CircleEdge {
        center_mm: [0.; 3],
        normal: [0., 0., 1.],
        radius_mm: 1.,
    };
    assert!(matches!(
        shell.semicircle_curve(circle, [1., 0., 0.], [0., 1., 0.]),
        Err(ReadError::Unsupported)
    ));
}

#[test]
fn exact_planar_slot_requires_two_opposed_semicircles_and_tangent_lines() {
    let profile = Stadium::new([[0.; 2], [6., 0.]], 1.).unwrap();
    let text =
        write_stadium_prism(&profile, 4., [0.; 3], [1., 0., 0.], [0., 1., 0.], false).unwrap();
    let entities = parse_entities(&text, TOLERANCE).unwrap();
    let mut shell = Shell::new(&entities, TOLERANCE, true);
    shell.validated_faces().unwrap();
    let mut wire = shell.stadium_loops[&0][0].edges.map(|id| {
        let edge = &shell.edges[&id];
        let forward = edge
            .uses
            .iter()
            .find(|use_| use_.face == 0)
            .unwrap()
            .forward;
        let [start, end] = if forward {
            edge.vertices
        } else {
            [edge.vertices[1], edge.vertices[0]]
        };
        Traversal {
            edge: id,
            start,
            end,
            forward,
        }
    });
    if matches!(shell.edges[&wire[0].edge].curve, Curve::Semicircle(_)) {
        wire.rotate_left(1);
    }
    let [line, first, second_line, second] = wire.each_ref();
    let plane = Plane {
        origin: [0.; 3],
        normal: [0., 0., -1.],
        u: [1., 0., 0.],
        v: [0., -1., 0.],
    };
    assert!(matches!(
        shell.stadium_trim(&wire, &plane),
        Ok(Trim::Stadium { .. })
    ));
    assert!(matches!(
        shell.stadium_trim(&wire[..3], &plane),
        Err(ReadError::Unsupported)
    ));
    assert!(matches!(
        shell.half_cylindrical_face(&plane, 1., true, [first, line, second_line, line]),
        Err(ReadError::Invalid)
    ));
    assert!(matches!(
        shell.half_cylindrical_face(&plane, 1., true, [first, line, second, line]),
        Err(ReadError::Invalid)
    ));
    assert!(matches!(
        shell.half_cylindrical_face(&plane, 1., true, [first, second, second, line]),
        Err(ReadError::Invalid)
    ));
    let malformed = |shell: &mut Shell<'_, '_>, edge: &Traversal, curve: Curve| {
        let stored = &mut shell.edges.get_mut(&edge.edge).unwrap().curve;
        let original = std::mem::replace(stored, curve);
        let result = shell.stadium_trim(&wire, &plane);
        shell.edges.get_mut(&edge.edge).unwrap().curve = original;
        result
    };
    for arc in [first, second] {
        assert!(matches!(
            malformed(&mut shell, arc, Curve::Line),
            Err(ReadError::Unsupported)
        ));
    }
    let Curve::Semicircle(mut arc) = shell.edges[&first.edge].curve else {
        panic!("expected a half-circle");
    };
    assert!(matches!(
        malformed(&mut shell, second_line, Curve::Semicircle(arc)),
        Err(ReadError::Unsupported)
    ));
    arc.radial_mid = arc.radial_mid.map(|x| -x);
    assert!(matches!(
        malformed(&mut shell, first, Curve::Semicircle(arc)),
        Err(ReadError::Invalid)
    ));
    shell.points.get_mut(&first.start).unwrap()[0] += 0.01;
    assert!(matches!(
        shell.stadium_trim(&wire, &plane),
        Err(ReadError::Invalid)
    ));
}
