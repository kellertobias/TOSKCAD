use multicad_kernel::{
    BooleanOperation, CurvedEdge2d, CurvedProfile, GeometryPolicy, PlaneFrame, RigidTransform,
    Solid,
    rust_kernel::{
        planar_boolean::{ConvexPiece, Union},
        step::write_warped_union,
        warped_union::{Band, Boss, Curve, remap},
    },
};

fn polygon(points: &[[f64; 2]]) -> Vec<CurvedEdge2d> {
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .map(|(&a, &b)| CurvedEdge2d::Line { a, b })
        .collect()
}

#[test]
fn native_step_reader_keeps_the_elliptical_trim_of_a_joined_fillet() {
    let policy = GeometryPolicy::default();
    let tolerance = policy.linear_tolerance_mm;
    let [a, b, c, d, e, f] = [
        [0., 3., 0.],
        [2., 4., 0.],
        [2., 4., 2.],
        [0., 10., 0.],
        [2., 10., 0.],
        [2., 10., 2.],
    ];
    let wedge = ConvexPiece::new(
        vec![
            vec![a, b, c],
            vec![d, f, e],
            vec![a, d, e, b],
            vec![b, e, f, c],
            vec![c, f, d, a],
        ],
        tolerance,
    )
    .unwrap();
    let block = ConvexPiece::from_box([2., 4., 0.], [3., 8., 2.], tolerance).unwrap();
    let planar = Union::new(&[wedge, block], tolerance).unwrap();
    let shell = remap(
        &planar,
        Band {
            center_z_mm: 2.,
            radius_mm: 2.,
        },
        tolerance,
    )
    .unwrap();
    assert_eq!(shell.solid_count(), 1);
    assert!(
        shell
            .edges()
            .iter()
            .any(|edge| matches!(edge.curve, Curve::Ellipse(arc) if {
                let [major, minor] = arc.ellipse().semiaxes_mm();
                major - minor > tolerance
            }))
    );

    let curved = CurvedProfile {
        frame: PlaneFrame {
            v: [0., 0., 1.],
            ..Default::default()
        },
        outer: vec![
            CurvedEdge2d::Line {
                a: [0., 0.],
                b: [2., 0.],
            },
            CurvedEdge2d::Line {
                a: [2., 0.],
                b: [2., 2.],
            },
            CurvedEdge2d::Arc {
                a: [2., 2.],
                b: [0., 0.],
                center: [0., 2.],
                clockwise: true,
            },
        ],
        holes: vec![],
    };
    let native = Solid::extrude_curved_profile_vector(&curved, [0., 10., 0.], policy).unwrap();
    let shear_tool = Solid::extrude_curved_profile_vector(
        &CurvedProfile {
            frame: PlaneFrame {
                origin: [0., 0., -1.],
                ..Default::default()
            },
            outer: polygon(&[[-1., -1.], [3., -1.], [3., 4.5], [-1., 2.5]]),
            holes: vec![],
        },
        [0., 0., 4.],
        policy,
    )
    .unwrap();
    let clipped = native
        .boolean(&shear_tool, BooleanOperation::Subtract, policy)
        .unwrap()
        .unwrap();
    let block = Solid::box_solid([1., 4., 2.], policy)
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: [2., 4., 0.],
            ..Default::default()
        })
        .unwrap();
    let native = clipped
        .boolean(&block, BooleanOperation::Union, policy)
        .unwrap()
        .unwrap();
    let pose = RigidTransform {
        rotation: [[0.8, -0.36, 0.48], [0.6, 0.48, -0.64], [0., 0.8, 0.6]],
        translation_mm: [13., -7., 42.],
    };
    let native = native.transformed(pose).unwrap();
    let step = write_warped_union(
        &shell,
        pose.translation_mm,
        [0.8, 0.6, 0.],
        [-0.36, 0.48, 0.8],
    )
    .unwrap();
    assert!(step.contains("=ELLIPSE("));
    let folder = tempfile::tempdir().unwrap();
    let file = folder.path().join("warped.step");
    std::fs::write(&file, step).unwrap();
    let independent = Solid::read_step(&file, policy).unwrap();
    assert_eq!(independent.solid_count().unwrap(), 1);
    let near = |a: f64, b: f64| assert!((a - b).abs() < 1e-6, "{a} != {b}");
    near(
        independent.volume_mm3().unwrap(),
        native.volume_mm3().unwrap(),
    );
    near(independent.volume_mm3().unwrap(), shell.volume_mm3());
    let area = |solid: &Solid| {
        solid
            .faces()
            .unwrap()
            .iter()
            .map(|face| face.area_mm2)
            .sum()
    };
    near(area(&independent), area(&native));
    let [a, b] = [independent.bounds().unwrap(), native.bounds().unwrap()];
    for (x, y) in a
        .min
        .into_iter()
        .chain(a.max)
        .zip(b.min.into_iter().chain(b.max))
    {
        near(x, y);
    }
}

#[test]
fn native_step_reader_keeps_two_exact_boss_rims_on_one_stock_face() {
    let policy = GeometryPolicy::default();
    let tolerance = policy.linear_tolerance_mm;
    let chart = ConvexPiece::from_box([0., 0., 3.], [10., 80., 30.], tolerance).unwrap();
    let shell = remap(
        &Union::new(&[chart], tolerance).unwrap(),
        Band {
            center_z_mm: 10.,
            radius_mm: 2.,
        },
        tolerance,
    )
    .unwrap();
    let bosses = [(20., 2.), (60., 3.)].map(|(y, radius_mm)| Boss {
        start_mm: [9.95, y, 20.],
        end_mm: [13.5, y, 20.],
        radius_mm,
        source: 1,
        wall_source_face: 2,
        cap_source_face: 1,
    });
    let joined = shell.attach_bosses(&bosses, tolerance).unwrap();

    let mut native = Solid::box_solid([10., 80., 27.], policy)
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: [0., 0., 11.],
            ..Default::default()
        })
        .unwrap();
    for boss in bosses {
        let tool = Solid::cylinder(
            PlaneFrame {
                origin: boss.start_mm,
                u: [0., 1., 0.],
                v: [0., 0., 1.],
            },
            boss.radius_mm,
            boss.end_mm[0] - boss.start_mm[0],
            policy,
        )
        .unwrap();
        native = native
            .boolean(&tool, BooleanOperation::Union, policy)
            .unwrap()
            .unwrap();
    }
    let pose = RigidTransform {
        rotation: [[0.8, -0.36, 0.48], [0.6, 0.48, -0.64], [0., 0.8, 0.6]],
        translation_mm: [13., -7., 42.],
    };
    let native = native.transformed(pose).unwrap();
    let step = write_warped_union(
        &joined,
        pose.translation_mm,
        [0.8, 0.6, 0.],
        [-0.36, 0.48, 0.8],
    )
    .unwrap();
    assert_eq!(step.matches("=FACE_BOUND(").count(), 2);
    assert_eq!(step.matches("=CIRCLE(").count(), 16);
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("two-bosses.step");
    std::fs::write(&path, step).unwrap();
    let reopened = Solid::read_step(&path, policy).unwrap();
    assert_eq!(reopened.solid_count().unwrap(), 1);
    let near = |a: f64, b: f64| assert!((a - b).abs() < 1e-6, "{a} != {b}");
    near(reopened.volume_mm3().unwrap(), native.volume_mm3().unwrap());
    near(reopened.volume_mm3().unwrap(), joined.volume_mm3());
    let area = |solid: &Solid| {
        solid
            .faces()
            .unwrap()
            .iter()
            .map(|face| face.area_mm2)
            .sum::<f64>()
    };
    near(area(&reopened), area(&native));
    let [a, b] = [reopened.bounds().unwrap(), native.bounds().unwrap()];
    for (x, y) in a
        .min
        .into_iter()
        .chain(a.max)
        .zip(b.min.into_iter().chain(b.max))
    {
        near(x, y);
    }
}
