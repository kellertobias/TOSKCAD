use super::*;
use crate::external_references::ExternalTool;
use crate::external_references::tests::{fixture, hit, scene};
use crate::sketch_constraints::ConstraintEditor;
use crate::viewport::{SceneReply, SceneStore};
use multicad_core::commands::{Actor, Channel};
struct Backend {
    base: Arc<Scene>,
    reply: Option<SceneReply>,
    partial_candidate: bool,
    bad_hash: bool,
}
impl Backend {
    fn reply(
        &mut self,
        document: &multicad_core::document::Document,
        stamp: SceneStamp,
        roots: Option<Vec<EntityId>>,
    ) -> Result<(), String> {
        let mut scene = Scene::new(
            stamp,
            self.base.geometries().values().cloned().collect(),
            self.base.reference_instances().to_vec(),
        )?
        .with_model_evaluation(Arc::new(multicad_core::model::ModelEvaluationInfo {
            source_hash: if self.bad_hash {
                ContentHash::of(b"unrelated source")
            } else {
                ContentHash::document(document)?
            },
            consumed_bodies: Default::default(),
            cut_targets: vec![],
        }))?;
        if let Some(roots) = roots {
            scene = scene.with_source_scope(roots)?;
        }
        self.reply = Some(SceneReply {
            stamp,
            result: Ok(Arc::new(scene)),
        });
        Ok(())
    }
}
impl SceneBackend for Backend {
    fn request(
        &mut self,
        document: &multicad_core::document::Document,
        stamp: SceneStamp,
    ) -> Result<(), String> {
        self.reply(
            document,
            stamp,
            self.partial_candidate
                .then(|| vec![self.base.instances()[0].geometry]),
        )
    }
    fn request_sources(
        &mut self,
        document: &multicad_core::document::Document,
        _: &ProjectResources,
        stamp: SceneStamp,
        roots: &[EntityId],
    ) -> Result<(), String> {
        self.reply(document, stamp, Some(roots.to_vec()))
    }
    fn poll(&mut self) -> Option<SceneReply> {
        self.reply.take()
    }
}
fn linked() -> (super::super::tests::Fixture, EntityId) {
    let mut fixture = fixture();
    let current = scene(&fixture, 1);
    let mut controller = Controller::default();
    controller.refresh(
        &fixture.session,
        Some(fixture.sketch),
        Some(current.clone()),
        Default::default(),
    );
    controller
        .begin(&fixture.session, ExternalTool::Project)
        .unwrap();
    controller
        .select(&fixture.session, hit(&fixture, &current, false))
        .unwrap();
    let proposal = controller.prepare(&fixture.session, Channel::Ui).unwrap();
    let reference = proposal.reference;
    fixture
        .session
        .commit(proposal.edit.preview.unwrap())
        .unwrap();
    (fixture, reference)
}
#[test]
fn repair_can_open_without_canonical_scene_but_only_current_source_scope_can_be_picked() {
    let (fixture, reference) = linked();
    let native = scene(&fixture, 1);
    let mut controller = Controller::default();
    controller.set_backend(Box::new(Backend {
        base: native,
        reply: None,
        partial_candidate: false,
        bad_hash: false,
    }));
    controller.refresh(
        &fixture.session,
        Some(fixture.sketch),
        None,
        Default::default(),
    );
    assert!(!controller.ready());
    controller
        .begin_repair(&fixture.session, reference)
        .unwrap();
    controller
        .request_source(
            &fixture.session,
            fixture.source.feature,
            &Default::default(),
        )
        .unwrap();
    assert!(controller.native_pending());
    assert!(!controller.can_preview(&fixture.session));
    controller.poll_native(&fixture.session, &Default::default());
    controller.refresh(
        &fixture.session,
        Some(fixture.sketch),
        None,
        Default::default(),
    );
    assert!(controller.ready());
    let inspected = controller.scene().unwrap();
    assert!(!inspected.is_full_model());
    controller
        .select(&fixture.session, hit(&fixture, &inspected, false))
        .unwrap();
    let proposal = controller.prepare(&fixture.session, Channel::Ui).unwrap();
    assert!(
        proposal
            .report
            .entries
            .iter()
            .any(|d| d.address.owner == reference)
    );
    let mut editor = ConstraintEditor::default();
    editor
        .accept_external_edit_with_report(
            &fixture.session,
            fixture.sketch,
            reference,
            proposal.edit,
            proposal.label,
            Some(proposal.report),
        )
        .unwrap();
    editor.require_external_native();
    assert!(!editor.external_can_apply(&fixture.session, fixture.sketch));
    controller.cancel();
    controller
        .validate_candidate(
            &fixture.session,
            editor.external_prepared().unwrap(),
            &Default::default(),
        )
        .unwrap();
    assert!(
        controller
            .poll_native(&fixture.session, &Default::default())
            .unwrap()
            .is_ok()
    );
    editor.complete_external_native(controller.candidate_validation().unwrap().clone());
    assert!(editor.external_can_apply(&fixture.session, fixture.sketch));
    let hash = ContentHash::document(fixture.session.document()).unwrap();
    assert_eq!(hash, controller.context.as_ref().unwrap().hash);
}
#[test]
fn source_scope_survives_visibility_and_isolation_and_cannot_publish_canonical_or_apply() {
    let (fixture, reference) = linked();
    let native = scene(&fixture, 1);
    let partial = Scene::new(
        native.stamp(),
        native.geometries().values().cloned().collect(),
        native.instances().to_vec(),
    )
    .unwrap()
    .with_model_evaluation(native.model_evaluation().unwrap().clone())
    .unwrap()
    .with_source_scope(vec![fixture.source.feature])
    .unwrap();
    let partial = partial
        .visible_subset(&[fixture.source.body].into())
        .unwrap();
    assert!(!partial.is_full_model());
    assert_eq!(
        partial
            .isolated_source(fixture.source.feature)
            .unwrap()
            .scope(),
        partial.scope()
    );
    let mut store = SceneStore::default();
    store.begin(partial.stamp()).unwrap();
    assert!(store.publish(partial).is_err());
    assert!(store.visible().is_none());
    for bad_hash in [false, true] {
        let mut controller = Controller::default();
        controller.set_backend(Box::new(Backend {
            base: native.clone(),
            reply: None,
            partial_candidate: !bad_hash,
            bad_hash,
        }));
        controller.refresh(
            &fixture.session,
            Some(fixture.sketch),
            Some(native.clone()),
            Default::default(),
        );
        controller
            .begin_repair(&fixture.session, reference)
            .unwrap();
        controller
            .select(&fixture.session, hit(&fixture, &native, false))
            .unwrap();
        let proposal = controller.prepare(&fixture.session, Channel::Ui).unwrap();
        controller.cancel();
        controller
            .validate_candidate(
                &fixture.session,
                proposal.edit.preview.as_ref().unwrap(),
                &Default::default(),
            )
            .unwrap();
        assert!(
            controller
                .poll_native(&fixture.session, &Default::default())
                .unwrap()
                .is_err()
        );
        assert!(controller.candidate_validation().unwrap().is_err());
    }
}
#[test]
fn a_source_change_rejects_pending_native_reference_receipts() {
    let (mut fixture, reference) = linked();
    let native = scene(&fixture, 1);
    let mut controller = Controller::default();
    controller.set_backend(Box::new(Backend {
        base: native.clone(),
        reply: None,
        partial_candidate: false,
        bad_hash: false,
    }));
    controller.refresh(
        &fixture.session,
        Some(fixture.sketch),
        Some(native.clone()),
        Default::default(),
    );
    controller
        .begin_repair(&fixture.session, reference)
        .unwrap();
    controller
        .select(&fixture.session, hit(&fixture, &native, false))
        .unwrap();
    let proposal = controller.prepare(&fixture.session, Channel::Ui).unwrap();
    controller
        .validate_candidate(
            &fixture.session,
            proposal.edit.preview.as_ref().unwrap(),
            &Default::default(),
        )
        .unwrap();
    fixture
        .session
        .apply(multicad_core::commands::Request {
            expected_revision: fixture.session.revision(),
            actor: Actor::local(Channel::Ui),
            label: "Other edit".into(),
            edits: vec![multicad_core::document::Mutation::Insert(
                multicad_core::document::Entity::new(
                    "other_value",
                    multicad_core::document::EntityData::Parameter {
                        expression: "2".into(),
                        unit: "mm".into(),
                        exposed: false,
                        inputs: vec![],
                    },
                ),
            )],
        })
        .unwrap();
    assert!(
        controller
            .poll_native(&fixture.session, &Default::default())
            .unwrap()
            .is_err()
    );
}
#[test]
fn foreign_session_with_identical_document_revision_and_hash_cannot_authorize_apply() {
    let (fixture, _) = linked();
    let base = scene(&fixture, 1);
    let prepared = fixture
        .session
        .prepare(multicad_core::commands::Request {
            expected_revision: fixture.session.revision(),
            actor: Actor::local(Channel::Ui),
            label: "Inspection receipt".into(),
            edits: vec![],
        })
        .unwrap();
    let mut native = Native {
        backend: Some(Box::new(Backend {
            base,
            reply: None,
            partial_candidate: false,
            bad_hash: false,
        })),
        ..Default::default()
    };
    native
        .validate_candidate(&fixture.session, &prepared, &Default::default())
        .unwrap();
    let mut foreign = Session::new(fixture.session.document().clone()).unwrap();
    let parameter = multicad_core::document::Entity::new(
        "temporary",
        multicad_core::document::EntityData::Parameter {
            expression: "1".into(),
            unit: "mm".into(),
            exposed: false,
            inputs: vec![],
        },
    );
    let id = parameter.id;
    for mutation in [
        multicad_core::document::Mutation::Insert(parameter),
        multicad_core::document::Mutation::Remove(id),
    ] {
        foreign
            .apply(multicad_core::commands::Request {
                expected_revision: foreign.revision(),
                actor: Actor::local(Channel::Ui),
                label: "Same source different session".into(),
                edits: vec![mutation],
            })
            .unwrap();
    }
    assert_eq!(foreign.document(), fixture.session.document());
    assert_eq!(foreign.revision(), fixture.session.revision());
    assert!(
        native
            .poll(&foreign, &Default::default())
            .unwrap()
            .unwrap_err()
            .contains("different session")
    );
}

#[test]
fn resources_changed_before_or_after_source_receipt_revoke_picking_without_source_changes() {
    for complete_first in [false, true] {
        let (fixture, reference) = linked();
        let before = fixture.session.document().clone();
        let native = scene(&fixture, 1);
        let mut controller = Controller::default();
        controller.set_backend(Box::new(Backend {
            base: native,
            reply: None,
            partial_candidate: false,
            bad_hash: false,
        }));
        controller.refresh(
            &fixture.session,
            Some(fixture.sketch),
            None,
            Default::default(),
        );
        controller
            .begin_repair(&fixture.session, reference)
            .unwrap();
        let resources = ProjectResources::default();
        controller
            .request_source(&fixture.session, fixture.source.feature, &resources)
            .unwrap();
        if complete_first {
            controller.poll_native(&fixture.session, &resources);
            controller.refresh(
                &fixture.session,
                Some(fixture.sketch),
                None,
                Default::default(),
            );
            let inspected = controller.scene().unwrap();
            controller
                .select(&fixture.session, hit(&fixture, &inspected, false))
                .unwrap();
            assert!(controller.can_preview(&fixture.session));
        }
        let mut changed = resources.clone();
        changed
            .metadata
            .insert("source-version".into(), serde_json::json!(2));
        assert!(
            controller
                .poll_native(&fixture.session, &changed)
                .unwrap()
                .is_err()
        );
        assert!(!controller.can_preview(&fixture.session));
        assert!(controller.scene().is_none());
        assert!(controller.native.scene.is_none());
        assert!(controller.active());
        assert_eq!(fixture.session.document(), &before);
        assert!(!controller.native_pending());
    }
}
#[test]
fn successful_candidate_receipt_is_revoked_when_resources_change_at_the_same_revision() {
    let fixture = fixture();
    let before = fixture.session.document().clone();
    let revision = fixture.session.revision();
    let undo = fixture.session.can_undo();
    let native = scene(&fixture, 1);
    let mut controller = Controller::default();
    controller.refresh(
        &fixture.session,
        Some(fixture.sketch),
        Some(native.clone()),
        Default::default(),
    );
    controller
        .begin(&fixture.session, ExternalTool::Project)
        .unwrap();
    controller
        .select(&fixture.session, hit(&fixture, &native, false))
        .unwrap();
    let proposal = controller.prepare(&fixture.session, Channel::Ui).unwrap();
    controller.set_backend(Box::new(Backend {
        base: native,
        reply: None,
        partial_candidate: false,
        bad_hash: false,
    }));
    let resources = ProjectResources::default();
    controller
        .validate_candidate(
            &fixture.session,
            proposal.edit.preview.as_ref().unwrap(),
            &resources,
        )
        .unwrap();
    assert!(
        controller
            .poll_native(&fixture.session, &resources)
            .unwrap()
            .is_ok()
    );
    assert!(controller.candidate_validation().unwrap().is_ok());
    let mut changed = resources.clone();
    changed
        .metadata
        .insert("source-version".into(), serde_json::json!(2));
    assert!(
        controller
            .poll_native(&fixture.session, &changed)
            .unwrap()
            .is_err()
    );
    assert!(controller.candidate_validation().unwrap().is_err());
    assert_eq!(fixture.session.document(), &before);
    assert_eq!(fixture.session.revision(), revision);
    assert_eq!(fixture.session.can_undo(), undo);
}
