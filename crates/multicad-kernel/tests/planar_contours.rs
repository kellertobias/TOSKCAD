use multicad_kernel::*;
fn top(solid: &Solid, z: f64) -> u32 {
    solid
        .faces()
        .unwrap()
        .into_iter()
        .find(|f| {
            f.plane.is_some_and(|(origin, normal)| {
                (origin[2] - z).abs() < 1e-6 && normal[2].abs() > 0.999999
            })
        })
        .unwrap()
        .index
}
fn xy(z: f64) -> PlaneFrame {
    PlaneFrame {
        origin: [0., 0., z],
        ..Default::default()
    }
}
#[test]
fn actual_panel_face_has_ordered_outer_and_hole_wires_with_exact_area_and_winding() {
    let panel = Solid::box_solid([120., 80., 18.], Default::default()).unwrap();
    let bore = Solid::cylinder(
        PlaneFrame {
            origin: [30., 20., -1.],
            ..Default::default()
        },
        5.,
        20.,
        Default::default(),
    )
    .unwrap();
    let panel = panel
        .boolean(&bore, BooleanOperation::Subtract, Default::default())
        .unwrap()
        .unwrap();
    let contours = panel
        .planar_face_contours(top(&panel, 18.), xy(18.))
        .unwrap();
    assert_eq!(contours.loops.len(), 2);
    assert_eq!(contours.edge_count(), 5);
    assert!((contours.area_mm2() - (120. * 80. - std::f64::consts::PI * 25.)).abs() < 1e-6);
    assert_eq!(contours.bounds_mm().unwrap(), [[0., 0.], [120., 80.]]);
    for wire in &contours.loops {
        assert_eq!(wire.signed_area() > 0., wire.outer);
    }
    let hole = contours.loops.iter().find(|l| !l.outer).unwrap();
    assert!(
        matches!(hole.edges[0],multicad_core::manufacturing::PlanarEdge::Arc{sweep_rad,..} if (sweep_rad+std::f64::consts::TAU).abs()<1e-10)
    );
    assert!(
        panel
            .planar_face_contours(top(&panel, 18.), xy(0.))
            .is_err(),
        "Never silently project a different plane"
    );
    assert!(panel.planar_face_contours(u32::MAX, xy(18.)).is_err());
}
#[test]
fn native_world_transform_is_applied_exactly_once_with_explicit_manufacturing_frame() {
    let original = Solid::box_solid([120., 80., 18.], Default::default()).unwrap();
    let world = original
        .transformed(RigidTransform {
            rotation: [[0., 0., 1.], [0., 1., 0.], [-1., 0., 0.]],
            translation_mm: [200., 0., 100.],
        })
        .unwrap();
    let face = world
        .faces()
        .unwrap()
        .into_iter()
        .find(|f| {
            f.plane
                .is_some_and(|(origin, n)| (origin[0] - 218.).abs() < 1e-6 && n[0].abs() > 0.999999)
        })
        .unwrap()
        .index;
    let contours = world
        .planar_face_contours(
            face,
            PlaneFrame {
                origin: [218., 0., 100.],
                u: [0., 0., -1.],
                v: [0., 1., 0.],
            },
        )
        .unwrap();
    assert!((contours.area_mm2() - 9600.).abs() < 1e-6);
    let bounds = contours.bounds_mm().unwrap();
    for (a, b) in bounds.into_iter().flatten().zip([0., 0., 120., 80.]) {
        assert!((a - b).abs() < 1e-6);
    }
    assert!(
        world
            .planar_face_contours(face, PlaneFrame::default())
            .is_err()
    );
}
#[test]
fn circular_and_semicircular_faces_remain_exact_and_curved_or_elliptic_targets_fail() {
    let cylinder = Solid::cylinder(PlaneFrame::default(), 5., 3., Default::default()).unwrap();
    let circle = cylinder
        .planar_face_contours(top(&cylinder, 3.), xy(3.))
        .unwrap();
    assert_eq!(circle.edge_count(), 1);
    assert!((circle.area_mm2() - std::f64::consts::PI * 25.).abs() < 1e-6);
    let curved = cylinder
        .faces()
        .unwrap()
        .into_iter()
        .find(|f| f.plane.is_none())
        .unwrap()
        .index;
    assert!(
        cylinder
            .planar_face_contours(curved, PlaneFrame::default())
            .unwrap_err()
            .message
            .contains("planar face")
    );
    let profile = Profile {
        frame: Default::default(),
        outer: vec![
            ProfileEdge::Arc {
                center: [0., 0.],
                a: [0., -5.],
                b: [0., 5.],
                clockwise: false,
            },
            ProfileEdge::Line {
                a: [0., 5.],
                b: [0., -5.],
            },
        ],
        holes: vec![],
    };
    let body = Solid::extrude_profile(&profile, 3., Default::default()).unwrap();
    let half = body.planar_face_contours(top(&body, 3.), xy(3.)).unwrap();
    assert!((half.area_mm2() - std::f64::consts::PI * 12.5).abs() < 1e-6);
    let panel = Solid::box_solid([120., 80., 18.], Default::default()).unwrap();
    let s = 2f64.sqrt().recip();
    let cutter = Solid::cylinder(
        PlaneFrame {
            origin: [50., 50., -10.],
            u: [1., 0., 0.],
            v: [0., s, s],
        },
        5.,
        100.,
        Default::default(),
    )
    .unwrap();
    let panel = panel
        .boolean(&cutter, BooleanOperation::Subtract, Default::default())
        .unwrap()
        .unwrap();
    assert!(
        panel
            .planar_face_contours(top(&panel, 18.), xy(18.))
            .unwrap_err()
            .message
            .contains("ellipse/spline")
    );
}
