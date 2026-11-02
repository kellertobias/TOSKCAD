use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Definition, Document, Entity, EntityData, EntityId, Link, Mutation},
    model::*,
    parameters, project_file,
};
use std::collections::BTreeMap;

fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn fixture() -> (Session, FenceTypeRequest, EntityId) {
    let material = |name: &str, color_rgba| {
        Entity::new(
            name,
            EntityData::Material {
                density_kg_m3: None,
                color_rgba,
                section_hatch: None,
            },
        )
    };
    let wood = material("Timber", [150, 110, 70, 255]);
    let metal = material("Metal", [100, 110, 120, 255]);
    let document = Document::new("Fence source transactions")
        .unwrap()
        .edited(&[
            Mutation::Insert(wood.clone()),
            Mutation::Insert(metal.clone()),
        ])
        .unwrap();
    let construction = FenceConstruction {
        height: length(1800.),
        post: FencePostProfile::Rectangular {
            width: length(90.),
            depth: length(90.),
        },
        post_material: wood.id,
        preferred_post_spacing: length(2000.),
        layout: FenceBayLayout::ShortLast,
        infills: vec![FenceInfill {
            id: EntityId::new(),
            name: "Panel".into(),
            material: wood.id,
            bottom: length(100.),
            height: length(1600.),
            normal_offset: length(0.),
            construction: FenceInfillKind::Panel {
                thickness: length(18.),
                width: FencePanelWidth::FitBay,
            },
        }],
    };
    (
        Session::new(document).unwrap(),
        FenceTypeRequest::new("Garden fence", construction),
        metal.id,
    )
}
fn request(ty: EntityId, name: &str, end: f64) -> FenceRunRequest {
    FenceRunRequest::new(
        name,
        FenceRunDefinition::new(
            ty,
            FencePath {
                vertices: [[0., 0.], [end, 0.]]
                    .into_iter()
                    .map(|p| FenceVertex {
                        id: EntityId::new(),
                        point: p.map(length),
                    })
                    .collect(),
                closed: false,
                base_z: length(0.),
            },
        ),
    )
}
fn create(session: &mut Session, ty: &FenceTypeRequest, run: &FenceRunRequest, with_type: bool) {
    let prepared = prepare_fence_change(
        session,
        session.revision(),
        actor(),
        &FenceChange {
            new_type: with_type.then(|| ty.clone()),
            run: Some(FenceRunChange::Create(run.clone())),
            ..Default::default()
        },
    )
    .unwrap();
    session.commit(prepared.model.prepared).unwrap();
}
fn outputs(document: &Document, run: EntityId) -> BTreeMap<FenceMemberKey, (EntityId, EntityId)> {
    fence_members(document, run)
        .unwrap()
        .into_iter()
        .map(|(key, feature, body)| (key, (feature, body)))
        .collect()
}

#[test]
fn preview_cancel_apply_history_and_tcd_keep_complete_typed_fence_source() {
    let (mut session, ty, _) = fixture();
    let before = session.document().clone();
    let run = request(ty.id, "Boundary", 4500.);
    let change = FenceChange {
        new_type: Some(ty.clone()),
        run: Some(FenceRunChange::Create(run.clone())),
        ..Default::default()
    };
    let cancelled = prepare_fence_change(&session, 0, actor(), &change).unwrap();
    assert_eq!(cancelled.members.len(), 7);
    let hierarchy = cancelled.model.prepared.document().hierarchy();
    assert!(hierarchy.members[&before.id()].contains(&run.definition.group));
    let grouped = &hierarchy.members[&run.definition.group];
    for (_, _, output) in &cancelled.members {
        assert!(grouped.contains(&output.body));
        assert!(grouped.contains(&output.feature));
    }
    assert_eq!(session.document(), &before);
    drop(cancelled);
    assert_eq!(session.revision(), 0);
    let prepared = prepare_fence_change(&session, 0, actor(), &change).unwrap();
    let staged = prepared.model.prepared.document().clone();
    for (_, _, output) in &prepared.members {
        assert!(
            matches!(&staged.get(output.body).unwrap().data, EntityData::Part { material: Some(m), features, .. } if *m == ty.construction.post_material && features == &[output.feature])
        );
        assert!(
            staged
                .get(output.body)
                .unwrap()
                .name
                .starts_with("Boundary / ")
        );
        assert_eq!(
            read_fence_member(&staged, output.feature).unwrap().run,
            run.id
        );
    }
    session.commit(prepared.model.prepared).unwrap();
    assert_eq!(
        read_fence_run(session.document(), run.id).unwrap(),
        run.definition
    );
    assert_eq!(
        read_fence_type(session.document(), ty.id).unwrap(),
        ty.construction
    );
    let project = project_file::ProjectFile::new(session.document().clone()).unwrap();
    for cap in [
        FENCE_TYPE_CAPABILITY,
        FENCE_RUN_CAPABILITY,
        FENCE_MEMBER_CAPABILITY,
    ] {
        assert!(project.capabilities().contains(cap));
    }
    let mut bytes = Vec::new();
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let supported = project_file::supported_capabilities();
    let opened = project_file::read(&mut bytes.as_slice(), Default::default(), &supported).unwrap();
    assert_eq!(&opened.project.document, session.document());
    assert_eq!(
        outputs(&opened.project.document, run.id),
        outputs(session.document(), run.id)
    );
    for cap in [
        FENCE_TYPE_CAPABILITY,
        FENCE_RUN_CAPABILITY,
        FENCE_MEMBER_CAPABILITY,
    ] {
        let mut old = supported.clone();
        old.remove(cap);
        assert!(project_file::read(&mut bytes.as_slice(), Default::default(), &old).is_err());
    }
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &before);
    session.redo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &staged);
    assert!(prepare_fence_change(&session, 0, actor(), &change).is_err());
}

#[test]
fn shared_edits_reconcile_all_runs_retain_members_and_keep_supported_instance_overrides() {
    let (mut session, ty, metal) = fixture();
    let first = request(ty.id, "First", 4500.);
    create(&mut session, &ty, &first, true);
    let mut second = request(ty.id, "Second", 3000.);
    second.definition.overrides.post_spacing = Some(length(2000.));
    second.definition.overrides.post_height = Some(length(2400.));
    create(&mut session, &ty, &second, false);
    let old_first = outputs(session.document(), first.id);
    let old_second = outputs(session.document(), second.id);
    let mut edit = ty.clone();
    edit.name = "Shared metal fence".into();
    edit.construction.height = length(2000.);
    edit.construction.preferred_post_spacing = length(1500.);
    edit.construction.post_material = metal;
    edit.construction.infills[0].material = metal;
    let mut users = fence_type_users(session.document(), ty.id).unwrap();
    users.sort();
    let mut expected = vec![first.id, second.id];
    expected.sort();
    assert_eq!(users, expected);
    let before = session.document().clone();
    let prepared = prepare_fence_change(
        &session,
        session.revision(),
        actor(),
        &FenceChange {
            edit_type: Some(edit),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(prepared.runs, expected);
    assert_eq!(session.document(), &before);
    session.commit(prepared.model.prepared).unwrap();
    assert_eq!(outputs(session.document(), first.id), old_first);
    assert_eq!(outputs(session.document(), second.id), old_second);
    let params = parameters::evaluate(session.document()).unwrap();
    let layout = resolve_fence_run(
        session.document(),
        &params,
        &read_fence_run(session.document(), first.id).unwrap(),
    )
    .unwrap();
    assert_eq!(layout.construction.height_mm, 2000.);
    assert_eq!(
        layout
            .bays
            .iter()
            .map(|b| b.center_spacing_mm)
            .collect::<Vec<_>>(),
        [1500.; 3]
    );
    let layout = resolve_fence_run(
        session.document(),
        &params,
        &read_fence_run(session.document(), second.id).unwrap(),
    )
    .unwrap();
    assert_eq!(layout.construction.height_mm, 2400.);
    assert_eq!(
        layout
            .bays
            .iter()
            .map(|b| b.center_spacing_mm)
            .collect::<Vec<_>>(),
        [2000., 1000.]
    );
    for (_, body) in old_first.values().chain(old_second.values()) {
        assert!(
            matches!(session.document().get(*body).unwrap().data, EntityData::Part { material: Some(m), .. } if m == metal)
        );
    }
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &before);
}

#[test]
fn a_duplicated_construction_switches_only_the_selected_run_without_changing_retained_parts() {
    let (mut session, ty, metal) = fixture();
    let mut first = request(ty.id, "First", 4500.);
    create(&mut session, &ty, &first, true);
    let second = request(ty.id, "Second", 4500.);
    create(&mut session, &ty, &second, false);
    let before = outputs(session.document(), first.id);
    let mut duplicate = FenceTypeRequest::new("Separate fence", ty.construction.clone());
    duplicate.construction.post_material = metal;
    first.definition.fence_type = duplicate.id;
    let prepared = prepare_fence_change(
        &session,
        session.revision(),
        actor(),
        &FenceChange {
            new_type: Some(duplicate.clone()),
            run: Some(FenceRunChange::Edit(first.clone())),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(prepared.runs, [first.id]);
    session.commit(prepared.model.prepared).unwrap();
    assert_eq!(outputs(session.document(), first.id), before);
    assert_eq!(
        read_fence_run(session.document(), second.id)
            .unwrap()
            .fence_type,
        ty.id
    );
    assert_eq!(
        read_fence_type(session.document(), ty.id).unwrap(),
        ty.construction
    );
    assert_eq!(
        read_fence_type(session.document(), duplicate.id).unwrap(),
        duplicate.construction
    );
}

#[test]
fn shared_pitch_change_and_explicit_post_repair_stage_atomically() {
    let (mut session, ty, _) = fixture();
    let mut run = request(ty.id, "Adjusted", 5000.);
    let key = FencePostKey::Interior {
        start: run.definition.path.vertices[0].id,
        end: run.definition.path.vertices[1].id,
        ordinal: 2,
    };
    run.definition.post_positions.push(FencePostPosition {
        post: key,
        offset: length(3500.),
    });
    create(&mut session, &ty, &run, true);
    let before = session.document().clone();
    let mut edit = ty.clone();
    edit.construction.preferred_post_spacing = length(2500.);
    assert!(
        prepare_fence_change(
            &session,
            session.revision(),
            actor(),
            &FenceChange {
                edit_type: Some(edit.clone()),
                ..Default::default()
            }
        )
        .is_err()
    );
    assert_eq!(session.document(), &before);
    run.definition.post_positions.clear();
    let prepared = prepare_fence_change(
        &session,
        session.revision(),
        actor(),
        &FenceChange {
            edit_type: Some(edit),
            run: Some(FenceRunChange::Edit(run.clone())),
            ..Default::default()
        },
    )
    .unwrap();
    session.commit(prepared.model.prepared).unwrap();
    assert_eq!(outputs(session.document(), run.id).len(), 5);
    assert!(!outputs(session.document(), run.id).contains_key(&FenceMemberKey::Post { post: key }));
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &before);
}

#[test]
fn removing_a_referenced_member_requires_repair_without_mutating_the_run_or_type() {
    let (mut session, ty, _) = fixture();
    let mut run = request(ty.id, "Boundary", 4500.);
    create(&mut session, &ty, &run, true);
    let post = FenceMemberKey::Post {
        post: FencePostKey::Interior {
            start: run.definition.path.vertices[0].id,
            end: run.definition.path.vertices[1].id,
            ordinal: 2,
        },
    };
    let (feature, _) = outputs(session.document(), run.id)[&post];
    let reference = Entity::new(
        "Dependent measurement",
        EntityData::Extension {
            definition: Definition {
                type_id: "test.measurement".into(),
                schema_version: 1,
                data: serde_json::json!({}),
            },
            inputs: vec![Link::new(feature, "measured_feature")],
        },
    );
    let request = multicad_core::commands::Request {
        expected_revision: session.revision(),
        actor: actor(),
        label: "Add dependent measurement".into(),
        edits: vec![Mutation::Insert(reference)],
    };
    session.commit(session.prepare(request).unwrap()).unwrap();
    let before = session.document().clone();
    run.definition.path.vertices[1].point[0] = length(3000.);
    let error = prepare_fence_change(
        &session,
        session.revision(),
        actor(),
        &FenceChange {
            run: Some(FenceRunChange::Edit(run)),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("Dependent measurement"));
    assert_eq!(session.document(), &before);
}

#[test]
fn malformed_versions_links_and_member_keys_are_rejected_without_flattening() {
    let (mut session, ty, _) = fixture();
    let run = request(ty.id, "Boundary", 4500.);
    create(&mut session, &ty, &run, true);
    let member = outputs(session.document(), run.id)
        .into_values()
        .next()
        .unwrap()
        .0;
    let mut invalid = session.document().get(member).unwrap().clone();
    if let EntityData::Feature { definition, .. } = &mut invalid.data {
        definition.schema_version = 2;
    }
    assert!(
        session
            .document()
            .edited(&[Mutation::Replace(invalid)])
            .is_err()
    );
    let mut invalid = session.document().get(run.id).unwrap().clone();
    if let EntityData::Extension { inputs, .. } = &mut invalid.data {
        inputs.clear();
    }
    assert!(
        session
            .document()
            .edited(&[Mutation::Replace(invalid)])
            .is_err()
    );
    let mut invalid = session.document().get(member).unwrap().clone();
    if let EntityData::Feature { definition, .. } = &mut invalid.data {
        definition.data["key"] = serde_json::json!({"kind":"post","post":{"kind":"interior","start":run.definition.path.vertices[0].id,"end":run.definition.path.vertices[1].id,"ordinal":0}});
    }
    assert!(
        session
            .document()
            .edited(&[Mutation::Replace(invalid)])
            .is_err()
    );
}

#[test]
fn long_unicode_source_names_keep_valid_unique_generated_labels() {
    let (mut session, mut ty, _) = fixture();
    ty.construction.infills[0].name = "界".repeat(300);
    let run = request(ty.id, &"庭".repeat(300), 4500.);
    create(&mut session, &ty, &run, true);
    assert_eq!(session.document().get(run.id).unwrap().name, run.name);
    for (_, _, body) in fence_members(session.document(), run.id).unwrap() {
        let name = &session.document().get(body).unwrap().name;
        assert!(name.len() <= 1024);
        assert!(name.contains('…'));
    }
}

#[test]
fn the_largest_single_run_fits_one_canonical_transaction_and_one_more_member_refuses() {
    let (session, mut ty, _) = fixture();
    let pickets = MAX_FENCE_MEMBERS - 2;
    let clear = pickets as f64 * 10. + (pickets + 1) as f64;
    ty.construction.preferred_post_spacing = length(1e9);
    ty.construction.infills[0].construction = FenceInfillKind::Pickets {
        width: length(10.),
        depth: length(20.),
        maximum_gap: length(1.),
    };
    let run = request(ty.id, "Maximum run", clear + 90.);
    let change = FenceChange {
        new_type: Some(ty),
        run: Some(FenceRunChange::Create(run)),
        ..Default::default()
    };
    let prepared = prepare_fence_change(&session, session.revision(), actor(), &change).unwrap();
    assert_eq!(prepared.model.outputs.len(), MAX_FENCE_MEMBERS);
    assert_eq!(session.revision(), 0);
    let mut too_many = change;
    if let Some(FenceRunChange::Create(run)) = &mut too_many.run {
        run.definition.path.vertices[1].point[0] = length(clear + 101.);
    }
    assert!(
        prepare_fence_change(&session, session.revision(), actor(), &too_many)
            .unwrap_err()
            .to_string()
            .contains("bounded")
    );
}

#[test]
fn another_runs_unrepaired_post_override_refuses_the_entire_shared_edit() {
    let (mut session, ty, _) = fixture();
    let adjusted = |name| {
        let mut run = request(ty.id, name, 5000.);
        run.definition.post_positions.push(FencePostPosition {
            post: FencePostKey::Interior {
                start: run.definition.path.vertices[0].id,
                end: run.definition.path.vertices[1].id,
                ordinal: 2,
            },
            offset: length(3500.),
        });
        run
    };
    let mut first = adjusted("First");
    let second = adjusted("Second needs repair");
    create(&mut session, &ty, &first, true);
    create(&mut session, &ty, &second, false);
    let before = session.document().clone();
    let mut edit = ty;
    edit.construction.preferred_post_spacing = length(2500.);
    first.definition.post_positions.clear();
    let change = FenceChange {
        edit_type: Some(edit),
        run: Some(FenceRunChange::Edit(first)),
        ..Default::default()
    };
    let error = prepare_fence_change(&session, session.revision(), actor(), &change).unwrap_err();
    assert!(error.to_string().contains("Second needs repair"));
    assert_eq!(session.document(), &before);
}

#[test]
fn level_edits_change_world_placement_while_retaining_the_authored_floor_offset() {
    let (mut session, ty, _) = fixture();
    let mut level = LevelDefinition {
        elevation: length(3000.),
        height: length(3000.),
        slab_thickness: length(200.),
    };
    let (level_id, prepared) =
        prepare_level(&session, session.revision(), actor(), "Upper floor", &level).unwrap();
    session.commit(prepared).unwrap();
    let mut run = request(ty.id, "Upper fence", 4500.);
    run.definition.level = Some(level_id);
    run.definition.path.base_z = length(100.);
    create(&mut session, &ty, &run, true);
    let before = outputs(session.document(), run.id);
    let layout = resolve_fence_run(
        session.document(),
        session.parameters(),
        &read_fence_run(session.document(), run.id).unwrap(),
    )
    .unwrap();
    assert!(layout.posts.iter().all(|p| p.position_mm[2] == 3100.));
    level.elevation = length(4000.);
    let prepared =
        prepare_level_edit(&session, session.revision(), actor(), level_id, &level).unwrap();
    session.commit(prepared).unwrap();
    let source = read_fence_run(session.document(), run.id).unwrap();
    assert_eq!(source.path.base_z, length(100.));
    let layout = resolve_fence_run(session.document(), session.parameters(), &source).unwrap();
    assert!(layout.posts.iter().all(|p| p.position_mm[2] == 4100.));
    assert!(layout.infills.iter().all(|p| p.position_mm[2] == 4200.));
    assert_eq!(outputs(session.document(), run.id), before);
}

#[test]
fn changing_infill_kind_replaces_physical_members_and_never_retargets_an_old_panel_reference() {
    let (mut session, ty, _) = fixture();
    let run = request(ty.id, "Boundary", 2000.);
    create(&mut session, &ty, &run, true);
    let original = outputs(session.document(), run.id);
    let old_panel = original
        .iter()
        .find(|(key, _)| matches!(key, FenceMemberKey::Infill { .. }))
        .unwrap();
    let key = *old_panel.0;
    let (old_feature, old_part) = *old_panel.1;
    let mut edit = ty;
    edit.construction.infills[0].construction = FenceInfillKind::Pickets {
        width: length(70.),
        depth: length(20.),
        maximum_gap: length(100.),
    };
    let change = FenceChange {
        edit_type: Some(edit),
        ..Default::default()
    };
    let prepared = prepare_fence_change(&session, session.revision(), actor(), &change).unwrap();
    let staged = prepared.model.prepared.document();
    let replacement = outputs(staged, run.id);
    assert_ne!(replacement[&key], (old_feature, old_part));
    assert!(staged.get(old_feature).is_none());
    assert!(staged.get(old_part).is_none());
    for (key, output) in &original {
        if matches!(key, FenceMemberKey::Post { .. }) {
            assert_eq!(replacement[key], *output);
        }
    }
    let measurement = Entity::new(
        "Panel-specific measurement",
        EntityData::Extension {
            definition: Definition {
                type_id: "test.measurement".into(),
                schema_version: 1,
                data: serde_json::json!({}),
            },
            inputs: vec![Link::new(old_part, "measured_part")],
        },
    );
    let request = multicad_core::commands::Request {
        expected_revision: session.revision(),
        actor: actor(),
        label: "Reference the complete panel".into(),
        edits: vec![Mutation::Insert(measurement)],
    };
    session.commit(session.prepare(request).unwrap()).unwrap();
    let before = session.document().clone();
    let error = prepare_fence_change(&session, session.revision(), actor(), &change).unwrap_err();
    assert!(error.to_string().contains("Panel-specific measurement"));
    assert_eq!(session.document(), &before);
}

fn parameter_driven_run(spacing: bool) -> (Session, Entity, FenceRunRequest) {
    use multicad_core::{constraint_sketch::Dimension, parameters::BoundExpression};
    let (mut session, mut ty, _) = fixture();
    let parameter = Entity::new(
        "driver",
        EntityData::Parameter {
            expression: if spacing { "2000" } else { "4500" }.into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let prepared = session
        .prepare(multicad_core::commands::Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Add fence driver".into(),
            edits: vec![Mutation::Insert(parameter.clone())],
        })
        .unwrap();
    session.commit(prepared).unwrap();
    let bound = Dimension::Expression {
        expression: BoundExpression::bind(session.document(), "=$driver", "mm", None).unwrap(),
    };
    let mut run = request(ty.id, "Driven boundary", 4500.);
    if spacing {
        ty.construction.preferred_post_spacing = bound;
    } else {
        run.definition.path.vertices[1].point[0] = bound;
    }
    create(&mut session, &ty, &run, true);
    (session, parameter, run)
}
fn driver_request(
    session: &Session,
    parameter: &Entity,
    value: &str,
) -> multicad_core::commands::Request {
    let mut parameter = parameter.clone();
    if let EntityData::Parameter { expression, .. } = &mut parameter.data {
        *expression = value.into();
    }
    multicad_core::commands::Request {
        expected_revision: session.revision(),
        actor: Actor::local(Channel::Mcp),
        label: "Change receiving parameter".into(),
        edits: vec![Mutation::Replace(parameter)],
    }
}
#[test]
fn receiving_parameter_changes_reconcile_real_member_source_atomically_and_reopen() {
    for (spacing, value, count) in [(false, "6500", 9), (true, "1000", 11)] {
        let (mut session, parameter, run) = parameter_driven_run(spacing);
        let before = session.document().clone();
        let old = outputs(&before, run.id);
        assert_eq!(old.len(), 7);
        let cancelled = session
            .prepare(driver_request(&session, &parameter, value))
            .unwrap();
        assert_eq!(outputs(cancelled.document(), run.id).len(), count);
        assert_eq!(session.document(), &before);
        drop(cancelled);
        let prepared = session
            .prepare(driver_request(&session, &parameter, value))
            .unwrap();
        let after = prepared.document().clone();
        let new = outputs(&after, run.id);
        let mut retained = 0;
        for (key, output) in &old {
            // Extending a segment replaces its last bay: the old corner-to-
            // interior pair no longer denotes that physical panel.
            if let Some(current) = new.get(key) {
                assert_eq!(current, output, "retained semantic member");
                retained += 1;
            }
        }
        assert!(
            retained >= 4,
            "corner and existing interior post identities survive"
        );
        for vertex in &run.definition.path.vertices {
            let key = FenceMemberKey::Post {
                post: FencePostKey::Vertex { vertex: vertex.id },
            };
            assert_eq!(new[&key], old[&key]);
        }
        let layout = resolve_fence_run(&after, prepared.parameters(), &run.definition).unwrap();
        assert_eq!(layout.member_specs().len(), count);
        session.commit(prepared).unwrap();
        session.undo(session.revision(), actor()).unwrap();
        assert_eq!(session.document(), &before);
        session.redo(session.revision(), actor()).unwrap();
        assert_eq!(session.document(), &after);
        let mut bytes = Vec::new();
        project_file::write(
            &project_file::ProjectFile::new(after.clone()).unwrap(),
            &mut bytes,
            Default::default(),
        )
        .unwrap();
        let reopened = project_file::read(
            &mut bytes.as_slice(),
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap();
        assert_eq!(reopened.project.document, after);
    }
}
#[test]
fn receiving_parameter_cannot_silently_remove_referenced_members() {
    let (mut session, parameter, run) = parameter_driven_run(false);
    let key = FenceMemberKey::Post {
        post: FencePostKey::Interior {
            start: run.definition.path.vertices[0].id,
            end: run.definition.path.vertices[1].id,
            ordinal: 2,
        },
    };
    let feature = outputs(session.document(), run.id)[&key].0;
    let reference = Entity::new(
        "Retained measurement",
        EntityData::Extension {
            definition: Definition {
                type_id: "test.measurement".into(),
                schema_version: 1,
                data: serde_json::json!({}),
            },
            inputs: vec![Link::new(feature, "measured_feature")],
        },
    );
    let prepared = session
        .prepare(multicad_core::commands::Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Add measurement".into(),
            edits: vec![Mutation::Insert(reference)],
        })
        .unwrap();
    session.commit(prepared).unwrap();
    let before = session.document().clone();
    let error = session
        .prepare(driver_request(&session, &parameter, "3000"))
        .unwrap_err();
    assert!(error.to_string().contains("Retained measurement"));
    assert_eq!(session.document(), &before);
    let error = session
        .prepare(driver_request(&session, &parameter, "0"))
        .unwrap_err();
    assert!(!error.to_string().is_empty());
    assert_eq!(session.document(), &before);
}
#[test]
fn unrelated_source_edits_preserve_member_metadata_without_reconciliation() {
    let (mut session, parameter, run) = parameter_driven_run(false);
    let body = outputs(session.document(), run.id)
        .into_values()
        .next()
        .unwrap()
        .1;
    let mut part = session.document().get(body).unwrap().clone();
    part.name = "Operator member label".into();
    part.description = "Retained note".into();
    let prepared = session
        .prepare(multicad_core::commands::Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Describe member".into(),
            edits: vec![Mutation::Replace(part.clone())],
        })
        .unwrap();
    session.commit(prepared).unwrap();
    assert_eq!(session.document().get(body), Some(&part));
    let prepared = session
        .prepare(driver_request(&session, &parameter, "4500"))
        .unwrap();
    assert_eq!(prepared.document(), session.document());
}
