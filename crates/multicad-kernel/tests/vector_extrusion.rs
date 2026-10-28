use multicad_core::{
    document::EntityId,
    references::{Attachment, GeometryIntent, Origin, Resolution, TopologyKind, resolve},
};
use multicad_kernel::*;
use std::f64::consts::PI;
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}
fn p() -> GeometryPolicy {
    GeometryPolicy::default()
}
fn line(a: [f64; 2], b: [f64; 2]) -> CurvedEdge2d {
    CurvedEdge2d::Line { a, b }
}
fn rectangle(a: [f64; 2], b: [f64; 2]) -> Vec<CurvedEdge2d> {
    vec![
        line(a, [b[0], a[1]]),
        line([b[0], a[1]], b),
        line(b, [a[0], b[1]]),
        line([a[0], b[1]], a),
    ]
}
fn profile() -> CurvedProfile {
    CurvedProfile {
        frame: Default::default(),
        outer: rectangle([0., 0.], [10., 6.]),
        holes: vec![vec![CurvedEdge2d::Circle {
            center: [5., 3.],
            radius_mm: 1.,
        }]],
    }
}
fn origins(profile: &CurvedProfile) -> Vec<Origin> {
    profile
        .outer
        .iter()
        .chain(profile.holes.iter().flatten())
        .map(|_| Origin {
            source: EntityId::new(),
            role: "curve".into(),
        })
        .collect()
}
fn origin(feature: EntityId, role: &str) -> Origin {
    Origin {
        source: feature,
        role: role.into(),
    }
}
#[test]
fn oblique_prism_keeps_exact_volume_offset_caps_and_orthonormal_side_frames() {
    let mut profile = profile();
    profile.frame.origin = [0., 0., 10.];
    let feature = EntityId::new();
    let origins = origins(&profile);
    let displacement = [5., -4., 100.];
    let original = profile.clone();
    let solid =
        Solid::extrude_curved_profile_vector_named(&profile, displacement, feature, &origins, p())
            .unwrap();
    near(solid.volume_mm3().unwrap(), (60. - PI) * 100.);
    assert_eq!(profile, original);
    let bounds = solid.bounds().unwrap();
    for (i, (min, max)) in [(0., 15.), (-4., 6.), (10., 110.)].into_iter().enumerate() {
        near(bounds.min[i], min);
        near(bounds.max[i], max);
    }
    let snapshot = solid.topology_snapshot(feature, 0).unwrap();
    let end = snapshot
        .elements
        .iter()
        .find(|e| e.origins.contains(&origin(feature, "cap.end")))
        .unwrap();
    assert_eq!(end.frames[0].frame.origin_mm, [5., -4., 110.]);
    let faces = solid.faces().unwrap();
    let mut count = 0;
    for element in snapshot
        .elements
        .iter()
        .filter(|e| e.key.kind == TopologyKind::Face)
    {
        for anchored in &element.frames {
            count += 1;
            anchored.frame.validate().unwrap();
            let (plane, normal) = faces
                .iter()
                .find(|f| f.index == element.key.ordinal)
                .unwrap()
                .plane
                .unwrap();
            let dot =
                |a: [f64; 3], b: [f64; 3]| a.into_iter().zip(b).map(|(a, b)| a * b).sum::<f64>();
            near(
                dot(
                    std::array::from_fn(|i| anchored.frame.origin_mm[i] - plane[i]),
                    normal,
                ),
                0.,
            );
            near(dot(anchored.frame.u, normal), 0.);
            near(dot(anchored.frame.v, normal), 0.);
        }
    }
    assert_eq!(count, 6);
    let curved = snapshot
        .elements
        .iter()
        .find(|e| e.key.kind == TopologyKind::Face && e.origins.contains(&origins[4]))
        .unwrap();
    assert!(curved.frames.is_empty());
}
#[test]
fn bspline_and_holes_extrude_both_directions_without_display_approximation() {
    let profile = CurvedProfile {
        frame: Default::default(),
        outer: vec![
            CurvedEdge2d::BSpline(BSpline2d {
                degree: 2,
                poles: vec![[0., 0.], [5., 10.], [10., 0.]],
                knots: vec![0., 1.],
                multiplicities: vec![3, 3],
                weights: None,
            }),
            line([10., 0.], [0., 0.]),
        ],
        holes: vec![rectangle([4., 1.], [6., 2.])],
    };
    let feature = EntityId::new();
    let origins = origins(&profile);
    for displacement in [[3., 7., 12.], [-3., -7., -12.]] {
        let solid = Solid::extrude_curved_profile_vector_named(
            &profile,
            displacement,
            feature,
            &origins,
            p(),
        )
        .unwrap();
        near(solid.volume_mm3().unwrap(), 376.);
        let bounds = solid.bounds().unwrap();
        let expected = if displacement[2] > 0. {
            [[0., 0., 0.], [13., 12., 12.]]
        } else {
            [[-3., -7., -12.], [10., 5., 0.]]
        };
        for (axis, (min, max)) in expected[0].into_iter().zip(expected[1]).enumerate() {
            near(bounds.min[axis], min);
            near(bounds.max[axis], max);
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("oblique-spline.step");
        solid.write_step(&path).unwrap();
        let reopened = Solid::read_step(&path, p()).unwrap();
        near(reopened.volume_mm3().unwrap(), 376.);
        let snapshot = solid.topology_snapshot(feature, 0).unwrap();
        let side = snapshot
            .elements
            .iter()
            .find(|e| e.key.kind == TopologyKind::Face && e.origins.contains(&origins[0]))
            .unwrap();
        assert!(side.frames.is_empty());
    }
}
#[test]
fn vector_uses_profile_normal_on_rotated_planes_and_normal_api_stays_compatible() {
    let mut profile = profile();
    profile.holes.clear();
    profile.frame = PlaneFrame {
        origin: [20., 30., 40.],
        u: [0., 1., 0.],
        v: [0., 0., 1.],
    };
    let solid = Solid::extrude_curved_profile_vector(&profile, [8., 3., -2.], p()).unwrap();
    near(solid.volume_mm3().unwrap(), 480.);
    let bounds = solid.bounds().unwrap();
    for (i, (min, max)) in [(20., 28.), (30., 43.), (38., 46.)].into_iter().enumerate() {
        near(bounds.min[i], min);
        near(bounds.max[i], max);
    }
    for height in [-8., 8.] {
        let normal = Solid::extrude_curved_profile(&profile, height, p()).unwrap();
        let vector = Solid::extrude_curved_profile_vector(
            &profile,
            profile.frame.normal().map(|n| n * height),
            p(),
        )
        .unwrap();
        near(normal.volume_mm3().unwrap(), vector.volume_mm3().unwrap());
        assert_eq!(normal.bounds().unwrap(), vector.bounds().unwrap());
    }
}
#[test]
fn named_cap_references_follow_full_displacement_and_survive_dimension_changes() {
    let profile = profile();
    let feature = EntityId::new();
    let origins = origins(&profile);
    let first = Solid::extrude_curved_profile_named(&profile, 10., feature, &origins, p())
        .unwrap()
        .topology_snapshot(feature, 0)
        .unwrap();
    let cap = first
        .elements
        .iter()
        .find(|e| e.origins.contains(&origin(feature, "cap.end")))
        .unwrap();
    let mut intent = GeometryIntent::capture(&first, cap.key).unwrap();
    intent.attachment = Some(Attachment {
        anchor: origin(feature, "cap.end"),
        offset_mm: [2., 3., 0.],
        angle_rad: 0.,
    });
    let next = Solid::extrude_curved_profile_vector_named(
        &profile,
        [5., -4., 20.],
        feature,
        &origins,
        p(),
    )
    .unwrap()
    .topology_snapshot(feature, 1)
    .unwrap();
    let Resolution::Resolved {
        frame: Some(frame), ..
    } = resolve(&intent, &next, 1)
    else {
        panic!("cap attachment unresolved")
    };
    assert_eq!(frame.origin_mm, [7., -1., 20.]);
}
#[test]
fn invalid_sweeps_fail_without_changing_valid_source_or_geometry() {
    let profile = profile();
    let original = profile.clone();
    let valid = Solid::extrude_curved_profile_vector(&profile, [3., 4., 5.], p()).unwrap();
    let before = valid.volume_mm3().unwrap();
    for displacement in [
        [0.; 3],
        [1., 2., 0.],
        [f64::NAN, 0., 1.],
        [0., 0., f64::INFINITY],
        [1e8, 0., 1.],
        [1., 0., 1e-7],
    ] {
        assert!(Solid::extrude_curved_profile_vector(&profile, displacement, p()).is_err());
        assert_eq!(profile, original);
        near(valid.volume_mm3().unwrap(), before);
    }
    let mut invalid = profile.clone();
    invalid.frame.u = [2., 0., 0.];
    assert!(Solid::extrude_curved_profile_vector(&invalid, [0., 0., 5.], p()).is_err());
    let feature = EntityId::new();
    let mut duplicate = origins(&profile);
    duplicate[1] = duplicate[0].clone();
    assert!(
        Solid::extrude_curved_profile_vector_named(
            &profile,
            [0., 0., 5.],
            feature,
            &duplicate,
            p()
        )
        .is_err()
    );
}
#[test]
fn vector_worker_keeps_exact_geometry_local_and_exports_only_current_keys() {
    let feature = EntityId::new();
    let profile = profile();
    let origins = origins(&profile);
    let worker = GeometryWorker::new().unwrap();
    let program = |revision, displacement_mm| GeometryProgram {
        revision,
        policy: p(),
        operations: vec![GeometryOperation::NamedVectorExtrude {
            profile: profile.clone(),
            displacement_mm,
            feature,
            curve_origins: origins.clone(),
        }],
        outputs: vec![0],
        mesh: Default::default(),
    };
    let snapshot = worker
        .submit(program(0, [5., -4., 10.]))
        .unwrap()
        .wait()
        .unwrap();
    let solid = snapshot.solids[0].as_ref().unwrap();
    near(solid.volume_mm3, (60. - PI) * 10.);
    assert!(!solid.topology.is_empty());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("worker-vector.step");
    worker
        .write_step_to_staging(solid.key, 0, path.clone())
        .unwrap()
        .wait()
        .unwrap();
    near(
        Solid::read_step(&path, p()).unwrap().volume_mm3().unwrap(),
        solid.volume_mm3,
    );
    worker
        .submit(program(1, [-2., 3., 12.]))
        .unwrap()
        .wait()
        .unwrap();
    assert!(
        worker
            .write_step_to_staging(solid.key, 0, directory.path().join("stale.step"))
            .is_err()
    );
}
