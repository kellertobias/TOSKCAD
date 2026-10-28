use multicad_core::{document::EntityId, references::*};
use multicad_kernel::*;
fn p() -> GeometryPolicy {
    GeometryPolicy::default()
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-5, "{a} != {b}");
}
fn rect(w: f64, h: f64, origin: [f64; 3]) -> CurvedProfile {
    let points = [[0., 0.], [w, 0.], [w, h], [0., h]];
    CurvedProfile {
        frame: PlaneFrame {
            origin,
            ..Default::default()
        },
        outer: (0..4)
            .map(|i| CurvedEdge2d::Line {
                a: points[i],
                b: points[(i + 1) % 4],
            })
            .collect(),
        holes: vec![],
    }
}
fn origins(count: usize) -> Vec<Origin> {
    (0..count)
        .map(|_| Origin {
            source: EntityId::new(),
            role: "curve".into(),
        })
        .collect()
}
fn verify(solid: &Solid, feature: EntityId) {
    assert_eq!(solid.solid_count().unwrap(), 1);
    let snapshot = solid.topology_snapshot(feature, 0).unwrap();
    for role in ["cap.start", "cap.end"] {
        let origin = Origin {
            source: feature,
            role: role.into(),
        };
        let cap = snapshot
            .elements
            .iter()
            .find(|e| e.key.kind == TopologyKind::Face && e.origins.contains(&origin))
            .unwrap();
        assert!(matches!(cap.geometry, Some(AnalyticGeometry::Plane { .. })));
        assert!(cap.frames.iter().any(|frame| frame.origin == origin));
    }
    for element in snapshot
        .elements
        .iter()
        .filter(|e| e.key.kind == TopologyKind::Face)
    {
        GeometryIntent::capture(&snapshot, element.key).unwrap();
    }
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("profile.step");
    solid.write_step(&path).unwrap();
    let loaded = Solid::read_step(&path, p()).unwrap();
    near(loaded.volume_mm3().unwrap(), solid.volume_mm3().unwrap());
}
#[test]
fn straight_sweep_and_right_corner_polyline_are_real_named_solids() {
    let profile = rect(10., 6., [0.; 3]);
    let before = profile.clone();
    let feature = EntityId::new();
    let curves = origins(4);
    let result = Solid::sweep_polyline_named(
        &profile,
        &[[0.; 3], [0., 0., 30.]],
        feature,
        &curves,
        &origins(1),
        p(),
    )
    .unwrap();
    near(result.volume_mm3().unwrap(), 1800.);
    verify(&result, feature);
    assert_eq!(profile, before);
    let circle = CurvedProfile {
        frame: PlaneFrame::default(),
        outer: vec![CurvedEdge2d::Circle {
            center: [0.; 2],
            radius_mm: 2.,
        }],
        holes: vec![],
    };
    let elbow = Solid::sweep_polyline_named(
        &circle,
        &[[0.; 3], [0., 0., 20.], [20., 0., 20.]],
        feature,
        &origins(1),
        &origins(2),
        p(),
    )
    .unwrap();
    near(elbow.volume_mm3().unwrap(), std::f64::consts::PI * 4. * 40.);
    verify(&elbow, feature);
}
#[test]
fn loft_authored_sections_preserve_correspondence_named_references_and_step() {
    let feature = EntityId::new();
    let sections = vec![rect(20., 10., [0.; 3]), rect(10., 5., [5., 2.5, 30.])];
    let before = sections.clone();
    let names = vec![origins(4), origins(4)];
    for ruled in [true, false] {
        let result = Solid::loft_profiles_named(&sections, feature, &names, ruled, p()).unwrap();
        near(result.volume_mm3().unwrap(), 3500.);
        verify(&result, feature);
        let snapshot = result.topology_snapshot(feature, 0).unwrap();
        let selected = snapshot
            .elements
            .iter()
            .find(|e| e.key.kind == TopologyKind::Face && e.origins.contains(&names[0][0]))
            .unwrap();
        let intent = GeometryIntent::capture(&snapshot, selected.key).unwrap();
        let edited = vec![sections[0].clone(), rect(12., 6., [4., 2., 35.])];
        let next = Solid::loft_profiles_named(&edited, feature, &names, ruled, p()).unwrap();
        assert!(matches!(
            resolve(&intent, &next.topology_snapshot(feature, 1).unwrap(), 1),
            Resolution::Resolved { .. }
        ));
    }
    assert_eq!(sections, before);
    let three = vec![
        rect(10., 6., [0.; 3]),
        rect(14., 8., [-2., -1., 15.]),
        rect(10., 6., [0., 0., 30.]),
    ];
    let names = vec![origins(4), origins(4), origins(4)];
    let ruled = Solid::loft_profiles_named(&three, feature, &names, true, p()).unwrap();
    near(ruled.volume_mm3().unwrap(), 2540.);
    verify(&ruled, feature);
    let smooth = Solid::loft_profiles_named(&three, feature, &names, false, p()).unwrap();
    assert!(smooth.volume_mm3().unwrap() > 1800.);
    verify(&smooth, feature);
}
#[test]
fn unsupported_profiles_and_bad_paths_fail_explicitly_without_mutating_sources() {
    let feature = EntityId::new();
    let profile = rect(10., 6., [0.; 3]);
    let before = profile.clone();
    let curves = origins(4);
    for path in [
        vec![[0.; 3]],
        vec![[0., 0., 1.], [0., 0., 20.]],
        vec![[0.; 3], [20., 0., 0.]],
        vec![[0.; 3], [0.; 3]],
        vec![
            [0.; 3],
            [0., 0., 10.],
            [10., 0., 10.],
            [10., 0., 0.],
            [0., 0., 10.],
        ],
    ] {
        assert!(
            Solid::sweep_polyline_named(
                &profile,
                &path,
                feature,
                &curves,
                &origins(path.len().saturating_sub(1)),
                p()
            )
            .is_err()
        );
    }
    let mut holes = profile.clone();
    holes.holes.push(rect(2., 2., [0.; 3]).outer);
    assert!(
        Solid::sweep_polyline_named(
            &holes,
            &[[0.; 3], [0., 0., 20.]],
            feature,
            &curves,
            &origins(1),
            p()
        )
        .err()
        .unwrap()
        .message
        .contains("without holes")
    );
    let mut missing = rect(5., 3., [0., 0., 20.]);
    missing.outer.pop();
    assert!(
        Solid::loft_profiles_named(
            &[profile.clone(), missing],
            feature,
            &[curves.clone(), origins(3)],
            true,
            p()
        )
        .err()
        .unwrap()
        .message
        .contains("correspondence")
    );
    assert!(
        Solid::loft_profiles_named(
            &[profile.clone(), profile.clone()],
            feature,
            &[curves.clone(), origins(4)],
            true,
            p()
        )
        .is_err()
    );
    assert_eq!(profile, before);
}
