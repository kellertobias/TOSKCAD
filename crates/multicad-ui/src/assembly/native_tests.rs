use super::Operation;
use super::*;
use crate::{
    assembly::{AssemblySelection, AssemblyTool},
    modeling::native_fixture,
    viewport::{Scene, SceneBackend, SceneReply, SceneStamp},
};
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::Document,
    model::*,
    project_file::{self, ProjectFile, ProjectResources},
};
use multicad_model::{Compiler, ModelError, ModelSnapshot, Runtime};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
struct NativeBackend {
    runtime: Runtime<Arc<Scene>>,
    stamp: Option<(u64, SceneStamp)>,
}
fn publish(
    document: &Document,
    snapshot: &ModelSnapshot,
    generation: u64,
) -> Result<Arc<Scene>, ModelError> {
    let scene =
        Arc::try_unwrap(native_fixture::scene(document, snapshot, generation)?).map_err(|_| {
            ModelError {
                entity: None,
                message: "Test scene unexpectedly shared".into(),
            }
        })?;
    let receipt = AssemblyEvaluationInfo {
        source_hash: snapshot.stamp.source_hash,
        leaves: snapshot
            .leaf_occurrences
            .values()
            .map(|leaf| AssemblyLeafReceipt {
                path: leaf.path.clone(),
                component: leaf.component,
                producer: leaf.producer,
                pose: leaf.pose,
            })
            .collect(),
    };
    scene
        .with_assembly_evaluation(Arc::new(receipt))
        .map(Arc::new)
        .map_err(|message| ModelError {
            entity: None,
            message,
        })
}
impl NativeBackend {
    fn new() -> Self {
        Self {
            runtime: Runtime::with_publisher(Compiler::default(), publish).unwrap(),
            stamp: None,
        }
    }
}
impl SceneBackend for NativeBackend {
    fn request(&mut self, document: &Document, stamp: SceneStamp) -> Result<(), String> {
        let generation = self.runtime.request_evaluation(
            document,
            stamp.revision,
            stamp.generation,
            &Default::default(),
        )?;
        self.stamp = Some((generation, stamp));
        Ok(())
    }
    fn poll(&mut self) -> Option<SceneReply> {
        let (generation, stamp) = self.stamp?;
        let reply = self.runtime.poll()?;
        if reply.generation != generation {
            return None;
        }
        self.stamp = None;
        Some(SceneReply {
            stamp,
            result: reply.result.map_err(|e| e.to_string()),
        })
    }
    fn cancel(&mut self) {
        self.stamp = None;
    }
}
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn setup() -> (Session, PrimitiveRequest, ComponentPlacementRequest) {
    let mut s = Session::new(Document::new("Native assembly controller").unwrap()).unwrap();
    let part = PrimitiveRequest::new(
        "Drawer rail",
        PrimitiveParameters::Box {
            size: [length(10.), length(20.), length(30.)],
            frame: Default::default(),
        },
    );
    s.commit(prepare_primitive(&s, 0, actor(), &part).unwrap().prepared)
        .unwrap();
    let mut placed = ComponentPlacementRequest::new(part.body, Default::default());
    placed.parameters.translation = [length(40.), length(20.), length(5.)];
    s.commit(
        prepare_place_component(&s, s.revision(), actor(), &placed)
            .unwrap()
            .prepared,
    )
    .unwrap();
    (s, part, placed)
}
fn controller() -> AssemblyController {
    let mut c = AssemblyController::default();
    c.set_backend(Box::new(NativeBackend::new()));
    c
}
fn preview(c: &mut AssemblyController, s: &Session) -> Arc<Scene> {
    c.preview(s, &Default::default(), Channel::Ui).unwrap();
    let until = Instant::now() + Duration::from_secs(20);
    while c.pending() {
        c.poll(s, &Default::default());
        assert!(Instant::now() < until, "native worker timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(c.can_apply(s, &Default::default()), "{:?}", c.error);
    c.display_scene(s, &Default::default()).unwrap()
}
fn joint(c: &mut AssemblyController) -> &mut JointRequest {
    let Operation::Joint(request) = &mut c.form.as_mut().unwrap().operation else {
        panic!("joint form")
    };
    request
}
#[test]
fn native_controller_joint_motion_named_poses_saved_edit_and_atomic_undo_reopen() {
    let (mut s, _part, placed) = setup();
    let mut c = controller();
    let original = s.document().clone();
    c.begin(
        AssemblyTool::Relationship,
        &s,
        &Default::default(),
        None,
        AssemblySelection {
            instances: vec![placed.instance],
            ..Default::default()
        },
        Channel::Ui,
    )
    .unwrap();
    let request = joint(&mut c);
    request.parameters.kind = JointKind::Slider;
    request.parameters.limits.translation[2] = Some(JointLimit {
        min: length(0.),
        max: length(80.),
    });
    let driver = request.driver;
    let scene = preview(&mut c, &s);
    assert_eq!(
        scene
            .assembly_evaluation()
            .unwrap()
            .leaves
            .iter()
            .find(|l| l.path.root == placed.instance)
            .unwrap()
            .pose
            .translation_mm,
        [40., 20., 5.]
    );
    c.apply(&mut s, &Default::default()).unwrap();
    assert_eq!(s.revision(), 3);
    assert!(c.display_scene(&s, &Default::default()).is_some());
    c.canonical_ready(&s);
    assert!(c.display_scene(&s, &Default::default()).is_none());
    let mut poses = vec![];
    for (name, value) in [("Closed", 0.), ("Half-open", 40.), ("Open", 80.)] {
        c.begin(
            AssemblyTool::NamedPose,
            &s,
            &Default::default(),
            None,
            Default::default(),
            Channel::Ui,
        )
        .unwrap();
        let Operation::Pose(request) = &mut c.form.as_mut().unwrap().operation else {
            panic!()
        };
        request.name = name.into();
        request.values[0].values.translation[2] = length(value);
        poses.push(request.pose);
        let scene = preview(&mut c, &s);
        assert_eq!(
            scene
                .assembly_evaluation()
                .unwrap()
                .leaves
                .iter()
                .find(|l| l.path.root == placed.instance)
                .unwrap()
                .pose
                .translation_mm,
            [40., 20., 5.]
        );
        c.apply(&mut s, &Default::default()).unwrap();
    }
    c.begin(
        AssemblyTool::Motion,
        &s,
        &Default::default(),
        None,
        AssemblySelection {
            joint: Some(driver),
            ..Default::default()
        },
        Channel::Ui,
    )
    .unwrap();
    joint(&mut c).parameters.values.translation[2] = length(40.);
    let scene = preview(&mut c, &s);
    assert_eq!(
        scene.native_instance_bounds(placed.instance).unwrap(),
        [[40., 20., 45.], [50., 40., 75.]]
    );
    let middle_candidate = c.candidate.as_ref().unwrap().prepared.document().clone();
    c.apply(&mut s, &Default::default()).unwrap();
    assert_eq!(s.document(), &middle_candidate);
    let before_pose = s.document().clone();
    c.begin(
        AssemblyTool::NamedPose,
        &s,
        &Default::default(),
        None,
        AssemblySelection {
            pose: Some(poses[2]),
            ..Default::default()
        },
        Channel::Ui,
    )
    .unwrap();
    c.form.as_mut().unwrap().operation = Operation::ApplyPose(poses[2]);
    let scene = preview(&mut c, &s);
    assert_eq!(
        scene.native_instance_bounds(placed.instance).unwrap()[0],
        [40., 20., 85.]
    );
    c.apply(&mut s, &Default::default()).unwrap();
    let open = s.document().clone();
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &before_pose);
    s.redo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &open);
    c.begin_existing(&s, &Default::default(), None, driver, Channel::Ui)
        .unwrap();
    let request = joint(&mut c);
    assert_eq!(request.driver, driver);
    request.parameters.values.translation[2] = length(81.);
    assert!(
        c.preview(&s, &Default::default(), Channel::Ui)
            .unwrap_err()
            .contains("inclusive limits")
    );
    assert!(!c.can_apply(&s, &Default::default()));
    joint(&mut c).parameters.values.translation[2] = length(20.);
    preview(&mut c, &s);
    c.cancel();
    assert_eq!(s.document(), &open);
    assert_ne!(s.document(), &original);
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(s.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let reopened = project_file::read(
        &mut std::io::Cursor::new(bytes),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(&reopened.project.document, s.document());
    let model = Compiler::default()
        .compile(&reopened.project.document, s.revision())
        .unwrap();
    assert_eq!(
        model.snapshot().occurrences[&placed.instance]
            .pose
            .translation_mm,
        [40., 20., 85.]
    );
}
#[test]
fn native_controller_reusable_nested_members_have_distinct_receipts_and_source_neutral_explode() {
    let (mut s, part, first) = setup();
    let mut c = controller();
    c.begin(
        AssemblyTool::Place,
        &s,
        &Default::default(),
        None,
        AssemblySelection {
            component: Some(part.body),
            ..Default::default()
        },
        Channel::Ui,
    )
    .unwrap();
    let Operation::Place(request) = &mut c.form.as_mut().unwrap().operation else {
        panic!()
    };
    request.parameters.translation = [length(70.), length(20.), length(5.)];
    let second = request.instance;
    preview(&mut c, &s);
    c.apply(&mut s, &Default::default()).unwrap();
    c.begin(
        AssemblyTool::CreateAssembly,
        &s,
        &Default::default(),
        None,
        AssemblySelection {
            instances: vec![first.instance, second],
            ..Default::default()
        },
        Channel::Ui,
    )
    .unwrap();
    let Operation::Assembly(request) = &mut c.form.as_mut().unwrap().operation else {
        panic!()
    };
    request.name = "Double rail".into();
    request.definition_parts = vec![part.body];
    let assembly = request.assembly;
    preview(&mut c, &s);
    c.apply(&mut s, &Default::default()).unwrap();
    let mut roots = vec![];
    let mut scene = None;
    for x in [100., 300.] {
        c.begin(
            AssemblyTool::Place,
            &s,
            &Default::default(),
            None,
            AssemblySelection {
                component: Some(assembly),
                ..Default::default()
            },
            Channel::Ui,
        )
        .unwrap();
        let Operation::Place(request) = &mut c.form.as_mut().unwrap().operation else {
            panic!()
        };
        request.parameters.translation = [length(x), length(0.), length(0.)];
        roots.push(request.instance);
        scene = Some(preview(&mut c, &s));
        c.apply(&mut s, &Default::default()).unwrap();
    }
    let model = Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap();
    let current = publish(s.document(), model.snapshot(), 999).unwrap();
    assert_eq!(current.assembly_evaluation().unwrap().leaves.len(), 4);
    assert!(
        current
            .assembly_evaluation()
            .unwrap()
            .leaves
            .iter()
            .all(|l| roots.contains(&l.path.root) && l.path.members.len() == 1)
    );
    assert!(scene.is_some());
    let before = s.document().clone();
    let revision = s.revision();
    c.begin(
        AssemblyTool::Explode,
        &s,
        &ProjectResources::default(),
        Some(&current),
        Default::default(),
        Channel::Ui,
    )
    .unwrap();
    assert!(!c.dirty());
    let offsets = c.display_leaf_offsets(&s, &Default::default()).unwrap();
    assert_eq!(offsets.len(), 4);
    assert_ne!(
        offsets[&OccurrencePath {
            root: roots[0],
            members: vec![first.instance]
        }],
        offsets[&OccurrencePath {
            root: roots[1],
            members: vec![second]
        }]
    );
    assert!(!c.can_apply(&s, &Default::default()));
    c.cancel();
    assert_eq!(s.document(), &before);
    assert_eq!(s.revision(), revision);
    c.begin_existing(
        &s,
        &Default::default(),
        Some(&current),
        assembly,
        Channel::Ui,
    )
    .unwrap();
    let Operation::Assembly(request) = &mut c.form.as_mut().unwrap().operation else {
        panic!()
    };
    request.members.reverse();
    let checked = preview(&mut c, &s);
    assert_eq!(
        checked
            .assembly_evaluation()
            .unwrap()
            .leaves
            .iter()
            .map(|l| l.path.clone())
            .collect::<std::collections::BTreeSet<_>>(),
        current
            .assembly_evaluation()
            .unwrap()
            .leaves
            .iter()
            .map(|l| l.path.clone())
            .collect()
    );
    c.cancel();
    assert_eq!(s.document(), &before);
}
#[test]
fn native_controller_all_joint_types_and_component_local_frame_picks() {
    use multicad_core::references::{Attachment, GeometryIntent, TopologyKind};
    for kind in [
        JointKind::Fixed,
        JointKind::Slider,
        JointKind::Revolute,
        JointKind::Cylindrical,
        JointKind::Planar,
    ] {
        let (mut s, part, placed) = setup();
        let model = Compiler::default()
            .compile(s.document(), s.revision())
            .unwrap();
        let scene = publish(s.document(), model.snapshot(), 1).unwrap();
        let before = s.document().clone();
        let mut c = controller();
        c.begin(
            AssemblyTool::Relationship,
            &s,
            &Default::default(),
            Some(&scene),
            AssemblySelection {
                instances: vec![placed.instance],
                ..Default::default()
            },
            Channel::Ui,
        )
        .unwrap();
        {
            let request = joint(&mut c);
            request.parameters.kind = kind;
            for (i, coord) in [JointCoordinate::X, JointCoordinate::Y, JointCoordinate::Z]
                .into_iter()
                .enumerate()
            {
                if kind.allows(coord) {
                    request.parameters.values.translation[i] = length(10. + i as f64);
                }
            }
            if kind.allows(JointCoordinate::Rotation) {
                request.parameters.values.rotation = angle(30.);
            }
        }
        if kind == JointKind::Fixed {
            c.form.as_mut().unwrap().pick = Some(super::super::AnchorSide::Moving);
            let snapshot = scene.reference_snapshots().remove(&part.body).unwrap();
            let face = snapshot
                .elements
                .iter()
                .find(|e| e.key.kind == TopologyKind::Face && e.frames.len() == 1)
                .unwrap();
            let mut intent = GeometryIntent::capture(&snapshot, face.key).unwrap();
            let missing = model::AlignmentInput {
                source: part.body,
                intent: intent.clone(),
            };
            assert!(
                c.set_anchor_input(
                    &s,
                    &Default::default(),
                    &scene,
                    super::super::AnchorSide::Moving,
                    missing
                )
                .unwrap_err()
                .contains("explicitly named")
            );
            intent.attachment = Some(Attachment {
                anchor: face.frames[0].origin.clone(),
                offset_mm: [0.; 3],
                angle_rad: 0.,
            });
            let input = model::AlignmentInput {
                source: part.body,
                intent,
            };
            assert!(
                c.set_anchor_input(
                    &s,
                    &Default::default(),
                    &scene,
                    super::super::AnchorSide::Moving,
                    model::AlignmentInput {
                        source: placed.instance,
                        ..input.clone()
                    }
                )
                .is_err()
            );
            c.set_anchor_input(
                &s,
                &Default::default(),
                &scene,
                super::super::AnchorSide::Moving,
                input,
            )
            .unwrap();
            assert!(c.anchor_pick().is_none());
        }
        let driver = joint(&mut c).driver;
        preview(&mut c, &s);
        c.apply(&mut s, &Default::default()).unwrap();
        let (source, inputs) = read_joint(s.document(), driver).unwrap();
        assert_eq!(source.parameters.kind, kind);
        assert_eq!(inputs.len(), usize::from(kind == JointKind::Fixed));
        s.undo(s.revision(), actor()).unwrap();
        assert_eq!(s.document(), &before);
        s.redo(s.revision(), actor()).unwrap();
        assert_eq!(
            read_joint(s.document(), driver).unwrap().0.parameters.kind,
            kind
        );
    }
}
#[test]
fn selected_relationship_and_saved_pose_edit_controls_keep_existing_ids() {
    let (mut s, _, placed) = setup();
    let request = JointRequest::new(
        placed.instance,
        JointParameters {
            kind: JointKind::Revolute,
            values: JointValues {
                rotation: angle(30.),
                ..Default::default()
            },
            limits: JointLimits {
                rotation: Some(JointLimit {
                    min: angle(0.),
                    max: angle(90.),
                }),
                ..Default::default()
            },
            ..Default::default()
        },
        vec![],
    );
    s.commit(
        prepare_joint(&s, s.revision(), actor(), &request)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let pose = NamedPoseRequest::new(
        "Half-open",
        vec![JointPoseValue {
            joint: request.driver,
            values: request.parameters.values.clone(),
        }],
    );
    s.commit(
        prepare_named_pose(&s, s.revision(), actor(), &pose)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let before = s.document().clone();
    let mut c = controller();
    c.begin(
        AssemblyTool::Relationship,
        &s,
        &Default::default(),
        None,
        AssemblySelection {
            instances: vec![placed.instance],
            ..Default::default()
        },
        Channel::Ui,
    )
    .unwrap();
    assert!(c.form.as_ref().unwrap().editing);
    assert_eq!(
        joint(&mut c),
        &JointRequest::existing(s.document(), request.driver).unwrap()
    );
    c.cancel();
    c.begin(
        AssemblyTool::NamedPose,
        &s,
        &Default::default(),
        None,
        AssemblySelection {
            pose: Some(pose.pose),
            ..Default::default()
        },
        Channel::Ui,
    )
    .unwrap();
    let ctx = crate::egui::Context::default();
    let mut point = None;
    let frame = |c: &mut AssemblyController, s: &mut Session, events: Vec<crate::egui::Event>| {
        let mut out = ctx.run_ui(
            crate::egui::RawInput {
                screen_rect: Some(crate::egui::Rect::from_min_size(
                    crate::egui::Pos2::ZERO,
                    crate::egui::vec2(250., 700.),
                )),
                events,
                focused: true,
                ..Default::default()
            },
            |ui| {
                crate::inspector_actions::scope(
                    ui,
                    Some(crate::inspector_actions::Owner::Assembly),
                    crate::registry::Mode::Model.color(),
                    |ui| c.show(ui, s, &Default::default()).unwrap(),
                );
            },
        );
        out.textures_delta.clear();
        out
    };
    for _ in 0..3 {
        let out = frame(&mut c, &mut s, vec![]);
        for clipped in out.shapes {
            if let crate::egui::Shape::Text(text) = clipped.shape
                && text.galley.text() == "Edit saved pose"
            {
                point = Some(text.galley.rect.translate(text.pos.to_vec2()).center());
            }
        }
    }
    let point = point.expect("painted Edit saved pose button");
    for pressed in [true, false] {
        frame(
            &mut c,
            &mut s,
            vec![
                crate::egui::Event::PointerMoved(point),
                crate::egui::Event::PointerButton {
                    pos: point,
                    button: crate::egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        );
    }
    assert!(c.form.as_ref().unwrap().editing);
    let Operation::Pose(edit) = &mut c.form.as_mut().unwrap().operation else {
        panic!()
    };
    assert_eq!(edit.pose, pose.pose);
    assert_eq!(edit.name, "Half-open");
    edit.values[0].values.rotation = angle(45.);
    preview(&mut c, &s);
    c.apply(&mut s, &Default::default()).unwrap();
    assert_eq!(
        read_named_pose(s.document(), pose.pose).unwrap().values[0]
            .values
            .rotation,
        angle(45.)
    );
    assert_eq!(
        read_joint(s.document(), request.driver)
            .unwrap()
            .0
            .parameters
            .values
            .rotation,
        angle(30.)
    );
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &before);
}
#[test]
fn legacy_physical_root_remains_in_native_candidate_leaf_proof() {
    let (mut s, part, member) = setup();
    let legacy = multicad_core::document::Entity::new(
        "Legacy physical assembly",
        multicad_core::document::EntityData::Assembly {
            instances: vec![member.instance],
        },
    );
    let legacy_id = legacy.id;
    s.commit(
        s.prepare(multicad_core::commands::Request {
            expected_revision: s.revision(),
            actor: actor(),
            label: "Legacy assembly source".into(),
            edits: vec![multicad_core::document::Mutation::Insert(legacy)],
        })
        .unwrap(),
    )
    .unwrap();
    let mut c = controller();
    c.begin(
        AssemblyTool::Place,
        &s,
        &Default::default(),
        None,
        AssemblySelection {
            component: Some(part.body),
            ..Default::default()
        },
        Channel::Ui,
    )
    .unwrap();
    let scene = preview(&mut c, &s);
    assert!(
        scene
            .assembly_evaluation()
            .unwrap()
            .leaves
            .iter()
            .any(|l| l.path
                == OccurrencePath {
                    root: legacy_id,
                    members: vec![member.instance]
                })
    );
    assert_eq!(scene.assembly_evaluation().unwrap().leaves.len(), 3);
    let before = s.document().clone();
    c.cancel();
    assert_eq!(s.document(), &before);
}
fn corrupt_publish(
    document: &Document,
    snapshot: &ModelSnapshot,
    generation: u64,
) -> Result<Arc<Scene>, ModelError> {
    let native = native_fixture::scene(document, snapshot, generation)?;
    let native = Arc::try_unwrap(native).map_err(|_| ModelError {
        entity: None,
        message: "Shared test scene".into(),
    })?;
    let mut leaves = snapshot
        .leaf_occurrences
        .values()
        .map(|leaf| AssemblyLeafReceipt {
            path: leaf.path.clone(),
            component: leaf.component,
            producer: leaf.producer,
            pose: leaf.pose,
        })
        .collect::<Vec<_>>();
    leaves[0].pose.translation_mm[0] += 1.;
    native
        .with_assembly_evaluation(Arc::new(AssemblyEvaluationInfo {
            source_hash: snapshot.stamp.source_hash,
            leaves,
        }))
        .map(Arc::new)
        .map_err(|message| ModelError {
            entity: None,
            message,
        })
}
#[test]
fn native_content_hash_alone_cannot_authorize_an_incorrect_leaf_pose() {
    let (mut s, part, _) = setup();
    let before = s.document().clone();
    let revision = s.revision();
    let mut c = AssemblyController::default();
    c.set_backend(Box::new(NativeBackend {
        runtime: Runtime::with_publisher(Compiler::default(), corrupt_publish).unwrap(),
        stamp: None,
    }));
    c.begin(
        AssemblyTool::Place,
        &s,
        &Default::default(),
        None,
        AssemblySelection {
            component: Some(part.body),
            ..Default::default()
        },
        Channel::Ui,
    )
    .unwrap();
    c.preview(&s, &Default::default(), Channel::Ui).unwrap();
    let until = Instant::now() + Duration::from_secs(20);
    while c.pending() {
        c.poll(&s, &Default::default());
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(!c.can_apply(&s, &Default::default()));
    assert!(
        c.error
            .as_deref()
            .unwrap()
            .contains("component, producer or pose")
    );
    assert!(c.apply(&mut s, &Default::default()).is_err());
    assert_eq!(s.document(), &before);
    assert_eq!(s.revision(), revision);
    c.cancel();
}
