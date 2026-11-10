use super::*;
use crate::{stadium::Stadium, step::write_normal_sweep};

const X: Point = [1., 0., 0.];
const Y: Point = [0., 1., 0.];
const TOLERANCE: f64 = 1e-7;

fn sweep() -> Sweep {
    let line = |a, b| ProfileEdge::Line { a, b };
    let stadium = Stadium::new([[16., 7.], [22., 7.]], 1.5).unwrap();
    Sweep::new(
        vec![
            vec![
                line([0., 0.], [40., 0.]),
                line([40., 0.], [40., 22.]),
                ProfileEdge::Arc {
                    center: [37., 22.],
                    a: [40., 22.],
                    b: [37., 25.],
                    clockwise: false,
                },
                line([37., 25.], [0., 25.]),
                line([0., 25.], [0., 0.]),
            ],
            vec![
                line([4., 6.], [4., 10.]),
                line([4., 10.], [8., 6.]),
                line([8., 6.], [4., 6.]),
            ],
            vec![ProfileEdge::Circle {
                center: [32., 10.],
                radius_mm: 1.5,
                clockwise: true,
            }],
            profile_ring(&Trim::Stadium {
                profile: stadium,
                ccw: false,
            }),
        ],
        5.,
        TOLERANCE,
        1e-5,
    )
    .unwrap()
}

#[test]
fn mixed_caps_with_polygon_round_and_stadium_holes_reconstruct_one_exact_sweep() {
    let sweep = sweep();
    let source = write_normal_sweep(&sweep, [0.; 3], X, Y, false).unwrap();
    let (readback, reconstructed) = read_import_solid(&source, TOLERANCE).unwrap();
    assert_eq!(readback.arcs.len(), 2);
    assert_eq!(readback.semicircles.len(), 4);
    assert_eq!(readback.circles.len(), 2);
    let ImportProfile::NormalSweep {
        sweep: reconstructed,
    } = reconstructed.profile
    else {
        panic!("mixed extrusion should reconstruct its curved profile");
    };
    assert_eq!(reconstructed.rings().len(), 4);
    assert!((sweep.volume_mm3() - reconstructed.volume_mm3()).abs() < TOLERANCE);
    assert!((sweep.volume_mm3() - readback.volume_mm3).abs() < TOLERANCE);
}

#[test]
fn major_arc_caps_prove_interior_extrema() {
    let a = [2., 0.];
    let b = [0., -2.];
    let sweep = Sweep::new(
        vec![vec![
            ProfileEdge::Arc {
                center: [0., 0.],
                a,
                b,
                clockwise: false,
            },
            ProfileEdge::Line { a: b, b: a },
        ]],
        3.,
        TOLERANCE,
        1e-5,
    )
    .unwrap();
    let source = write_normal_sweep(&sweep, [0.; 3], X, Y, false).unwrap();
    let read = read_analytic_solid(&source, TOLERANCE).unwrap();
    assert_eq!(read.bounds_mm, [[-2., -2., 0.], [2., 2., 3.]]);
    assert!(
        read.arcs
            .iter()
            .all(|arc| (arc.sweep_rad - 0.75 * TAU).abs() < TOLERANCE)
    );
    assert!((read.volume_mm3 - sweep.volume_mm3()).abs() < TOLERANCE);
}

#[test]
fn mixed_profiles_and_partial_walls_refuse_inconsistent_topology() {
    let source = write_normal_sweep(&sweep(), [0.; 3], X, Y, false).unwrap();
    let entities = parse_entities(&source, TOLERANCE).unwrap();
    let mut shell = Shell::new(&entities, TOLERANCE, true);
    let mut faces = shell.validated_faces().unwrap();
    let z = [0., 0., 1.];
    let (low, low_arc) = shell
        .edges
        .iter()
        .find_map(|(&id, edge)| match edge.curve {
            Curve::Arc(arc) if arc.center_mm[2] == 0. => Some((id, arc)),
            _ => None,
        })
        .unwrap();
    let high = shell
        .edges
        .iter()
        .find_map(|(&id, edge)| match edge.curve {
            Curve::Arc(arc) if arc.center_mm[2] == 5. => Some(id),
            _ => None,
        })
        .unwrap();
    let traversal = |id: u32, forward: bool, [a, b]: [u32; 2]| {
        let [start, end] = if forward { [a, b] } else { [b, a] };
        Traversal {
            edge: id,
            start,
            end,
            forward,
        }
    };
    let a = traversal(low, true, shell.edges[&low].vertices);
    let b = traversal(high, false, shell.edges[&high].vertices);
    let rail = |start, end| {
        let (&id, edge) = shell
            .edges
            .iter()
            .find(|(_, edge)| edge.vertices == [start, end] || edge.vertices == [end, start])
            .unwrap();
        traversal(id, edge.vertices[0] == start, edge.vertices)
    };
    let rail_out = rail(a.end, b.start);
    let rail_back = rail(b.end, a.start);
    let plane = Plane {
        origin: low_arc.center_mm,
        normal: z,
        u: X,
        v: Y,
    };
    let check = |radius, outward, b, back| {
        shell.partial_cylindrical_face(&plane, radius, outward, [&a, &rail_out, b, back])
    };
    assert!(check(3., true, &b, &rail_back).is_ok());
    assert!(matches!(
        check(3., true, &rail_out, &rail_back),
        Err(ReadError::Invalid)
    ));
    assert!(matches!(
        check(3., true, &b, &rail_out),
        Err(ReadError::Invalid)
    ));
    assert!(matches!(
        check(2., true, &b, &rail_back),
        Err(ReadError::Invalid)
    ));
    assert!(matches!(
        check(3., false, &b, &rail_back),
        Err(ReadError::Invalid)
    ));

    let mut ids: Vec<_> = shell
        .edges
        .iter()
        .filter_map(|(&id, edge)| {
            edge.uses
                .iter()
                .any(|edge_use| edge_use.face == 0)
                .then_some(id)
        })
        .collect();
    assert!(shell.normal_sweep_profile(&ids, 0, z, 0.).is_none());
    assert!(shell.normal_sweep_profile(&ids, 1, z, 5.).is_none());
    assert!(shell.normal_sweep_profile(&[], 0, z, 5.).is_none());
    let missing = ids.remove(0);
    assert!(shell.normal_sweep_profile(&ids, 0, z, 5.).is_none());
    ids.insert(0, missing);
    let off_cap = Plane {
        origin: [0., 0., 1.],
        ..plane
    };
    assert!(matches!(
        shell.mixed_trim(&[a], &off_cap),
        Err(ReadError::Invalid)
    ));
    let a = traversal(low, true, shell.edges[&low].vertices);
    ids.push(low);
    assert!(shell.normal_sweep_profile(&ids, 0, z, 5.).is_none());
    ids.pop();
    let tilted = ArcEdge {
        normal: [0., 1., 0.],
        ..low_arc
    };
    shell.edges.get_mut(&low).unwrap().curve = Curve::Arc(tilted);
    assert!(matches!(
        shell.profile_edge(&a, &plane),
        Err(ReadError::Invalid)
    ));
    assert!(shell.normal_sweep_profile(&ids, 0, z, 5.).is_none());
    shell.edges.get_mut(&low).unwrap().curve = Curve::Arc(low_arc);
    let wall = shell.edges[&low]
        .uses
        .iter()
        .find(|item| item.face != 0)
        .unwrap()
        .face;
    faces[wall] = Face::Planar {
        origin: plane.origin,
        normal: z,
        area: 1.,
    };
    assert_eq!(shell.check_incidence(&faces), Err(ReadError::Invalid));
    let uses = &mut shell.edges.get_mut(&low).unwrap().uses;
    uses[1].forward = uses[0].forward;
    assert_eq!(shell.check_incidence(&faces), Err(ReadError::Invalid));
}

#[test]
fn exact_planar_validation_distinguishes_coincident_and_complementary_arcs() {
    let normal = [0., 0., 1.];
    let center_mm = [0.; 3];
    let circle = CircleEdge {
        normal,
        center_mm,
        radius_mm: 2.,
    };
    let entities = BTreeMap::new();
    let shell = Shell::new(&entities, TOLERANCE, true);
    let Curve::Arc(arc) = shell
        .open_circle_curve(circle, [2., 0., 0.], [0., 2., 0.])
        .unwrap()
    else {
        panic!("quarter arc");
    };
    let half = Curve::Semicircle(SemicircleEdge {
        center_mm,
        normal,
        radius_mm: 2.,
        radial_mid: Y,
    });
    assert!(duplicate_curve(half, half, TOLERANCE));
    assert!(duplicate_curve(Curve::Arc(arc), Curve::Arc(arc), TOLERANCE));
    assert!(!duplicate_curve(Curve::Arc(arc), half, TOLERANCE));
    assert!(matches!(
        shell.semicircle_curve(circle, [2., 0., 0.], [-2., 0., 1e-4]),
        Err(ReadError::Invalid)
    ));
    assert!(matches!(
        shell.open_circle_curve(circle, [2., 0., 0.], [0., 2., 1e-4]),
        Err(ReadError::Invalid)
    ));
    let mixed = Trim::Mixed(sweep().rings()[0].clone());
    assert!(mixed.signed_area() > 0.);
    assert!(!mixed.strictly_contains(&mixed, TOLERANCE));
    assert!(!mixed.disjoint(&mixed, TOLERANCE));
    let stadium = Trim::Stadium {
        profile: Stadium::new([[1., 1.], [6., 1.]], 1.).unwrap(),
        ccw: true,
    };
    assert!(crate::normal_sweep::signed_area(&profile_ring(&stadium)) > 0.);
    assert_eq!(
        face_area(&mixed, &[stadium], TOLERANCE),
        Err(ReadError::Invalid)
    );
}
