use super::controller::Purpose;
use super::*;
use crate::{
    camera::Camera,
    egui,
    sketch_constraints::ConstraintEditor,
    viewport::{DisplayEdge, DisplayPoint, Geometry, Hit, Instance, Scene, SceneStamp, Transform},
};
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::source::Source,
    document::{Document, Entity, EntityData, EntityId, Mutation},
    model::{self, PrimitiveParameters, PrimitiveRequest},
    project_file::ContentHash,
    references::{self, AnalyticGeometry},
    sketch::Datum,
    sketch_external::{self, Behavior},
};
use multicad_kernel::{GeometryPolicy, MeshOptions, Solid};
use std::{collections::BTreeSet, sync::Arc};
pub(crate) struct Fixture {
    pub session: Session,
    pub sketch: EntityId,
    pub source: PrimitiveRequest,
}
pub(crate) fn fixture() -> Fixture {
    let (sketch, edits) = Source::create(Datum::Xy, "External consumer").unwrap();
    let mut session = Session::new(
        Document::new("External UI source contracts")
            .unwrap()
            .edited(&edits)
            .unwrap(),
    )
    .unwrap();
    let source = PrimitiveRequest::new(
        "Native source",
        PrimitiveParameters::Box {
            size: [model::length(30.), model::length(20.), model::length(10.)],
            frame: Default::default(),
        },
    );
    let prepared =
        model::prepare_primitive(&session, 0, Actor::local(Channel::Ui), &source).unwrap();
    session.commit(prepared.prepared).unwrap();
    Fixture {
        session,
        sketch,
        source,
    }
}
pub(crate) fn scene(fixture: &Fixture, generation: u64) -> Arc<Scene> {
    let owner = fixture.source.feature;
    let solid = Solid::box_named([30., 20., 10.], owner, GeometryPolicy::default()).unwrap();
    let mesh = solid.mesh(MeshOptions::default()).unwrap();
    let geometry = Geometry::new(
        owner,
        mesh.vertices.as_chunks::<3>().0.to_vec(),
        mesh.indices,
        mesh.face_indices,
        solid
            .topology_snapshot(owner, fixture.session.revision())
            .unwrap()
            .elements,
    )
    .unwrap()
    .with_topology_display(
        solid
            .display_edges(0.01)
            .unwrap()
            .into_iter()
            .map(|edge| DisplayEdge {
                index: edge.index,
                points_mm: edge.points_mm,
            })
            .collect(),
        solid
            .display_vertices()
            .unwrap()
            .into_iter()
            .map(|point| DisplayPoint {
                index: point.index,
                position_mm: point.position_mm,
            })
            .collect(),
        0.01,
    )
    .unwrap();
    Arc::new(
        Scene::new(
            SceneStamp {
                document: fixture.session.document().id(),
                revision: fixture.session.revision(),
                generation,
            },
            vec![Arc::new(geometry)],
            vec![Instance {
                id: fixture.source.body,
                geometry: owner,
                transform: Transform::default(),
                color: [160, 180, 200, 255],
            }],
        )
        .unwrap()
        .with_model_evaluation(Arc::new(model::ModelEvaluationInfo {
            screw_connection_checks: vec![],
            source_hash: ContentHash::document(fixture.session.document()).unwrap(),
            consumed_bodies: BTreeSet::new(),
            cut_targets: vec![],
        }))
        .unwrap(),
    )
}
pub(crate) fn hit(fixture: &Fixture, scene: &Scene, top: bool) -> Hit {
    let snapshot = scene
        .reference_snapshots()
        .remove(&fixture.source.feature)
        .unwrap();
    let element=snapshot.elements.iter().find(|element|matches!(element.geometry,Some(AnalyticGeometry::Line{a_mm,b_mm})if a_mm[1].abs()<1e-7&&b_mm[1].abs()<1e-7&&(a_mm[0]-b_mm[0]).abs()>29.&&(a_mm[2]-if top{10.}else{0.}).abs()<1e-7&&(b_mm[2]-a_mm[2]).abs()<1e-7)).unwrap();
    Hit {
        instance: fixture.source.body,
        geometry: fixture.source.feature,
        key: element.key,
        distance_mm: 1.,
        point_mm: [15., 0., if top { 10. } else { 0. }],
        intent: Some(references::GeometryIntent::capture(&snapshot, element.key).unwrap()),
        stamp: scene.stamp(),
    }
}
fn controller(fixture: &Fixture, scene: Arc<Scene>) -> Controller {
    let mut controller = Controller::default();
    controller.refresh(
        &fixture.session,
        Some(fixture.sketch),
        Some(scene),
        Transform::default(),
    );
    controller
        .begin(&fixture.session, ExternalTool::Project)
        .unwrap();
    controller
}
#[test]
fn real_pointer_capture_and_shared_preview_are_source_neutral_until_one_undoable_apply() {
    let mut fixture = fixture();
    let scene = scene(&fixture, 1);
    let mut controller = controller(&fixture, scene.clone());
    let before = fixture.session.document().clone();
    controller.active.as_mut().unwrap().include_hidden = true;
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900., 600.));
    let camera = Camera {
        yaw: 0.,
        pitch: 1.0,
        scale: 10.,
        target: [15., 10., 0.],
        perspective: false,
    };
    let screen = camera.project([15., 0., 0.], rect);
    let point = egui::pos2(screen[0], screen[1]);
    assert!(
        controller.pointer(Pointer {
            camera,
            rect,
            position: Some(point),
            clicked: true,
            section: None,
            session: &fixture.session
        }),
        "{:?}",
        controller.error()
    );
    assert!(matches!(
        controller.highlight(),
        Some(crate::viewport::Selection::Edge { .. })
    ));
    let first = controller
        .prepare(&fixture.session, Channel::CommandLine)
        .unwrap();
    let reference = first.reference;
    let candidate = first.edit.preview.as_ref().unwrap().document().clone();
    let repeated = controller
        .prepare(&fixture.session, Channel::CommandLine)
        .unwrap();
    assert_eq!(repeated.reference, reference);
    assert_eq!(
        repeated.edit.preview.as_ref().unwrap().document(),
        &candidate
    );
    assert_eq!(fixture.session.document(), &before);
    let mut editor = ConstraintEditor::default();
    editor.set_evaluation(Some(references::inspect(
        &before,
        fixture.session.revision(),
        &scene.reference_snapshots(),
    )));
    editor
        .accept_external_edit(
            &fixture.session,
            fixture.sketch,
            reference,
            first.edit,
            first.label,
        )
        .unwrap();
    assert!(editor.external_pending());
    assert!(editor.external_can_apply(&fixture.session, fixture.sketch));
    assert_eq!(editor.preview_document(), Some(&candidate));
    let context = egui::Context::default();
    let output = context.run_ui(egui::RawInput::default(), |ui| {
        editor.show(ui, &mut fixture.session, fixture.sketch, None)
    });
    let labels: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| {
            if let egui::Shape::Text(text) = &shape.shape {
                Some(text.galley.text().to_owned())
            } else {
                None
            }
        })
        .collect();
    assert!(
        !labels
            .iter()
            .any(|label| label.contains("Select geometry (in order)")),
        "Pending metadata cannot be flattened by another constraint edit"
    );
    output.drop_without_applying_deltas();
    editor.apply(&mut fixture.session).unwrap();
    assert_eq!(fixture.session.document(), &candidate);
    assert_eq!(
        fixture.session.activity().last().unwrap().actor.channel,
        Channel::CommandLine
    );
    fixture
        .session
        .undo(fixture.session.revision(), Actor::local(Channel::Ui))
        .unwrap();
    assert_eq!(fixture.session.document(), &before);
    fixture
        .session
        .redo(fixture.session.revision(), Actor::local(Channel::Ui))
        .unwrap();
    assert_eq!(fixture.session.document(), &candidate);
}
#[test]
fn invalid_new_pick_foreign_session_and_stale_native_generation_cannot_reuse_a_previous_selection()
{
    let fixture = fixture();
    let scene = scene(&fixture, 1);
    let mut controller = controller(&fixture, scene.clone());
    controller
        .select(&fixture.session, hit(&fixture, &scene, false))
        .unwrap();
    assert!(controller.prepare(&fixture.session, Channel::Ui).is_ok());
    assert!(
        controller
            .select(&fixture.session, hit(&fixture, &scene, true))
            .is_err()
    );
    assert!(controller.prepare(&fixture.session, Channel::Ui).is_err());
    controller
        .select(&fixture.session, hit(&fixture, &scene, false))
        .unwrap();
    let replacement = super::tests::scene(&fixture, 2);
    controller.refresh(
        &fixture.session,
        Some(fixture.sketch),
        Some(replacement),
        Transform::default(),
    );
    assert!(controller.prepare(&fixture.session, Channel::Ui).is_err());
    let initial = fixture
        .session
        .document()
        .edited(&[
            Mutation::Remove(fixture.source.body),
            Mutation::Remove(fixture.source.feature),
        ])
        .unwrap();
    let mut foreign = Session::new(initial).unwrap();
    let prepared =
        model::prepare_primitive(&foreign, 0, Actor::local(Channel::Ui), &fixture.source).unwrap();
    foreign.commit(prepared.prepared).unwrap();
    assert_eq!(foreign.document(), fixture.session.document());
    assert_eq!(foreign.revision(), fixture.session.revision());
    assert!(!controller.ready_for(&foreign));
    assert!(controller.begin(&foreign, ExternalTool::Project).is_err());
}
#[test]
fn checked_behavior_remove_and_repair_share_the_same_preview_lane_and_keep_output_ids() {
    let mut fixture = fixture();
    let current = scene(&fixture, 1);
    let mut controller = controller(&fixture, current.clone());
    controller
        .select(&fixture.session, hit(&fixture, &current, false))
        .unwrap();
    let proposal = controller.prepare(&fixture.session, Channel::Ui).unwrap();
    let reference = proposal.reference;
    let mut editor = ConstraintEditor::default();
    editor
        .accept_external_edit(
            &fixture.session,
            fixture.sketch,
            reference,
            proposal.edit,
            proposal.label,
        )
        .unwrap();
    editor.apply(&mut fixture.session).unwrap();
    let original = sketch_external::read(fixture.session.document(), reference)
        .unwrap()
        .definition
        .output;
    controller.cancel();
    controller.refresh(
        &fixture.session,
        Some(fixture.sketch),
        Some(scene(&fixture, 2)),
        Transform::default(),
    );
    let proposal = controller
        .behavior(&fixture.session, reference, Behavior::Derived)
        .unwrap();
    editor
        .accept_external_edit(
            &fixture.session,
            fixture.sketch,
            reference,
            proposal.edit,
            proposal.label,
        )
        .unwrap();
    editor.apply(&mut fixture.session).unwrap();
    let binding = sketch_external::read(fixture.session.document(), reference).unwrap();
    assert_eq!(binding.definition.output, original);
    assert_eq!(binding.definition.behavior, Behavior::Derived);
    let current = scene(&fixture, 3);
    controller.refresh(
        &fixture.session,
        Some(fixture.sketch),
        Some(current.clone()),
        Transform::default(),
    );
    controller
        .begin_repair(&fixture.session, reference)
        .unwrap();
    assert!(
        matches!(controller.active.as_ref().unwrap().purpose,Purpose::Repair(id)if id==reference)
    );
    controller
        .select(&fixture.session, hit(&fixture, &current, false))
        .unwrap();
    let proposal = controller.prepare(&fixture.session, Channel::Ui).unwrap();
    editor
        .accept_external_edit(
            &fixture.session,
            fixture.sketch,
            reference,
            proposal.edit,
            proposal.label,
        )
        .unwrap();
    editor.discard();
    assert_eq!(
        sketch_external::read(fixture.session.document(), reference)
            .unwrap()
            .definition
            .output,
        original
    );
    controller.cancel();
    let proposal = controller.remove(&fixture.session, reference).unwrap();
    editor
        .accept_external_edit(
            &fixture.session,
            fixture.sketch,
            reference,
            proposal.edit,
            proposal.label,
        )
        .unwrap();
    editor.apply(&mut fixture.session).unwrap();
    assert!(fixture.session.document().get(reference).is_none());
}
#[test]
fn unrelated_transaction_changes_and_later_source_edits_cannot_authorize_reference_apply() {
    let mut fixture = fixture();
    let current = scene(&fixture, 1);
    let mut controller = controller(&fixture, current.clone());
    controller
        .select(&fixture.session, hit(&fixture, &current, false))
        .unwrap();
    let mut proposal = controller.prepare(&fixture.session, Channel::Ui).unwrap();
    let candidate = proposal.edit.preview.as_ref().unwrap().document();
    let extra = Entity::new(
        "Unrelated",
        EntityData::Parameter {
            expression: "1".into(),
            unit: "mm".into(),
            exposed: false,
            inputs: vec![],
        },
    );
    let malicious = fixture
        .session
        .prepare(Request {
            expected_revision: fixture.session.revision(),
            actor: Actor::local(Channel::Ui),
            label: "Foreign metadata".into(),
            edits: vec![
                Mutation::Replace(candidate.get(fixture.sketch).unwrap().clone()),
                Mutation::Insert(candidate.get(proposal.reference).unwrap().clone()),
                Mutation::Insert(extra),
            ],
        })
        .unwrap();
    proposal.edit.semantic.as_mut().unwrap().stamp.source_hash =
        ContentHash::document(malicious.document()).unwrap();
    proposal.edit.preview = Some(malicious);
    let mut editor = ConstraintEditor::default();
    assert!(
        editor
            .accept_external_edit(
                &fixture.session,
                fixture.sketch,
                proposal.reference,
                proposal.edit,
                proposal.label
            )
            .unwrap_err()
            .contains("unrelated")
    );
    let proposal = controller.prepare(&fixture.session, Channel::Ui).unwrap();
    editor
        .accept_external_edit(
            &fixture.session,
            fixture.sketch,
            proposal.reference,
            proposal.edit,
            proposal.label,
        )
        .unwrap();
    let before = fixture.session.document().clone();
    fixture
        .session
        .apply(Request {
            expected_revision: fixture.session.revision(),
            actor: Actor::local(Channel::Mcp),
            label: "Source changed".into(),
            edits: vec![Mutation::Insert(Entity::new(
                "new_parameter",
                EntityData::Parameter {
                    expression: "2".into(),
                    unit: "mm".into(),
                    exposed: false,
                    inputs: vec![],
                },
            ))],
        })
        .unwrap();
    assert!(!editor.external_can_apply(&fixture.session, fixture.sketch));
    assert!(editor.apply(&mut fixture.session).is_err());
    assert!(fixture.session.document().get(proposal.reference).is_none());
    assert_ne!(fixture.session.document(), &before);
}
#[test]
fn missing_hash_or_placed_context_is_unavailable_and_narrow_reference_controls_do_not_expand() {
    let fixture = fixture();
    let current = scene(&fixture, 1);
    let mut controller = controller(&fixture, current.clone());
    let bad = Arc::new(
        Scene::new(
            current.stamp(),
            current.geometries().values().cloned().collect(),
            current.reference_instances().to_vec(),
        )
        .unwrap(),
    );
    controller.refresh(
        &fixture.session,
        Some(fixture.sketch),
        Some(bad),
        Transform::default(),
    );
    assert!(!controller.ready_for(&fixture.session));
    assert!(controller.prepare(&fixture.session, Channel::Ui).is_err());
    controller.cancel();
    let pose = Transform::from_placement(multicad_core::document::Placement {
        translation_mm: [10., 0., 0.],
        ..Default::default()
    })
    .unwrap();
    controller.refresh(
        &fixture.session,
        Some(fixture.sketch),
        Some(current.clone()),
        pose,
    );
    assert!(!controller.ready_for(&fixture.session));
    assert!(
        controller
            .begin(&fixture.session, ExternalTool::Project)
            .is_err()
    );
    controller.refresh(
        &fixture.session,
        Some(fixture.sketch),
        Some(current),
        Transform::default(),
    );
    controller
        .begin(&fixture.session, ExternalTool::Project)
        .unwrap();
    let before = fixture.session.document().clone();
    let context = egui::Context::default();
    for _ in 0..100 {
        context
            .run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200., 1000.),
                    )),
                    ..Default::default()
                },
                |ui| {
                    let panel = egui::Panel::right("narrow-external-inspector")
                        .default_size(180.)
                        .min_size(0.)
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical().show(ui, |ui| {
                                assert!(
                                    controller
                                        .inspector(
                                            ui,
                                            &fixture.session,
                                            fixture.sketch,
                                            None,
                                            true,
                                            &Default::default()
                                        )
                                        .is_none()
                                );
                            });
                        });
                    assert!(
                        panel.response.rect.width() <= 181.,
                        "Inspector grew to {}",
                        panel.response.rect.width()
                    );
                },
            )
            .drop_without_applying_deltas();
    }
    assert_eq!(fixture.session.document(), &before);
}
