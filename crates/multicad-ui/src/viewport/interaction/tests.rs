use super::*;
use crate::viewport::{Geometry, Instance, Ray, Transform};
use multicad_core::document::Definition;
use multicad_kernel::{MeshOptions, Solid};

fn fixture() -> (Session, Arc<Scene>, Vec<EntityId>) {
    let source = Entity::new(
        "Box source",
        EntityData::Feature {
            definition: Definition {
                type_id: "test.native-box".into(),
                schema_version: 1,
                data: serde_json::json!({}),
            },
            inputs: vec![],
            suppressed: false,
        },
    );
    let part = Entity::new(
        "Shared box",
        EntityData::Part {
            features: vec![source.id],
            material: None,
            tags: BTreeSet::from(["test-parts".into()]),
        },
    );
    let mut edits = vec![
        Mutation::Insert(source.clone()),
        Mutation::Insert(part.clone()),
    ];
    let mut ids = vec![];
    let mut instances = vec![];
    for i in 0..5 {
        let placement = Placement {
            translation_mm: [i as f64 * 30., 0., 0.],
            ..Default::default()
        };
        let entity = Entity::new(
            format!("Box {:02}", i + 1),
            EntityData::Instance {
                component: part.id,
                placement,
                placement_source: None,
            },
        );
        ids.push(entity.id);
        instances.push(Instance {
            id: entity.id,
            geometry: source.id,
            transform: Transform::from_placement(placement).unwrap(),
            color: [140, 160, 180, 255],
        });
        edits.push(Mutation::Insert(entity));
    }
    let doc = Document::new("Selection test")
        .unwrap()
        .edited(&edits)
        .unwrap();
    let solid = Solid::box_named([20., 12., 8.], source.id, Default::default()).unwrap();
    let mesh = solid.mesh(MeshOptions::default()).unwrap();
    let geometry = Arc::new(
        Geometry::new(
            source.id,
            mesh.vertices.as_chunks::<3>().0.to_vec(),
            mesh.indices,
            mesh.face_indices,
            solid.topology_snapshot(source.id, 0).unwrap().elements,
        )
        .unwrap(),
    );
    let scene = Arc::new(
        Scene::new(
            SceneStamp {
                document: doc.id(),
                revision: 0,
                generation: 1,
            },
            vec![geometry],
            instances,
        )
        .unwrap(),
    );
    (Session::new(doc).unwrap(), scene, ids)
}
fn tree_frame(
    ctx: &egui::Context,
    interaction: &mut Interaction,
    document: &Document,
    scene: &Scene,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    ctx.run_ui(
        egui::RawInput {
            focused: true,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(260., 420.))),
            events,
            ..Default::default()
        },
        |ui| {
            interaction.tree(ui, document, scene);
        },
    )
}
#[test]
fn tree_modifier_selection_toggles_and_ranges_without_editing_source() {
    let (session, scene, ids) = fixture();
    let before = session.document().clone();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut interaction = Interaction::default();
    tree_frame(&ctx, &mut interaction, session.document(), &scene, vec![])
        .drop_without_applying_deltas();
    let output = tree_frame(&ctx, &mut interaction, session.document(), &scene, vec![]);
    let tree = output.platform_output.accesskit_update.as_ref().unwrap();
    let points: Vec<_> = (1..=5)
        .map(|i| {
            let name = format!("Box {i:02}");
            let bounds = tree
                .nodes
                .iter()
                .find(|(_, node)| node.label() == Some(&name))
                .and_then(|(_, node)| node.bounds())
                .unwrap();
            Pos2::new(
                ((bounds.x0 + bounds.x1) * 0.5) as f32,
                ((bounds.y0 + bounds.y1) * 0.5) as f32,
            )
        })
        .collect();
    output.drop_without_applying_deltas();
    let mut click = |index, modifiers| {
        for pressed in [true, false] {
            let p = points[index];
            tree_frame(
                &ctx,
                &mut interaction,
                session.document(),
                &scene,
                vec![
                    egui::Event::ModifiersChanged(modifiers),
                    egui::Event::PointerMoved(p),
                    egui::Event::PointerButton {
                        pos: p,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers,
                    },
                    egui::Event::ModifiersChanged(egui::Modifiers::NONE),
                ],
            )
            .drop_without_applying_deltas();
        }
        interaction.selected_ids()
    };
    assert_eq!(click(0, egui::Modifiers::NONE), BTreeSet::from([ids[0]]));
    assert_eq!(
        click(2, egui::Modifiers::COMMAND),
        BTreeSet::from([ids[0], ids[2]])
    );
    assert_eq!(
        click(4, egui::Modifiers::SHIFT),
        BTreeSet::from([ids[2], ids[3], ids[4]])
    );
    assert_eq!(
        click(3, egui::Modifiers::COMMAND),
        BTreeSet::from([ids[2], ids[4]])
    );
    assert_eq!(
        interaction.selected_bounds(&scene),
        Some([[60., 0., 0.], [140., 12., 8.]])
    );
    assert_eq!(session.document(), &before);
}
#[test]
fn viewport_modifier_click_adds_parts_and_scene_refresh_retains_only_existing_ids() {
    let (session, scene, ids) = fixture();
    let ctx = egui::Context::default();
    let mut interaction = Interaction::default();
    let camera = Camera {
        yaw: 0.,
        pitch: std::f32::consts::FRAC_PI_2,
        scale: 2.,
        target: [70., 6., 8.],
        perspective: false,
    };
    let mut frame = |scene: Arc<Scene>, events| {
        ctx.run_ui(
            egui::RawInput {
                focused: true,
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(600., 400.))),
                events,
                ..Default::default()
            },
            |ui| {
                let (_, response) =
                    ui.allocate_exact_size(egui::vec2(600., 400.), egui::Sense::click());
                interaction.show(
                    ui,
                    &response,
                    scene.clone(),
                    camera,
                    DisplayStyle::Filled,
                    true,
                );
                if response.clicked() {
                    assert!(interaction.hovered_instance().is_some());
                    let click_modifiers = crate::selection_input::click_modifiers(ui, &response);
                    if click_modifiers.shift {
                        assert_eq!(interaction.hovered_instance(), interaction.range_anchor);
                        assert!(!interaction.has_topology_selection());
                    } else {
                        assert!(interaction.has_topology_selection());
                    }
                }
            },
        )
        .drop_without_applying_deltas();
        interaction.selected_ids()
    };
    frame(scene.clone(), vec![]);
    // run_ui's root inset is zero; the camera uses the allocated viewport bounds.
    let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(600., 400.));
    // Shift adds parts; Command on a face toggles faces instead (CAD-152).
    for (index, modifiers) in [(0, egui::Modifiers::NONE), (2, egui::Modifiers::SHIFT)] {
        let p = camera.project([index as f32 * 30. + 10., 6., 8.], rect);
        let p = Pos2::new(p[0], p[1]);
        for pressed in [true, false] {
            let selected = frame(
                scene.clone(),
                vec![
                    egui::Event::ModifiersChanged(modifiers),
                    egui::Event::PointerMoved(p),
                    egui::Event::PointerButton {
                        pos: p,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers,
                    },
                    egui::Event::ModifiersChanged(egui::Modifiers::NONE),
                ],
            );
            if !pressed {
                assert_eq!(
                    selected,
                    if index == 0 {
                        BTreeSet::from([ids[0]])
                    } else {
                        BTreeSet::from([ids[0], ids[2]])
                    }
                );
            }
        }
    }
    let remaining = Arc::new(
        Scene::new(
            SceneStamp {
                generation: 2,
                ..scene.stamp()
            },
            scene.geometries().values().cloned().collect(),
            vec![scene.instances()[2].clone()],
        )
        .unwrap(),
    );
    assert_eq!(frame(remaining.clone(), vec![]), BTreeSet::from([ids[2]]));
    let other = Arc::new(
        Scene::new(
            SceneStamp {
                document: EntityId::new(),
                generation: 3,
                ..scene.stamp()
            },
            scene.geometries().values().cloned().collect(),
            remaining.instances().to_vec(),
        )
        .unwrap(),
    );
    assert!(frame(other, vec![]).is_empty());
    assert_eq!(session.revision(), 0);
}
fn viewer_frame(
    ctx: &egui::Context,
    interaction: &mut Interaction,
    scene: Arc<Scene>,
    camera: Camera,
    events: Vec<egui::Event>,
) {
    ctx.run_ui(
        egui::RawInput {
            focused: true,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(600., 400.))),
            events,
            ..Default::default()
        },
        |ui| {
            let (_, response) =
                ui.allocate_exact_size(egui::vec2(600., 400.), egui::Sense::click());
            interaction.show(
                ui,
                &response,
                scene.clone(),
                camera,
                DisplayStyle::Filled,
                true,
            );
        },
    )
    .drop_without_applying_deltas();
}
fn viewer_click(
    ctx: &egui::Context,
    interaction: &mut Interaction,
    scene: &Arc<Scene>,
    camera: Camera,
    p: Pos2,
    modifiers: egui::Modifiers,
) {
    for pressed in [true, false] {
        viewer_frame(
            ctx,
            interaction,
            scene.clone(),
            camera,
            vec![
                egui::Event::ModifiersChanged(modifiers),
                egui::Event::PointerMoved(p),
                egui::Event::PointerButton {
                    pos: p,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers,
                },
                egui::Event::ModifiersChanged(egui::Modifiers::NONE),
            ],
        );
    }
}
#[test]
fn command_click_toggles_faces_plain_click_replaces_and_stale_faces_are_dropped() {
    let (session, scene, ids) = fixture();
    let before = session.document().clone();
    let ctx = egui::Context::default();
    let mut interaction = Interaction::default();
    let camera = Camera {
        yaw: 0.,
        pitch: std::f32::consts::FRAC_PI_2,
        scale: 2.,
        target: [70., 6., 8.],
        perspective: false,
    };
    let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(600., 400.));
    let top = |index: usize| {
        let p = camera.project([index as f32 * 30. + 10., 6., 8.], rect);
        Pos2::new(p[0], p[1])
    };
    let faces = |interaction: &Interaction| -> Vec<EntityId> {
        interaction
            .selected_faces()
            .iter()
            .inspect(|hit| assert_eq!(hit.key.kind, TopologyKind::Face))
            .map(|hit| hit.instance)
            .collect()
    };
    viewer_frame(&ctx, &mut interaction, scene.clone(), camera, vec![]);
    let command = egui::Modifiers::COMMAND;
    let none = egui::Modifiers::NONE;
    // A plain click selects one face; Command-click adds a face of another body.
    viewer_click(&ctx, &mut interaction, &scene, camera, top(0), none);
    assert_eq!(faces(&interaction), vec![ids[0]]);
    viewer_click(&ctx, &mut interaction, &scene, camera, top(2), command);
    assert_eq!(faces(&interaction), vec![ids[0], ids[2]]);
    assert_eq!(interaction.selected_ids(), BTreeSet::from([ids[0], ids[2]]));
    assert!(interaction.has_topology_selection());
    // The primary single-hit view is the most recently added face.
    assert_eq!(interaction.selected_hit().unwrap().instance, ids[2]);
    // Command-click on a selected face removes it; again adds it back.
    viewer_click(&ctx, &mut interaction, &scene, camera, top(2), command);
    assert_eq!(faces(&interaction), vec![ids[0]]);
    viewer_click(&ctx, &mut interaction, &scene, camera, top(2), command);
    viewer_click(&ctx, &mut interaction, &scene, camera, top(3), command);
    assert_eq!(faces(&interaction), vec![ids[0], ids[2], ids[3]]);
    // Display refinement of the same revision keeps faces that still exist.
    let refined = Arc::new(
        Scene::new(
            SceneStamp {
                generation: 2,
                ..scene.stamp()
            },
            scene.geometries().values().cloned().collect(),
            scene
                .instances()
                .iter()
                .filter(|i| i.id != ids[3])
                .cloned()
                .collect(),
        )
        .unwrap(),
    );
    viewer_frame(&ctx, &mut interaction, refined.clone(), camera, vec![]);
    assert_eq!(faces(&interaction), vec![ids[0], ids[2]]);
    assert!(
        interaction
            .selected_faces()
            .iter()
            .all(|hit| hit.stamp == refined.stamp())
    );
    // A plain click replaces the set with one face.
    viewer_click(&ctx, &mut interaction, &refined, camera, top(4), none);
    assert_eq!(faces(&interaction), vec![ids[4]]);
    // Shift-click still builds a part set and drops the face set.
    viewer_click(&ctx, &mut interaction, &refined, camera, top(0), command);
    viewer_click(
        &ctx,
        &mut interaction,
        &refined,
        camera,
        top(1),
        egui::Modifiers::SHIFT,
    );
    assert!(interaction.selected_faces().is_empty());
    assert_eq!(
        interaction.selected_ids(),
        BTreeSet::from([ids[0], ids[1], ids[4]])
    );
    // A new source revision clears faces that cannot be revalidated.
    viewer_click(&ctx, &mut interaction, &refined, camera, top(0), none);
    viewer_click(&ctx, &mut interaction, &refined, camera, top(2), command);
    assert_eq!(faces(&interaction).len(), 2);
    let edited = Arc::new(
        Scene::new(
            SceneStamp {
                revision: 1,
                generation: 3,
                ..scene.stamp()
            },
            scene.geometries().values().cloned().collect(),
            scene.instances().to_vec(),
        )
        .unwrap(),
    );
    viewer_frame(&ctx, &mut interaction, edited, camera, vec![]);
    assert!(interaction.selected_faces().is_empty());
    // Escape reaches the host as clear().
    viewer_click(&ctx, &mut interaction, &scene, camera, top(0), none);
    interaction.clear();
    assert!(interaction.selected_faces().is_empty());
    assert!(!interaction.has_topology_selection());
    assert_eq!(session.document(), &before);
}
#[test]
fn linked_duplication_is_one_undoable_transaction_and_rejects_stale_selection() {
    let (mut session, scene, ids) = fixture();
    let before = session.document().clone();
    let mut interaction = Interaction {
        last_stamp: Some(scene.stamp()),
        ..Default::default()
    };
    interaction.select_part(ids[0], false);
    interaction.select_part(ids[2], true);
    interaction.duplicate_selected(&mut session).unwrap();
    let created = interaction.selected_ids();
    assert_eq!(created.len(), 2);
    assert!(created.is_disjoint(&ids.iter().copied().collect()));
    assert_eq!(session.revision(), 1);
    for id in &created {
        let copy = session.document().get(*id).unwrap();
        assert!(
            ids.iter()
                .any(|id| session.document().get(*id).unwrap().data == copy.data)
        );
    }
    assert_eq!(
        session.activity().last().unwrap().actor.channel,
        Channel::Ui
    );
    let after = session.document().clone();
    assert!(interaction.duplicate_selected(&mut session).is_err());
    assert_eq!(session.document(), &after);
    session.undo(1, Actor::local(Channel::Ui)).unwrap();
    assert_eq!(session.document(), &before);
    session.redo(2, Actor::local(Channel::Ui)).unwrap();
    assert_eq!(session.document(), &after);
    // Named topology and manufacturing source were never copied or altered.
    assert!(
        scene
            .pick_face(Ray::new([10., 6., 100.], [0., 0., -1.]).unwrap())
            .unwrap()
            .intent
            .is_some()
    );
}

#[test]
fn profile_connection_hardware_presentation_keeps_only_selected_world_poses() {
    let (_, scene, ids) = fixture();
    let visible = BTreeSet::from([ids[1], ids[3]]);
    let view = scene.presentation_subset(&visible).unwrap();
    assert_eq!(view.scope(), &crate::viewport::SceneScope::Presentation);
    assert_eq!(view.stamp(), scene.stamp());
    assert_eq!(
        view.instances()
            .iter()
            .map(|item| item.id)
            .collect::<BTreeSet<_>>(),
        visible
    );
    for id in visible {
        assert_eq!(view.instance_bounds(id), scene.instance_bounds(id));
    }
    assert_eq!(scene.instances().len(), ids.len());
    let ray = Ray::new([40., 6., 100.], [0., 0., -1.]).unwrap();
    assert!(scene.pick_face(ray).unwrap().intent.is_some());
    assert!(view.pick_face(ray).is_none());
    assert!(
        scene
            .presentation_subset(&BTreeSet::from([EntityId::new()]))
            .is_err()
    );
}

#[test]
fn canonical_tree_selection_maps_definition_occurrences_without_mutating_source() {
    let (session, scene, ids) = fixture();
    let part = session
        .document()
        .entities()
        .find(|entity| matches!(entity.data, EntityData::Part { .. }))
        .unwrap()
        .id;
    let before = session.document().clone();
    let mut interaction = Interaction::default();
    interaction
        .select_source_entities(session.document(), 0, &scene, &BTreeSet::from([part]))
        .unwrap();
    assert_eq!(interaction.selected_ids(), ids.iter().copied().collect());
    interaction
        .select_source_entities(session.document(), 0, &scene, &BTreeSet::from([ids[1]]))
        .unwrap();
    assert_eq!(interaction.selected_ids(), BTreeSet::from([ids[1]]));
    let visible = scene.visible_subset(&BTreeSet::from([ids[0]])).unwrap();
    interaction
        .select_source_entities(session.document(), 0, &visible, &BTreeSet::from([ids[1]]))
        .unwrap();
    assert!(interaction.selected_ids().is_empty());
    interaction
        .select_source_entities(
            session.document(),
            0,
            &visible,
            &BTreeSet::from([session.document().id()]),
        )
        .unwrap();
    assert!(interaction.selected_ids().is_empty());
    assert!(
        interaction
            .select_source_entities(session.document(), 1, &scene, &BTreeSet::from([part]))
            .is_err()
    );
    assert!(
        interaction
            .select_source_entities(
                session.document(),
                0,
                &scene,
                &BTreeSet::from([EntityId::new()])
            )
            .is_err()
    );
    assert_eq!(session.document(), &before);
    assert_eq!(session.revision(), 0);
}

#[test]
fn topology_and_hover_accessors_distinguish_face_hits_from_body_sets() {
    let (_, scene, ids) = fixture();
    let mut interaction = Interaction::default();
    assert!(!interaction.has_topology_selection());
    assert_eq!(interaction.hovered_instance(), None);
    let hit = scene
        .pick_face(Ray::new([10., 6., 100.], [0., 0., -1.]).unwrap())
        .unwrap();
    interaction.selected = Some(hit.clone());
    interaction.hovered = Some(hit);
    assert!(interaction.has_topology_selection());
    assert_eq!(interaction.hovered_instance(), Some(ids[0]));
    interaction.clear();
    assert!(!interaction.has_topology_selection());
    assert_eq!(interaction.hovered_instance(), None);
}

#[test]
fn duplication_of_a_driven_instance_clones_pose_ownership_instead_of_sharing_a_driver() {
    use multicad_core::model::{
        self, InstanceCopyRequest, InstanceCopySource, NumericTransform, length,
    };
    let (mut session, scene, ids) = fixture();
    let request = InstanceCopyRequest::new(
        InstanceCopySource::Instance { instance: ids[0] },
        NumericTransform {
            translation: [length(10.), length(5.), length(0.)],
            ..Default::default()
        },
    );
    let prepared = model::prepare_instance_copies(
        &session,
        0,
        Actor::local(Channel::Ui),
        std::slice::from_ref(&request),
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
    let before = session.document().clone();
    let mut interaction = Interaction {
        last_stamp: Some(SceneStamp {
            revision: session.revision(),
            ..scene.stamp()
        }),
        ..Default::default()
    };
    interaction.select_part(request.instance, false);
    interaction.duplicate_selected(&mut session).unwrap();
    let created = *interaction.selected_ids().first().unwrap();
    let EntityData::Instance {
        placement_source: Some(driver),
        ..
    } = session.document().get(created).unwrap().data
    else {
        panic!()
    };
    assert_ne!(driver, request.driver);
    assert_eq!(
        model::read_instance_placement(session.document(), driver)
            .unwrap()
            .owner,
        created
    );
    assert_eq!(
        model::resolve_instance_pose(session.document(), created, session.parameters()).unwrap(),
        model::resolve_instance_pose(session.document(), request.instance, session.parameters())
            .unwrap()
    );
    session
        .undo(session.revision(), Actor::local(Channel::Ui))
        .unwrap();
    assert_eq!(session.document(), &before);
}

#[test]
fn host_section_override_is_used_and_isolated_source_can_explicitly_disable_it() {
    let mut interaction = Interaction::default();
    let plane = SectionPlane::new([1., 0., 0.], 10.).unwrap();
    interaction.set_section_override(Some(plane));
    let effective = interaction.effective_section(false).unwrap();
    assert!(!effective.contains([9., 0., 0.]));
    assert!(effective.contains([11., 0., 0.]));
    assert_eq!(interaction.effective_section(false), Some(plane));
    assert!(
        interaction.effective_section(true).is_none(),
        "Derived plan surfaces remain unsectioned"
    );
    interaction.set_section_override(None);
    assert!(
        interaction.effective_section(false).is_none(),
        "Frozen predecessor picking must not inherit a host clip"
    );
}
