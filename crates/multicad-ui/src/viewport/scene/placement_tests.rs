use super::*;
use multicad_core::model::{
    AssemblyEvaluationInfo, AssemblyLeafReceipt, OccurrencePath, RigidPose,
};
use multicad_core::project_file::ContentHash;
use multicad_kernel::{MeshOptions, Solid};

fn scene() -> (Scene, EntityId, EntityId) {
    let id = EntityId::new();
    let other = EntityId::new();
    let solid = Solid::box_named([2., 3., 4.], id, Default::default()).unwrap();
    let mesh = solid.mesh(MeshOptions::default()).unwrap();
    let geometry = Arc::new(
        Geometry::new(
            id,
            mesh.vertices.as_chunks::<3>().0.to_vec(),
            mesh.indices,
            mesh.face_indices,
            solid.topology_snapshot(id, 0).unwrap().elements,
        )
        .unwrap(),
    );
    let scene = Scene::new(
        SceneStamp {
            document: EntityId::new(),
            revision: 7,
            generation: 2,
        },
        vec![geometry],
        vec![
            Instance {
                id,
                geometry: id,
                transform: Transform::default(),
                color: [1; 4],
            },
            Instance {
                id: other,
                geometry: id,
                transform: Transform::from_rigid_pose(RigidPose {
                    translation_mm: [20., 0., 0.],
                    ..Default::default()
                })
                .unwrap(),
                color: [2; 4],
            },
        ],
    )
    .unwrap();
    (scene, id, other)
}
fn delta() -> RigidPose {
    RigidPose {
        rotation: [[0., -1., 0.], [1., 0., 0.], [0., 0., 1.]],
        translation_mm: [10., 5., 0.],
    }
}
#[test]
fn placement_moves_only_selected_mesh_and_cannot_publish_as_native_scene() {
    let (scene, id, other) = scene();
    let view = scene.presentation_placement(id, delta()).unwrap();
    assert_eq!(view.scope(), &SceneScope::Presentation);
    assert_eq!(
        view.instances[0].transform.point([2., 0., 0.]),
        [10., 7., 0.]
    );
    assert_eq!(
        view.instances
            .iter()
            .find(|item| item.id == other)
            .unwrap()
            .transform,
        scene.instances[1].transform
    );
    assert_eq!(scene.instances[0].transform, Transform::default());
    assert!(Arc::ptr_eq(&view.geometries[&id], &scene.geometries[&id]));
    assert!(Arc::ptr_eq(
        &view.reference_instances,
        &scene.reference_instances
    ));
    assert_eq!(view.stamp(), scene.stamp());
    assert!(
        view.model_evaluation.is_none()
            && view.assembly_evaluation.is_none()
            && view.native_references.is_none()
            && view.native_bounds.is_none()
    );
    assert!(view.presentation_placement(id, delta()).is_err());
    let mut feed = SceneStore::default();
    feed.begin(scene.stamp()).unwrap();
    assert!(feed.publish(view).is_err());
}
#[test]
fn nested_placement_expands_whole_root_and_preserves_unselected_sibling_pose() {
    let (mut scene, root, other) = scene();
    let member = EntityId::new();
    let hash = ContentHash::of(b"placement fixture");
    scene.model_evaluation = Some(Arc::new(multicad_core::model::ModelEvaluationInfo {
        screw_connection_checks: vec![],
        source_hash: hash,
        consumed_bodies: Default::default(),
        cut_targets: vec![],
    }));
    scene.assembly_evaluation = Some(Arc::new(AssemblyEvaluationInfo {
        source_hash: hash,
        leaves: vec![
            AssemblyLeafReceipt {
                path: OccurrencePath {
                    root,
                    members: vec![member],
                },
                component: EntityId::new(),
                producer: root,
                pose: RigidPose {
                    translation_mm: [3., 0., 0.],
                    ..Default::default()
                },
            },
            AssemblyLeafReceipt {
                path: OccurrencePath {
                    root,
                    members: vec![EntityId::new()],
                },
                component: EntityId::new(),
                producer: root,
                pose: RigidPose {
                    translation_mm: [6., 0., 0.],
                    ..Default::default()
                },
            },
        ],
    }));
    let view = scene.presentation_placement(member, delta()).unwrap();
    assert_eq!(view.instances.len(), 3);
    assert!(
        !view
            .instances
            .iter()
            .any(|item| item.id == root || item.id == member)
    );
    assert_eq!(view.instances[0].id, other);
    assert_eq!(view.instances[1].transform.point([0.; 3]), [10., 8., 0.]);
    assert_eq!(view.instances[2].transform.point([0.; 3]), [6., 0., 0.]);
    assert!(view.assembly_evaluation().is_none());
    let all = scene.presentation_placement(root, delta()).unwrap();
    assert_eq!(all.instances.len(), 2);
    assert_eq!(all.instances[0].id, root);
    assert_eq!(all.instances[0].transform.point([0.; 3]), [10., 5., 0.]);
    assert_eq!(all.instances[1].transform, scene.instances[1].transform);
    let receipt = Arc::make_mut(scene.assembly_evaluation.as_mut().unwrap());
    let prototype = receipt.leaves[0].clone();
    for _ in 0..127 {
        let mut leaf = prototype.clone();
        leaf.path.members.push(EntityId::new());
        receipt.leaves.push(leaf);
    }
    // Both direct and nested placement support a larger native assembly.
    assert!(scene.presentation_placement(root, delta()).is_ok());
    let expanded = scene.presentation_placement(member, delta()).unwrap();
    assert_eq!(expanded.instances.len(), 130);
    Arc::make_mut(scene.assembly_evaluation.as_mut().unwrap())
        .leaves
        .truncate(2);
    scene.instances = vec![scene.instances[1].clone()].into();
    assert!(scene.presentation_placement(member, delta()).is_err());
}
#[test]
fn member_selection_uses_only_its_native_faces_inside_the_visible_root() {
    let (mut scene, root, other) = scene();
    let owner = EntityId::new();
    let member = EntityId::new();
    let part = EntityId::new();
    let hash = ContentHash::of(b"member selection fixture");
    scene.model_evaluation = Some(Arc::new(multicad_core::model::ModelEvaluationInfo {
        source_hash: hash,
        screw_connection_checks: vec![],
        consumed_bodies: Default::default(),
        cut_targets: vec![],
    }));
    scene.assembly_evaluation = Some(Arc::new(AssemblyEvaluationInfo {
        source_hash: hash,
        leaves: vec![AssemblyLeafReceipt {
            path: OccurrencePath {
                root,
                members: vec![member],
            },
            component: part,
            producer: root,
            pose: RigidPose::default(),
        }],
    }));
    let geometry = Arc::get_mut(scene.geometries.get_mut(&root).unwrap()).unwrap();
    let face = geometry
        .topology
        .iter_mut()
        .find(|face| face.key.kind == TopologyKind::Face)
        .unwrap();
    face.origins.insert(multicad_core::references::Origin {
        source: owner,
        role: format!("assembly/member/{member}"),
    });
    let ordinal = face.key.ordinal;
    let selected = BTreeSet::from([member]);
    assert_eq!(
        scene.member_faces(&selected, &BTreeSet::new()),
        [(root, ordinal)]
    );
    assert_eq!(
        scene.member_faces(&BTreeSet::from([part]), &BTreeSet::new()),
        [(root, ordinal)]
    );
    assert!(
        scene
            .member_faces(&selected, &BTreeSet::from([root]))
            .is_empty()
    );
    let visible = scene.visible_subset(&BTreeSet::from([other])).unwrap();
    assert!(visible.member_faces(&selected, &BTreeSet::new()).is_empty());
}
#[test]
fn placement_rejects_unknown_and_nonrigid_or_nonfinite_delta() {
    let (scene, id, _) = scene();
    assert!(
        scene
            .presentation_placement(EntityId::new(), delta())
            .is_err()
    );
    let mut invalid = delta();
    invalid.translation_mm[0] = f64::NAN;
    assert!(scene.presentation_placement(id, invalid).is_err());
    invalid = delta();
    invalid.rotation[0][0] = 2.;
    assert!(scene.presentation_placement(id, invalid).is_err());
}

#[test]
fn only_instanced_geometry_builds_its_picking_index_until_isolated() {
    let shown = EntityId::new();
    let hidden = EntityId::new();
    let stamp = SceneStamp {
        document: EntityId::new(),
        revision: 2,
        generation: 3,
    };
    let geometry = |id, z| {
        Arc::new(
            Geometry::new_deferred(
                id,
                vec![[0., 0., z], [2., 0., z], [0., 2., z], [90., 90., 90.]],
                vec![0, 1, 2],
                vec![0],
                vec![],
            )
            .unwrap(),
        )
    };
    let visible_geometry = geometry(shown, 0.);
    let hidden_geometry = geometry(hidden, 1.);
    let visible_size = visible_geometry.storage_bytes();
    let hidden_size = hidden_geometry.storage_bytes();
    let scene = Scene::new(
        stamp,
        vec![visible_geometry.clone(), hidden_geometry.clone()],
        vec![Instance {
            id: shown,
            geometry: shown,
            transform: Default::default(),
            color: [1; 4],
        }],
    )
    .unwrap();
    assert!(visible_geometry.picking.get().is_some());
    assert!(visible_geometry.storage_bytes() > visible_size);
    assert!(hidden_geometry.picking.get().is_none());
    assert_eq!(hidden_geometry.storage_bytes(), hidden_size);
    assert_eq!(scene.world_bounds(), Some([[0., 0., 0.], [2., 2., 0.]]));
    let before = hidden_geometry.bounds();
    assert_eq!(before.min, [0., 0., 1.]);
    assert_eq!(before.max, [2., 2., 1.]);
    assert!(hidden_geometry.picking.get().is_none());
    assert!(scene.reference_snapshots().contains_key(&hidden));

    let ray = Ray::new([0.25, 0.25, 2.], [0., 0., -1.]).unwrap();
    assert_eq!(scene.pick_face(ray).unwrap().geometry, shown);
    assert!(hidden_geometry.picking.get().is_none());
    let isolated = scene.isolated_source(hidden).unwrap();
    assert!(hidden_geometry.picking.get().is_some());
    assert!(hidden_geometry.storage_bytes() > hidden_size);
    assert!(Arc::ptr_eq(
        &isolated.geometries()[&hidden],
        &hidden_geometry
    ));
    assert_eq!(isolated.world_bounds(), Some([before.min, before.max]));
    let hit = isolated.pick_face(ray).unwrap();
    assert_eq!(hit.geometry, hidden);
    assert_eq!(hit.point_mm, [0.25, 0.25, 1.]);
}
