use super::{backend, hierarchy};
use multicad_core::{
    commands::*,
    document::*,
    hierarchy::{Action, Index},
    project_file::ProjectFile,
};
use multicad_ui::{
    application::DocumentStorage,
    storage::SaveMode,
    viewport::{SceneStamp, VisibilityFilter},
};
use std::collections::BTreeSet;
fn evaluate(document: &Document, revision: u64) -> backend::Evaluation {
    backend::evaluate(
        document,
        SceneStamp {
            document: document.id(),
            revision,
            generation: revision + 1,
        },
        || Ok(()),
    )
    .unwrap()
}
fn id(document: &Document, name: &str) -> EntityId {
    document.entities().find(|e| e.name == name).unwrap().id
}
#[test]
fn real_independent_bodies_expose_nested_sibling_and_hidden_cut_populations() {
    let document = hierarchy::document().unwrap();
    let evaluation = evaluate(&document, 0);
    let index = Index::new(&document).unwrap();
    let parts: Vec<_> = document
        .entities()
        .filter(|e| matches!(e.data, EntityData::Part { .. }))
        .collect();
    assert_eq!(parts.len(), 6);
    let producers: BTreeSet<_> = parts
        .iter()
        .map(|part| {
            let EntityData::Part { features, .. } = &part.data else {
                unreachable!()
            };
            assert_eq!(features.len(), 1);
            features[0]
        })
        .collect();
    assert_eq!(producers.len(), 6);
    assert_eq!(evaluation.scene.geometries().len(), 6);
    for body in parts {
        let instance = evaluation
            .scene
            .instances()
            .iter()
            .find(|instance| instance.id == body.id)
            .unwrap();
        let EntityData::Part { features, .. } = &body.data else {
            unreachable!()
        };
        assert_eq!(instance.geometry, features[0]);
        assert!(
            evaluation
                .scene
                .geometry_snapshot(instance.geometry)
                .unwrap()
                .elements
                .iter()
                .any(|element| element
                    .origins
                    .iter()
                    .any(|origin| origin.source == features[0]))
        );
    }
    assert_eq!(evaluation.scene.instances().len(), 7);
    let visible = VisibilityFilter::default()
        .apply(&evaluation.scene, &document, 0)
        .unwrap();
    assert_eq!(visible.instances().len(), 5);
    let names: BTreeSet<_> = index
        .active_cut_bodies()
        .unwrap()
        .iter()
        .map(|entry| document.get(*entry).unwrap().name.as_str())
        .collect();
    assert_eq!(names, BTreeSet::from(["Visible side A", "Grouped shelf A"]));
    assert_eq!(
        index
            .default_cut_bodies(Some(id(&document, "Drawer module")))
            .unwrap(),
        vec![id(&document, "Drawer bottom")]
    );
    assert_eq!(
        index
            .default_cut_bodies(Some(id(&document, "Cabinet B")))
            .unwrap(),
        vec![id(&document, "Sibling top B")]
    );
    assert!(evaluation.report.all_resolved());
    let evidence = evaluation.fixture_evidence.unwrap();
    assert_eq!(evidence["canonical_body_count"], 6);
    assert!(
        evidence["bodies"]
            .as_array()
            .unwrap()
            .iter()
            .all(|body| body["native_volume_mm3"].as_f64().unwrap() > 0.)
    );
}
#[test]
fn one_producer_dimension_edit_changes_only_that_real_body_geometry() {
    let document = hierarchy::document().unwrap();
    let before = evaluate(&document, 0);
    let body = id(&document, "Grouped shelf A");
    let EntityData::Part { features, .. } = &document.get(body).unwrap().data else {
        unreachable!()
    };
    let producer = features[0];
    let mut feature = document.get(producer).unwrap().clone();
    let EntityData::Feature { definition, .. } = &mut feature.data else {
        unreachable!()
    };
    definition.data["size_mm"][2] = serde_json::json!(24.);
    let changed = document.edited(&[Mutation::Replace(feature)]).unwrap();
    let after = evaluate(&changed, 1);
    for instance in before.scene.instances() {
        let new = after
            .scene
            .instances()
            .iter()
            .find(|new| new.id == instance.id)
            .unwrap();
        assert_eq!(new.geometry, instance.geometry);
        let old_bounds = before.scene.instance_bounds(instance.id).unwrap();
        let new_bounds = after.scene.instance_bounds(instance.id).unwrap();
        if instance.geometry == producer {
            assert!(new_bounds[1][2] > old_bounds[1][2] + 11.);
        } else {
            assert_eq!(old_bounds, new_bounds);
        }
    }
}
#[test]
fn native_roundtrip_and_move_preserve_all_body_sources_and_cut_scope_is_undoable() {
    let document = hierarchy::document().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hierarchy.multicad");
    let mut store = super::files::SnapshotStore;
    store
        .save_project(
            &path,
            &ProjectFile::new(document.clone()).unwrap(),
            SaveMode::Create,
        )
        .unwrap();
    let loaded = store.open_project(&path).unwrap();
    assert_eq!(loaded.file.document, document);
    assert_eq!(
        evaluate(&loaded.file.document, 0).scene.instances().len(),
        7
    );
    let body = id(&document, "Grouped shelf A");
    let sibling = id(&document, "Cabinet B");
    let original = document.get(body).unwrap().clone();
    let mut session = Session::new(document).unwrap();
    let pending = multicad_core::hierarchy::prepare(
        &session,
        0,
        Actor::local(Channel::Ui),
        Action::Move {
            entries: vec![body],
            parent: Some(sibling),
            before: None,
        },
    )
    .unwrap();
    session.commit(pending.prepared).unwrap();
    assert_eq!(session.document().get(body), Some(&original));
    assert_eq!(
        Index::new(session.document())
            .unwrap()
            .owning_component(body)
            .unwrap(),
        Some(sibling)
    );
    assert_eq!(evaluate(session.document(), 1).scene.instances().len(), 7);
    session.undo(1, Actor::local(Channel::Ui)).unwrap();
    assert_eq!(
        Index::new(session.document())
            .unwrap()
            .active_cut_bodies()
            .unwrap()
            .len(),
        2
    );
    session.redo(2, Actor::local(Channel::Ui)).unwrap();
    assert_eq!(
        Index::new(session.document())
            .unwrap()
            .active_cut_bodies()
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn unsupported_feature_and_fake_independent_body_aliases_fail_honestly() {
    let document = hierarchy::document().unwrap();
    let body = document.get(id(&document, "Visible side A")).unwrap();
    let alias = Entity::new("Fake independent body", body.data.clone());
    let changed = document.edited(&[Mutation::Insert(alias)]).unwrap();
    assert!(
        backend::evaluate(
            &changed,
            SceneStamp {
                document: changed.id(),
                revision: 0,
                generation: 1
            },
            || Ok(())
        )
        .err()
        .unwrap()
        .contains("distinct native producers")
    );
}
