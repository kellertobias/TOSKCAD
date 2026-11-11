use super::*;
use multicad_core::references::{Attachment, GeometryIntent, Resolution, TopologyKind, resolve};
use std::f64::consts::{PI, TAU};

fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 2e-6, "{a} != {b}");
}
fn compare(a: &Solid, b: &Solid) {
    near(a.volume_mm3().unwrap(), b.volume_mm3().unwrap());
    near(
        a.faces().unwrap().iter().map(|f| f.area_mm2).sum(),
        b.faces().unwrap().iter().map(|f| f.area_mm2).sum(),
    );
    let (a, b) = (a.bounds().unwrap(), b.bounds().unwrap());
    for (a, b) in a
        .min
        .into_iter()
        .chain(a.max)
        .zip(b.min.into_iter().chain(b.max))
    {
        near(a, b);
    }
}
fn physical(solid: &Solid) {
    assert!(solid.uses_rust_geometry());
    solid
        .manufacturing_mesh_with_receipt(Default::default())
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("constructed.step");
    solid.write_step(&path).unwrap();
    let read = Solid::import_rust_step(
        &std::fs::read_to_string(&path).unwrap(),
        EntityId::new(),
        Default::default(),
    )
    .unwrap();
    compare(solid, &read);
    #[cfg(feature = "occt")]
    {
        let native = Solid::read_step(&path, Default::default()).unwrap();
        assert!(!native.uses_rust_geometry());
        compare(solid, &native);
    }
}
fn frame() -> PlaneFrame {
    PlaneFrame {
        origin: [7., -2., 3.],
        u: [0.8, 0.6, 0.],
        v: [-0.36, 0.48, 0.8],
    }
}
fn assert_backend(solid: &Solid) {
    assert_eq!(solid.uses_rust_geometry(), !crate::OCCT_LINKED);
}

#[test]
fn public_primitive_constructors_select_linked_native_or_owned_exact_geometry() {
    let policy = GeometryPolicy::default();
    let feature = EntityId::new();
    let owned = box_solid([3., 4., 5.], feature, policy).unwrap();
    let public = Solid::box_solid([3., 4., 5.], policy).unwrap();
    assert_backend(&public);
    near(owned.volume_mm3().unwrap(), 60.);
    compare(&owned, &public);
    physical(&owned);
    let owned = named_box([3., 4., 5.], feature, policy).unwrap();
    let public = Solid::box_named([3., 4., 5.], feature, policy).unwrap();
    assert_backend(&public);
    compare(&owned, &public);
    let topology = owned.topology_snapshot(feature, 0).unwrap();
    for role in ["y.min", "x.max", "y.max", "x.min"] {
        assert!(
            topology
                .elements
                .iter()
                .any(|e| e.key.kind == TopologyKind::Face
                    && e.origins.contains(&Origin {
                        source: feature,
                        role: role.into()
                    }))
        );
    }
    physical(&owned);
    for placement in [
        PlaneFrame::default(),
        frame(),
        PlaneFrame {
            v: [0., -1., 0.],
            ..PlaneFrame::default()
        },
        PlaneFrame {
            origin: [1e-30, 0., 0.],
            ..PlaneFrame::default()
        },
    ] {
        let owned = cylinder(placement, 1.5, 3., feature, policy).unwrap();
        let public = Solid::cylinder(placement, 1.5, 3., policy).unwrap();
        assert_backend(&public);
        near(owned.volume_mm3().unwrap(), 6.75 * PI);
        compare(&owned, &public);
        physical(&owned);
        if placement.origin[0] == 1e-30 {
            let topology = owned.topology_snapshot(feature, 0).unwrap();
            assert!(topology.elements.iter().any(|element|matches!(element.geometry,Some(multicad_core::references::AnalyticGeometry::Plane{frame})if frame.origin_mm==placement.origin)));
            assert!(
                owned.axial_cylinder().is_none(),
                "local placement must not be misclassified as an exact world-axis descriptor"
            );
        }
    }
}

#[test]
fn profile_constructors_retain_holes_signed_extrusion_frames_and_authored_references() {
    let policy = GeometryPolicy::default();
    let feature = EntityId::new();
    for placement in [PlaneFrame::default(), frame()] {
        for height in [-3., 3.] {
            let mut source = Profile::rectangle(8., 6.);
            source.frame = placement;
            source.holes.push(vec![
                ProfileEdge::Line {
                    a: [1., 1.],
                    b: [3., 1.],
                },
                ProfileEdge::Line {
                    a: [3., 1.],
                    b: [3., 2.],
                },
                ProfileEdge::Line {
                    a: [3., 2.],
                    b: [1., 2.],
                },
                ProfileEdge::Line {
                    a: [1., 2.],
                    b: [1., 1.],
                },
            ]);
            let ids = origins(feature, 8);
            let owned = profile(&source, height, feature, &ids, policy).unwrap();
            let public =
                Solid::extrude_profile_named(&source, height, feature, &ids, policy).unwrap();
            assert_backend(&public);
            near(owned.volume_mm3().unwrap(), 138.);
            compare(&owned, &public);
            let topology = owned.topology_snapshot(feature, 0).unwrap();
            let cap = topology
                .elements
                .iter()
                .find(|e| {
                    e.key.kind == TopologyKind::Face
                        && e.origins.contains(&Origin {
                            source: feature,
                            role: "cap.start".into(),
                        })
                })
                .unwrap();
            let mut intent = GeometryIntent::capture(&topology, cap.key).unwrap();
            intent.attachment = Some(Attachment {
                anchor: cap.frames[0].origin.clone(),
                offset_mm: [0.25, -0.5, 0.75],
                angle_rad: 0.3,
            });
            assert!(matches!(
                resolve(&intent, &topology, 0),
                Resolution::Resolved { frame: Some(_), .. }
            ));
            for origin in &ids {
                assert!(
                    topology
                        .elements
                        .iter()
                        .any(|e| e.key.kind == TopologyKind::Face && e.origins.contains(origin))
                );
            }
            physical(&owned);
        }
    }
}

#[test]
fn circular_and_mixed_arc_profiles_preserve_real_curves_and_native_oracle_independence() {
    let policy = GeometryPolicy::default();
    let feature = EntityId::new();
    for placement in [PlaneFrame::default(), frame()] {
        for height in [-3., 3.] {
            let mut source = CurvedProfile {
                frame: placement,
                outer: vec![CurvedEdge2d::Circle {
                    center: [2., -1.],
                    radius_mm: 2.,
                }],
                holes: vec![],
            };
            for bore in [false, true] {
                if bore {
                    source.holes = vec![vec![CurvedEdge2d::Circle {
                        center: [2., -1.],
                        radius_mm: 0.5,
                    }]];
                }
                let ids = origins(feature, if bore { 2 } else { 1 });
                source.checked(policy).unwrap();
                let owned = normal(&source, height, feature, &ids, policy).unwrap();
                let public =
                    Solid::extrude_curved_profile_named(&source, height, feature, &ids, policy)
                        .unwrap();
                assert_backend(&public);
                near(
                    owned.volume_mm3().unwrap(),
                    3. * PI * if bore { 3.75 } else { 4. },
                );
                compare(&owned, &public);
                physical(&owned);
                if !bore {
                    assert!(owned.axial_cylinder_frame().is_some());
                }
            }
            let source = CurvedProfile {
                frame: placement,
                outer: vec![
                    CurvedEdge2d::Line {
                        a: [0., 0.],
                        b: [4., 0.],
                    },
                    CurvedEdge2d::Arc {
                        center: [4., 1.],
                        a: [4., 0.],
                        b: [4., 2.],
                        clockwise: false,
                    },
                    CurvedEdge2d::Line {
                        a: [4., 2.],
                        b: [0., 2.],
                    },
                    CurvedEdge2d::Line {
                        a: [0., 2.],
                        b: [0., 0.],
                    },
                ],
                holes: vec![vec![CurvedEdge2d::Circle {
                    center: [1., 1.],
                    radius_mm: 0.25,
                }]],
            };
            let ids = origins(feature, 5);
            source.checked(policy).unwrap();
            let owned = normal(&source, height, feature, &ids, policy).unwrap();
            let public =
                Solid::extrude_curved_profile_named(&source, height, feature, &ids, policy)
                    .unwrap();
            assert_backend(&public);
            near(owned.volume_mm3().unwrap(), 3. * (8. + 7. * PI / 16.));
            compare(&owned, &public);
            physical(&owned);
        }
    }
}

#[test]
fn straight_vector_and_manufacturing_profiles_keep_their_own_geometry_budget() {
    let policy = GeometryPolicy::default();
    let feature = EntityId::new();
    let points = [[0., 0.], [4., 0.], [4., 3.], [0., 3.]];
    let source = CurvedProfile {
        frame: PlaneFrame::default(),
        outer: points
            .into_iter()
            .zip(points.into_iter().cycle().skip(1))
            .map(|(a, b)| CurvedEdge2d::Line { a, b })
            .collect(),
        holes: vec![],
    };
    let ids = origins(feature, 4);
    source.checked(policy).unwrap();
    let owned = vector(&source, [2., -1., 5.], feature, &ids, policy).unwrap();
    let public =
        Solid::extrude_curved_profile_vector_named(&source, [2., -1., 5.], feature, &ids, policy)
            .unwrap();
    assert_backend(&public);
    near(owned.volume_mm3().unwrap(), 60.);
    compare(&owned, &public);
    physical(&owned);
    let points: Vec<_> = (0..70)
        .map(|i| {
            let t = TAU * i as f64 / 70.;
            [10. * t.cos(), 10. * t.sin()]
        })
        .collect();
    let source = CurvedProfile {
        frame: PlaneFrame::default(),
        outer: points
            .iter()
            .copied()
            .zip(points.iter().copied().cycle().skip(1))
            .map(|(a, b)| CurvedEdge2d::Line { a, b })
            .collect(),
        holes: vec![],
    };
    assert_eq!(
        Solid::extrude_curved_profile(&source, 2., policy)
            .err()
            .unwrap()
            .kind,
        GeometryErrorKind::Limit
    );
    let public = Solid::extrude_panel_profile_vector_named(
        &source,
        [0., 0., 2.],
        feature,
        &origins(feature, 70),
        policy,
    )
    .unwrap();
    assert_backend(&public);
    near(public.volume_mm3().unwrap(), 70. * 100. * (TAU / 70.).sin());
    #[cfg(not(feature = "occt"))]
    physical(&public);
}

#[test]
fn failed_constructor_admission_is_typed_and_keeps_authored_source_unchanged() {
    let policy = GeometryPolicy::default();
    let feature = EntityId::new();
    assert_eq!(
        box_solid([f64::NAN, 1., 1.], feature, policy)
            .err()
            .unwrap()
            .kind,
        GeometryErrorKind::Input
    );
    assert_eq!(
        box_solid([2_f64.sqrt(), 3_f64.sqrt(), 1.], feature, policy)
            .err()
            .unwrap()
            .kind,
        GeometryErrorKind::Limit
    );
    let source = CurvedProfile {
        frame: PlaneFrame::default(),
        outer: vec![CurvedEdge2d::Circle {
            center: [0.; 2],
            radius_mm: 2.,
        }],
        holes: vec![],
    };
    let before = source.clone();
    assert_eq!(
        vector(
            &source,
            [0.01, 0., 3.],
            feature,
            &origins(feature, 1),
            policy
        )
        .err()
        .unwrap()
        .kind,
        GeometryErrorKind::Unsupported
    );
    assert!(normal(&source, 3., EntityId::nil(), &origins(feature, 1), policy).is_err());
    let mut lines = CurvedProfile {
        frame: PlaneFrame::default(),
        outer: vec![
            CurvedEdge2d::Line {
                a: [0., 0.],
                b: [1., 0.],
            },
            CurvedEdge2d::Line {
                a: [1., 0.],
                b: [1., 1.],
            },
            CurvedEdge2d::Line {
                a: [1., 1.],
                b: [0., 1.],
            },
            CurvedEdge2d::Line {
                a: [0., 1.],
                b: [0., 0.],
            },
        ],
        holes: vec![],
    };
    let mut ids = origins(feature, 4);
    ids[1] = ids[0].clone();
    assert!(normal(&lines, 3., feature, &ids, policy).is_err());
    lines.outer.pop();
    assert!(normal(&lines, 3., feature, &origins(feature, 3), policy).is_err());
    assert_eq!(source, before);
}

#[test]
fn anonymous_geometry_has_no_fabricated_persistent_sources_after_share_or_pose() {
    let policy = GeometryPolicy::default();
    let owner = EntityId::new();
    let points = [[0., 0.], [4., 0.], [4., 3.], [0., 3.]];
    let source = CurvedProfile {
        frame: PlaneFrame::default(),
        outer: points
            .into_iter()
            .zip(points.into_iter().cycle().skip(1))
            .map(|(a, b)| CurvedEdge2d::Line { a, b })
            .collect(),
        holes: vec![],
    };
    for solid in [
        Solid::box_solid([4., 3., 2.], policy).unwrap(),
        Solid::cylinder(PlaneFrame::default(), 2., 3., policy).unwrap(),
        Solid::extrude_profile(&Profile::rectangle(4., 3.), 2., policy).unwrap(),
        Solid::extrude_curved_profile(&source, 2., policy).unwrap(),
        Solid::extrude_curved_profile_vector(&source, [1., 0., 2.], policy).unwrap(),
    ] {
        for value in [
            solid.share().unwrap(),
            solid
                .transformed(RigidTransform {
                    rotation: [[0., -1., 0.], [1., 0., 0.], [0., 0., 1.]],
                    translation_mm: [3., 4., 5.],
                })
                .unwrap(),
        ] {
            let snapshot = value.topology_snapshot(owner, 0).unwrap();
            snapshot.validate().unwrap();
            assert!(
                snapshot
                    .elements
                    .iter()
                    .all(|e| e.origins.is_empty() && e.boundary.is_empty() && e.frames.is_empty())
            );
            assert!(snapshot.elements.iter().any(|e| e.geometry.is_some()));
            for face in snapshot
                .elements
                .iter()
                .filter(|e| e.key.kind == TopologyKind::Face)
            {
                assert!(GeometryIntent::capture(&snapshot, face.key).is_err());
            }
        }
    }
    let aliased = Solid::box_solid([4., 3., 2.], policy)
        .unwrap()
        .with_component_origin(owner, EntityId::new())
        .unwrap();
    let aliased = aliased
        .transformed(RigidTransform {
            translation_mm: [2., 3., 4.],
            ..Default::default()
        })
        .unwrap();
    let snapshot = aliased.topology_snapshot(owner, 0).unwrap();
    snapshot.validate().unwrap();
    assert!(snapshot.elements.iter().all(|element| {
        element
            .origins
            .iter()
            .all(|origin| origin.source == owner && origin.role.starts_with("component/"))
    }));
    if !crate::OCCT_LINKED {
        let stock = Solid::box_solid([4., 3., 2.], policy).unwrap();
        let scaled = stock
            .affine_transformed_named(
                crate::AffineTransform {
                    matrix: [[2., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
                    ..Default::default()
                },
                EntityId::new(),
                policy,
            )
            .unwrap();
        let snapshot = scaled.topology_snapshot(owner, 0).unwrap();
        snapshot.validate().unwrap();
        assert!(
            snapshot
                .elements
                .iter()
                .all(|e| e.origins.is_empty() && e.frames.is_empty())
        );
        let result = stock
            .boolean(
                &Solid::box_solid([2., 2., 3.], policy).unwrap(),
                BooleanOperation::Subtract,
                policy,
            )
            .unwrap();
        let result = result.unwrap();
        near(result.volume_mm3().unwrap(), 16.);
        let result_snapshot = result.topology_snapshot(owner, 0).unwrap();
        result_snapshot.validate().unwrap();
        assert!(
            result_snapshot
                .elements
                .iter()
                .all(|e| e.origins.is_empty() && e.frames.is_empty())
        );
        let shear = crate::AffineTransform {
            matrix: [[1., 0.25, 0.], [0., 1., 0.], [0., 0., 1.]],
            ..Default::default()
        };
        let first = stock
            .affine_transformed_named(shear, EntityId::new(), policy)
            .unwrap();
        let second = stock
            .affine_transformed_named(
                crate::AffineTransform {
                    translation_mm: [1., 0., 0.],
                    ..shear
                },
                EntityId::new(),
                policy,
            )
            .unwrap();
        let intersection = first
            .boolean(&second, BooleanOperation::Intersect, policy)
            .unwrap()
            .unwrap();
        near(intersection.volume_mm3().unwrap(), 18.);
        let snapshot = intersection.topology_snapshot(owner, 0).unwrap();
        snapshot.validate().unwrap();
        assert!(
            snapshot
                .elements
                .iter()
                .all(|element| element.origins.is_empty() && element.frames.is_empty())
        );
        let named = Solid::box_named([4., 3., 2.], owner, policy).unwrap();
        let mixed = named
            .boolean(
                &Solid::box_solid([2., 2., 3.], policy).unwrap(),
                BooleanOperation::Subtract,
                policy,
            )
            .unwrap()
            .unwrap();
        let snapshot = mixed.topology_snapshot(owner, 0).unwrap();
        snapshot.validate().unwrap();
        assert_eq!(
            snapshot
                .elements
                .iter()
                .filter(
                    |element| element.key.kind == TopologyKind::Face && element.origins.is_empty()
                )
                .count(),
            2
        );
        assert!(
            snapshot
                .elements
                .iter()
                .any(|element| element.origins.contains(&Origin {
                    source: owner,
                    role: "cap.end".into()
                }))
        );
        let bore = Solid::cylinder(
            PlaneFrame {
                origin: [2., 1.5, 0.],
                ..Default::default()
            },
            0.5,
            3.,
            policy,
        )
        .unwrap();
        let bored = named
            .boolean(&bore, BooleanOperation::Subtract, policy)
            .unwrap()
            .unwrap();
        let snapshot = bored.topology_snapshot(owner, 0).unwrap();
        snapshot.validate().unwrap();
        assert!(
            snapshot
                .elements
                .iter()
                .all(|element| element.origins.iter().all(|origin| origin.source == owner))
        );
        assert!(snapshot.elements.iter().any(|element|element.key.kind==TopologyKind::Face && element.origins.is_empty()));
        let posed = bored
            .transformed(RigidTransform {
                rotation: [[0., -1., 0.], [1., 0., 0.], [0., 0., 1.]],
                translation_mm: [3., 4., 5.],
            })
            .unwrap();
        let posed_snapshot = posed.topology_snapshot(owner, 0).unwrap();
        posed_snapshot.validate().unwrap();
        assert!(
            posed_snapshot
                .elements
                .iter()
                .all(|element| element.origins.iter().all(|origin| origin.source == owner))
        );
    }
}
