use multicad_core::{document::EntityId, references::*};
use multicad_kernel::{
    rust_kernel::{Circle, Rational, cylinder::AxialCylinder},
    *,
};
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
        holes: vec![],
    }
}
fn cylinder_source(
    feature: EntityId,
    frame: PlaneFrame,
    center_uv: [f64; 2],
    signed_height: f64,
    authored: bool,
    origin: Origin,
    policy: GeometryPolicy,
) -> Solid {
    let geometry = AxialCylinder::new(
        feature.uuid_bits(),
        Circle::new(
            [Rational::new(0, 1).unwrap(); 2],
            Rational::new(5, 1).unwrap(),
        )
        .unwrap(),
        Rational::new(0, 1).unwrap(),
        Rational::new((signed_height.abs() * 4.) as i64, 4).unwrap(),
    )
    .unwrap();
    let placed = PlaneFrame {
        origin: frame.point(center_uv),
        u: frame.u,
        v: frame.v.map(|x| if signed_height < 0. { -x } else { x }),
    };
    if authored {
        Solid::from_axial_cylinder_sweep(
            geometry,
            placed,
            center_uv,
            signed_height < 0.,
            origin,
            policy,
        )
        .unwrap()
    } else {
        Solid::from_axial_cylinder(geometry, PlaneFrame::default(), policy)
            .unwrap()
            .transformed(RigidTransform {
                rotation: std::array::from_fn(|i| [placed.u[i], placed.v[i], placed.normal()[i]]),
                translation_mm: placed.origin,
            })
            .unwrap()
    }
}
#[test]
fn rust_cylinder_mirror_preserves_every_named_boundary_and_reflects_cap_and_rim_frames() {
    let policy = GeometryPolicy::default();
    for frame in [
        PlaneFrame::default(),
        PlaneFrame {
            origin: [5., -9., 21.],
            u: [0.8, 0.6, 0.],
            v: [-0.36, 0.48, 0.8],
        },
    ] {
        for (height, authored) in [(3.25, false), (-3.25, false), (3.25, true), (-3.25, true)] {
            let source_id = EntityId::new();
            let origin = Origin {
                source: EntityId::new(),
                role: "circle".into(),
            };
            let source = cylinder_source(
                source_id,
                frame,
                [1., 2.],
                height,
                authored,
                origin.clone(),
                policy,
            )
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
                assert_eq!(after.elements.len(), 5);
                assert_eq!(mirrored.solid_count().unwrap(), 1);
                near(mirrored.volume_mm3().unwrap(), PI * 25. * 3.25);
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
                            // Primitive rims are unauthored circles: proper-chart V
                            // reverses the parameter direction, not the support.
                            near_point(
                                new.v,
                                reflected(plane, old.v, false)
                                    .map(|x| if authored { x } else { -x }),
                            );
                            for angle in [0_f64, 0.3, 1.2, 4.7] {
                                let point = |frame: ReferenceFrame, angle: f64| {
                                    std::array::from_fn(|i| {
                                        frame.origin_mm[i]
                                            + old_r
                                                * (frame.u[i] * angle.cos()
                                                    + frame.v[i] * angle.sin())
                                    })
                                };
                                near_point(
                                    point(new, if authored { angle } else { -angle }),
                                    reflected(plane, point(old, angle), true),
                                );
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
                let path = directory.path().join("mirrored-cylinder.step");
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
                        std::slice::from_ref(&origin),
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
fn rust_cylinder_mirror_refuses_bad_policy_and_out_of_bounds_without_mutating_the_source() {
    let policy = GeometryPolicy::default();
    let id = EntityId::new();
    let source = cylinder_source(
        id,
        Default::default(),
        [0.; 2],
        3.,
        true,
        Origin {
            source: id,
            role: "circle".into(),
        },
        policy,
    );
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
    assert_eq!(source.topology_snapshot(id, 0).unwrap(), before);
}

#[test]
fn rust_cylinder_mirror_uses_the_true_inverse_for_admitted_source_chart_residuals() {
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
        let source = cylinder_source(
            feature,
            frame,
            uv,
            3.,
            true,
            Origin {
                source: feature,
                role: "circle".into(),
            },
            policy,
        );
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

#[test]
fn mirrored_primitive_and_authored_cylinders_remain_exact_chamfer_draft_and_shell_inputs() {
    let policy = GeometryPolicy::default();
    let plane = PlaneFrame {
        origin: [3., -2., 7.],
        u: [0.8, 0., -0.6],
        v: [0., 1., 0.],
    };
    for frame in [
        PlaneFrame::default(),
        PlaneFrame {
            origin: [17., -8., 13.],
            u: [0.8, 0.6, 0.],
            v: [-0.36, 0.48, 0.8],
        },
    ] {
        for (authored, height) in [(false, 8.), (true, 8.), (true, -8.)] {
            let owner = EntityId::new();
            let source = cylinder_source(
                owner,
                frame,
                [1., 2.],
                height,
                authored,
                Origin {
                    source: owner,
                    role: "circle".into(),
                },
                policy,
            );
            let mirrored = source
                .mirrored_named(plane, EntityId::new(), policy)
                .unwrap();
            let snapshot = mirrored.topology_snapshot(owner, 0).unwrap();
            let Some(AnalyticGeometry::CircularArc { frame: rim, .. }) = snapshot
                .elements
                .iter()
                .find(|e| e.key.kind == TopologyKind::Edge && e.key.ordinal == 0)
                .unwrap()
                .geometry
            else {
                panic!()
            };
            let local = PlaneFrame {
                origin: rim.origin_mm,
                u: reflected(plane, frame.u, false),
                v: reflected(plane, frame.v, false).map(|x| if height < 0. { x } else { -x }),
            };
            let directory = tempfile::tempdir().unwrap();
            let source_path = directory.path().join("source.step");
            mirrored.write_step(&source_path).unwrap();
            for operation in 0..3 {
                let feature = EntityId::new();
                let edge = |key| TreatmentEdge {
                    key,
                    origin: Origin {
                        source: feature,
                        role: "chamfer".into(),
                    },
                };
                let face = |key| TreatmentFace {
                    key,
                    origin: Origin {
                        source: feature,
                        role: "wall".into(),
                    },
                };
                let angle = 5_f64.to_radians();
                let apply = |solid: &Solid, edge_key, cap_key, wall_key| match operation {
                    0 => solid.treat_edges_named(
                        &[edge(edge_key)],
                        EdgeTreatment::Chamfer { distance_mm: 0.5 },
                        feature,
                        policy,
                    ),
                    1 => solid.draft_faces_named(
                        &[face(wall_key)],
                        local.normal(),
                        local,
                        angle,
                        feature,
                        policy,
                    ),
                    _ => solid.shell_named(&[face(cap_key)], 0.5, feature, policy),
                };
                let result = apply(
                    &mirrored,
                    TopologyKey {
                        kind: TopologyKind::Edge,
                        ordinal: 0,
                    },
                    TopologyKey {
                        kind: TopologyKind::Face,
                        ordinal: 0,
                    },
                    TopologyKey {
                        kind: TopologyKind::Face,
                        ordinal: 2,
                    },
                )
                .unwrap();
                let expected = match operation {
                    0 => PI * (25. * 8. - 0.5 * (5. * 0.5 - 0.25 / 3.)),
                    1 => {
                        let r = 5. - 8. * angle.tan();
                        PI * 8. / 3. * (25. + 5. * r + r * r)
                    }
                    _ => PI * (25. * 8. - 4.5_f64.powi(2) * 7.5),
                };
                near(result.volume_mm3().unwrap(), expected);
                result
                    .manufacturing_mesh_with_receipt(Default::default())
                    .unwrap();
                let output = result.topology_snapshot(feature, 1).unwrap();
                // The untouched cap survives all three operations with its
                // original authored attachment, independently of source kind.
                let old = snapshot
                    .elements
                    .iter()
                    .find(|e| e.key.kind == TopologyKind::Face && e.key.ordinal == 1)
                    .unwrap();
                let mut intent = GeometryIntent::capture(&snapshot, old.key).unwrap();
                intent.boundary = None;
                let Resolution::Resolved { key, .. } = resolve(&intent, &output, 1) else {
                    panic!("lost opposite cap")
                };
                let current = output.elements.iter().find(|e| e.key == key).unwrap();
                for anchor in &old.frames {
                    let new = current
                        .frames
                        .iter()
                        .find(|a| a.origin == anchor.origin)
                        .unwrap();
                    near_point(anchor.frame.origin_mm, new.frame.origin_mm);
                    near_point(anchor.frame.u, new.frame.u);
                    let normal = anchor.frame.normal();
                    let align = normal
                        .into_iter()
                        .zip(local.normal())
                        .map(|(a, b)| a * b)
                        .sum::<f64>();
                    // Shell intentionally orients retained cap attachments
                    // outward, as it does for an unmirrored authored cylinder.
                    let expected_v = anchor
                        .frame
                        .v
                        .map(|x| if operation == 2 && align < 0. { -x } else { x });
                    near_point(expected_v, new.frame.v);
                }
                let path = directory.path().join(format!("operation-{operation}.step"));
                result.write_step(&path).unwrap();
                let imported = Solid::import_rust_step(
                    &std::fs::read_to_string(&path).unwrap(),
                    feature,
                    policy,
                )
                .unwrap();
                near(imported.volume_mm3().unwrap(), expected);
                if OCCT_LINKED {
                    let native = Solid::read_step(&source_path, policy).unwrap();
                    let native_snapshot = native.topology_snapshot(owner, 0).unwrap();
                    let edge_key = native_snapshot.elements.iter().find(|e| matches!(e.geometry,
                        Some(AnalyticGeometry::CircularArc { frame, .. }) if frame.origin_mm.into_iter().zip(local.origin).all(|(a,b)| (a-b).abs()<1e-6))).unwrap().key;
                    let cap_key = native
                        .faces()
                        .unwrap()
                        .into_iter()
                        .find(|f| {
                            f.plane.is_some_and(|p| {
                                p.0.into_iter()
                                    .zip(local.origin)
                                    .all(|(a, b)| (a - b).abs() < 1e-6)
                            })
                        })
                        .unwrap()
                        .index;
                    let wall_key = native
                        .faces()
                        .unwrap()
                        .into_iter()
                        .find(|f| f.plane.is_none())
                        .unwrap()
                        .index;
                    let native_result = apply(
                        &native,
                        edge_key,
                        TopologyKey {
                            kind: TopologyKind::Face,
                            ordinal: cap_key,
                        },
                        TopologyKey {
                            kind: TopologyKind::Face,
                            ordinal: wall_key,
                        },
                    )
                    .unwrap();
                    near(native_result.volume_mm3().unwrap(), expected);
                    near_point(
                        native_result.bounds().unwrap().min,
                        result.bounds().unwrap().min,
                    );
                    near_point(
                        native_result.bounds().unwrap().max,
                        result.bounds().unwrap().max,
                    );
                    near(
                        Solid::read_step(&path, policy)
                            .unwrap()
                            .volume_mm3()
                            .unwrap(),
                        expected,
                    );
                }
            }
        }
    }
}
