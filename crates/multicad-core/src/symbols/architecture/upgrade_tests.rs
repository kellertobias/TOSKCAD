use super::*;
use crate::symbols::{compare_update, prepare_update};
fn replace_fence(
    definition: &SymbolDefinition,
    edit: impl FnOnce(&mut FenceConstruction),
) -> SymbolDefinition {
    let mut next = definition.clone();
    let blueprint = blueprint_for_definition(&next).unwrap().unwrap();
    let mut construction = read_fence_type(&next.template, blueprint.construction).unwrap();
    edit(&mut construction);
    let mut entity = next.template.get(blueprint.construction).unwrap().clone();
    entity.data = EntityData::Extension {
        definition: Definition {
            type_id: FENCE_TYPE_KIND.into(),
            schema_version: FENCE_SCHEMA,
            data: serde_json::to_value(&construction).unwrap(),
        },
        inputs: construction.inputs(),
    };
    next.template = next.template.edited(&[Mutation::Replace(entity)]).unwrap();
    next.revision = "r2".into();
    next.validate().unwrap();
    next
}
fn members(document: &Document, run: EntityId) -> BTreeMap<FenceMemberKey, (EntityId, EntityId)> {
    fence_members(document, run)
        .unwrap()
        .into_iter()
        .map(|(key, feature, body)| (key, (feature, body)))
        .collect()
}
#[test]
fn explicit_upgrade_preserves_pins_overrides_context_and_compatible_members_and_is_undoable() {
    let (mut authoring, mut blueprint, width, _) = fence();
    let spacing = EntityId::new();
    blueprint.dimensions.push(ArchitectureDimensionSlot {
        id: spacing,
        name: "Spacing".into(),
        target: ArchitectureDimensionTarget::FenceSpacing,
    });
    let definition = author(&mut authoring, &blueprint, vec![width]);
    let (mut host, id) = consumer(&definition);
    let mut requests = Vec::new();
    for (index, end) in [4500., 6500.].into_iter().enumerate() {
        let mut inputs = ArchitectureInputs::default();
        inputs.dimensions.insert(
            blueprint.dimensions[0].id,
            length(2000. + index as f64 * 200.),
        );
        if index == 1 {
            inputs.dimensions.insert(spacing, length(2000.));
        }
        let request = ArchitecturePlaceRequest::new(
            host.document(),
            id,
            "Pinned fence",
            inputs,
            fence_context(end),
        )
        .unwrap();
        let prepared =
            prepare_architecture_place(&host, host.revision(), actor(), &request).unwrap();
        host.commit(prepared.prepared).unwrap();
        requests.push(request);
    }
    let before = host.document().clone();
    let next = replace_fence(&definition, |fence| {
        fence.preferred_post_spacing = length(1000.);
        fence.height = length(2100.);
    });
    let report = compare_update(host.document(), id, &next).unwrap();
    assert_eq!(report.affected_instances.len(), 2);
    assert!(
        report.architecture_error.is_none(),
        "{:?}",
        report.architecture_error
    );
    assert!(
        report
            .architecture_changes
            .iter()
            .any(|c| !c.removed_members.is_empty())
    );
    assert!(
        report
            .architecture_changes
            .iter()
            .all(|c| c.changed_inputs.contains(&spacing))
    );
    assert_eq!(host.document(), &before);
    let request = SymbolUpdateRequest::new(host.document(), id, next.clone()).unwrap();
    assert_eq!(request.maps.len(), 2);
    assert!(request.instance_overrides.is_empty());
    assert_eq!(request.architecture_inputs.len(), 2);
    let cancelled = prepare_update(&host, host.revision(), actor(), &request).unwrap();
    assert_eq!(host.document(), &before);
    drop(cancelled);
    let prepared = prepare_update(&host, host.revision(), actor(), &request).unwrap();
    assert_eq!(prepared.instances.len(), 2);
    for (index, placed) in requests.iter().enumerate() {
        let ArchitectureOutput::Fence { run, group } = placed.output else {
            unreachable!()
        };
        let record =
            read_architecture_instance(prepared.prepared.document(), placed.record).unwrap();
        assert_eq!(record.ids, placed.ids);
        assert_eq!(record.inputs, placed.inputs);
        assert_eq!(record.pin, next.pin().unwrap());
        let canonical = read_fence_run(prepared.prepared.document(), run).unwrap();
        assert_eq!(canonical, read_fence_run(&before, run).unwrap());
        assert_eq!(canonical.group, group);
        let layout = resolve_fence_run(
            prepared.prepared.document(),
            prepared.prepared.parameters(),
            &canonical,
        )
        .unwrap();
        assert_eq!(layout.construction.height_mm, 2000. + index as f64 * 200.);
        let old = members(&before, run);
        let new = members(prepared.prepared.document(), run);
        assert_eq!(new.len(), if index == 0 { 11 } else { 9 });
        for (key, output) in &old {
            if let Some(current) = new.get(key) {
                assert_eq!(current, output);
            }
        }
    }
    host.commit(prepared.prepared).unwrap();
    let after = host.document().clone();
    host.undo(host.revision(), actor()).unwrap();
    assert_eq!(host.document(), &before);
    host.redo(host.revision(), actor()).unwrap();
    assert_eq!(host.document(), &after);
    assert!(prepare_update(&host, 0, actor(), &request).is_err());
}

fn replace_blueprint(
    definition: &SymbolDefinition,
    edit: impl FnOnce(&mut ArchitectureBlueprint),
) -> SymbolDefinition {
    let mut next = definition.clone();
    let mut blueprint = blueprint_for_definition(&next).unwrap().unwrap();
    edit(&mut blueprint);
    let mut root = next.template.get(next.root).unwrap().clone();
    root.data = EntityData::Extension {
        definition: blueprint.definition().unwrap(),
        inputs: blueprint.inputs(),
    };
    next.template = next.template.edited(&[Mutation::Replace(root)]).unwrap();
    next.revision = "r2".into();
    next.validate().unwrap();
    next
}
fn placed_fence() -> (
    Session,
    EntityId,
    SymbolDefinition,
    ArchitecturePlaceRequest,
    EntityId,
) {
    let (mut authoring, blueprint, width, _) = fence();
    let slot = blueprint.dimensions[0].id;
    let definition = author(&mut authoring, &blueprint, vec![width]);
    let (mut host, id) = consumer(&definition);
    let mut inputs = ArchitectureInputs::default();
    inputs.dimensions.insert(slot, length(2000.));
    let request = ArchitecturePlaceRequest::new(
        host.document(),
        id,
        "Pinned boundary",
        inputs,
        fence_context(4500.),
    )
    .unwrap();
    let prepared = prepare_architecture_place(&host, host.revision(), actor(), &request).unwrap();
    host.commit(prepared.prepared).unwrap();
    (host, id, definition, request, slot)
}
#[test]
fn removed_or_retargeted_inputs_require_explicit_reset_and_never_silently_change_meaning() {
    for retarget in [false, true] {
        let (mut host, id, definition, placed, slot) = placed_fence();
        let before = host.document().clone();
        let next = replace_blueprint(&definition, |blueprint| {
            if retarget {
                blueprint.dimensions[0].target = ArchitectureDimensionTarget::FenceSpacing;
            } else {
                blueprint.dimensions.clear();
            }
        });
        let report = compare_update(host.document(), id, &next).unwrap();
        assert!(report.architecture_error.is_some());
        let change = &report.architecture_changes[0];
        if retarget {
            assert!(change.changed_inputs.contains(&slot));
            assert!(
                report
                    .architecture_error
                    .as_ref()
                    .unwrap()
                    .contains("reset")
            );
        } else {
            assert!(change.removed_inputs.contains(&slot));
        }
        let mut request = SymbolUpdateRequest::new(host.document(), id, next.clone()).unwrap();
        assert!(prepare_update(&host, host.revision(), actor(), &request).is_err());
        assert_eq!(host.document(), &before);
        request
            .architecture_inputs
            .get_mut(&placed.record)
            .unwrap()
            .dimensions
            .remove(&slot);
        let prepared = prepare_update(&host, host.revision(), actor(), &request).unwrap();
        let record =
            read_architecture_instance(prepared.prepared.document(), placed.record).unwrap();
        assert!(!record.inputs.dimensions.contains_key(&slot));
        host.commit(prepared.prepared).unwrap();
        host.undo(host.revision(), actor()).unwrap();
        assert_eq!(host.document(), &before);
    }
}
#[test]
fn referenced_member_removal_reports_repair_and_refuses_the_entire_definition_update() {
    let (mut host, id, definition, placed, _) = placed_fence();
    let ArchitectureOutput::Fence { run, .. } = placed.output else {
        unreachable!()
    };
    let source = read_fence_run(host.document(), run).unwrap();
    let endpoint = FencePostKey::Vertex {
        vertex: source.path.vertices[1].id,
    };
    let measured = members(host.document(), run)
        .into_iter()
        .find_map(|(key, (_, body))| {
            matches!(key, FenceMemberKey::Infill { infill } if infill.bay.end == endpoint)
                .then_some(body)
        })
        .unwrap();
    let reference = Entity::new(
        "Retained fence measurement",
        EntityData::Extension {
            definition: Definition {
                type_id: "test.measurement".into(),
                schema_version: 1,
                data: serde_json::json!({}),
            },
            inputs: vec![Link::new(measured, "measured_body")],
        },
    );
    let prepared = host
        .prepare(Request {
            expected_revision: host.revision(),
            actor: actor(),
            label: "Add reference".into(),
            edits: vec![Mutation::Insert(reference.clone())],
        })
        .unwrap();
    host.commit(prepared).unwrap();
    let before = host.document().clone();
    let next = replace_fence(&definition, |fence| {
        fence.preferred_post_spacing = length(1000.)
    });
    let report = compare_update(host.document(), id, &next).unwrap();
    assert!(
        report
            .architecture_error
            .as_ref()
            .unwrap()
            .contains("Retained fence measurement")
    );
    assert!(
        report.architecture_changes[0]
            .removed_members
            .contains(&measured)
    );
    let request = SymbolUpdateRequest::new(host.document(), id, next).unwrap();
    assert!(prepare_update(&host, host.revision(), actor(), &request).is_err());
    assert_eq!(host.document(), &before);
    let prepared = host
        .prepare(Request {
            expected_revision: host.revision(),
            actor: actor(),
            label: "Explicitly remove dependent measurement".into(),
            edits: vec![Mutation::Remove(reference.id)],
        })
        .unwrap();
    host.commit(prepared).unwrap();
    let prepared = prepare_update(&host, host.revision(), actor(), &request).unwrap();
    assert!(prepared.prepared.document().get(measured).is_none());
}
#[test]
fn construction_replacement_rebinds_real_output_without_reallocating_compatible_source_or_members()
{
    let (mut host, id, definition, placed, _) = placed_fence();
    let old_blueprint = blueprint_for_definition(&definition).unwrap().unwrap();
    let mut next = definition.clone();
    let mut blueprint = old_blueprint.clone();
    let mut construction = next.template.get(blueprint.construction).unwrap().clone();
    construction.id = EntityId::new();
    blueprint.construction = construction.id;
    let mut root = next.template.get(next.root).unwrap().clone();
    root.data = EntityData::Extension {
        definition: blueprint.definition().unwrap(),
        inputs: blueprint.inputs(),
    };
    next.template = next
        .template
        .edited(&[
            Mutation::Remove(old_blueprint.construction),
            Mutation::Insert(construction),
            Mutation::Replace(root),
        ])
        .unwrap();
    next.revision = "r2".into();
    next.validate().unwrap();
    let before = host.document().clone();
    let mut request = SymbolUpdateRequest::new(host.document(), id, next.clone()).unwrap();
    let source_root = request.maps[&placed.record][&definition.root];
    request
        .maps
        .get_mut(&placed.record)
        .unwrap()
        .insert(definition.root, EntityId::new());
    assert!(prepare_update(&host, host.revision(), actor(), &request).is_err());
    request
        .maps
        .get_mut(&placed.record)
        .unwrap()
        .insert(definition.root, source_root);
    let prepared = prepare_update(&host, host.revision(), actor(), &request).unwrap();
    let ArchitectureOutput::Fence { run, .. } = placed.output else {
        unreachable!()
    };
    assert_eq!(
        members(prepared.prepared.document(), run),
        members(&before, run)
    );
    let record = read_architecture_instance(prepared.prepared.document(), placed.record).unwrap();
    assert!(!record.ids.contains_key(&old_blueprint.construction));
    assert_eq!(
        read_fence_run(prepared.prepared.document(), run)
            .unwrap()
            .fence_type,
        record.ids[&blueprint.construction]
    );
    assert!(
        prepared
            .prepared
            .document()
            .get(placed.ids[&old_blueprint.construction])
            .is_none()
    );
    host.commit(prepared.prepared).unwrap();
    host.undo(host.revision(), actor()).unwrap();
    assert_eq!(host.document(), &before);
}
#[test]
fn a_pin_upgrade_cannot_change_its_architectural_role() {
    let (host, id, definition, _, _) = placed_fence();
    let (mut source, blueprint, _) = wall();
    let mut other = author(&mut source, &blueprint, vec![]);
    other.symbol = definition.symbol;
    other.revision = "r2".into();
    assert!(
        compare_update(host.document(), id, &other)
            .unwrap_err()
            .to_string()
            .contains("new symbol identity")
    );
    assert!(SymbolUpdateRequest::new(host.document(), id, other).is_err());
}

#[test]
fn changed_material_choices_can_be_explicitly_repaired_and_default_options_are_compared() {
    let (mut host, id, definition, placed, _) = placed_fence();
    let blueprint = blueprint_for_definition(&definition).unwrap().unwrap();
    let slot = &blueprint.materials[0];
    let mut inputs = placed.inputs.clone();
    inputs.materials.insert(
        slot.id,
        ArchitectureMaterialValue::Preset {
            material: slot.choices[0],
        },
    );
    let prepared = prepare_architecture_edit(
        &host,
        host.revision(),
        actor(),
        &ArchitectureEditRequest {
            record: placed.record,
            inputs,
            context: None,
        },
    )
    .unwrap();
    host.commit(prepared.prepared).unwrap();
    let before = host.document().clone();
    let wood = read_fence_type(&definition.template, blueprint.construction)
        .unwrap()
        .post_material;
    let next = replace_blueprint(&definition, |b| b.materials[0].choices = vec![wood]);
    let report = compare_update(host.document(), id, &next).unwrap();
    assert!(report.architecture_error.is_some());
    assert!(
        report.architecture_changes[0]
            .changed_inputs
            .contains(&slot.id)
    );
    let mut update = SymbolUpdateRequest::new(host.document(), id, next).unwrap();
    assert!(prepare_update(&host, host.revision(), actor(), &update).is_err());
    update
        .architecture_inputs
        .get_mut(&placed.record)
        .unwrap()
        .materials
        .insert(
            slot.id,
            ArchitectureMaterialValue::Preset { material: wood },
        );
    let repaired = compare_update_request(host.document(), &update).unwrap();
    assert!(repaired.architecture_error.is_none());
    let prepared = prepare_update(&host, host.revision(), actor(), &update).unwrap();
    let record = read_architecture_instance(prepared.prepared.document(), placed.record).unwrap();
    let ArchitectureOutput::Fence { run, .. } = placed.output else {
        unreachable!()
    };
    let construction = read_fence_type(
        prepared.prepared.document(),
        read_fence_run(prepared.prepared.document(), run)
            .unwrap()
            .fence_type,
    )
    .unwrap();
    assert_eq!(construction.post_material, record.ids[&wood]);
    assert_eq!(host.document(), &before);
    let layout = replace_fence(&definition, |f| f.layout = FenceBayLayout::Equal);
    assert!(
        compare_update(host.document(), id, &layout)
            .unwrap()
            .architecture_changes[0]
            .changed_inputs
            .contains(&blueprint.options[0].id)
    );
}
#[test]
fn named_removed_wall_layers_are_visible_before_update_and_cannot_be_silently_reused() {
    let (mut authoring, blueprint, layers) = wall();
    let definition = author(&mut authoring, &blueprint, vec![]);
    let (mut host, id) = consumer(&definition);
    let placed = ArchitecturePlaceRequest::new(
        host.document(),
        id,
        "Layered wall",
        ArchitectureInputs::default(),
        wall_context(3000.),
    )
    .unwrap();
    let prepared = prepare_architecture_place(&host, host.revision(), actor(), &placed).unwrap();
    host.commit(prepared.prepared).unwrap();
    let before = host.document().clone();
    let mut next = definition.clone();
    let WallTypeConstruction::Layered(mut construction) =
        read_wall_construction(&next.template, blueprint.construction).unwrap()
    else {
        unreachable!()
    };
    construction.layers.layers.retain(|l| l.id != layers[1]);
    let mut source = next.template.get(blueprint.construction).unwrap().clone();
    source.data = EntityData::Extension {
        definition: construction.definition().unwrap(),
        inputs: construction.inputs(),
    };
    next.template = next.template.edited(&[Mutation::Replace(source)]).unwrap();
    next.revision = "r2".into();
    let report = compare_update(host.document(), id, &next).unwrap();
    assert_eq!(
        report.architecture_changes[0].removed_layers,
        vec![ArchitectureSourceChange {
            id: layers[1],
            name: "Air".into()
        }]
    );
    assert!(report.architecture_error.is_none());
    let update = SymbolUpdateRequest::new(host.document(), id, next).unwrap();
    let prepared = prepare_update(&host, host.revision(), actor(), &update).unwrap();
    let record = read_architecture_instance(prepared.prepared.document(), placed.record).unwrap();
    assert!(!record.ids.contains_key(&layers[1]));
    for layer in [layers[0], layers[2]] {
        assert_eq!(record.ids[&layer], placed.ids[&layer]);
    }
    host.commit(prepared.prepared).unwrap();
    host.undo(host.revision(), actor()).unwrap();
    assert_eq!(host.document(), &before);
}
#[test]
fn a_recipe_kind_change_reports_and_replaces_physical_infills_without_retargeting_posts() {
    let (host, id, definition, placed, _) = placed_fence();
    let ArchitectureOutput::Fence { run, .. } = placed.output else {
        unreachable!()
    };
    let before = members(host.document(), run);
    let next = replace_fence(&definition, |f| {
        f.infills[0].construction = FenceInfillKind::Rail { depth: length(18.) }
    });
    let report = compare_update(host.document(), id, &next).unwrap();
    assert_eq!(report.architecture_changes[0].changed_recipes.len(), 1);
    assert_eq!(report.architecture_changes[0].removed_members.len(), 3);
    assert!(report.architecture_error.is_none());
    let update = SymbolUpdateRequest::new(host.document(), id, next).unwrap();
    let prepared = prepare_update(&host, host.revision(), actor(), &update).unwrap();
    let after = members(prepared.prepared.document(), run);
    for (key, output) in &before {
        match key {
            FenceMemberKey::Post { .. } => assert_eq!(after[key], *output),
            FenceMemberKey::Infill { .. } => {
                assert_ne!(after[key], *output);
                assert!(prepared.prepared.document().get(output.0).is_none());
                assert!(prepared.prepared.document().get(output.1).is_none());
            }
        }
    }
}
