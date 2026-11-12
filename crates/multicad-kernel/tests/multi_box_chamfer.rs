use multicad_core::{document::EntityId, references::*};
use multicad_kernel::*;

fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}
fn selected(
    source: &Solid,
    owner: EntityId,
    feature: EntityId,
    along: usize,
) -> Vec<TreatmentEdge> {
    source
        .topology_snapshot(owner, 0)
        .unwrap()
        .elements
        .iter()
        .filter_map(|e| match e.geometry {
            Some(AnalyticGeometry::Line { a_mm, b_mm })
                if e.key.kind == TopologyKind::Edge
                    && (a_mm[along] - b_mm[along]).abs() > 1.
                    && (0..3).all(|i| i == along || a_mm[i] == b_mm[i]) =>
            {
                Some(TreatmentEdge {
                    key: e.key,
                    origin: Origin {
                        source: feature,
                        role: format!("edge/{}", e.key.ordinal),
                    },
                })
            }
            _ => None,
        })
        .collect()
}

#[test]
fn simultaneous_parallel_bevels_keep_distances_ancestry_mesh_and_exchange() {
    let policy = GeometryPolicy::default();
    for along in 0..3 {
        for count in 2..=4 {
            let owner = EntityId::new();
            let feature = EntityId::new();
            let stock = Solid::box_named([60., 40., 30.], owner, policy).unwrap();
            let before = stock.topology_snapshot(owner, 0).unwrap();
            let mut edges = selected(&stock, owner, feature, along);
            edges.truncate(count);
            let result = stock
                .treat_edges_named(
                    &edges,
                    EdgeTreatment::Chamfer { distance_mm: 3. },
                    feature,
                    policy,
                )
                .unwrap();
            let expected = 72000. - count as f64 * 4.5 * [60., 40., 30.][along];
            near(result.volume_mm3().unwrap(), expected);
            let topology = result.topology_snapshot(feature, 1).unwrap();
            assert_eq!(result.faces().unwrap().len(), 6 + count);
            for face in before
                .elements
                .iter()
                .filter(|e| e.key.kind == TopologyKind::Face)
            {
                let intent = GeometryIntent::capture(&before, face.key).unwrap();
                assert!(matches!(
                    resolve(&intent, &topology, 1),
                    Resolution::Resolved { .. }
                ));
            }
            let mesh = result.manufacturing_mesh(MeshOptions::default()).unwrap();
            let proof = rust_kernel::manufacturing::certify(&rust_kernel::display::DisplayMesh {
                vertices: mesh.vertices,
                indices: mesh.indices,
                face_indices: mesh.face_indices,
            })
            .unwrap();
            near(proof.signed_volume_mm3, expected);
            for edge in &edges {
                let faces: Vec<_> = topology
                    .elements
                    .iter()
                    .filter(|e| {
                        e.key.kind == TopologyKind::Face && e.origins.contains(&edge.origin)
                    })
                    .collect();
                assert_eq!(faces.len(), 1);
                let face = faces[0];
                let AnalyticGeometry::Plane { frame } = face.geometry.unwrap() else {
                    panic!()
                };
                let source_edge = before.elements.iter().find(|e| e.key == edge.key).unwrap();
                let AnalyticGeometry::Line { a_mm, .. } = source_edge.geometry.unwrap() else {
                    panic!()
                };
                let signed: f64 = (0..3)
                    .map(|i| (a_mm[i] - frame.origin_mm[i]) * frame.normal()[i])
                    .sum();
                near(signed.abs(), 3. / 2_f64.sqrt());
                let intent = GeometryIntent::capture(&topology, face.key).unwrap();
                let edited = stock
                    .treat_edges_named(
                        &edges,
                        EdgeTreatment::Chamfer { distance_mm: 4. },
                        feature,
                        policy,
                    )
                    .unwrap();
                assert!(matches!(
                    resolve(&intent, &edited.topology_snapshot(feature, 2).unwrap(), 2),
                    Resolution::Resolved { .. }
                ));
            }
            let mut reverse = edges.clone();
            reverse.reverse();
            let reordered = stock
                .treat_edges_named(
                    &reverse,
                    EdgeTreatment::Chamfer { distance_mm: 3. },
                    feature,
                    policy,
                )
                .unwrap();
            assert_eq!(
                result.topology_snapshot(feature, 1).unwrap(),
                reordered.topology_snapshot(feature, 1).unwrap()
            );
            let pose = RigidTransform {
                rotation: [[0.8, -0.36, 0.48], [0.6, 0.48, -0.64], [0., 0.8, 0.6]],
                translation_mm: [80., 60., 20.],
            };
            let moved_stock = stock.transformed(pose).unwrap();
            let moved = moved_stock
                .treat_edges_named(
                    &edges,
                    EdgeTreatment::Chamfer { distance_mm: 3. },
                    feature,
                    policy,
                )
                .unwrap();
            near(moved.volume_mm3().unwrap(), expected);
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("beveled.step");
            moved.write_step(&path).unwrap();
            let imported = Solid::read_step(&path, policy).unwrap();
            near(imported.volume_mm3().unwrap(), expected);
            assert_eq!(imported.faces().unwrap().len(), 6 + count);
            assert_eq!(stock.topology_snapshot(owner, 0).unwrap(), before);
        }
    }
}

#[test]
fn touching_intersecting_and_nonbox_bevels_refuse_without_mutation() {
    let p = GeometryPolicy::default();
    let owner = EntityId::new();
    let feature = EntityId::new();
    let stock = Solid::box_named([60., 40., 30.], owner, p).unwrap();
    let before = stock.topology_snapshot(owner, 0).unwrap();
    let edges = selected(&stock, owner, feature, 2);
    for distance in [20., 21., 40.] {
        assert!(
            stock
                .treat_edges_named(
                    &edges,
                    EdgeTreatment::Chamfer {
                        distance_mm: distance
                    },
                    feature,
                    p
                )
                .is_err()
        );
    }
    let mixed = [
        edges[0].clone(),
        selected(&stock, owner, feature, 0)[0].clone(),
    ];
    assert_eq!(
        stock
            .treat_edges_named(
                &mixed,
                EdgeTreatment::Chamfer { distance_mm: 2. },
                feature,
                p
            )
            .err()
            .unwrap()
            .kind,
        GeometryErrorKind::Unsupported
    );
    assert!(
        stock
            .treat_edges_named(
                &[edges[0].clone(), edges[0].clone()],
                EdgeTreatment::Chamfer { distance_mm: 2. },
                feature,
                p
            )
            .is_err()
    );
    let notch = Solid::box_named([30., 20., 30.], EntityId::new(), p)
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: [30., 20., 0.],
            ..Default::default()
        })
        .unwrap();
    let nonbox = stock
        .boolean(&notch, BooleanOperation::Subtract, p)
        .unwrap()
        .unwrap();
    let nonbox_before = nonbox.topology_snapshot(owner, 0).unwrap();
    let mut nonbox_edges = selected(&nonbox, owner, feature, 2);
    nonbox_edges.truncate(2);
    assert_eq!(
        nonbox
            .treat_edges_named(
                &nonbox_edges,
                EdgeTreatment::Chamfer { distance_mm: 2. },
                feature,
                p
            )
            .err()
            .unwrap()
            .kind,
        GeometryErrorKind::Unsupported
    );
    assert_eq!(nonbox.topology_snapshot(owner, 0).unwrap(), nonbox_before);
    assert_eq!(stock.topology_snapshot(owner, 0).unwrap(), before);
    let valid = stock
        .treat_edges_named(
            &edges,
            EdgeTreatment::Chamfer { distance_mm: 2. },
            feature,
            p,
        )
        .unwrap();
    near(valid.volume_mm3().unwrap(), 72000. - 4. * 2. * 30.);
}

#[test]
fn placed_authored_cap_anchors_remain_on_their_original_cap_planes() {
    let policy = GeometryPolicy::default();
    let owner = EntityId::new();
    let feature = EntityId::new();
    let stock = Solid::box_named([60., 40., 30.], owner, policy)
        .unwrap()
        .with_feature_cap_roles(owner, "custom.start", "custom.end")
        .unwrap();
    let edges = selected(&stock, owner, feature, 2)[..2].to_vec();
    let pose = RigidTransform {
        rotation: [[0.8, -0.36, 0.48], [0.6, 0.48, -0.64], [0., 0.8, 0.6]],
        translation_mm: [80., 60., 20.],
    };
    let stock = stock.transformed(pose).unwrap();
    let before = stock.topology_snapshot(owner, 0).unwrap();
    for (selection, treatment) in [
        (&edges[..], EdgeTreatment::Chamfer { distance_mm: 3. }),
        (&edges[..1], EdgeTreatment::Chamfer { distance_mm: 3. }),
        (&edges[..1], EdgeTreatment::Fillet { radius_mm: 3. }),
    ] {
        let result = stock
            .treat_edges_named(selection, treatment, feature, policy)
            .unwrap();
        let after = result.topology_snapshot(feature, 1).unwrap();
        for role in ["custom.start", "custom.end"] {
            let origin = Origin {
                source: owner,
                role: role.into(),
            };
            let old = before
                .elements
                .iter()
                .find(|e| e.key.kind == TopologyKind::Face && e.origins.contains(&origin))
                .unwrap();
            let face = after
                .elements
                .iter()
                .find(|e| e.key.kind == TopologyKind::Face && e.origins.contains(&origin))
                .unwrap();
            let anchor = old.frames.iter().find(|a| a.origin == origin).unwrap();
            let preserved = face.frames.iter().find(|a| a.origin == origin).unwrap();
            for (original, actual) in anchor
                .frame
                .origin_mm
                .into_iter()
                .chain(anchor.frame.u)
                .chain(anchor.frame.v)
                .zip(
                    preserved
                        .frame
                        .origin_mm
                        .into_iter()
                        .chain(preserved.frame.u)
                        .chain(preserved.frame.v),
                )
            {
                assert!((original - actual).abs() <= policy.linear_tolerance_mm);
            }
            let AnalyticGeometry::Plane { frame } = face.geometry.unwrap() else {
                panic!()
            };
            let normal = frame.normal();
            near(
                (0..3)
                    .map(|i| (preserved.frame.origin_mm[i] - frame.origin_mm[i]) * normal[i])
                    .sum(),
                0.,
            );
            near((0..3).map(|i| preserved.frame.u[i] * normal[i]).sum(), 0.);
            near((0..3).map(|i| preserved.frame.v[i] * normal[i]).sum(), 0.);
            assert!(matches!(
                resolve(
                    &GeometryIntent::capture(&before, old.key).unwrap(),
                    &after,
                    1
                ),
                Resolution::Resolved { .. }
            ));
        }
    }
    assert_eq!(stock.topology_snapshot(owner, 0).unwrap(), before);
}
