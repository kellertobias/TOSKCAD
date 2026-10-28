use multicad_core::{document::EntityId, references::*};
use multicad_kernel::*;
use std::collections::BTreeSet;

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
