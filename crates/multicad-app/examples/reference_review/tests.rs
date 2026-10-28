use super::backend::*;
use multicad_core::{commands::*, document::*, references::*};
use multicad_ui::{
    application::DocumentStorage,
    viewport::{Ray, SceneFeed, SceneStamp},
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
fn stamp(session: &Session) -> SceneStamp {
    SceneStamp {
        document: session.document().id(),
        revision: session.revision(),
        generation: session.revision() + 1,
    }
}
fn change(session: &mut Session, values: &[(&str, &str)]) {
    let edits = values
        .iter()
        .map(|(name, value)| {
            let mut entity = session
                .document()
                .entities()
                .find(|e| e.name == *name)
                .expect("parameter")
                .clone();
            let EntityData::Parameter { expression, .. } = &mut entity.data else {
                panic!("parameter")
            };
            *expression = (*value).into();
            Mutation::Replace(entity)
        })
        .collect();
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: Actor::local(Channel::Ui),
            label: "Edit review parameters".into(),
            edits,
        })
        .unwrap();
}
fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-5, "{actual} != {expected}");
}
fn evaluated(session: &Session) -> Evaluation {
    evaluate(session.document(), stamp(session), || Ok(())).unwrap()
}
#[test]
fn sketch_on_final_drilled_feature_does_not_block_its_producer_rebuild() {
    use multicad_core::sketch::{self, CreationPlane, PlaneOptions};
    let mut session = Session::new(document().unwrap()).unwrap();
    let initial = evaluated(&session);
    let original = initial.report.entries[0].clone();
    let hole = initial.scene.instances()[0].geometry;
    let mut feed = SceneFeed::default();
    feed.set_backend(Box::new(Backend::new()));
    settle(&mut feed, &session);
    let snapshot = feed.visible().unwrap().geometry_snapshot(hole).unwrap();
    let key = snapshot
        .elements
        .iter()
        .find(|e| {
            e.key.kind == TopologyKind::Face
                && e.origins.contains(&Origin {
                    source: original.source,
                    role: "cap.end".into(),
                })
        })
        .unwrap()
        .key;
    let new = sketch::prepare_new(
        &session,
        0,
        Actor::local(Channel::Ui),
        "Unconsumed final-face sketch",
        CreationPlane::Face {
            snapshot: &snapshot,
            key,
            anchor: None,
        },
        PlaneOptions {
            normal_offset_mm: 2.,
            angle_rad: 0.1,
        },
    )
    .unwrap();
    let sketch_id = new.sketch;
    let plane_id = new.plane;
    session.commit(new.prepared).unwrap();
    settle(&mut feed, &session);
    assert!(feed.is_current(session.document().id(), session.revision()));
    let rebuilt = evaluated(&session);
    assert!(rebuilt.report.all_resolved());
    assert_eq!(rebuilt.report.entries.len(), 2);
    assert_eq!(rebuilt.scene.instances()[0].geometry, hole);
    near(rebuilt.display_volume_mm3, initial.display_volume_mm3);
    assert!(
        !session
            .document()
            .affected([sketch_id])
            .unwrap()
            .contains(&hole)
    );
    change(
        &mut session,
        &[("panel_height", "24"), ("panel_width", "120")],
    );
    settle(&mut feed, &session);
    assert!(feed.is_current(session.document().id(), session.revision()));
    let resized = evaluated(&session);
    assert!(resized.report.all_resolved());
    let (mut source, frame, _) = multicad_core::constraint_sketch::source::Source::read_evaluated(
        session.document(),
        sketch_id,
        session.revision(),
        &resized.report,
    )
    .unwrap();
    near(frame.origin[2], 26.);
    source.add_rectangle([10., 10.], [30., 30.]).unwrap();
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: Actor::local(Channel::Ui),
            label: "Edit unconsumed final-face sketch".into(),
            edits: source.replace(session.document(), sketch_id).unwrap(),
        })
        .unwrap();
    settle(&mut feed, &session);
    assert!(feed.is_current(session.document().id(), session.revision()));
    let changed = evaluated(&session);
    assert!(changed.report.all_resolved());
    near(changed.display_volume_mm3, resized.display_volume_mm3);
    assert_eq!(changed.scene.instances()[0].geometry, hole);
    let visible = feed.visible().unwrap().clone();
    change(&mut session, &[("topology_mode", "1")]);
    settle(&mut feed, &session);
    assert!(!feed.is_current(session.document().id(), session.revision()));
    assert!(Arc::ptr_eq(feed.visible().unwrap(), &visible));
    let references = feed.references().unwrap();
    assert!(matches!(
        references.report.get(original.address).unwrap().resolution,
        Resolution::Ambiguous { .. }
    ));
    assert!(
        references
            .report
            .entries
            .iter()
            .any(|d| d.address.owner == plane_id && d.resolution == Resolution::MissingSource)
    );
    assert!(!references.candidate.geometries().contains_key(&hole));
    assert_eq!(
        references.candidate.instances()[0].geometry,
        original.source
    );
}
#[test]
fn malformed_drilling_source_is_an_error_instead_of_a_published_raw_panel() {
    let mut document = document().unwrap();
    let mut sketch = document
        .entities()
        .find(|e| e.name == "Face-mounted drill circle")
        .unwrap()
        .clone();
    let EntityData::Sketch { definition, .. } = &mut sketch.data else {
        unreachable!()
    };
    definition.type_id = "test.unsupported_sketch_provider".into();
    document.replace(sketch).unwrap();
    assert!(
        evaluate(
            &document,
            SceneStamp {
                document: document.id(),
                revision: 0,
                generation: 1
            },
            || Ok(())
        )
        .is_err()
    );
}
#[test]
fn duplicate_instance_transaction_shares_native_assets_and_renders_all_placements() {
    let mut session = Session::new(document().unwrap()).unwrap();
    let before = session.document().clone();
    let original = session
        .document()
        .entities()
        .find(|e| matches!(e.data, EntityData::Instance { .. }))
        .unwrap()
        .clone();
    let EntityData::Instance { component, .. } = original.data else {
        unreachable!()
    };
    let second = Entity::new(
        "Second panel",
        EntityData::Instance {
            component,
            placement: Placement {
                translation_mm: [140., 0., 0.],
                ..Default::default()
            },
            placement_source: None,
        },
    );
    let third = Entity::new(
        "Third rotated panel",
        EntityData::Instance {
            component,
            placement: Placement {
                translation_mm: [0., 100., 30.],
                rotation_xyzw: [
                    0.,
                    0.,
                    std::f64::consts::FRAC_1_SQRT_2,
                    std::f64::consts::FRAC_1_SQRT_2,
                ],
            },
            placement_source: None,
        },
    );
    session
        .apply(Request {
            expected_revision: 0,
            actor: Actor::local(Channel::Ui),
            label: "Duplicate panel instances".into(),
            edits: vec![
                Mutation::Insert(second.clone()),
                Mutation::Insert(third.clone()),
            ],
        })
        .unwrap();
    let result = evaluated(&session);
    assert_eq!(result.scene.instances().len(), 3);
    assert_eq!(result.scene.geometries().len(), 2); // Raw source and one shared drilled asset.
    let ids: std::collections::BTreeSet<_> =
        result.scene.instances().iter().map(|i| i.id).collect();
    assert_eq!(
        ids,
        std::collections::BTreeSet::from([original.id, second.id, third.id])
    );
    let asset = result.scene.instances()[0].geometry;
    assert!(result.scene.instances().iter().all(|i| i.geometry == asset));
    let snapshot = result.scene.reference_snapshots();
    for (id, point) in [
        (original.id, [10., 10., 18.]),
        (second.id, [150., 10., 18.]),
        (third.id, [-10., 110., 48.]),
    ] {
        assert_eq!(snapshot[&id].owner, id);
        let hit = result
            .scene
            .pick_face(Ray::new([point[0], point[1], 100.], [0., 0., -1.]).unwrap())
            .unwrap();
        assert_eq!(hit.instance, id);
        for (actual, expected) in hit.point_mm.into_iter().zip(point) {
            near(actual, expected);
        }
    }
    for center in [[20., 25., 100.], [160., 25., 100.], [-25., 120., 100.]] {
        assert!(
            result
                .scene
                .pick_face(Ray::new(center, [0., 0., -1.]).unwrap())
                .is_none()
        );
    }
    session.undo(1, Actor::local(Channel::Ui)).unwrap();
    assert_eq!(session.document(), &before);
    assert_eq!(evaluated(&session).scene.instances().len(), 1);
}
#[test]
fn unsupported_instance_components_fail_instead_of_disappearing_from_the_fixture() {
    let mut document = document().unwrap();
    let other = Entity::new(
        "Unsupported empty Part",
        EntityData::Part {
            features: vec![],
            material: None,
            tags: Default::default(),
        },
    );
    let instance = Entity::new(
        "Unsupported component instance",
        EntityData::Instance {
            component: other.id,
            placement: Default::default(),
            placement_source: None,
        },
    );
    document.insert(other).unwrap();
    document.insert(instance).unwrap();
    let result = evaluate(
        &document,
        SceneStamp {
            document: document.id(),
            revision: 0,
            generation: 1,
        },
        || Ok(()),
    );
    assert!(matches!(result,Err(error) if error.contains("unsupported component")));
}
#[test]
fn placed_instance_keeps_raw_producer_frames_and_exposes_true_world_attachment_and_hole() {
    let mut session = Session::new(document().unwrap()).unwrap();
    let mut instance = session
        .document()
        .entities()
        .find(|e| matches!(e.data, EntityData::Instance { .. }))
        .unwrap()
        .clone();
    let EntityData::Instance { placement, .. } = &mut instance.data else {
        unreachable!()
    };
    *placement = Placement {
        translation_mm: [10., 20., 30.],
        rotation_xyzw: [
            0.,
            0.,
            std::f64::consts::FRAC_1_SQRT_2,
            std::f64::consts::FRAC_1_SQRT_2,
        ],
    };
    let instance_id = instance.id;
    session
        .apply(Request {
            expected_revision: 0,
            actor: Actor::local(Channel::Ui),
            label: "Place review instance".into(),
            edits: vec![Mutation::Replace(instance)],
        })
        .unwrap();
    let result = evaluated(&session);
    assert!(result.report.all_resolved());
    let diagnostic = &result.report.entries[0];
    let snapshots = result.scene.reference_snapshots();
    let source = &snapshots[&diagnostic.source];
    let world = &snapshots[&instance_id];
    let top = |snapshot: &TopologySnapshot| {
        snapshot
            .elements
            .iter()
            .find(|e| {
                e.key.kind == TopologyKind::Face
                    && e.origins.contains(&Origin {
                        source: diagnostic.source,
                        role: "cap.end".into(),
                    })
            })
            .unwrap()
            .key
    };
    let mut intent = GeometryIntent::capture(source, top(source)).unwrap();
    intent.attachment = Some(Attachment {
        anchor: Origin {
            source: diagnostic.source,
            role: "cap.end".into(),
        },
        offset_mm: [20., 25., 0.],
        angle_rad: 0.,
    });
    let Resolution::Resolved {
        frame: Some(raw), ..
    } = resolve(&intent, source, session.revision())
    else {
        panic!("source frame")
    };
    assert_eq!(raw.origin_mm, [20., 25., 18.]);
    let Resolution::Resolved {
        frame: Some(placed),
        ..
    } = resolve(&intent, world, session.revision())
    else {
        panic!("placed frame")
    };
    for (actual, expected) in placed.origin_mm.into_iter().zip([-15., 40., 48.]) {
        near(actual, expected);
    }
    for (actual, expected) in placed.u.into_iter().zip([0., 1., 0.]) {
        near(actual, expected);
    }
    // The genuine drilled void moves with the same rigid instance transform.
    assert!(
        result
            .scene
            .pick_face(Ray::new([-15., 40., 100.], [0., 0., -1.]).unwrap())
            .is_none()
    );
    let cap = result
        .scene
        .pick_face(Ray::new([0., 30., 100.], [0., 0., -1.]).unwrap())
        .unwrap();
    for (actual, expected) in cap.point_mm.into_iter().zip([0., 30., 48.]) {
        near(actual, expected);
    }
    let isolated = result.scene.isolated_source(instance_id).unwrap();
    let hit = isolated
        .pick_face(Ray::new([0., 30., 100.], [0., 0., -1.]).unwrap())
        .unwrap();
    near(hit.point_mm[2], 48.);
}
#[test]
fn face_drill_follows_dimensions_split_repair_is_explicit_and_save_load_preserves_it() {
    let mut session = Session::new(document().unwrap()).unwrap();
    let initial = evaluated(&session);
    assert!(initial.report.all_resolved());
    near(initial.source_volume_mm3, 100. * 60. * 18.);
    near(
        initial.display_volume_mm3,
        100. * 60. * 18. - 9. * std::f64::consts::PI * 18.,
    );
    assert_eq!(initial.hole_center_world, Some([20., 25., 18.]));
    let diagnostic = initial.report.entries.first().unwrap();
    let address = diagnostic.address;
    let panel = diagnostic.source;
    let original_reference = diagnostic.intent.id;
    let mut gate = ReferenceGate::default();
    gate.consider(0, initial.scene.clone(), initial.report)
        .unwrap();

    change(
        &mut session,
        &[("panel_width", "120"), ("panel_height", "24")],
    );
    let resized = evaluated(&session);
    assert!(resized.report.all_resolved());
    assert_eq!(resized.hole_center_world, Some([20., 25., 24.]));
    near(
        resized.display_volume_mm3,
        120. * 60. * 24. - 9. * std::f64::consts::PI * 24.,
    );
    gate.consider(session.revision(), resized.scene.clone(), resized.report)
        .unwrap();
    let last_valid = gate.visible().unwrap().clone();

    change(&mut session, &[("topology_mode", "1")]);
    let broken = evaluated(&session);
    let Resolution::Ambiguous { candidates } = &broken.report.get(address).unwrap().resolution
    else {
        panic!("Cut must split the source top")
    };
    assert_eq!(candidates.len(), 2);
    assert!(broken.hole_center_world.is_none());
    assert_eq!(broken.display_volume_mm3, broken.source_volume_mm3);
    assert!(
        !gate
            .consider(
                session.revision(),
                broken.scene.clone(),
                broken.report.clone()
            )
            .unwrap()
    );
    assert!(Arc::ptr_eq(gate.visible().unwrap(), &last_valid));
    let snapshot = broken.scene.reference_snapshots().remove(&panel).unwrap();
    let candidate = broken
        .scene
        .isolated_source(panel)
        .unwrap()
        .pick_face(Ray::new([20., 25., 100.], [0., 0., -1.]).unwrap())
        .unwrap()
        .key;
    assert!(candidates.contains(&candidate));
    let before = session.document().clone();
    let preview = prepare_repair(
        &session,
        session.revision(),
        Actor::local(Channel::Ui),
        address,
        &snapshot,
        candidate,
        None,
    )
    .unwrap();
    assert_eq!(session.document(), &before);
    assert!(preview.affected.iter().any(|id| {
        session
            .document()
            .get(*id)
            .is_some_and(|e| e.name == "Dependent through drilling")
    }));
    drop(preview);
    assert_eq!(session.document(), &before);
    let preview = prepare_repair(
        &session,
        session.revision(),
        Actor::local(Channel::Ui),
        address,
        &snapshot,
        candidate,
        None,
    )
    .unwrap();
    session.commit(preview.prepared).unwrap();
    let after = session.document().clone();
    let repaired = evaluated(&session);
    assert!(repaired.report.all_resolved());
    assert_eq!(
        repaired.report.get(address).unwrap().intent.id,
        original_reference
    );
    assert_eq!(repaired.hole_center_world, Some([20., 25., 24.]));
    near(
        repaired.display_volume_mm3,
        110. * 60. * 24. - 9. * std::f64::consts::PI * 24.,
    );
    session
        .undo(session.revision(), Actor::local(Channel::Ui))
        .unwrap();
    assert_eq!(session.document(), &before);
    assert!(!evaluated(&session).report.all_resolved());
    session
        .redo(session.revision(), Actor::local(Channel::Ui))
        .unwrap();
    assert_eq!(session.document(), &after);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("repaired.multicad.json");
    let mut storage = crate::files::SnapshotStore;
    storage.save(&path, session.document()).unwrap();
    let reopened = Session::new(storage.load(&path).unwrap()).unwrap();
    let restored = evaluated(&reopened);
    assert!(restored.report.all_resolved());
    assert_eq!(
        restored.report.get(address).unwrap().intent,
        repaired.report.get(address).unwrap().intent
    );
    near(restored.display_volume_mm3, repaired.display_volume_mm3);
}
#[test]
fn removed_top_is_unresolved_after_reload_and_never_uses_saved_plane_fallback() {
    let mut session = Session::new(document().unwrap()).unwrap();
    let initial = evaluated(&session);
    let address = initial.report.entries[0].address;
    change(&mut session, &[("topology_mode", "2")]);
    let result = evaluated(&session);
    assert_eq!(
        result.report.get(address).unwrap().resolution,
        Resolution::Missing
    );
    assert!(result.hole_center_world.is_none());
    near(result.source_volume_mm3, 100. * 60. * 9.);
    let bytes = serde_json::to_vec(session.document()).unwrap();
    let reloaded: Document = serde_json::from_slice(&bytes).unwrap();
    let result = evaluate(
        &reloaded,
        SceneStamp {
            document: reloaded.id(),
            revision: 1,
            generation: 2,
        },
        || Ok(()),
    )
    .unwrap();
    assert_eq!(
        result.report.get(address).unwrap().resolution,
        Resolution::Missing
    );
    let sketch = reloaded
        .entities()
        .find(|e| matches!(e.data, EntityData::Sketch { .. }))
        .unwrap();
    assert!(
        multicad_core::constraint_sketch::source::Source::read_evaluated(
            &reloaded,
            sketch.id,
            1,
            &result.report
        )
        .is_err()
    );
}
fn settle(feed: &mut SceneFeed, session: &Session) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        feed.tick(session.document(), session.revision());
        if !feed.pending() {
            assert!(
                feed.error().is_none()
                    || feed.references().is_some_and(
                        |e| e.stamp.revision == session.revision() && !e.report.all_resolved()
                    ),
                "{:?}",
                feed.error()
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Native reference fixture timed out"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn async_native_backend_publishes_through_live_reference_gate_and_recovers_after_repair() {
    let mut session = Session::new(document().unwrap()).unwrap();
    let mut feed = SceneFeed::default();
    feed.set_backend(Box::new(Backend::new()));
    settle(&mut feed, &session);
    assert!(feed.is_current(session.document().id(), session.revision()));
    let original = feed.visible().unwrap().clone();
    change(&mut session, &[("topology_mode", "1")]);
    settle(&mut feed, &session);
    assert!(!feed.is_current(session.document().id(), session.revision()));
    assert!(Arc::ptr_eq(feed.visible().unwrap(), &original));
    let evaluation = feed.references().unwrap();
    let diagnostic = &evaluation.report.entries[0];
    let snapshot = &evaluation.snapshots[&diagnostic.source];
    let candidate = evaluation
        .candidate
        .isolated_source(diagnostic.source)
        .unwrap()
        .pick_face(Ray::new([20., 25., 100.], [0., 0., -1.]).unwrap())
        .unwrap()
        .key;
    let preview = prepare_repair(
        &session,
        session.revision(),
        Actor::local(Channel::Mcp),
        diagnostic.address,
        snapshot,
        candidate,
        None,
    )
    .unwrap();
    session.commit(preview.prepared).unwrap();
    settle(&mut feed, &session);
    assert!(feed.is_current(session.document().id(), session.revision()));
    assert!(!Arc::ptr_eq(feed.visible().unwrap(), &original));
    assert!(feed.references().unwrap().report.all_resolved());
}
