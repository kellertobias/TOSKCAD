use multicad_core::{
    document::EntityId,
    references::{GeometryIntent, Origin, Resolution, TopologyKind, resolve},
};
use multicad_kernel::{
    BooleanOperation, CurvedEdge2d, GeometryPolicy, NormalReliefMotif, OCCT_LINKED, PlaneFrame,
    RigidTransform, Solid,
    rust_kernel::{Rational, normal_relief::Kind},
};

fn point(frame: PlaneFrame, p: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| {
        frame.origin[i] + frame.u[i] * p[0] + frame.v[i] * p[1] + frame.normal()[i] * p[2]
    })
}

fn fixture(kind: Kind, frame: PlaneFrame, feature: EntityId) -> (Solid, Solid, EntityId) {
    let policy = GeometryPolicy::default();
    let owner = EntityId::new();
    let blank = Solid::rust_box(
        owner.uuid_bits(),
        [12, 10, 3].map(|v| Rational::new(v, 1).unwrap()),
        frame,
        policy,
    )
    .unwrap();
    let cap = PlaneFrame {
        origin: point(frame, [0., 0., 3.]),
        ..frame
    };
    let face = blank
        .faces()
        .unwrap()
        .into_iter()
        .find(|f| {
            f.plane.is_some_and(|(_, n)| {
                n.into_iter()
                    .zip(frame.normal())
                    .map(|(a, b)| a * b)
                    .sum::<f64>()
                    > 0.999
            })
        })
        .unwrap();
    let motifs = [([3., 3.], 1., 0.5), ([8., 6.], 0.75, 1.25)]
        .into_iter()
        .enumerate()
        .map(|(i, (center, radius_mm, height_mm))| NormalReliefMotif {
            boundary: vec![CurvedEdge2d::Circle { center, radius_mm }],
            height_mm,
            origin: Origin {
                source: feature,
                role: format!("motif/{i}"),
            },
        })
        .collect::<Vec<_>>();
    let relief = blank
        .rust_normal_relief_on_cap(face.index, cap, &motifs, kind, feature, policy, None)
        .unwrap();
    (blank, relief, owner)
}

#[test]
fn exact_relief_keeps_ancestry_picking_material_and_independent_exchange() {
    let policy = GeometryPolicy::default();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("relief.step");
    for frame in [
        PlaneFrame::default(),
        PlaneFrame {
            origin: [7., -3., 2.],
            u: [0.8, 0.6, 0.],
            v: [-0.36, 0.48, 0.8],
        },
    ] {
        for kind in [Kind::Raised, Kind::Recessed] {
            let feature = EntityId::new();
            let (blank, relief, owner) = fixture(kind, frame, feature);
            let sign = if kind == Kind::Raised { 1. } else { -1. };
            let expected = 360. + sign * std::f64::consts::PI * (0.5 + 0.75_f64.powi(2) * 1.25);
            assert!((relief.volume_mm3().unwrap() - expected).abs() < 1e-9);
            let before = blank.topology_snapshot(owner, 0).unwrap();
            let after = relief.topology_snapshot(owner, 1).unwrap();
            assert_eq!(
                after
                    .elements
                    .iter()
                    .filter(|e| e.key.kind == TopologyKind::Edge)
                    .count(),
                16
            );
            assert_eq!(
                after
                    .elements
                    .iter()
                    .filter(|e| e.key.kind == TopologyKind::Vertex)
                    .count(),
                8
            );
            for face in before
                .elements
                .iter()
                .filter(|e| e.key.kind == TopologyKind::Face)
            {
                let intent = GeometryIntent::capture(&before, face.key).unwrap();
                assert!(matches!(
                    resolve(&intent, &after, 1),
                    Resolution::Resolved { .. }
                ));
            }
            let mesh = relief
                .manufacturing_mesh_with_receipt(Default::default())
                .unwrap();
            assert!(!mesh.mesh.indices.is_empty());
            assert_eq!(mesh.mesh.face_indices.len(), mesh.mesh.indices.len() / 3);
            for face in relief.faces().unwrap() {
                assert!(mesh.mesh.face_indices.contains(&face.index));
            }
            relief.write_step(&path).unwrap();
            let text = std::fs::read_to_string(&path).unwrap();
            let members = multicad_kernel::rust_kernel::step_read::read_import_compound(
                &text,
                policy.linear_tolerance_mm,
            )
            .unwrap();
            assert!(
                relief.step_readback_matches(
                    &after,
                    &members
                        .iter()
                        .map(|(r, s)| (r.clone(), Some(s.clone())))
                        .collect::<Vec<_>>(),
                    policy.linear_tolerance_mm
                )
            );
            let imported = Solid::import_rust_step(&text, feature, policy).unwrap();
            assert!((imported.volume_mm3().unwrap() - expected).abs() < 1e-6);
            imported
                .manufacturing_mesh_with_receipt(Default::default())
                .unwrap();
            let probes = if kind == Kind::Raised {
                vec![
                    ([3., 3., 3.2], true),
                    ([3., 3., 3.8], false),
                    ([8., 6., 4.], true),
                    ([6., 5., 3.2], false),
                    ([3., 3., 1.], true),
                ]
            } else {
                vec![
                    ([3., 3., 2.8], false),
                    ([3., 3., 2.], true),
                    ([8., 6., 2.], false),
                    ([8., 6., 1.], true),
                    ([6., 5., 2.8], true),
                ]
            };
            for &(p, expected) in &probes {
                assert_eq!(
                    relief.rust_normal_relief_contains(point(frame, p)).unwrap(),
                    expected
                );
                match imported.rust_normal_relief_contains(point(frame, p)) {
                    Ok(value) => assert_eq!(value, expected),
                    Err(cause) => {
                        assert_eq!(cause.kind, multicad_kernel::GeometryErrorKind::Unsupported);
                        assert!(matches!(members[0].1.profile,multicad_kernel::rust_kernel::step_read::ImportProfile::PocketedPolygon { .. }));
                    }
                }
            }
            imported.write_step(&path).unwrap();
            let rebuilt = std::fs::read_to_string(&path).unwrap();
            let rebuilt_members = multicad_kernel::rust_kernel::step_read::read_import_compound(
                &rebuilt,
                policy.linear_tolerance_mm,
            )
            .unwrap();
            assert!(
                relief.step_readback_matches(
                    &after,
                    &rebuilt_members
                        .into_iter()
                        .map(|(r, s)| (r, Some(s)))
                        .collect::<Vec<_>>(),
                    policy.linear_tolerance_mm
                )
            );
            if OCCT_LINKED {
                let native = Solid::read_step(&path, policy).unwrap();
                assert_eq!(native.solid_count().unwrap(), 1);
                assert!((native.volume_mm3().unwrap() - expected).abs() < 1e-5);
                for &(p, expected) in &probes {
                    let probe = Solid::box_solid([0.01; 3], policy)
                        .unwrap()
                        .transformed(RigidTransform {
                            translation_mm: point(frame, p),
                            ..Default::default()
                        })
                        .unwrap();
                    let volume = native
                        .boolean(&probe, BooleanOperation::Intersect, policy)
                        .unwrap()
                        .map_or(0., |s| s.volume_mm3().unwrap());
                    assert!((volume / 1e-6 - f64::from(expected)).abs() < 1e-3);
                }
            }
            assert_eq!(blank.volume_mm3().unwrap(), 360.);
        }
    }
}

#[test]
fn side_cap_relief_rejects_shifted_planes_and_cancels_without_changing_stock() {
    use multicad_kernel::GeometryErrorKind;
    use std::cell::Cell;
    let policy = GeometryPolicy::default();
    let owner = EntityId::new();
    let feature = EntityId::new();
    let blank = Solid::rust_box(
        owner.uuid_bits(),
        [12, 10, 3].map(|v| Rational::new(v, 1).unwrap()),
        PlaneFrame::default(),
        policy,
    )
    .unwrap();
    let source = blank.topology_snapshot(owner, 0).unwrap();
    let cap = PlaneFrame {
        origin: [12., 0., 0.],
        u: [0., 1., 0.],
        v: [0., 0., 1.],
    };
    let face = blank
        .faces()
        .unwrap()
        .into_iter()
        .find(|f| f.plane.is_some_and(|(_, n)| n[0] > 0.99))
        .unwrap()
        .index;
    let motifs = [NormalReliefMotif {
        boundary: vec![CurvedEdge2d::Circle {
            center: [3., 1.5],
            radius_mm: 0.5,
        }],
        height_mm: 0.3,
        origin: Origin {
            source: feature,
            role: "side.circle".into(),
        },
    }];
    for kind in [Kind::Raised, Kind::Recessed] {
        let result = blank
            .rust_normal_relief_on_cap(face, cap, &motifs, kind, feature, policy, None)
            .unwrap();
        let expected =
            360. + if kind == Kind::Raised { 1. } else { -1. } * std::f64::consts::PI * 0.25 * 0.3;
        assert!((result.volume_mm3().unwrap() - expected).abs() < 1e-10);
        let query = [11.9, 3., 1.5];
        assert_eq!(
            result.rust_normal_relief_contains(query).unwrap(),
            kind == Kind::Raised
        );
        for offset in [-1e-11, 1e-11] {
            let shifted = PlaneFrame {
                origin: [12. + offset, 0., 0.],
                ..cap
            };
            assert_eq!(
                blank
                    .rust_normal_relief_on_cap(face, shifted, &motifs, kind, feature, policy, None)
                    .err()
                    .unwrap()
                    .kind,
                GeometryErrorKind::Unsupported
            );
        }
    }
    let count = Cell::new(0);
    let count_probe = || {
        count.set(count.get() + 1);
        false
    };
    blank
        .rust_normal_relief_on_cap(
            face,
            cap,
            &motifs,
            Kind::Raised,
            feature,
            policy,
            Some(&count_probe),
        )
        .unwrap();
    assert!(count.get() > 1);
    for boundary in 1..=count.get() {
        let calls = Cell::new(0);
        let pulse = || {
            calls.set(calls.get() + 1);
            calls.get() == boundary
        };
        let error = blank
            .rust_normal_relief_on_cap(
                face,
                cap,
                &motifs,
                Kind::Raised,
                feature,
                policy,
                Some(&pulse),
            )
            .err()
            .unwrap();
        assert_eq!(error.kind, GeometryErrorKind::Cancelled);
        assert!(error.message.ends_with("superseded"));
        assert_eq!(blank.topology_snapshot(owner, 0).unwrap(), source);
        blank
            .rust_normal_relief_on_cap(face, cap, &motifs, Kind::Raised, feature, policy, None)
            .unwrap();
    }
}

#[test]
fn mixed_compounds_prove_each_relief_member_and_preserve_imported_geometry() {
    let policy = GeometryPolicy::default();
    let (_, raised, _) = fixture(Kind::Raised, PlaneFrame::default(), EntityId::new());
    let (_, recessed, _) = fixture(
        Kind::Recessed,
        PlaneFrame {
            origin: [30., 7., -2.],
            u: [0.8, 0.6, 0.],
            v: [-0.36, 0.48, 0.8],
        },
        EntityId::new(),
    );
    let plain = Solid::rust_box(
        EntityId::new().uuid_bits(),
        [2, 3, 4].map(|v| Rational::new(v, 1).unwrap()),
        PlaneFrame {
            origin: [60., 0., 0.],
            ..Default::default()
        },
        policy,
    )
    .unwrap();
    let compound =
        Solid::compound_named(&[&raised, &plain, &recessed], EntityId::new(), policy).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("relief-compound.step");
    compound.write_step(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let (count, volume, bounds) = compound
        .verify_authored_step_compound(&text, policy.linear_tolerance_mm)
        .unwrap()
        .unwrap();
    assert_eq!(count, 3);
    assert!((volume - 744.).abs() < 1e-7);
    let actual = compound.bounds().unwrap();
    for (a, b) in bounds
        .into_iter()
        .flatten()
        .zip(actual.min.into_iter().chain(actual.max))
    {
        assert!((a - b).abs() < 1e-7);
    }
    let imported = Solid::import_rust_step(&text, EntityId::new(), policy).unwrap();
    assert_eq!(imported.solid_count().unwrap(), 3);
    assert!((imported.volume_mm3().unwrap() - volume).abs() < 1e-7);
    imported
        .manufacturing_mesh_with_receipt(Default::default())
        .unwrap();
    imported.write_step(&path).unwrap();
    let rebuilt = std::fs::read_to_string(&path).unwrap();
    // An imported compound can legitimately reorder independently identified
    // members. Its reader still proves all imported finite boundaries.
    let imported_again = Solid::import_rust_step(&rebuilt, EntityId::new(), policy).unwrap();
    assert!((imported_again.volume_mm3().unwrap() - volume).abs() < 1e-7);
    let moved = raised
        .transformed(RigidTransform {
            translation_mm: [0.1, 0., 0.],
            ..Default::default()
        })
        .unwrap();
    let wrong =
        Solid::compound_named(&[&moved, &plain, &recessed], EntityId::new(), policy).unwrap();
    assert!(
        wrong
            .verify_authored_step_compound(&text, policy.linear_tolerance_mm)
            .unwrap()
            .is_err()
    );
}

#[test]
fn unrelated_relief_edit_retains_imported_finite_face_datums() {
    let policy = GeometryPolicy::default();
    let feature = EntityId::new();
    let import = EntityId::new();
    let (blank, first, _) = fixture(Kind::Raised, PlaneFrame::default(), feature);
    let motifs = [([3., 3.], 1.), ([8., 6.], 0.8)]
        .into_iter()
        .enumerate()
        .map(|(i, (center, radius_mm))| NormalReliefMotif {
            boundary: vec![CurvedEdge2d::Circle { center, radius_mm }],
            height_mm: if i == 0 { 0.5 } else { 1.25 },
            origin: Origin {
                source: feature,
                role: format!("motif/{i}"),
            },
        })
        .collect::<Vec<_>>();
    let second = blank
        .rust_normal_relief_on_cap(
            1,
            PlaneFrame {
                origin: [0., 0., 3.],
                ..Default::default()
            },
            &motifs,
            Kind::Raised,
            feature,
            policy,
            None,
        )
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("edit.step");
    first.write_step(&path).unwrap();
    let old = Solid::import_rust_step(&std::fs::read_to_string(&path).unwrap(), import, policy)
        .unwrap()
        .topology_snapshot(import, 0)
        .unwrap();
    second.write_step(&path).unwrap();
    let new = Solid::import_rust_step(&std::fs::read_to_string(&path).unwrap(), import, policy)
        .unwrap()
        .topology_snapshot(import, 1)
        .unwrap();
    let bottom = old.elements.iter().find(|e|matches!(e.geometry,Some(multicad_core::references::AnalyticGeometry::Plane {frame}) if frame.normal()[2] < -0.99)).unwrap();
    let mut intent = GeometryIntent::capture(&old, bottom.key).unwrap();
    intent.attachment = Some(multicad_core::references::Attachment {
        anchor: bottom.frames[0].origin.clone(),
        offset_mm: [0.; 3],
        angle_rad: 0.,
    });
    let Resolution::Resolved { key, frame, .. } = resolve(&intent, &new, 1) else {
        panic!("unchanged finite bottom datum lost");
    };
    assert_eq!(
        frame,
        old.elements
            .iter()
            .find(|e| e.key == bottom.key)
            .unwrap()
            .frames
            .first()
            .map(|a| a.frame)
    );
    assert_eq!(
        new.elements.iter().find(|e| e.key == key).unwrap().origins,
        bottom.origins
    );
}
