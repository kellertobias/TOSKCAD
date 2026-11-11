use multicad_core::{
    document::EntityId,
    references::{
        Attachment, GeometryIntent, Origin, Resolution, TopologyKind, TopologySnapshot, resolve,
    },
};
use multicad_kernel::{
    BlindThreadCut, GeometryErrorKind, GeometryPolicy, HelicalThread, MeshOptions, PlaneFrame,
    RigidTransform, Solid,
    rust_kernel::{Circle, Rational, cylinder::AxialCylinder},
};
fn n(v: i64) -> Rational {
    Rational::new(v, 1).unwrap()
}
fn spec() -> HelicalThread {
    HelicalThread {
        major_radius_mm: 3.,
        pitch_mm: 1.,
        length_mm: 3.7,
        depth_mm: 5. * 3_f64.sqrt() / 16.,
        crest_width_mm: 0.125,
        lower_flank_deg: 30.,
        upper_flank_deg: 30.,
        radial_taper: 0.,
        starts: 1,
        left_hand: false,
    }
}
fn placed(frame: PlaneFrame, p: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| {
        frame.origin[i] + frame.u[i] * p[0] + frame.v[i] * p[1] + frame.normal()[i] * p[2]
    })
}
fn pose(frame: PlaneFrame) -> RigidTransform {
    let normal = frame.normal();
    RigidTransform {
        rotation: std::array::from_fn(|i| [frame.u[i], frame.v[i], normal[i]]),
        translation_mm: frame.origin,
    }
}
fn frames() -> [PlaneFrame; 4] {
    [
        PlaneFrame::default(),
        PlaneFrame {
            origin: [7., -3., 2.],
            u: [0.8, 0.6, 0.],
            v: [-0.36, 0.48, 0.8],
        },
        PlaneFrame {
            u: [1. + 4e-11, 0., 0.],
            ..Default::default()
        },
        PlaneFrame {
            origin: [7., -3., 2.],
            u: [0.8 * (1. + 4e-11), 0.6 * (1. + 4e-11), 0.],
            v: [-0.36, 0.48, 0.8],
        },
    ]
}
fn close(a: [f64; 3], b: [f64; 3]) {
    for (a, b) in a.into_iter().zip(b) {
        assert!(
            (a - b).abs() <= 64. * f64::EPSILON * a.abs().max(b.abs()).max(1.),
            "{a} vs {b}"
        );
    }
}
fn verify_source_intents(before: &TopologySnapshot, after: &TopologySnapshot) {
    for element in &before.elements {
        for old in &element.frames {
            let new = after
                .elements
                .iter()
                .flat_map(|e| &e.frames)
                .find(|a| a.origin == old.origin)
                .expect("retained authored anchor");
            close(old.frame.origin_mm, new.frame.origin_mm);
            close(old.frame.u, new.frame.u);
            close(old.frame.v, new.frame.v);
        }
        if element.key.kind != TopologyKind::Face || element.frames.is_empty() {
            continue;
        }
        let mut intent = GeometryIntent::capture(before, element.key).unwrap();
        intent.attachment = Some(Attachment {
            anchor: element.frames[0].origin.clone(),
            offset_mm: [2., 3., 0.],
            angle_rad: 0.2,
        });
        let saved: GeometryIntent =
            serde_json::from_str(&serde_json::to_string(&intent).unwrap()).unwrap();
        let resolution = |snapshot: &TopologySnapshot| {
            let Resolution::Resolved {
                frame: Some(frame), ..
            } = resolve(&saved, snapshot, snapshot.revision)
            else {
                panic!("lost saved stock attachment: {saved:?}")
            };
            frame
        };
        let expected = resolution(before);
        let actual = resolution(after);
        close(expected.origin_mm, actual.origin_mm);
        close(expected.u, actual.u);
        close(expected.v, actual.v);
        // A real dependent primitive uses the saved sketch/attachment chart,
        // including the authored V direction on the lower stock cap.
        let dependent = |frame: multicad_core::references::ReferenceFrame| {
            Solid::rust_box(
                EntityId::new().uuid_bits(),
                [n(1); 3],
                PlaneFrame {
                    origin: frame.origin_mm,
                    u: frame.u,
                    v: frame.v,
                },
                GeometryPolicy::default(),
            )
            .unwrap()
            .bounds()
            .unwrap()
        };
        let expected = dependent(expected);
        let actual = dependent(actual);
        close(expected.min, actual.min);
        close(expected.max, actual.max);
    }
}
fn mesh(cut: &Solid) {
    let receipt = cut
        .manufacturing_mesh_with_receipt(MeshOptions {
            linear_deflection_mm: 0.1,
            angular_deflection_rad: 0.5,
            maximum_triangles: 100_000,
        })
        .unwrap();
    assert_eq!(receipt.edge_pairs as usize * 2, receipt.mesh.indices.len());
}
#[cfg(feature = "occt")]
fn native_compare(cut: &Solid, frame: PlaneFrame, entry: PlaneFrame, blind: bool) {
    let policy = GeometryPolicy::default();
    let stock = if blind {
        Solid::box_solid([30., 20., 20.], policy)
            .unwrap()
            .transformed(pose(frame))
            .unwrap()
    } else {
        Solid::cylinder(frame, 12., 3.7, policy).unwrap()
    };
    let tool = Solid::helical_thread_named(spec(), EntityId::new(), policy)
        .unwrap()
        .transformed(pose(entry))
        .unwrap();
    assert!(!stock.uses_rust_geometry());
    assert!(!tool.uses_rust_geometry());
    let native = stock
        .boolean(&tool, multicad_kernel::BooleanOperation::Subtract, policy)
        .unwrap()
        .unwrap();
    assert!((native.volume_mm3().unwrap() - cut.volume_mm3().unwrap()).abs() < 1e-5);
    let a = native.bounds().unwrap();
    let b = cut.bounds().unwrap();
    for (a, b) in a
        .min
        .into_iter()
        .chain(a.max)
        .zip(b.min.into_iter().chain(b.max))
    {
        assert!((a - b).abs() < 5e-6);
    }
}
#[test]
fn receiving_threads_keep_saved_cap_charts_for_both_directions_and_clocking() {
    let policy = GeometryPolicy::default();
    let directory = tempfile::tempdir().unwrap();
    for (frame_index, frame) in frames().into_iter().enumerate() {
        for reversed in [false, true] {
            for angle in [0_f64, 0.37] {
                let stock_id = EntityId::new();
                let stock = Solid::from_axial_cylinder_sweep(
                    AxialCylinder::new(
                        stock_id.uuid_bits(),
                        Circle::new([n(0); 2], n(12)).unwrap(),
                        n(0),
                        Rational::new(37, 10).unwrap(),
                    )
                    .unwrap(),
                    frame,
                    [0.; 2],
                    false,
                    Origin {
                        source: stock_id,
                        role: "circle".into(),
                    },
                    policy,
                )
                .unwrap();
                let (s, c) = angle.sin_cos();
                let sign = if reversed { -1. } else { 1. };
                let entry = PlaneFrame {
                    origin: placed(frame, [0., 0., if reversed { 3.7 } else { 0. }]),
                    u: std::array::from_fn(|i| c * frame.u[i] + s * frame.v[i]),
                    v: std::array::from_fn(|i| sign * (-s * frame.u[i] + c * frame.v[i])),
                };
                let before = stock.topology_snapshot(stock_id, 0).unwrap();
                let cut = stock
                    .rust_cut_internal_thread(spec(), 1., EntityId::new(), entry, policy)
                    .unwrap();
                verify_source_intents(&before, &cut.topology_snapshot(stock_id, 1).unwrap());
                mesh(&cut);
                assert_eq!(stock.topology_snapshot(stock_id, 0).unwrap(), before);
                if frame_index < 2 {
                    let path = directory.path().join("receiver.step");
                    cut.write_step(&path).unwrap();
                    let text = std::fs::read_to_string(&path).unwrap();
                    assert!(
                        (cut.verify_helical_step(&text).unwrap().unwrap()
                            - cut.volume_mm3().unwrap())
                        .abs()
                            < 1e-6
                    );
                    let imported = Solid::import_rust_step(&text, EntityId::new(), policy).unwrap();
                    assert!(
                        (imported.volume_mm3().unwrap() - cut.volume_mm3().unwrap()).abs() < 1e-6
                    );
                    mesh(&imported);
                    #[cfg(feature = "occt")]
                    {
                        native_compare(&cut, frame, entry, false);
                        let read = Solid::read_step(&path, policy).unwrap();
                        assert!(
                            (read.volume_mm3().unwrap() - cut.volume_mm3().unwrap()).abs() < 1e-5
                        );
                    }
                }
            }
        }
    }
}
#[test]
fn blind_threads_preserve_original_cap_uv_and_full_receiving_stock() {
    let policy = GeometryPolicy::default();
    for (frame_index, frame) in frames().into_iter().enumerate() {
        for top in [false, true] {
            let stock_id = EntityId::new();
            let stock = Solid::rust_box(stock_id.uuid_bits(), [n(30), n(20), n(20)], frame, policy)
                .unwrap();
            let before = stock.topology_snapshot(stock_id, 0).unwrap();
            let entry = PlaneFrame {
                origin: placed(frame, [10., 10., if top { 20. } else { 0. }]),
                u: frame.u,
                v: frame.v.map(|v| if top { -v } else { v }),
            };
            let cut = stock
                .rust_cut_blind_internal_thread(
                    BlindThreadCut {
                        profile: spec(),
                        entry,
                        receiving_radius_mm: 3.,
                        minimum_wall_mm: 1.,
                        minimum_floor_mm: 1.,
                    },
                    EntityId::new(),
                    policy,
                )
                .unwrap();
            verify_source_intents(&before, &cut.topology_snapshot(stock_id, 1).unwrap());
            close(stock.bounds().unwrap().min, cut.bounds().unwrap().min);
            close(stock.bounds().unwrap().max, cut.bounds().unwrap().max);
            mesh(&cut);
            let displaced = PlaneFrame {
                origin: placed(frame, [10., 10., 20.1]),
                ..entry
            };
            assert!(
                stock
                    .rust_cut_blind_internal_thread(
                        BlindThreadCut {
                            profile: spec(),
                            entry: displaced,
                            receiving_radius_mm: 3.,
                            minimum_wall_mm: 1.,
                            minimum_floor_mm: 1.
                        },
                        EntityId::new(),
                        policy
                    )
                    .is_err()
            );
            assert_eq!(stock.topology_snapshot(stock_id, 0).unwrap(), before);
            #[cfg(feature = "occt")]
            if frame_index < 2 {
                native_compare(&cut, frame, entry, true);
            }
            #[cfg(not(feature = "occt"))]
            let _ = frame_index;
        }
    }
}
#[test]
fn remap_failures_do_not_mutate_receiving_stock() {
    let policy = GeometryPolicy::default();
    let stock_id = EntityId::new();
    let frame = frames()[2];
    let stock = Solid::from_axial_cylinder_sweep(
        AxialCylinder::new(
            stock_id.uuid_bits(),
            Circle::new([n(0); 2], n(12)).unwrap(),
            n(0),
            Rational::new(37, 10).unwrap(),
        )
        .unwrap(),
        frame,
        [0.; 2],
        false,
        Origin {
            source: stock_id,
            role: "circle".into(),
        },
        policy,
    )
    .unwrap();
    let before = stock.topology_snapshot(stock_id, 0).unwrap();
    let misplaced = PlaneFrame {
        origin: [0.1, 0., 0.],
        ..frame
    };
    assert_eq!(
        stock
            .rust_cut_internal_thread(spec(), 1., EntityId::new(), misplaced, policy)
            .err()
            .unwrap()
            .kind,
        GeometryErrorKind::Unsupported
    );
    let cut = stock
        .rust_cut_internal_thread(spec(), 1., EntityId::new(), frame, policy)
        .unwrap();
    verify_source_intents(&before, &cut.topology_snapshot(stock_id, 1).unwrap());
    assert_eq!(stock.topology_snapshot(stock_id, 0).unwrap(), before);
    let moved = cut
        .transformed(pose(PlaneFrame {
            origin: [2., 3., 4.],
            ..Default::default()
        }))
        .unwrap();
    assert_eq!(moved.volume_mm3().unwrap(), cut.volume_mm3().unwrap());
}

#[test]
fn far_blind_stock_requires_finite_full_extent_reconstruction() {
    let policy = GeometryPolicy::default();
    let mut accepted = 0;
    for angle in [0.13_f64, 0.37, 0.71, 1.03, 1.27] {
        let (s, c) = angle.sin_cos();
        let frame = PlaneFrame {
            u: [c, s, 0.],
            v: [-s, c, 0.],
            ..Default::default()
        };
        for clock in [0_f64, 0.37, 0.71] {
            let stock_id = EntityId::new();
            let stock = Solid::rust_box(
                stock_id.uuid_bits(),
                [n(10_000_000), n(20), n(20)],
                frame,
                policy,
            )
            .unwrap();
            let before = stock.topology_snapshot(stock_id, 0).unwrap();
            let (s, c) = clock.sin_cos();
            let entry = PlaneFrame {
                origin: placed(frame, [9_000_000., 10., 20.]),
                u: std::array::from_fn(|i| c * frame.u[i] + s * frame.v[i]),
                v: std::array::from_fn(|i| s * frame.u[i] - c * frame.v[i]),
            };
            match stock.rust_cut_blind_internal_thread(
                BlindThreadCut {
                    profile: spec(),
                    entry,
                    receiving_radius_mm: 3.,
                    minimum_wall_mm: 1.,
                    minimum_floor_mm: 1.,
                },
                EntityId::new(),
                policy,
            ) {
                Ok(cut) => {
                    accepted += 1;
                    let after = cut.topology_snapshot(stock_id, 1).unwrap();
                    for old in before.elements.iter().flat_map(|e| &e.frames) {
                        let new = after
                            .elements
                            .iter()
                            .flat_map(|e| &e.frames)
                            .find(|a| a.origin == old.origin)
                            .unwrap();
                        for (a, b) in [
                            (old.frame.origin_mm, new.frame.origin_mm),
                            (old.frame.u, new.frame.u),
                            (old.frame.v, new.frame.v),
                        ] {
                            let error = a
                                .into_iter()
                                .zip(b)
                                .map(|(a, b)| (a - b).abs())
                                .sum::<f64>();
                            assert!(
                                error <= policy.linear_tolerance_mm / 16.,
                                "{angle} {clock}: {error}"
                            );
                        }
                    }
                }
                Err(error) => {
                    eprintln!("precision refusal angle={angle}, clock={clock}: {error}");
                    assert_eq!(error.kind, GeometryErrorKind::Unsupported);
                }
            }
            assert_eq!(stock.topology_snapshot(stock_id, 0).unwrap(), before);
        }
    }
    assert!(accepted > 0);
}

#[test]
fn side_tap_keeps_the_independent_third_axis_of_long_receiving_stock() {
    let policy = GeometryPolicy::default();
    let scale = 1. + 4e-11;
    let frame = PlaneFrame {
        u: [scale, 0., 0.],
        ..Default::default()
    };
    let stock_id = EntityId::new();
    let stock =
        Solid::rust_box(stock_id.uuid_bits(), [n(30), n(20), n(500)], frame, policy).unwrap();
    let before = stock.topology_snapshot(stock_id, 0).unwrap();
    let entry = PlaneFrame {
        origin: placed(frame, [10., 0., 250.]),
        u: frame.u,
        v: frame.normal().map(|v| -v),
    };
    let cut = stock
        .rust_cut_blind_internal_thread(
            BlindThreadCut {
                profile: spec(),
                entry,
                receiving_radius_mm: 3.,
                minimum_wall_mm: 1.,
                minimum_floor_mm: 1.,
            },
            EntityId::new(),
            policy,
        )
        .unwrap();
    // UV-only relative storage recomputed the stock's third column as a
    // cross product and shrank this physical outer surface from500*s to500/s.
    assert!((500. * scale - 500. / scale).abs() > policy.linear_tolerance_mm / 16.);
    let receipt = cut
        .manufacturing_mesh_with_receipt(MeshOptions {
            linear_deflection_mm: 0.1,
            angular_deflection_rad: 0.5,
            maximum_triangles: 100_000,
        })
        .unwrap();
    let z = receipt
        .mesh
        .vertices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|p| p[2])
        .fold(f64::NEG_INFINITY, f64::max);
    assert!((z - stock.bounds().unwrap().max[2]).abs() < policy.linear_tolerance_mm / 16.);
    verify_source_intents(&before, &cut.topology_snapshot(stock_id, 1).unwrap());
    assert_eq!(stock.topology_snapshot(stock_id, 0).unwrap(), before);
    close(stock.bounds().unwrap().min, cut.bounds().unwrap().min);
    close(stock.bounds().unwrap().max, cut.bounds().unwrap().max);
}

#[test]
fn receiving_descriptor_center_and_bottom_are_world_translation_not_local_extent() {
    let policy = GeometryPolicy::default();
    let id = EntityId::new();
    let frame = PlaneFrame {
        origin: [1_000_000., -2_000_000., 3_000_000.],
        u: [1. + 4e-11, 0., 0.],
        v: [0., 1., 0.],
    };
    let cylinder = AxialCylinder::new(
        id.uuid_bits(),
        Circle::new([n(1_000_000), n(-2_000_000)], n(12)).unwrap(),
        n(3_000_000),
        Rational::new(37, 10).unwrap(),
    )
    .unwrap();
    let stock = Solid::from_axial_cylinder(cylinder, frame, policy).unwrap();
    let before = stock.topology_snapshot(id, 0).unwrap();
    let (s, c) = 0.37_f64.sin_cos();
    let entry = PlaneFrame {
        origin: frame.origin,
        u: std::array::from_fn(|i| c * frame.u[i] + s * frame.v[i]),
        v: std::array::from_fn(|i| -s * frame.u[i] + c * frame.v[i]),
    };
    let cut = stock
        .rust_cut_internal_thread(spec(), 1., EntityId::new(), entry, policy)
        .unwrap();
    verify_source_intents(&before, &cut.topology_snapshot(id, 1).unwrap());
    assert_eq!(stock.topology_snapshot(id, 0).unwrap(), before);
    assert!(
        (stock.volume_mm3().unwrap()
            - cut.volume_mm3().unwrap()
            - Solid::rust_helical_thread(
                spec(),
                None,
                EntityId::new(),
                PlaneFrame::default(),
                policy
            )
            .unwrap()
            .volume_mm3()
            .unwrap())
        .abs()
            < 1e-10
    );
    mesh(&cut);
}
