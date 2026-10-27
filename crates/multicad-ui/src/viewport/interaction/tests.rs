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
            },
        )
        .drop_without_applying_deltas();
        interaction.selected_ids()
    };
    frame(scene.clone(), vec![]);
    // run_ui's root inset is zero; the camera uses the allocated viewport bounds.
    let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(600., 400.));
    for (index, modifiers) in [(0, egui::Modifiers::NONE), (2, egui::Modifiers::COMMAND)] {
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
