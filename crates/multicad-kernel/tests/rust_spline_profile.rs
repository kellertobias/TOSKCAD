use multicad_core::{
    document::EntityId,
    references::{Attachment, GeometryIntent, Origin, Resolution, TopologyKind, resolve},
};
use multicad_kernel::{
    BSpline2d, CurvedEdge2d, CurvedProfile, GeometryErrorKind, GeometryPolicy, MeshOptions,
    PlaneFrame, RigidTransform, Solid,
};
use std::cell::Cell;
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 5e-6, "{a} != {b}");
}
fn profile(frame: PlaneFrame, hole: bool) -> CurvedProfile {
    CurvedProfile {
        frame,
        outer: vec![
            CurvedEdge2d::BSpline(BSpline2d {
                degree: 2,
                poles: vec![[0., 0.], [5., 10.], [10., 0.]],
                knots: vec![0., 1.],
                multiplicities: vec![3, 3],
                weights: None,
            }),
            CurvedEdge2d::Line {
                a: [10., 0.],
                b: [0., 0.],
            },
        ],
        holes: if hole {
            vec![vec![CurvedEdge2d::Circle {
                center: [5., 2.],
                radius_mm: 0.25,
            }]]
        } else {
            vec![]
        },
    }
}
fn origins(sketch: EntityId, count: usize) -> Vec<Origin> {
    (0..count)
        .map(|index| Origin {
            source: sketch,
            role: format!("curve/{}", EntityId::from_uuid_bits(index as u128 + 1)),
        })
        .collect()
}
fn compare(a: &Solid, b: &Solid) {
    near(a.volume_mm3().unwrap(), b.volume_mm3().unwrap());
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
#[test]
fn authored_spline_sweeps_keep_source_origins_cap_charts_and_real_step_in_both_senses() {
    let policy = GeometryPolicy::default();
    let feature = EntityId::new();
    let sketch = EntityId::new();
    for frame in [
        PlaneFrame::default(),
        PlaneFrame {
            origin: [7., -2., 3.],
            u: [0.8, 0.6, 0.],
            v: [-0.36, 0.48, 0.8],
        },
    ] {
        for (height, hole) in [(-3., false), (3., false), (-3., true), (3., true)] {
            let source = profile(frame, hole);
            let retained = source.clone();
            let ids = origins(sketch, 2 + usize::from(hole));
            let solid =
                Solid::rust_extrude_spline_profile_named(&source, height, feature, &ids, policy)
                    .unwrap();
            assert!(solid.uses_rust_geometry());
            assert_eq!(source, retained);
            near(
                solid.volume_mm3().unwrap(),
                100. - if hole {
                    3. * std::f64::consts::PI * 0.25_f64.powi(2)
                } else {
                    0.
                },
            );
            let snapshot = solid.topology_snapshot(feature, 0).unwrap();
            snapshot.validate().unwrap();
            if hole {
                let rims: Vec<_> = snapshot
                    .elements
                    .iter()
                    .filter_map(|element| {
                        if let Some(multicad_core::references::AnalyticGeometry::CircularArc {
                            frame: rim,
                            radius_mm,
                            start_rad,
                            sweep_rad,
                        }) = element.geometry
                        {
                            Some((rim, radius_mm, start_rad, sweep_rad))
                        } else {
                            None
                        }
                    })
                    .collect();
                assert_eq!(rims.len(), 2);
                for (rim, radius, start, sweep) in rims {
                    near(radius, 0.25);
                    near(start, 0.);
                    near(sweep, std::f64::consts::TAU);
                    let center = std::array::from_fn::<_, 3, _>(|i| {
                        frame.origin[i] + 5. * frame.u[i] + 2. * frame.v[i]
                    });
                    let axial = (0..3)
                        .map(|i| (rim.origin_mm[i] - center[i]) * frame.normal()[i])
                        .sum::<f64>();
                    assert!(axial.abs() < 1e-7 || (axial - height).abs() < 1e-7);
                    for (i, center) in center.into_iter().enumerate() {
                        near(rim.u[i], frame.u[i]);
                        near(rim.v[i], frame.v[i]);
                        let quarter = rim.origin_mm[i] + radius * rim.v[i];
                        near(
                            quarter,
                            center + axial * frame.normal()[i] + radius * frame.v[i],
                        );
                    }
                }
            }
            for origin in &ids {
                assert!(
                    snapshot
                        .elements
                        .iter()
                        .any(|e| e.key.kind == TopologyKind::Face && e.origins.contains(origin))
                );
            }
            let mut intents = Vec::new();
            for (role, end) in [("cap.start", false), ("cap.end", true)] {
                let cap = snapshot
                    .elements
                    .iter()
                    .find(|e| {
                        e.key.kind == TopologyKind::Face
                            && e.origins.contains(&Origin {
                                source: feature,
                                role: role.into(),
                            })
                    })
                    .unwrap();
                let [anchor] = cap.frames.as_slice() else {
                    panic!("cap lacks unique authored chart");
                };
                for (a, b) in anchor
                    .frame
                    .u
                    .into_iter()
                    .chain(anchor.frame.v)
                    .zip(frame.u.into_iter().chain(frame.v))
                {
                    near(a, b);
                }
                for i in 0..3 {
                    near(
                        anchor.frame.origin_mm[i],
                        frame.origin[i] + if end { frame.normal()[i] * height } else { 0. },
                    );
                }
                let mut intent = GeometryIntent::capture(&snapshot, cap.key).unwrap();
                intent.attachment = Some(Attachment {
                    anchor: anchor.origin.clone(),
                    offset_mm: [0.25, -0.5, 0.75],
                    angle_rad: 0.3,
                });
                intents.push(intent);
            }
            solid
                .manufacturing_mesh_with_receipt(MeshOptions {
                    linear_deflection_mm: 0.002,
                    angular_deflection_rad: 0.12,
                    maximum_triangles: 50_000,
                })
                .unwrap();
            assert!(
                solid
                    .manufacturing_mesh_with_receipt(MeshOptions {
                        linear_deflection_mm: policy.linear_tolerance_mm / 256.,
                        angular_deflection_rad: 0.12,
                        maximum_triangles: 50_000,
                    })
                    .is_err()
            );
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("authored-spline.step");
            solid.write_step(&path).unwrap();
            let imported = Solid::import_rust_step(
                &std::fs::read_to_string(&path).unwrap(),
                EntityId::new(),
                policy,
            )
            .unwrap();
            compare(&solid, &imported);
            let transform = RigidTransform {
                rotation: [[0., -1., 0.], [1., 0., 0.], [0., 0., 1.]],
                translation_mm: [3., 4., 5.],
            };
            let moved = solid.transformed(transform).unwrap();
            let output = moved.topology_snapshot(feature, 0).unwrap();
            output.validate().unwrap();
            for intent in &intents {
                assert!(matches!(
                    resolve(intent, &output, 0),
                    Resolution::Resolved { frame: Some(_), .. }
                ));
            }
            #[cfg(feature = "occt")]
            {
                let native =
                    Solid::extrude_curved_profile_named(&source, height, feature, &ids, policy)
                        .unwrap();
                assert!(!native.uses_rust_geometry());
                compare(&solid, &native);
                let read = Solid::read_step(&path, policy).unwrap();
                assert!(!read.uses_rust_geometry());
                compare(&solid, &read);
            }
        }
    }
}
#[test]
fn cancelled_and_unproved_spline_extrusions_leave_source_and_existing_solid_intact() {
    let policy = GeometryPolicy::default();
    let source = profile(PlaneFrame::default(), false);
    let before = source.clone();
    let feature = EntityId::new();
    let ids = origins(EntityId::new(), 2);
    let stable =
        Solid::rust_extrude_spline_profile_named(&source, 3., feature, &ids, policy).unwrap();
    let topology = stable.topology_snapshot(feature, 0).unwrap();
    let count = Cell::new(0);
    let cancel = || {
        count.set(count.get() + 1);
        count.get() > 20
    };
    assert_eq!(
        Solid::rust_extrude_spline_profile_with_interrupt(
            &source,
            3.,
            feature,
            &ids,
            policy,
            Some(&cancel)
        )
        .err()
        .unwrap()
        .kind,
        GeometryErrorKind::Cancelled
    );
    assert_eq!(source, before);
    assert_eq!(stable.topology_snapshot(feature, 0).unwrap(), topology);
    let oversized = CurvedProfile {
        outer: vec![source.outer[0].clone(); 1025],
        ..source.clone()
    };
    let checks = Cell::new(0);
    let observe = || {
        checks.set(checks.get() + 1);
        false
    };
    assert_eq!(
        Solid::rust_extrude_spline_profile_with_interrupt(
            &oversized,
            3.,
            feature,
            &[],
            policy,
            Some(&observe)
        )
        .err()
        .unwrap()
        .kind,
        GeometryErrorKind::Limit
    );
    assert_eq!(
        checks.get(),
        0,
        "oversized source reached expensive curve predicates"
    );
    let mut rational = source.clone();
    let CurvedEdge2d::BSpline(spline) = &mut rational.outer[0] else {
        panic!()
    };
    spline.weights = Some(vec![1., 2., 1.]);
    assert_eq!(
        Solid::rust_extrude_spline_profile_named(&rational, 3., feature, &ids, policy)
            .err()
            .unwrap()
            .kind,
        GeometryErrorKind::Unsupported
    );
    assert_eq!(source, before);
    assert_eq!(stable.topology_snapshot(feature, 0).unwrap(), topology);
    let retry =
        Solid::rust_extrude_spline_profile_named(&source, 4., feature, &ids, policy).unwrap();
    near(retry.volume_mm3().unwrap(), 400. / 3.);
}

#[test]
fn spline_fragment_identity_cannot_follow_a_reversed_loop_to_another_finite_patch() {
    let policy = GeometryPolicy::default();
    let feature = EntityId::new();
    let source = profile(PlaneFrame::default(), false);
    let ids = origins(EntityId::new(), 2);
    let original =
        Solid::rust_extrude_spline_profile_named(&source, 3., feature, &ids, policy).unwrap();
    let first = original.topology_snapshot(feature, 0).unwrap();
    let intents: Vec<_> = first
        .elements
        .iter()
        .filter(|element| {
            element.key.kind == TopologyKind::Face
                && element
                    .origins
                    .iter()
                    .any(|origin| origin.role.starts_with("spline.trim.v1/"))
        })
        .map(|element| GeometryIntent::capture(&first, element.key).unwrap())
        .collect();
    assert!(intents.len() >= 4);
    let mut rotated = source.clone();
    rotated.outer.rotate_left(1);
    let mut rotated_ids = ids.clone();
    rotated_ids.rotate_left(1);
    let same =
        Solid::rust_extrude_spline_profile_named(&rotated, 3., feature, &rotated_ids, policy)
            .unwrap()
            .topology_snapshot(feature, 0)
            .unwrap();
    for intent in &intents {
        assert!(matches!(
            resolve(intent, &same, 0),
            Resolution::Resolved { .. }
        ));
    }
    let mut changed = source.clone();
    let CurvedEdge2d::BSpline(curve) = &mut changed.outer[0] else {
        panic!()
    };
    curve.poles[1][1] = -10.;
    let replaced = Solid::rust_extrude_spline_profile_named(&changed, 3., feature, &ids, policy)
        .unwrap()
        .topology_snapshot(feature, 0)
        .unwrap();
    for intent in &intents {
        assert!(matches!(resolve(intent, &replaced, 0), Resolution::Missing));
    }
    let negative = Solid::rust_extrude_spline_profile_named(&source, -3., feature, &ids, policy)
        .unwrap()
        .topology_snapshot(feature, 0)
        .unwrap();
    for intent in &intents {
        assert!(matches!(resolve(intent, &negative, 0), Resolution::Missing));
    }
}
