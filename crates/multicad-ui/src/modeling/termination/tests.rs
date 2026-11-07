use super::*;
use crate::{
    navigation::AxisView,
    viewport::{Geometry, Instance, SceneStamp},
};
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Entity, Mutation, Placement},
    model::{self, ModelEvaluationInfo, PrimitiveParameters, PrimitiveRequest},
    references::{AnchoredFrame, Origin},
};
use multicad_kernel::{MeshOptions, Solid};
use std::sync::Arc;
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn rect() -> egui::Rect {
    egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900., 600.))
}
fn camera(z: f32) -> Camera {
    let mut c = Camera {
        target: [50., 30., z],
        scale: 3.,
        ..Default::default()
    };
    c.orient(AxisView::PositiveZ);
    c
}
fn fixture(
    planes: usize,
    multiple: bool,
    placement: Placement,
) -> (Session, Scene, EntityId, Vec<EntityId>) {
    let mut session = Session::new(Document::new("Canonical end picks").unwrap()).unwrap();
    let request = PrimitiveRequest::new(
        "Target panel",
        PrimitiveParameters::Box {
            size: [length(100.), length(60.), length(18.)],
            frame: Default::default(),
        },
    );
    let p = model::prepare_primitive(&session, 0, actor(), &request).unwrap();
    session.commit(p.prepared).unwrap();
    let mut ids = vec![];
    let mut edits = vec![];
    for i in 0..planes {
        let plane = Entity::new(
            format!("Target plane {i}"),
            EntityData::Plane {
                origin_mm: [50., 30., 30.],
                normal: [0., 0., 1.],
                anchor: None,
            },
        );
        ids.push(plane.id);
        edits.push(Mutation::Insert(plane));
    }
    if !edits.is_empty() {
        session
            .apply(Request {
                expected_revision: session.revision(),
                actor: actor(),
                label: "Construction planes".into(),
                edits,
            })
            .unwrap();
    }
    let solid = Solid::box_named([100., 60., 18.], request.feature, Default::default()).unwrap();
    let mesh = solid.mesh(MeshOptions::default()).unwrap();
    let mut topology = solid
        .topology_snapshot(request.feature, session.revision())
        .unwrap()
        .elements;
    if multiple {
        let cap = Origin {
            source: request.feature,
            role: "cap.end".into(),
        };
        let face = topology
            .iter_mut()
            .find(|e| e.key.kind == TopologyKind::Face && e.origins.contains(&cap))
            .unwrap();
        let alias = Origin {
            source: request.feature,
            role: "alternate.cap".into(),
        };
        let frame = face.frames[0].frame;
        face.origins.insert(alias.clone());
        face.frames.push(AnchoredFrame {
            origin: alias,
            frame,
        });
    }
    let geometry = Arc::new(
        Geometry::new(
            request.feature,
            mesh.vertices.as_chunks::<3>().0.to_vec(),
            mesh.indices,
            mesh.face_indices,
            topology,
        )
        .unwrap(),
    );
    let scene = Scene::new(
        SceneStamp {
            document: session.document().id(),
            revision: session.revision(),
            generation: 1,
        },
        vec![geometry],
        vec![Instance {
            id: request.body,
            geometry: request.feature,
            transform: Transform::from_placement(placement).unwrap(),
            color: [140, 160, 180, 255],
        }],
    )
    .unwrap()
    .with_model_evaluation(Arc::new(ModelEvaluationInfo {
        screw_connection_checks: vec![],
        source_hash: ContentHash::document(session.document()).unwrap(),
        consumed_bodies: Default::default(),
        cut_targets: vec![],
    }))
    .unwrap();
    (session, scene, request.feature, ids)
}
fn viewport(
    picker: &mut EndPicker,
    session: &Session,
    scene: Option<&Scene>,
    camera: Camera,
    clicked: bool,
    forbidden: &[EntityId],
) -> Option<(EndSide, PickChoice)> {
    let ctx = egui::Context::default();
    let mut out = None;
    let output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(rect()),
            ..Default::default()
        },
        |ui| {
            let painter = ui.painter().clone();
            out = picker.viewport(
                session.document(),
                session.revision(),
                ContentHash::document(session.document()).unwrap(),
                TerminationViewport {
                    scene,
                    report: None,
                    camera,
                    rect: rect(),
                    pointer: Some(rect().center()),
                    clicked,
                    painter: &painter,
                    section: None,
                },
                forbidden,
            );
        },
    );
    output.drop_without_applying_deltas();
    out
}
#[test]
fn real_named_face_pick_captures_local_feature_intent_without_selecting_any_frame() {
    let (session, scene, producer, _) = fixture(0, true, Default::default());
    let before = session.document().clone();
    let mut picker = EndPicker::default();
    picker.begin(EndSide::Forward, EndKind::Face);
    assert!(viewport(&mut picker, &session, Some(&scene), camera(18.), false, &[]).is_none());
    assert!(matches!(picker.highlight(), Some(Selection::Face { .. })));
    let cached = match picker.hovered.as_ref().unwrap() {
        PickChoice::Face { input, .. } => input.intent.id,
        _ => panic!(),
    };
    let (_, choice) =
        viewport(&mut picker, &session, Some(&scene), camera(18.), true, &[]).unwrap();
    let PickChoice::Face { input, .. } = choice else {
        panic!()
    };
    assert_eq!(input.source, producer);
    assert_eq!(input.intent.id, cached);
    assert!(input.intent.attachment.is_none());
    assert_eq!(input.intent.kind, TopologyKind::Face);
    assert!(picker.active.is_none());
    assert_eq!(session.document(), &before);
}
#[test]
fn current_existing_plane_can_be_selected_in_viewer_and_ambiguous_overlaps_use_list() {
    let (session, scene, _, ids) = fixture(1, false, Default::default());
    let before = session.document().clone();
    let mut picker = EndPicker::default();
    picker.begin(EndSide::Backward, EndKind::Plane);
    let (side, choice) =
        viewport(&mut picker, &session, Some(&scene), camera(30.), true, &[]).unwrap();
    assert_eq!(side, EndSide::Backward);
    let PickChoice::Plane { id, frame } = choice else {
        panic!()
    };
    assert_eq!(id, ids[0]);
    assert_eq!(frame.origin, [50., 30., 30.]);
    assert_eq!(session.document(), &before);
    let (session, scene, _, _) = fixture(2, false, Default::default());
    picker.begin(EndSide::Forward, EndKind::Plane);
    assert!(viewport(&mut picker, &session, Some(&scene), camera(30.), true, &[]).is_none());
    assert!(
        picker
            .message()
            .unwrap()
            .contains("Multiple construction planes")
    );
}
#[test]
fn stale_candidate_missing_model_info_transformed_occurrence_and_self_faces_reject() {
    let (session, scene, producer, _) = fixture(0, false, Default::default());
    let mut picker = EndPicker::default();
    picker.begin(EndSide::Forward, EndKind::Face);
    assert!(
        viewport(
            &mut picker,
            &session,
            Some(&scene),
            camera(18.),
            true,
            &[producer]
        )
        .is_none()
    );
    assert!(picker.message().unwrap().contains("own result"));
    let bad = scene
        .with_model_evaluation(Arc::new(ModelEvaluationInfo {
            screw_connection_checks: vec![],
            source_hash: ContentHash::of(b"candidate"),
            consumed_bodies: Default::default(),
            cut_targets: vec![],
        }))
        .unwrap();
    assert!(viewport(&mut picker, &session, Some(&bad), camera(18.), true, &[]).is_none());
    assert!(picker.message().unwrap().contains("candidate"));
    assert!(viewport(&mut picker, &session, None, camera(18.), true, &[]).is_none());
    let (session, scene, _, _) = fixture(
        0,
        false,
        Placement {
            translation_mm: [0., 0., 5.],
            ..Default::default()
        },
    );
    picker.begin(EndSide::Forward, EndKind::Face);
    assert!(viewport(&mut picker, &session, Some(&scene), camera(23.), true, &[]).is_none());
    assert!(picker.message().unwrap().contains("unplaced"));
}
#[test]
fn cancelled_picker_and_field_selection_are_source_neutral_and_keep_exact_reference_values() {
    let (session, scene, _, ids) = fixture(1, false, Default::default());
    let before = session.document().clone();
    let mut picker = EndPicker::default();
    picker.begin(EndSide::Forward, EndKind::Plane);
    viewport(&mut picker, &session, Some(&scene), camera(30.), false, &[]);
    picker.cancel();
    assert!(picker.highlight().is_none());
    assert!(picker.active.is_none());
    let end = Termination::ToPlane {
        plane: ids[0],
        offset: length(12.),
    };
    let field = EndCondition::new(&end, &[]).unwrap();
    assert_eq!(
        field.resolve(session.document(), true, true).unwrap(),
        (end, None)
    );
    assert_eq!(session.document(), &before);
}
