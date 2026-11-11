use multicad_core::{document::EntityId, references::*};
use multicad_kernel::*;
use std::collections::BTreeSet;

#[test]
fn rust_placed_box_edges_keep_exact_fillets_and_explicit_chamfer_sides() {
    use multicad_kernel::rust_kernel::{Rational, polygon::PolygonPrism};
    let owner = EntityId::new();
    let stock = Solid::rust_box(
        owner.uuid_bits(),
        [30, 20, 10].map(|n| Rational::new(n, 1).unwrap()),
        PlaneFrame::default(),
        policy(),
    )
    .unwrap();
    let points = [[0, 0], [30, 0], [30, 20], [0, 20]];
    let origins: Vec<_> = (0..4)
        .map(|i| Origin {
            source: owner,
            role: format!("profile/{i}"),
        })
        .collect();
    let polygon = Solid::from_polygon_prism(
        PolygonPrism::new(
            vec![
                points
                    .map(|p| p.map(|n| Rational::new(n, 1).unwrap()))
                    .to_vec(),
            ],
            Rational::new(10, 1).unwrap(),
        )
        .unwrap(),
        PlaneFrame::default(),
        owner,
        &origins,
        &[false; 4],
        policy(),
    )
    .unwrap();
    let profile = CurvedProfile {
        frame: PlaneFrame::default(),
        outer: points
            .into_iter()
            .zip(points.into_iter().cycle().skip(1))
            .map(|(a, b)| CurvedEdge2d::Line {
                a: a.map(|n| n as f64),
                b: b.map(|n| n as f64),
            })
            .collect(),
        holes: vec![],
    };
    let linear =
        Solid::rust_extrude_linear_named(&profile, [0., 0., 10.], 0., owner, &origins, policy())
            .unwrap();
    let wide = Solid::rust_box(
        EntityId::new().uuid_bits(),
        [60, 20, 10].map(|n| Rational::new(n, 1).unwrap()),
        PlaneFrame::default(),
        policy(),
    )
    .unwrap();
    let tool = stock
        .transformed(RigidTransform {
            translation_mm: [30., 0., 0.],
            ..Default::default()
        })
        .unwrap();
    let orthogonal = wide
        .boolean(&tool, BooleanOperation::Subtract, policy())
        .unwrap()
        .unwrap();
    let narrow = Solid::rust_box(
        EntityId::new().uuid_bits(),
        [15, 20, 10].map(|n| Rational::new(n, 1).unwrap()),
        PlaneFrame::default(),
        policy(),
    )
    .unwrap();
    let scaled = narrow
        .affine_transformed_named(
            AffineTransform {
                matrix: [[2., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
                translation_mm: [0.; 3],
            },
            EntityId::new(),
            policy(),
        )
        .unwrap();
    for stock in [stock, polygon, linear, orthogonal, scaled] {
        let topology = stock.topology_snapshot(owner, 0).unwrap();
        let point_near =
            |a: [f64; 3], b: [f64; 3]| a.into_iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-5);
        let dot = |a: [f64; 3], b: [f64; 3]| a.into_iter().zip(b).map(|(a, b)| a * b).sum::<f64>();
        let edges: Vec<_> = topology
            .elements
            .iter()
            .filter(|element| element.key.kind == TopologyKind::Edge)
            .collect();
        assert_eq!(edges.len(), 12);
        for frame in [
            PlaneFrame {
                origin: [12.5, -7., 2.],
                ..Default::default()
            },
            PlaneFrame {
                origin: [17., -3., 21.],
                u: [0.8, 0.6, 0.],
                v: [-0.36, 0.48, 0.8],
            },
        ] {
            let placement = RigidTransform {
                rotation: std::array::from_fn(|i| [frame.u[i], frame.v[i], frame.normal()[i]]),
                translation_mm: frame.origin,
            };
            let source = stock.transformed(placement).unwrap();
            let before = source.topology_snapshot(owner, 0).unwrap();
            for edge in &edges {
                let Some(AnalyticGeometry::Line { a_mm: a, b_mm: b }) = edge.geometry else {
                    panic!("box edge must be straight");
                };
                let length = a
                    .into_iter()
                    .zip(b)
                    .map(|(a, b)| (a - b).powi(2))
                    .sum::<f64>()
                    .sqrt();
                let face = topology
                    .elements
                    .iter()
                    .find(|element| {
                        if element.key.kind != TopologyKind::Face {
                            return false;
                        }
                        let Some(AnalyticGeometry::Plane { frame }) = element.geometry else {
                            return false;
                        };
                        [a, b].into_iter().all(|p| {
                            dot(
                                frame.normal(),
                                std::array::from_fn(|i| p[i] - frame.origin_mm[i]),
                            )
                            .abs()
                                < 1e-7
                        })
                    })
                    .unwrap();
                for fillet in [true, false] {
                    let feature = EntityId::new();
                    let capture = selected(edge.key, feature);
                    let result = if fillet {
                        source.treat_edges_named(
                            std::slice::from_ref(&capture),
                            EdgeTreatment::Fillet { radius_mm: 2. },
                            feature,
                            policy(),
                        )
                    } else {
                        source.chamfer_edges_sided_named(
                            &[SidedTreatmentEdge {
                                edge: capture.clone(),
                                reference_face: face.key,
                            }],
                            SidedChamfer::Distances {
                                first_mm: 1.,
                                second_mm: 2.,
                            },
                            feature,
                            policy(),
                        )
                    }
                    .unwrap();
                    let expected = 6000.
                        - if fillet {
                            (4. - std::f64::consts::PI) * length
                        } else {
                            length
                        };
                    near(result.volume_mm3().unwrap(), expected);
                    assert_eq!(result.solid_count().unwrap(), 1);
                    let output = result.topology_snapshot(feature, 1).unwrap();
                    assert_eq!(generated(&output, std::slice::from_ref(&capture)).len(), 1);
                    output.validate().unwrap();
                    result
                        .manufacturing_mesh_with_receipt(Default::default())
                        .unwrap();
                    assert_eq!(source.topology_snapshot(owner, 0).unwrap(), before);
                    let temporary = tempfile::tempdir().unwrap();
                    let path = temporary.path().join("placed-edge.step");
                    result.write_step(&path).unwrap();
                    let imported = Solid::import_rust_step(
                        &std::fs::read_to_string(&path).unwrap(),
                        feature,
                        policy(),
                    )
                    .unwrap();
                    near(imported.volume_mm3().unwrap(), expected);
                    if OCCT_LINKED {
                        let native_stock =
                            Solid::box_named([30., 20., 10.], owner, policy()).unwrap();
                        let native_topology = native_stock.topology_snapshot(owner, 0).unwrap();
                        let native_edge = native_topology.elements.iter().find(|element| {
                        matches!(element.geometry, Some(AnalyticGeometry::Line { a_mm, b_mm }) if
                            (point_near(a,a_mm) && point_near(b,b_mm)) || (point_near(a,b_mm) && point_near(b,a_mm)))
                    }).unwrap();
                        let native_capture = TreatmentEdge {
                            key: native_edge.key,
                            ..capture.clone()
                        };
                        let native_result = if fillet {
                        native_stock.treat_edges_named(&[native_capture], EdgeTreatment::Fillet { radius_mm: 2. }, feature, policy())
                    } else {
                        let Some(AnalyticGeometry::Plane { frame: reference }) = face.geometry else { unreachable!() };
                        let native_face = native_topology.elements.iter().find(|element| {
                            matches!(element.geometry, Some(AnalyticGeometry::Plane { frame }) if
                                point_near(frame.normal(), reference.normal()) &&
                                (dot(frame.normal(), frame.origin_mm) - dot(reference.normal(), reference.origin_mm)).abs() < 1e-7)
                        }).unwrap();
                        native_stock.chamfer_edges_sided_named(&[SidedTreatmentEdge { edge: native_capture, reference_face: native_face.key }], SidedChamfer::Distances { first_mm: 1., second_mm: 2. }, feature, policy())
                    }.unwrap().transformed(placement).unwrap();
                        near(native_result.volume_mm3().unwrap(), expected);
                        let area = |solid: &Solid| {
                            solid
                                .faces()
                                .unwrap()
                                .iter()
                                .map(|face| face.area_mm2)
                                .sum::<f64>()
                        };
                        near(area(&result), area(&native_result));
                        let native_bounds = native_result.bounds().unwrap();
                        let bounds = result.bounds().unwrap();
                        assert!(point_near(bounds.min, native_bounds.min));
                        assert!(point_near(bounds.max, native_bounds.max));
                        let readback = Shape::read_step(&path).unwrap();
                        assert!(readback.valid().unwrap());
                        near(readback.volume().unwrap(), expected);
                    }
                }
            }
        }
    }
}

fn policy() -> GeometryPolicy {
    GeometryPolicy::default()
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-5, "{a} != {b}");
}
fn selected(key: TopologyKey, feature: EntityId) -> TreatmentEdge {
    TreatmentEdge {
        key,
        origin: Origin {
            source: feature,
            role: format!("edge/{}", EntityId::new()),
        },
    }
}
fn vertical(solid: &Solid, owner: EntityId, x: f64, y: f64) -> TopologyKey {
    solid
        .topology_snapshot(owner, 0)
        .unwrap()
        .elements
        .into_iter()
        .find(|e| {
            matches!(e.geometry, Some(AnalyticGeometry::Line {a_mm:a,b_mm:b})
            if a[0]==x && b[0]==x && a[1]==y && b[1]==y && (a[2]-b[2]).abs()>1.)
        })
        .unwrap()
        .key
}
fn generated<'a>(
    snapshot: &'a TopologySnapshot,
    edges: &[TreatmentEdge],
) -> Vec<&'a TopologyElement> {
    snapshot
        .elements
        .iter()
        .filter(|e| {
            e.key.kind == TopologyKind::Face
                && edges.iter().any(|edge| e.origins.contains(&edge.origin))
        })
        .collect()
}

#[test]
fn exact_multi_edge_box_fillet_and_chamfer_have_expected_volume_and_immutable_source() {
    let owner = EntityId::new();
    let feature = EntityId::new();
    let source = Solid::box_named([60., 40., 30.], owner, policy()).unwrap();
    let before = source.topology_snapshot(owner, 0).unwrap();
    let edges = [
        selected(vertical(&source, owner, 0., 0.), feature),
        selected(vertical(&source, owner, 60., 40.), feature),
    ];
    let original_volume = source.volume_mm3().unwrap();
    for (treatment, removed, planar) in [
        (
            EdgeTreatment::Fillet { radius_mm: 3. },
            2. * (1. - std::f64::consts::PI / 4.) * 9. * 30.,
            false,
        ),
        (
            EdgeTreatment::Chamfer { distance_mm: 3. },
            2. * 0.5 * 9. * 30.,
            true,
        ),
    ] {
        let result = source
            .treat_edges_named(&edges, treatment, feature, policy())
            .unwrap();
        assert_eq!(result.solid_count().unwrap(), 1);
        near(result.volume_mm3().unwrap(), original_volume - removed);
        let snapshot = result.topology_snapshot(feature, 1).unwrap();
        let new_faces = generated(&snapshot, &edges);
        assert_eq!(new_faces.len(), 2);
        for face in new_faces {
            assert_eq!(
                matches!(face.geometry, Some(AnalyticGeometry::Plane { .. })),
                planar
            );
            assert_eq!(!face.frames.is_empty(), planar);
            GeometryIntent::capture(&snapshot, face.key).unwrap();
        }
        assert_eq!(source.topology_snapshot(owner, 0).unwrap(), before);
        near(source.volume_mm3().unwrap(), original_volume);
    }
}

#[test]
fn all_box_edges_include_exact_vertex_generated_corner_origins_without_fake_planes() {
    let owner = EntityId::new();
    let feature = EntityId::new();
    let source = Solid::box_named([60., 40., 30.], owner, policy()).unwrap();
    let before = source.topology_snapshot(owner, 0).unwrap();
    let edges: Vec<_> = before
        .elements
        .iter()
        .filter(|e| e.key.kind == TopologyKind::Edge)
        .map(|e| selected(e.key, feature))
        .collect();
    let result = source
        .treat_edges_named(
            &edges,
            EdgeTreatment::Fillet { radius_mm: 2. },
            feature,
            policy(),
        )
        .unwrap();
    let snapshot = result.topology_snapshot(feature, 1).unwrap();
    let faces = generated(&snapshot, &edges);
    assert_eq!(
        faces.len(),
        20,
        "12 edge cylinders and 8 vertex corner blends"
    );
    assert_eq!(
        faces
            .iter()
            .filter(|e| edges
                .iter()
                .filter(|edge| e.origins.contains(&edge.origin))
                .count()
                == 3)
            .count(),
        8
    );
    for face in faces {
        assert!(face.geometry.is_none());
        assert!(face.frames.is_empty());
        GeometryIntent::capture(&snapshot, face.key).unwrap();
    }
    assert_eq!(source.topology_snapshot(owner, 0).unwrap(), before);
}

#[test]
fn chamfer_face_reference_and_anchor_follow_upstream_dimensions_and_round_trip_step() {
    let owner = EntityId::new();
    let feature = EntityId::new();
    let source = Solid::box_named([60., 40., 30.], owner, policy()).unwrap();
    let before = source.topology_snapshot(owner, 0).unwrap();
    let key = vertical(&source, owner, 60., 0.);
    let source_intent = GeometryIntent::capture(&before, key).unwrap();
    let mut edge = selected(key, feature);
    let initial = source
        .treat_edges_named(
            std::slice::from_ref(&edge),
            EdgeTreatment::Chamfer { distance_mm: 3. },
            feature,
            policy(),
        )
        .unwrap();
    let initial_topology = initial.topology_snapshot(feature, 0).unwrap();
    let face = generated(&initial_topology, std::slice::from_ref(&edge))[0];
    let mut intent = GeometryIntent::capture(&initial_topology, face.key).unwrap();
    intent.attachment = Some(Attachment {
        anchor: edge.origin.clone(),
        offset_mm: [0.; 3],
        angle_rad: 0.,
    });
    let edited = Solid::box_named([75., 48., 35.], owner, policy()).unwrap();
    let Resolution::Resolved { key, .. } = resolve(
        &source_intent,
        &edited.topology_snapshot(owner, 1).unwrap(),
        1,
    ) else {
        panic!("source edge lost exact ancestry")
    };
    edge.key = key;
    let result = edited
        .treat_edges_named(
            &[edge],
            EdgeTreatment::Chamfer { distance_mm: 4. },
            feature,
            policy(),
        )
        .unwrap();
    let snapshot = result.topology_snapshot(feature, 1).unwrap();
    let Resolution::Resolved {
        frame: Some(frame),
        geometry: Some(AnalyticGeometry::Plane { frame: analytic }),
        ..
    } = resolve(&intent, &snapshot, 1)
    else {
        panic!("chamfer face lost its exact anchor")
    };
    assert_eq!(frame, analytic);
    near(frame.normal().iter().map(|v| v * v).sum::<f64>(), 1.);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("chamfer.step");
    result.write_step(&path).unwrap();
    let loaded = Solid::read_step(&path, policy()).unwrap();
    near(result.volume_mm3().unwrap(), loaded.volume_mm3().unwrap());
    for (a, b) in result
        .bounds()
        .unwrap()
        .min
        .into_iter()
        .chain(result.bounds().unwrap().max)
        .zip(
            loaded
                .bounds()
                .unwrap()
                .min
                .into_iter()
                .chain(loaded.bounds().unwrap().max),
        )
    {
        near(a, b);
    }
    assert_eq!(source.topology_snapshot(owner, 0).unwrap(), before);
}

#[test]
fn named_cylinder_rims_treat_exactly_but_seam_and_impossible_radius_fail_without_mutation() {
    let owner = EntityId::new();
    let feature = EntityId::new();
    let source = Solid::extrude_profile_named(
        &Profile {
            frame: PlaneFrame::default(),
            outer: vec![ProfileEdge::Circle {
                center: [0.; 2],
                radius_mm: 10.,
            }],
            holes: vec![],
        },
        30.,
        owner,
        &[Origin {
            source: owner,
            role: "circle".into(),
        }],
        policy(),
    )
    .unwrap();
    let before = source.topology_snapshot(owner, 0).unwrap();
    let rims: Vec<_> = before
        .elements
        .iter()
        .filter(|e| matches!(e.geometry, Some(AnalyticGeometry::CircularArc { .. })))
        .map(|e| selected(e.key, feature))
        .collect();
    assert_eq!(rims.len(), 2);
    for treatment in [
        EdgeTreatment::Fillet { radius_mm: 2. },
        EdgeTreatment::Chamfer { distance_mm: 2. },
    ] {
        let result = source
            .treat_edges_named(&rims, treatment, feature, policy())
            .unwrap();
        assert_eq!(result.solid_count().unwrap(), 1);
        assert!(result.volume_mm3().unwrap() < source.volume_mm3().unwrap());
        let snapshot = result.topology_snapshot(feature, 0).unwrap();
        for face in generated(&snapshot, &rims) {
            assert!(face.frames.is_empty());
            assert!(!matches!(
                face.geometry,
                Some(AnalyticGeometry::Plane { .. })
            ));
        }
    }
    let seam = before
        .elements
        .iter()
        .find(|e| {
            e.key.kind == TopologyKind::Edge
                && matches!(e.geometry, Some(AnalyticGeometry::Line { .. }))
        })
        .unwrap();
    let error = source
        .treat_edges_named(
            &[selected(seam.key, feature)],
            EdgeTreatment::Fillet { radius_mm: 2. },
            feature,
            policy(),
        )
        .err()
        .unwrap();
    assert!(error.message.contains("seams"));
    assert!(
        source
            .treat_edges_named(
                &rims,
                EdgeTreatment::Fillet { radius_mm: 1000. },
                feature,
                policy()
            )
            .is_err()
    );
    assert_eq!(source.topology_snapshot(owner, 0).unwrap(), before);
}

#[test]
fn actual_degenerate_cone_apex_edge_is_rejected_explicitly() {
    let source = Solid::revolve_curved_profile(
        &CurvedProfile {
            frame: PlaneFrame {
                origin: [0.; 3],
                u: [1., 0., 0.],
                v: [0., 0., 1.],
            },
            outer: vec![
                CurvedEdge2d::Line {
                    a: [0., 0.],
                    b: [10., 0.],
                },
                CurvedEdge2d::Line {
                    a: [10., 0.],
                    b: [0., 20.],
                },
                CurvedEdge2d::Line {
                    a: [0., 20.],
                    b: [0., 0.],
                },
            ],
            holes: vec![],
        },
        RevolutionAxis {
            origin_mm: [0.; 3],
            direction: [0., 0., 1.],
        },
        std::f64::consts::TAU,
        policy(),
    )
    .unwrap();
    let feature = EntityId::new();
    let snapshot = source.topology_snapshot(feature, 0).unwrap();
    let degenerate = snapshot
        .elements
        .iter()
        .find(|e| e.key.kind == TopologyKind::Edge && e.geometry.is_none())
        .unwrap();
    let error = source
        .treat_edges_named(
            &[selected(degenerate.key, feature)],
            EdgeTreatment::Fillet { radius_mm: 1. },
            feature,
            policy(),
        )
        .err()
        .unwrap();
    assert!(error.message.contains("Degenerate"), "{error}");
    assert_eq!(source.topology_snapshot(feature, 0).unwrap(), snapshot);
}

#[test]
fn invalid_sizes_kinds_keys_duplicates_origins_and_disconnected_inputs_are_rejected() {
    let owner = EntityId::new();
    let feature = EntityId::new();
    let source = Solid::box_named([60., 40., 30.], owner, policy()).unwrap();
    let edge = selected(vertical(&source, owner, 0., 0.), feature);
    for radius_mm in [0., -1., f64::NAN, f64::INFINITY, 1e8] {
        assert!(
            source
                .treat_edges_named(
                    std::slice::from_ref(&edge),
                    EdgeTreatment::Fillet { radius_mm },
                    feature,
                    policy()
                )
                .is_err()
        );
    }
    let mut wrong_kind = edge.clone();
    wrong_kind.key.kind = TopologyKind::Face;
    let mut missing = edge.clone();
    missing.key.ordinal = u32::MAX;
    let mut wrong_owner = edge.clone();
    wrong_owner.origin.source = owner;
    let mut invalid_origin = edge.clone();
    invalid_origin.origin.role = " ".into();
    let mut duplicate_origin = selected(vertical(&source, owner, 60., 40.), feature);
    duplicate_origin.origin = edge.origin.clone();
    for edges in [
        vec![],
        vec![wrong_kind],
        vec![missing],
        vec![wrong_owner],
        vec![invalid_origin],
        vec![edge.clone(), edge.clone()],
        vec![edge.clone(), duplicate_origin],
    ] {
        assert_eq!(
            source
                .treat_edges_named(
                    &edges,
                    EdgeTreatment::Chamfer { distance_mm: 2. },
                    feature,
                    policy()
                )
                .err()
                .unwrap()
                .kind,
            GeometryErrorKind::Input
        );
    }
    let shifted = source
        .transformed(RigidTransform {
            translation_mm: [100., 0., 0.],
            ..Default::default()
        })
        .unwrap();
    let compound = source
        .boolean(&shifted, BooleanOperation::Union, policy())
        .unwrap()
        .unwrap();
    assert_eq!(compound.solid_count().unwrap(), 2);
    assert!(
        compound
            .treat_edges_named(
                &[edge],
                EdgeTreatment::Fillet { radius_mm: 2. },
                feature,
                policy()
            )
            .err()
            .unwrap()
            .message
            .contains("connected")
    );
    assert_eq!(
        source
            .topology_snapshot(owner, 0)
            .unwrap()
            .elements
            .iter()
            .flat_map(|e| &e.origins)
            .filter(|o| o.source == feature)
            .collect::<BTreeSet<_>>()
            .len(),
        0
    );
}
