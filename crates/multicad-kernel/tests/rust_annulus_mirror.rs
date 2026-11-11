use multicad_core::{document::EntityId, references::*};
use multicad_kernel::*;
use std::f64::consts::{PI, TAU};

fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-7, "{a} != {b}");
}
fn near_point(a: [f64; 3], b: [f64; 3]) {
    for (a, b) in a.into_iter().zip(b) {
        near(a, b);
    }
}
fn reflected(plane: PlaneFrame, value: [f64; 3], point: bool) -> [f64; 3] {
    let n = plane.normal();
    let squared = n.into_iter().map(|v| v * v).sum::<f64>();
    let base = if point { plane.origin } else { [0.; 3] };
    let distance = 2. * (0..3).map(|i| n[i] * (value[i] - base[i])).sum::<f64>() / squared;
    std::array::from_fn(|i| value[i] - distance * n[i])
}
fn profile(frame: PlaneFrame) -> CurvedProfile {
    CurvedProfile {
        frame,
        outer: vec![CurvedEdge2d::Circle {
            center: [1., 2.],
            radius_mm: 5.,
        }],
        holes: vec![vec![CurvedEdge2d::Circle {
            center: [1., 2.],
            radius_mm: 2.,
        }]],
    }
}
#[test]
fn rust_annulus_mirror_preserves_every_named_boundary_and_reflects_cap_and_rim_frames() {
    let policy = GeometryPolicy::default();
    for frame in [
        PlaneFrame::default(),
        PlaneFrame {
            origin: [5., -9., 21.],
            u: [0.8, 0.6, 0.],
            v: [-0.36, 0.48, 0.8],
        },
    ] {
        for height in [3.25, -3.25] {
            let source_id = EntityId::new();
            let origins = ["bore", "outer"].map(|role| Origin {
                source: EntityId::new(),
                role: role.into(),
            });
            let source = Solid::rust_annulus_from_sketch(
                [2., 5.],
                height,
                source_id,
                frame,
                [1., 2.],
                origins.clone(),
                policy,
            )
            .unwrap()
            .with_member_origin(EntityId::new(), 3)
            .unwrap();
            let before = source.topology_snapshot(source_id, 0).unwrap();
            let source_bounds = source.bounds().unwrap();
            for plane in [
                PlaneFrame::default(),
                PlaneFrame {
                    origin: [3., -2., 7.],
                    u: [0.8, 0., -0.6],
                    v: [0., 1., 0.],
                },
                PlaneFrame {
                    origin: [3., -2., 7.],
                    u: [0., 0.8, 0.6],
                    v: [1., 0., 0.],
                },
                PlaneFrame {
                    u: [(1_f64 + 8e-11).sqrt(), 0., 0.],
                    ..Default::default()
                },
            ] {
                let result_id = EntityId::new();
                let mirrored = source.mirrored_named(plane, result_id, policy).unwrap();
                let after = mirrored.topology_snapshot(result_id, 1).unwrap();
                assert_eq!(after.elements.len(), 8);
                assert_eq!(mirrored.solid_count().unwrap(), 1);
                near(mirrored.volume_mm3().unwrap(), PI * 21. * 3.25);
                for original in &before.elements {
                    let mut intent = GeometryIntent::capture(&before, original.key).unwrap();
                    intent.boundary = Some(original.boundary.clone());
                    if original.key.kind == TopologyKind::Face && original.key.ordinal < 2 {
                        intent.attachment = Some(Attachment {
                            anchor: Origin {
                                source: source_id,
                                role: if original.key.ordinal == 0 {
                                    "cap.start"
                                } else {
                                    "cap.end"
                                }
                                .into(),
                            },
                            offset_mm: [0.; 3],
                            angle_rad: 0.,
                        });
                    }
                    let Resolution::Resolved {
                        key,
                        frame: attachment,
                        ..
                    } = resolve(&intent, &after, 1)
                    else {
                        panic!("Lost mirrored boundary {intent:?}")
                    };
                    let current = after.elements.iter().find(|e| e.key == key).unwrap();
                    assert_eq!(original.origins, current.origins);
                    assert_eq!(original.boundary, current.boundary);
                    match (original.geometry, current.geometry) {
                        (
                            Some(AnalyticGeometry::Plane { frame: old }),
                            Some(AnalyticGeometry::Plane { frame: new }),
                        ) => {
                            near_point(new.origin_mm, reflected(plane, old.origin_mm, true));
                            near_point(new.normal(), reflected(plane, old.normal(), false));
                            for anchor in &original.frames {
                                let reflected_anchor = current
                                    .frames
                                    .iter()
                                    .find(|item| item.origin == anchor.origin)
                                    .unwrap()
                                    .frame;
                                near_point(
                                    reflected_anchor.origin_mm,
                                    reflected(plane, anchor.frame.origin_mm, true),
                                );
                                near_point(
                                    reflected_anchor.u,
                                    reflected(plane, anchor.frame.u, false),
                                );
                                near_point(
                                    reflected_anchor.v,
                                    reflected(plane, anchor.frame.v, false).map(|v| -v),
                                );
                                near_point(
                                    reflected_anchor.normal(),
                                    reflected(plane, anchor.frame.normal(), false),
                                );
                            }
                            let attached = attachment.unwrap();
                            let old = original
                                .frames
                                .iter()
                                .find(|a| a.origin.source == source_id)
                                .unwrap()
                                .frame;
                            near_point(attached.origin_mm, reflected(plane, old.origin_mm, true));
                            near_point(attached.u, reflected(plane, old.u, false));
                        }
                        (
                            Some(AnalyticGeometry::CircularArc {
                                frame: old,
                                radius_mm: old_r,
                                start_rad: old_start,
                                sweep_rad: old_sweep,
                            }),
                            Some(AnalyticGeometry::CircularArc {
                                frame: new,
                                radius_mm: new_r,
                                start_rad: new_start,
                                sweep_rad: new_sweep,
                            }),
                        ) => {
                            assert_eq!(old_r, new_r);
                            assert_eq!(old_start, new_start);
                            assert_eq!(old_sweep, new_sweep);
                            near(old_sweep.abs(), TAU);
                            near_point(new.origin_mm, reflected(plane, old.origin_mm, true));
                            near_point(new.u, reflected(plane, old.u, false));
                            near_point(new.v, reflected(plane, old.v, false));
                            for angle in [0_f64, 0.3, 1.2, 4.7] {
                                let point = |frame: ReferenceFrame| {
                                    std::array::from_fn(|i| {
                                        frame.origin_mm[i]
                                            + old_r
                                                * (frame.u[i] * angle.cos()
                                                    + frame.v[i] * angle.sin())
                                    })
                                };
                                near_point(point(new), reflected(plane, point(old), true));
                            }
                        }
                        (None, None) => {}
                        other => panic!("Changed boundary kind {other:?}"),
                    }
                }
                let mesh = mirrored
                    .manufacturing_mesh_with_receipt(MeshOptions::default())
                    .unwrap();
                assert!(!mesh.mesh.indices.is_empty());
                assert_eq!(mesh.edge_pairs as usize * 2, mesh.mesh.indices.len());
                let repeated = mirrored
                    .mirrored_named(plane, EntityId::new(), policy)
                    .unwrap();
                let bounds = repeated.bounds().unwrap();
                near_point(bounds.min, source_bounds.min);
                near_point(bounds.max, source_bounds.max);
                let restored = repeated.topology_snapshot(result_id, 2).unwrap();
                for original in &before.elements {
                    let current = restored
                        .elements
                        .iter()
                        .find(|e| e.key == original.key)
                        .unwrap();
                    for anchor in &original.frames {
                        let restored_frame = current
                            .frames
                            .iter()
                            .find(|item| item.origin == anchor.origin)
                            .unwrap()
                            .frame;
                        near_point(restored_frame.origin_mm, anchor.frame.origin_mm);
                        near_point(restored_frame.u, anchor.frame.u);
                        near_point(restored_frame.v, anchor.frame.v);
                        let mut datum = GeometryIntent::capture(&before, original.key).unwrap();
                        datum.boundary = Some(original.boundary.clone());
                        datum.attachment = Some(Attachment {
                            anchor: anchor.origin.clone(),
                            offset_mm: [0.25, -0.5, 0.75],
                            angle_rad: 0.3,
                        });
                        let Resolution::Resolved {
                            frame: Some(expected),
                            ..
                        } = resolve(&datum, &before, 0)
                        else {
                            panic!("source datum")
                        };
                        let Resolution::Resolved {
                            frame: Some(actual),
                            ..
                        } = resolve(&datum, &restored, 2)
                        else {
                            panic!("restored datum")
                        };
                        near_point(actual.origin_mm, expected.origin_mm);
                        near_point(actual.u, expected.u);
                        near_point(actual.v, expected.v);
                    }
                }
                let rotation = [[0., -1., 0.], [1., 0., 0.], [0., 0., 1.]];
                let rotated = |point: [f64; 3]| {
                    rotation.map(|row| (0..3).map(|i| row[i] * point[i]).sum::<f64>())
                };
                let posed = mirrored
                    .transformed(RigidTransform {
                        rotation,
                        translation_mm: [7., 1., -3.],
                    })
                    .unwrap();
                let placed = posed.topology_snapshot(result_id, 2).unwrap();
                for old in &after.elements {
                    let current = placed.elements.iter().find(|e| e.key == old.key).unwrap();
                    for (a, b) in old.frames.iter().zip(&current.frames) {
                        near_point(
                            b.frame.origin_mm,
                            std::array::from_fn(|i| {
                                rotated(a.frame.origin_mm)[i] + [7., 1., -3.][i]
                            }),
                        );
                        near_point(rotated(a.frame.u), b.frame.u);
                        near_point(rotated(a.frame.v), b.frame.v);
                    }
                }
                let directory = tempfile::tempdir().unwrap();
                let path = directory.path().join("mirrored-annulus.step");
                mirrored.write_step(&path).unwrap();
                let imported = Solid::import_rust_step(
                    &std::fs::read_to_string(&path).unwrap(),
                    EntityId::new(),
                    policy,
                )
                .unwrap();
                let bounds = mirrored.bounds().unwrap();
                near_point(imported.bounds().unwrap().min, bounds.min);
                near_point(imported.bounds().unwrap().max, bounds.max);
                near(
                    imported.volume_mm3().unwrap(),
                    mirrored.volume_mm3().unwrap(),
                );
                if OCCT_LINKED {
                    let native = Solid::extrude_curved_profile_vector_named(
                        &profile(frame),
                        frame.normal().map(|n| n * height),
                        source_id,
                        &[origins[1].clone(), origins[0].clone()],
                        policy,
                    )
                    .unwrap()
                    .mirrored_named(plane, result_id, policy)
                    .unwrap();
                    near_point(native.bounds().unwrap().min, bounds.min);
                    near_point(native.bounds().unwrap().max, bounds.max);
                    near(native.volume_mm3().unwrap(), mirrored.volume_mm3().unwrap());
                    let native = Solid::read_step(&path, policy).unwrap();
                    near(native.volume_mm3().unwrap(), mirrored.volume_mm3().unwrap());
                    assert_eq!(native.solid_count().unwrap(), 1);
                }
                assert_eq!(source.topology_snapshot(source_id, 0).unwrap(), before);
                assert_eq!(source.bounds().unwrap(), source_bounds);
            }
        }
    }
}

#[test]
fn rust_annulus_mirror_refuses_bad_policy_and_out_of_bounds_without_mutating_the_source() {
    let policy = GeometryPolicy::default();
    let id = EntityId::new();
    let source = Solid::rust_annulus(2., 5., 3., id, Default::default(), policy).unwrap();
    let before = source.topology_snapshot(id, 0).unwrap();
    let mut other = policy;
    other.minimum_feature_mm *= 2.;
    assert_eq!(
        source
            .mirrored_named(Default::default(), EntityId::new(), other)
            .err()
            .unwrap()
            .kind,
        GeometryErrorKind::Unsupported
    );
    for plane in [
        PlaneFrame {
            origin: [0., 0., policy.maximum_coordinate_mm],
            ..Default::default()
        },
        PlaneFrame {
            u: [0.; 3],
            ..Default::default()
        },
    ] {
        assert_eq!(
            source
                .mirrored_named(plane, EntityId::new(), policy)
                .err()
                .unwrap()
                .kind,
            GeometryErrorKind::Input
        );
    }
    let mirrored = source
        .mirrored_named(Default::default(), EntityId::new(), policy)
        .unwrap();
    for shape in [&source, &mirrored] {
        let snapshot = shape.topology_snapshot(id, 0).unwrap();
        let edge = snapshot
            .elements
            .iter()
            .find(|e| e.key.kind == TopologyKind::Edge)
            .unwrap();
        let treatment = EntityId::new();
        let selection = TreatmentEdge {
            key: edge.key,
            origin: Origin {
                source: treatment,
                role: "chamfer".into(),
            },
        };
        assert_eq!(
            shape
                .treat_edges_named(
                    &[selection],
                    EdgeTreatment::Chamfer { distance_mm: 0.25 },
                    treatment,
                    policy
                )
                .err()
                .unwrap()
                .kind,
            GeometryErrorKind::Unsupported
        );
    }
    assert_eq!(source.topology_snapshot(id, 0).unwrap(), before);
}

#[test]
fn rust_annulus_mirror_uses_the_true_inverse_for_admitted_source_chart_residuals() {
    let policy = GeometryPolicy::default();
    for frame in [
        PlaneFrame {
            origin: [-1e6, 0., 0.],
            u: [(1_f64 + 8e-11).sqrt(), 0., 0.],
            v: [0., 1., 0.],
        },
        PlaneFrame {
            origin: [-1e6, -1e6, 0.],
            u: [1., 0., 0.],
            v: [8e-11, 1., 0.],
        },
    ] {
        let uv = if frame.origin[1] == 0. {
            [1e6, 0.]
        } else {
            [1e6, 1e6]
        };
        let feature = EntityId::new();
        let source = Solid::rust_annulus_from_sketch(
            [2., 5.],
            3.,
            feature,
            frame,
            uv,
            ["inner", "outer"].map(|role| Origin {
                source: EntityId::new(),
                role: role.into(),
            }),
            policy,
        )
        .unwrap();
        let before = source.topology_snapshot(feature, 0).unwrap();
        let mirrored = source
            .mirrored_named(Default::default(), EntityId::new(), policy)
            .unwrap();
        let after = mirrored.topology_snapshot(feature, 1).unwrap();
        let restored = mirrored
            .mirrored_named(Default::default(), EntityId::new(), policy)
            .unwrap()
            .topology_snapshot(feature, 2)
            .unwrap();
        for old in before
            .elements
            .iter()
            .filter(|e| e.key.kind == TopologyKind::Face && e.key.ordinal < 2)
        {
            let new = after.elements.iter().find(|e| e.key == old.key).unwrap();
            let back = restored.elements.iter().find(|e| e.key == old.key).unwrap();
            near_point(
                new.frames[0].frame.origin_mm,
                reflected(Default::default(), old.frames[0].frame.origin_mm, true),
            );
            near_point(
                back.frames[0].frame.origin_mm,
                old.frames[0].frame.origin_mm,
            );
            near_point(back.frames[0].frame.u, old.frames[0].frame.u);
            near_point(back.frames[0].frame.v, old.frames[0].frame.v);
            let mut datum = GeometryIntent::capture(&before, old.key).unwrap();
            datum.attachment = Some(Attachment {
                anchor: old.frames[0].origin.clone(),
                offset_mm: [0.5, -0.25, 1.],
                angle_rad: 0.2,
            });
            let Resolution::Resolved {
                frame: Some(expected),
                ..
            } = resolve(&datum, &before, 0)
            else {
                panic!("source datum")
            };
            let Resolution::Resolved {
                frame: Some(actual),
                ..
            } = resolve(&datum, &restored, 2)
            else {
                panic!("restored datum")
            };
            near_point(actual.origin_mm, expected.origin_mm);
            near_point(actual.u, expected.u);
            near_point(actual.v, expected.v);
        }
    }
}
