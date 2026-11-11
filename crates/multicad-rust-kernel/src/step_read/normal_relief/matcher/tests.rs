use super::*;
#[test]
fn typed_supports_and_straight_profile_edges_do_not_mask_geometry_changes() {
    let frame = Frame::new([0.; 3], [1., 0., 0.], [0., 1., 0.]).unwrap();
    let plane = Surface::Plane {
        origin_mm: [0.; 3],
        u: [1., 0., 0.],
        v: [0., 1., 0.],
    };
    let cylinder = Surface::Cylinder {
        origin_mm: [0.; 3],
        axis: [0., 0., 1.],
        u: [1., 0., 0.],
        radius_mm: 1.,
        outward: true,
    };
    assert!(support(plane, frame, plane, frame, 1e-8, 3.).is_ok());
    assert!(support(cylinder, frame, cylinder, frame, 1e-8, 3.).is_ok());
    assert!(support(plane, frame, cylinder, frame, 1e-8, 3.).is_err());
    assert!(support(cylinder, frame, plane, frame, 1e-8, 3.).is_err());
    assert!(matches!(
        curve(
            ReliefCurve::Profile {
                edge: ProfileEdge::Line {
                    a: [0.; 2],
                    b: [1.; 2]
                },
                z_mm: 2.
            },
            frame
        ),
        Curve::Line
    ));
}
#[test]
fn legacy_circular_pockets_match_the_complete_same_physical_relief() {
    use crate::{normal_relief::Kind, normal_sweep::Sweep};
    let rings = vec![super::super::tests::rectangle([0.; 2], [12., 10.])];
    let mut rings = rings;
    rings.push(vec![ProfileEdge::Circle {
        center: [6., 7.],
        radius_mm: 0.3,
        clockwise: true,
    }]);
    let base = Sweep::new(rings, 3., 1e-7, 1e-5).unwrap();
    let motifs = [([3., 3.], 1., 0.5), ([8., 6.], 0.75, 1.25)]
        .into_iter()
        .map(|(center, radius_mm, depth)| {
            Sweep::new(
                vec![vec![ProfileEdge::Circle {
                    center,
                    radius_mm,
                    clockwise: false,
                }]],
                depth,
                1e-7,
                1e-5,
            )
            .unwrap()
        })
        .collect();
    let relief = Relief::new(base, motifs, Kind::Recessed, 1e-7, 1e-5).unwrap();
    for (origin, u, v) in [
        ([0.; 3], [1., 0., 0.], [0., 1., 0.]),
        ([80., 60., 20.], [0.8, 0.6, 0.], [-0.36, 0.48, 0.8]),
        ([0.; 3], [1., 0., 0.], [0., -1., 0.]),
    ] {
        let text = crate::step::write_normal_relief(&relief, origin, u, v).unwrap();
        let (_, mut decoded) = read_import_solid(&text, 1e-7).unwrap();
        assert!(matches!(
            decoded.profile,
            ImportProfile::PocketedPolygon { .. }
        ));
        assert!(normal_relief_import_matches(
            &decoded,
            &relief,
            [origin, u, v],
            1e-7
        ));
        let ImportProfile::PocketedPolygon { pockets, .. } = &mut decoded.profile else {
            unreachable!()
        };
        pockets[0].depth_mm += 0.001;
        assert!(!normal_relief_import_matches(
            &decoded,
            &relief,
            [origin, u, v],
            1e-7
        ));
    }
}
#[test]
fn empty_or_wrong_legacy_descriptors_are_not_synthesized_into_material() {
    let frame = Frame::new([0.; 3], [1., 0., 0.], [0., 1., 0.]).unwrap();
    let mut imported = ImportSolid {
        origin_mm: [0.; 3],
        u: frame.axes[0],
        v: frame.axes[1],
        height_mm: 3.,
        profile: ImportProfile::Cylinder { radius_mm: 2. },
    };
    assert!(matches!(
        pocketed(&imported, frame, 1e-7),
        Err(ReadError::Unsupported)
    ));
    imported.profile = ImportProfile::PocketedPolygon {
        rings: vec![vec![[0., 0.], [12., 0.], [12., 10.], [0., 10.]]],
        circular_holes: vec![],
        pockets: vec![],
    };
    assert!(pocketed(&imported, frame, 1e-7).is_err());
    let expected = super::super::tests::fixture(crate::normal_relief::Kind::Recessed);
    assert!(!normal_relief_import_matches(
        &imported,
        &expected,
        [[0.; 3], frame.axes[0], frame.axes[1]],
        1e-7
    ));
}
