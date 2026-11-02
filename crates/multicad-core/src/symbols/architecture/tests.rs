use super::*;
use crate::{
    commands::Channel,
    constraint_sketch::Dimension,
    model::*,
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
};
use std::collections::BTreeMap;
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn material(name: &str, color: [u8; 4]) -> Entity {
    Entity::new(
        name,
        EntityData::Material {
            density_kg_m3: None,
            color_rgba: color,
            section_hatch: None,
        },
    )
}
fn parameter(name: &str, value: &str) -> Entity {
    Entity::new(
        name,
        EntityData::Parameter {
            expression: value.into(),
            unit: "mm".into(),
            exposed: false,
            inputs: vec![],
        },
    )
}
fn fence() -> (Session, ArchitectureBlueprint, EntityId, String) {
    let wood = material("Timber", [150, 110, 70, 255]);
    let metal = material("Metal", [90, 100, 110, 255]);
    let width = parameter("post_width", "90");
    let document = Document::new("Architectural author")
        .unwrap()
        .edited(&[
            Mutation::Insert(wood.clone()),
            Mutation::Insert(metal.clone()),
            Mutation::Insert(width.clone()),
        ])
        .unwrap();
    let mut session = Session::new(document).unwrap();
    let literal = EntityId::new().to_string();
    let recipe = EntityId::new();
    let construction = FenceConstruction {
        height: length(1800.),
        post: FencePostProfile::Rectangular {
            width: Dimension::Expression {
                expression: BoundExpression::bind(session.document(), "=$post_width", "mm", None)
                    .unwrap(),
            },
            depth: length(90.),
        },
        post_material: wood.id,
        preferred_post_spacing: length(2000.),
        layout: FenceBayLayout::ShortLast,
        infills: vec![FenceInfill {
            id: recipe,
            name: literal.clone(),
            material: wood.id,
            bottom: length(100.),
            height: length(1600.),
            normal_offset: length(0.),
            construction: FenceInfillKind::Panel {
                thickness: length(18.),
                width: FencePanelWidth::Purchased {
                    width: length(1910.),
                    short_bay: FenceShortPanelPolicy::Cut,
                },
            },
        }],
    };
    let ty = FenceTypeRequest::new("Fence construction", construction);
    let path = FencePath {
        vertices: [[0., 0.], [4000., 0.]]
            .into_iter()
            .map(|p| FenceVertex {
                id: EntityId::new(),
                point: p.map(length),
            })
            .collect(),
        closed: false,
        base_z: length(0.),
    };
    let run = FenceRunRequest::new(
        "Author preview fixture",
        FenceRunDefinition::new(ty.id, path),
    );
    let prepared = prepare_fence_change(
        &session,
        0,
        actor(),
        &FenceChange {
            new_type: Some(ty.clone()),
            run: Some(FenceRunChange::Create(run)),
            ..Default::default()
        },
    )
    .unwrap();
    session.commit(prepared.model.prepared).unwrap();
    let mut blueprint = ArchitectureBlueprint::new(ArchitectureRole::Fence, ty.id);
    blueprint.dimensions.push(ArchitectureDimensionSlot {
        id: EntityId::new(),
        name: EntityId::new().to_string(),
        target: ArchitectureDimensionTarget::FenceHeight,
    });
    blueprint.materials.push(ArchitectureMaterialSlot {
        id: EntityId::new(),
        name: "Post material".into(),
        target: ArchitectureMaterialTarget::FencePost,
        choices: vec![metal.id],
        allow_project_materials: true,
    });
    blueprint.options.push(ArchitectureOptionSlot {
        id: EntityId::new(),
        name: "Bay layout".into(),
        target: ArchitectureOptionTarget::FenceLayout,
        choices: vec![
            ArchitectureOptionValue::FenceLayout {
                layout: FenceBayLayout::Equal,
            },
            ArchitectureOptionValue::FenceLayout {
                layout: FenceBayLayout::ShortLast,
            },
        ],
    });
    (session, blueprint, width.id, literal)
}
fn wall() -> (Session, ArchitectureBlueprint, [EntityId; 3]) {
    let brick = material("Brick", [170, 90, 60, 255]);
    let finish = material("Finish", [230, 230, 230, 255]);
    let mut session = Session::new(
        Document::new("Wall author")
            .unwrap()
            .edited(&[
                Mutation::Insert(brick.clone()),
                Mutation::Insert(finish.clone()),
            ])
            .unwrap(),
    )
    .unwrap();
    let ids = [EntityId::new(), EntityId::new(), EntityId::new()];
    let value = LayeredWallTypeDefinition {
        height: length(2700.),
        reference: WallLayerReference::Boundary {
            layer: ids[0],
            side: WallLayerSide::Inside,
        },
        layers: WallLayerStack {
            layers: vec![
                WallLayer {
                    id: ids[0],
                    name: EntityId::new().to_string(),
                    role: Some(EntityId::new().to_string()),
                    construction: WallLayerKind::Solid {
                        thickness: length(120.),
                        material: brick.id,
                    },
                },
                WallLayer {
                    id: ids[1],
                    name: "Air".into(),
                    role: None,
                    construction: WallLayerKind::Cavity {
                        thickness: length(40.),
                    },
                },
                WallLayer {
                    id: ids[2],
                    name: "Finish".into(),
                    role: None,
                    construction: WallLayerKind::Coating {
                        material: finish.id,
                    },
                },
            ],
        },
    };
    let (ty, p) =
        prepare_layered_wall_type(&session, 0, actor(), "Layered construction", &value).unwrap();
    session.commit(p).unwrap();
    let mut blueprint = ArchitectureBlueprint::new(ArchitectureRole::Wall, ty);
    blueprint.dimensions.push(ArchitectureDimensionSlot {
        id: EntityId::new(),
        name: "Outer thickness".into(),
        target: ArchitectureDimensionTarget::WallLayerThickness { layer: ids[0] },
    });
    blueprint.materials.push(ArchitectureMaterialSlot {
        id: EntityId::new(),
        name: "Finish material".into(),
        target: ArchitectureMaterialTarget::WallLayer { layer: ids[2] },
        choices: vec![brick.id, finish.id],
        allow_project_materials: false,
    });
    blueprint.options.push(ArchitectureOptionSlot {
        id: EntityId::new(),
        name: "Reference".into(),
        target: ArchitectureOptionTarget::WallReference,
        choices: vec![
            ArchitectureOptionValue::WallReference {
                reference: WallLayerReference::Outside,
            },
            ArchitectureOptionValue::WallReference {
                reference: WallLayerReference::Boundary {
                    layer: ids[0],
                    side: WallLayerSide::Inside,
                },
            },
        ],
    });
    (session, blueprint, ids)
}
fn author(
    session: &mut Session,
    blueprint: &ArchitectureBlueprint,
    exposed: Vec<EntityId>,
) -> SymbolDefinition {
    let (root, p) = prepare_blueprint(
        session,
        session.revision(),
        actor(),
        "Reusable construction",
        blueprint,
    )
    .unwrap();
    session.commit(p).unwrap();
    let request = SymbolAuthorRequest::new(
        "Architectural definition",
        root,
        SymbolKind::Dynamic,
        exposed,
        None,
    );
    let before = session.document().clone();
    let p = super::super::prepare_author(session, session.revision(), actor(), &request).unwrap();
    assert_eq!(session.document(), &before);
    session.commit(p.prepared).unwrap();
    super::super::read_definition(session.document(), request.definition).unwrap()
}

#[test]
fn construction_pin_excludes_the_authored_path_and_rejects_generic_placement() {
    let (mut session, blueprint, width, _) = fence();
    let definition = author(&mut session, &blueprint, vec![width]);
    assert_eq!(
        blueprint_for_definition(&definition).unwrap(),
        Some(blueprint.clone())
    );
    assert!(definition.template.entities().all(|e| !matches!(
        e.data,
        EntityData::Part { .. } | EntityData::Feature { .. } | EntityData::Group { .. }
    )));
    assert!(fence_runs(&definition.template).next().is_none());
    assert_eq!(
        definition
            .template
            .entities()
            .filter(|e| matches!(e.data, EntityData::Material { .. }))
            .count(),
        2,
        "alternate material remains in the pin even though the construction does not currently use it"
    );
    let mut host = Session::new(Document::new("Consumer").unwrap()).unwrap();
    let p = super::super::prepare_import(&host, 0, actor(), &definition).unwrap();
    let id = p.definition;
    host.commit(p.prepared).unwrap();
    let before = host.document().clone();
    let error = SymbolPlaceRequest::new(
        host.document(),
        id,
        "Fence",
        BTreeMap::new(),
        Default::default(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("path"));
    assert_eq!(host.document(), &before);
    let file = ProjectFile::new(host.document().clone()).unwrap();
    let mut bytes = Vec::new();
    project_file::write(&file, &mut bytes, Default::default()).unwrap();
    let caps = project_file::supported_capabilities();
    let mut old = caps.clone();
    old.remove(super::CAPABILITY);
    assert!(project_file::read(&mut bytes.as_slice(), Default::default(), &old).is_err());
    let opened = project_file::read(&mut bytes.as_slice(), Default::default(), &caps).unwrap();
    let read = super::super::read_definition(&opened.project.document, id).unwrap();
    assert_eq!(read.pin().unwrap(), definition.pin().unwrap());
    assert_eq!(read, definition);
}
#[test]
fn typed_fence_remapping_keeps_bindings_and_uuid_looking_labels_literal() {
    let (mut session, blueprint, width, literal) = fence();
    let definition = author(&mut session, &blueprint, vec![width]);
    let identities = super::super::remap::identities(&definition).unwrap();
    assert!(!identities.contains(&EntityId::parse(&literal).unwrap()));
    assert!(!identities.contains(&EntityId::parse(&blueprint.dimensions[0].name).unwrap()));
    let map = identities
        .into_iter()
        .map(|id| (id, EntityId::new()))
        .collect::<BTreeMap<_, _>>();
    let entities = definition
        .template
        .entities()
        .map(|e| Mutation::Insert(super::super::remap::remap(e, &map).unwrap()))
        .collect::<Vec<_>>();
    let mapped = Document::new("Private source")
        .unwrap()
        .edited(&entities)
        .unwrap();
    let b = read_blueprint(&mapped, map[&definition.root]).unwrap();
    assert_eq!(b.dimensions[0].name, blueprint.dimensions[0].name);
    assert_eq!(b.dimensions[0].id, map[&blueprint.dimensions[0].id]);
    let construction = read_fence_type(&mapped, b.construction).unwrap();
    assert_eq!(construction.infills[0].name, literal);
    assert_eq!(
        construction.infills[0].id,
        map[&read_fence_type(&definition.template, blueprint.construction)
            .unwrap()
            .infills[0]
            .id]
    );
    let FencePostProfile::Rectangular {
        width: Dimension::Expression { expression },
        ..
    } = &construction.post
    else {
        panic!("retained bound post width")
    };
    assert_eq!(expression.text, "=$post_width");
    assert_eq!(expression.bindings[0].target, map[&width]);
    let parameters = crate::parameters::evaluate(&mapped).unwrap();
    assert_eq!(
        construction
            .resolve(&mapped, &parameters)
            .unwrap()
            .preferred_post_spacing_mm,
        2000.
    );
}
#[test]
fn wall_layer_targets_and_boundary_choices_remap_without_changing_labels() {
    let (mut session, blueprint, ids) = wall();
    let definition = author(&mut session, &blueprint, vec![]);
    let identities = super::super::remap::identities(&definition).unwrap();
    let original =
        match read_wall_construction(&definition.template, blueprint.construction).unwrap() {
            WallTypeConstruction::Layered(w) => w,
            _ => unreachable!(),
        };
    assert!(!identities.contains(&EntityId::parse(&original.layers.layers[0].name).unwrap()));
    let map = identities
        .into_iter()
        .map(|id| (id, EntityId::new()))
        .collect::<BTreeMap<_, _>>();
    let entities = definition
        .template
        .entities()
        .map(|e| Mutation::Insert(super::super::remap::remap(e, &map).unwrap()))
        .collect::<Vec<_>>();
    let mapped = Document::new("Private layers")
        .unwrap()
        .edited(&entities)
        .unwrap();
    let b = read_blueprint(&mapped, map[&definition.root]).unwrap();
    assert_eq!(
        b.dimensions[0].target,
        ArchitectureDimensionTarget::WallLayerThickness {
            layer: map[&ids[0]]
        }
    );
    assert_eq!(
        b.options[0].choices[1],
        ArchitectureOptionValue::WallReference {
            reference: WallLayerReference::Boundary {
                layer: map[&ids[0]],
                side: WallLayerSide::Inside
            }
        }
    );
    let ArchitectureConstruction::Wall(value) =
        ArchitectureConstruction::read(&mapped, b.role, b.construction).unwrap()
    else {
        unreachable!()
    };
    assert_eq!(value.layers.layers[0].name, original.layers.layers[0].name);
    assert_eq!(value.layers.layers[0].role, original.layers.layers[0].role);
    assert_eq!(value.layers.layers[0].id, map[&ids[0]]);
    value
        .resolve_layers(
            &mapped,
            &crate::parameters::evaluate(&mapped).unwrap(),
            &Default::default(),
        )
        .unwrap();
}
#[test]
fn unsupported_roles_targets_and_choices_fail_without_staging_source() {
    let (session, blueprint, _, _) = fence();
    let before = session.document().clone();
    let mut bad = blueprint.clone();
    bad.role = ArchitectureRole::Wall;
    assert!(prepare_blueprint(&session, session.revision(), actor(), "Wrong role", &bad).is_err());
    let mut bad = blueprint.clone();
    bad.dimensions[0].target = ArchitectureDimensionTarget::FencePostDiameter;
    assert!(bad.validate(session.document()).is_err());
    let mut bad = blueprint.clone();
    bad.options[0].choices = vec![ArchitectureOptionValue::WallReference {
        reference: WallLayerReference::Center,
    }];
    assert!(bad.validate(session.document()).is_err());
    let mut bad = blueprint.clone();
    bad.dimensions.push(bad.dimensions[0].clone());
    assert!(bad.validate(session.document()).is_err());
    let mut bad = blueprint.clone();
    bad.materials[0].choices = vec![blueprint.construction];
    assert!(bad.validate(session.document()).is_err());
    let mut bad = serde_json::to_value(blueprint).unwrap();
    bad["receiving_path"] = serde_json::json!([[0, 0], [1000, 0]]);
    assert!(serde_json::from_value::<ArchitectureBlueprint>(bad).is_err());
    assert_eq!(session.document(), &before);
    let (session, mut blueprint, ids) = wall();
    blueprint.dimensions[0].target =
        ArchitectureDimensionTarget::WallLayerThickness { layer: ids[2] };
    assert!(blueprint.validate(session.document()).is_err());
    blueprint.dimensions[0].target = ArchitectureDimensionTarget::WallHeight;
    blueprint.materials[0].target = ArchitectureMaterialTarget::WallLayer { layer: ids[1] };
    assert!(blueprint.validate(session.document()).is_err());
}
#[test]
fn blueprint_definition_requires_dynamic_used_exposures_and_path_derived_plan() {
    let (mut session, blueprint, width, _) = fence();
    let definition = author(&mut session, &blueprint, vec![width]);
    let mut bad = definition.clone();
    bad.kind = SymbolKind::Static;
    assert!(bad.validate().is_err());
    let unused = parameter("unused_input", "100");
    let mut bad = definition.clone();
    bad.template = bad
        .template
        .edited(&[Mutation::Insert(unused.clone())])
        .unwrap();
    bad.exposed.push(unused.id);
    assert!(bad.validate().unwrap_err().to_string().contains("used by"));
    let mut bad = definition;
    bad.plan = Some(SymbolPlan {
        sketch: EntityId::new(),
    });
    assert!(
        bad.validate()
            .unwrap_err()
            .to_string()
            .contains("fixed symbol plan")
    );
}
#[test]
fn ordinary_symbol_pin_and_legacy_file_capabilities_remain_unchanged() {
    let mut session = Session::new(Document::new("Ordinary symbol").unwrap()).unwrap();
    let request = PrimitiveRequest::new(
        "Box",
        PrimitiveParameters::Box {
            size: [length(80.), length(20.), length(10.)],
            frame: Default::default(),
        },
    );
    let p = prepare_primitive(&session, 0, actor(), &request).unwrap();
    session.commit(p.prepared).unwrap();
    let request = SymbolAuthorRequest::new(
        "Box definition",
        request.body,
        SymbolKind::Dynamic,
        vec![],
        None,
    );
    let p = super::super::prepare_author(&session, session.revision(), actor(), &request).unwrap();
    session.commit(p.prepared).unwrap();
    let definition = super::super::read_definition(session.document(), request.definition).unwrap();
    let pin = definition.pin().unwrap();
    assert!(blueprint_for_definition(&definition).unwrap().is_none());
    assert!(!required_capability(session.document()));
    let file = ProjectFile::new(session.document().clone()).unwrap();
    let mut bytes = vec![];
    project_file::write(&file, &mut bytes, Default::default()).unwrap();
    let mut old = project_file::supported_capabilities();
    old.remove(super::CAPABILITY);
    let read = project_file::read(&mut bytes.as_slice(), Default::default(), &old).unwrap();
    assert_eq!(
        super::super::read_definition(&read.project.document, request.definition)
            .unwrap()
            .pin()
            .unwrap(),
        pin
    );
    SymbolPlaceRequest::new(
        session.document(),
        request.definition,
        "Box placed",
        BTreeMap::new(),
        Default::default(),
    )
    .unwrap();
}

#[test]
fn blueprint_exception_never_accepts_nested_symbols_or_opaque_recursive_payloads() {
    let (mut session, blueprint, width, _) = fence();
    let definition = author(&mut session, &blueprint, vec![width]);
    for forbidden in [
        super::super::DEFINITION_TYPE,
        super::super::INSTANCE_TYPE,
        "symbols.unreviewed_provider",
    ] {
        let mut wire = serde_json::to_value(&definition).unwrap();
        let nested = Entity::new(
            "Nested source",
            EntityData::Extension {
                definition: Definition {
                    type_id: forbidden.into(),
                    schema_version: 1,
                    data: serde_json::json!({}),
                },
                inputs: vec![],
            },
        );
        wire["template"]["entities"]
            .as_object_mut()
            .unwrap()
            .insert(nested.id.to_string(), serde_json::to_value(nested).unwrap());
        let container = Entity::new(
            "Untrusted definition",
            EntityData::Extension {
                definition: Definition {
                    type_id: super::super::DEFINITION_TYPE.into(),
                    schema_version: 1,
                    data: wire,
                },
                inputs: vec![],
            },
        );
        let error = Document::new("Consumer")
            .unwrap()
            .edited(&[Mutation::Insert(container)])
            .unwrap_err();
        assert!(
            error.to_string().contains("Nested embedded symbol"),
            "{forbidden}: {error}"
        );
    }
    let mut wire = serde_json::to_value(&definition).unwrap();
    let root = definition.root.to_string();
    wire["template"]["entities"][&root]["data"]["definition"]["data"]["template"] =
        serde_json::json!({"unexpected_nested_source":true});
    let container = Entity::new(
        "Malformed blueprint",
        EntityData::Extension {
            definition: Definition {
                type_id: super::super::DEFINITION_TYPE.into(),
                schema_version: 1,
                data: wire,
            },
            inputs: vec![],
        },
    );
    let error = Document::new("Consumer")
        .unwrap()
        .edited(&[Mutation::Insert(container)])
        .unwrap_err();
    assert!(error.to_string().contains("unknown field"), "{error}");
}

fn private_map(definition: &SymbolDefinition) -> BTreeMap<EntityId, EntityId> {
    super::super::remap::identities(definition)
        .unwrap()
        .into_iter()
        .map(|id| (id, EntityId::new()))
        .collect()
}
fn configured_document(
    definition: &SymbolDefinition,
    ids: &BTreeMap<EntityId, EntityId>,
    values: &ArchitectureInputs,
    receiving: &Document,
) -> Document {
    let entities = configured_source(definition, ids, values, receiving).unwrap();
    receiving
        .edited(
            &entities
                .into_values()
                .map(Mutation::Insert)
                .collect::<Vec<_>>(),
        )
        .unwrap()
}
#[test]
fn exposed_inputs_change_only_declared_private_fields_and_preserve_the_pin() {
    let (mut session, blueprint, width, _) = fence();
    let definition = author(&mut session, &blueprint, vec![width]);
    let pin = definition.pin().unwrap();
    let ids = private_map(&definition);
    let mut values = ArchitectureInputs::default();
    values.parameters.insert(
        width,
        SymbolValue {
            value: 10.,
            unit: "cm".into(),
        },
    );
    values.dimensions.insert(
        blueprint.dimensions[0].id,
        Dimension::Literal {
            value: 2.,
            unit: "m".into(),
        },
    );
    values.materials.insert(
        blueprint.materials[0].id,
        ArchitectureMaterialValue::Preset {
            material: blueprint.materials[0].choices[0],
        },
    );
    values.options.insert(
        blueprint.options[0].id,
        ArchitectureOptionValue::FenceLayout {
            layout: FenceBayLayout::Equal,
        },
    );
    let document = configured_document(&definition, &ids, &values, session.document());
    let construction = read_fence_type(&document, ids[&blueprint.construction]).unwrap();
    let parameters = crate::parameters::evaluate(&document).unwrap();
    let resolved = construction.resolve(&document, &parameters).unwrap();
    assert_eq!(resolved.height_mm, 2000.);
    assert_eq!(resolved.layout, FenceBayLayout::Equal);
    assert_eq!(
        resolved.post,
        ResolvedFencePostProfile::Rectangular {
            width_mm: 100.,
            depth_mm: 90.
        }
    );
    assert_eq!(
        construction.post_material,
        ids[&blueprint.materials[0].choices[0]]
    );
    assert_eq!(construction.infills[0].height, length(1600.));
    let original = read_fence_type(&definition.template, blueprint.construction).unwrap();
    assert_eq!(
        construction.infills[0].material,
        ids[&original.infills[0].material]
    );
    assert!(matches!(
        &document.get(ids[&width]).unwrap().data,
        EntityData::Parameter { exposed: false, .. }
    ));
    assert_eq!(definition.pin().unwrap(), pin);
    assert_eq!(
        read_fence_type(session.document(), blueprint.construction).unwrap(),
        original
    );
}
#[test]
fn receiving_formulas_and_materials_keep_receiving_identities_and_disjoint_private_maps() {
    let (mut session, blueprint, width, _) = fence();
    let definition = author(&mut session, &blueprint, vec![width]);
    let height = parameter("receiving_height", "2100");
    let paint = material("Receiving blue", [30, 90, 180, 255]);
    let receiving = Document::new("Receiving project")
        .unwrap()
        .edited(&[
            Mutation::Insert(height.clone()),
            Mutation::Insert(paint.clone()),
        ])
        .unwrap();
    let mut values = ArchitectureInputs::default();
    values.dimensions.insert(
        blueprint.dimensions[0].id,
        Dimension::Expression {
            expression: BoundExpression::bind(&receiving, "=$receiving_height", "mm", None)
                .unwrap(),
        },
    );
    values.materials.insert(
        blueprint.materials[0].id,
        ArchitectureMaterialValue::Project { material: paint.id },
    );
    let one = private_map(&definition);
    let two = private_map(&definition);
    assert!(
        one.values()
            .all(|id| !two.values().any(|other| other == id))
    );
    let document = configured_document(&definition, &one, &values, &receiving);
    let document =
        configured_document(&definition, &two, &ArchitectureInputs::default(), &document);
    let parameters = crate::parameters::evaluate(&document).unwrap();
    let a = read_fence_type(&document, one[&blueprint.construction]).unwrap();
    let b = read_fence_type(&document, two[&blueprint.construction]).unwrap();
    assert_eq!(a.resolve(&document, &parameters).unwrap().height_mm, 2100.);
    assert_eq!(b.resolve(&document, &parameters).unwrap().height_mm, 1800.);
    assert_eq!(a.post_material, paint.id);
    assert_ne!(b.post_material, paint.id);
    assert_eq!(a.height.expression().unwrap().bindings[0].target, height.id);
    assert!(a.inputs().iter().any(|l| l.target == height.id));
    assert!(a.inputs().iter().any(|l| l.target == paint.id));
    assert_eq!(receiving.get(paint.id).unwrap(), &paint);
}
#[test]
fn private_unknown_wrong_unit_and_unapproved_input_values_are_refused() {
    let (mut session, blueprint, width, _) = fence();
    let definition = author(&mut session, &blueprint, vec![width]);
    let ids = private_map(&definition);
    let before = session.document().clone();
    let pin = definition.pin().unwrap();
    let mut cases = vec![];
    let mut value = ArchitectureInputs::default();
    value.parameters.insert(
        EntityId::new(),
        SymbolValue {
            value: 2.,
            unit: "mm".into(),
        },
    );
    cases.push(value);
    let mut value = ArchitectureInputs::default();
    value.parameters.insert(
        width,
        SymbolValue {
            value: 2.,
            unit: "deg".into(),
        },
    );
    cases.push(value);
    let mut value = ArchitectureInputs::default();
    value.dimensions.insert(EntityId::new(), length(2000.));
    cases.push(value);
    let mut value = ArchitectureInputs::default();
    value.dimensions.insert(
        blueprint.dimensions[0].id,
        Dimension::Literal {
            value: 2.,
            unit: "deg".into(),
        },
    );
    cases.push(value);
    let mut value = ArchitectureInputs::default();
    value.materials.insert(
        blueprint.materials[0].id,
        ArchitectureMaterialValue::Preset { material: width },
    );
    cases.push(value);
    let mut value = ArchitectureInputs::default();
    value.materials.insert(
        blueprint.materials[0].id,
        ArchitectureMaterialValue::Project { material: width },
    );
    cases.push(value);
    let mut value = ArchitectureInputs::default();
    value.options.insert(
        blueprint.options[0].id,
        ArchitectureOptionValue::WallReference {
            reference: WallLayerReference::Center,
        },
    );
    cases.push(value);
    for values in cases {
        assert!(configured_source(&definition, &ids, &values, session.document()).is_err());
        assert_eq!(session.document(), &before);
        assert_eq!(definition.pin().unwrap(), pin);
    }
    let mut malformed = ids.clone();
    let keys = malformed.keys().copied().take(2).collect::<Vec<_>>();
    malformed.insert(keys[0], malformed[&keys[1]]);
    assert!(
        configured_source(
            &definition,
            &malformed,
            &Default::default(),
            session.document()
        )
        .is_err()
    );
    let mut restricted = definition.clone();
    let mut root = restricted.template.get(definition.root).unwrap().clone();
    let mut restricted_blueprint = blueprint.clone();
    restricted_blueprint.materials[0].allow_project_materials = false;
    root.data = EntityData::Extension {
        definition: restricted_blueprint.definition().unwrap(),
        inputs: restricted_blueprint.inputs(),
    };
    restricted.template = restricted
        .template
        .edited(&[Mutation::Replace(root)])
        .unwrap();
    let original = read_fence_type(session.document(), blueprint.construction).unwrap();
    let mut values = ArchitectureInputs::default();
    values.materials.insert(
        blueprint.materials[0].id,
        ArchitectureMaterialValue::Project {
            material: original.post_material,
        },
    );
    assert!(
        configured_source(&restricted, &ids, &values, session.document())
            .unwrap_err()
            .to_string()
            .contains("private")
    );
}
#[test]
fn configured_wall_layers_retain_private_air_and_map_authored_boundary_choices() {
    let (mut session, blueprint, layers) = wall();
    let definition = author(&mut session, &blueprint, vec![]);
    let ids = private_map(&definition);
    let mut values = ArchitectureInputs::default();
    values
        .dimensions
        .insert(blueprint.dimensions[0].id, length(250.));
    values.materials.insert(
        blueprint.materials[0].id,
        ArchitectureMaterialValue::Preset {
            material: blueprint.materials[0].choices[0],
        },
    );
    values.options.insert(
        blueprint.options[0].id,
        blueprint.options[0].choices[1].clone(),
    );
    let document = configured_document(&definition, &ids, &values, session.document());
    let WallTypeConstruction::Layered(value) =
        read_wall_construction(&document, ids[&blueprint.construction]).unwrap()
    else {
        unreachable!()
    };
    let parameters = crate::parameters::evaluate(&document).unwrap();
    let resolved = value
        .resolve_layers(&document, &parameters, &Default::default())
        .unwrap();
    assert_eq!(resolved.stack.thickness_mm, 290.);
    assert_eq!(
        value.layers.layers[1].construction,
        WallLayerKind::Cavity {
            thickness: length(40.)
        }
    );
    assert_eq!(
        value.reference,
        WallLayerReference::Boundary {
            layer: ids[&layers[0]],
            side: WallLayerSide::Inside
        }
    );
    assert_eq!(
        value.layers.layers[2].construction.material(),
        Some(ids[&blueprint.materials[0].choices[0]])
    );
    let WallTypeConstruction::Layered(original) =
        read_wall_construction(session.document(), blueprint.construction).unwrap()
    else {
        unreachable!()
    };
    assert_eq!(
        original.layers.layers[0].construction.thickness(),
        Some(&length(120.))
    );
}

fn consumer(definition: &SymbolDefinition) -> (Session, EntityId) {
    let mut host = Session::new(Document::new("Consumer").unwrap()).unwrap();
    let prepared = super::super::prepare_import(&host, 0, actor(), definition).unwrap();
    let id = prepared.definition;
    host.commit(prepared.prepared).unwrap();
    (host, id)
}
fn fence_context(end: f64) -> ArchitectureContext {
    ArchitectureContext::Fence {
        path: FencePath {
            vertices: [[0., 0.], [end, 0.]]
                .into_iter()
                .map(|point| FenceVertex {
                    id: EntityId::new(),
                    point: point.map(length),
                })
                .collect(),
            closed: false,
            base_z: length(0.),
        },
        level: None,
        post_positions: vec![],
    }
}
fn wall_context(end: f64) -> ArchitectureContext {
    ArchitectureContext::Wall {
        path: vec![[length(0.), length(0.)], [length(end), length(0.)]],
        closed: false,
        level: None,
        base_z: length(0.),
        flipped: false,
    }
}
#[test]
fn two_architectural_fences_have_real_independent_runs_private_sources_and_offline_pins() {
    let (mut authoring, blueprint, width, _) = fence();
    let definition = author(&mut authoring, &blueprint, vec![width]);
    let pin = definition.pin().unwrap();
    let (mut host, id) = consumer(&definition);
    let mut requests = vec![];
    for (length, height, count) in [(4500., 2000., 7), (6500., 2200., 9)] {
        let mut inputs = ArchitectureInputs::default();
        inputs
            .dimensions
            .insert(blueprint.dimensions[0].id, model::length(height));
        let request = ArchitecturePlaceRequest::new(
            host.document(),
            id,
            "Boundary",
            inputs,
            fence_context(length),
        )
        .unwrap();
        let before = host.document().clone();
        let cancelled =
            prepare_architecture_place(&host, host.revision(), actor(), &request).unwrap();
        assert_eq!(host.document(), &before);
        drop(cancelled);
        let prepared =
            prepare_architecture_place(&host, host.revision(), actor(), &request).unwrap();
        let ArchitectureOutput::Fence { run, group } = request.output else {
            unreachable!()
        };
        assert_eq!(
            fence_members(prepared.prepared.document(), run)
                .unwrap()
                .len(),
            count
        );
        host.commit(prepared.prepared).unwrap();
        let record = read_architecture_instance(host.document(), request.record).unwrap();
        assert_eq!(record.pin, pin);
        let run = read_fence_run(host.document(), run).unwrap();
        assert_eq!(run.group, group);
        assert_eq!(run.fence_type, request.ids[&blueprint.construction]);
        let layout = resolve_fence_run(host.document(), host.parameters(), &run).unwrap();
        assert_eq!(layout.construction.height_mm, height);
        assert!(
            super::super::owned_entities_in_session(&host)
                .unwrap()
                .contains(&request.ids[&width])
        );
        assert!(
            !super::super::owned_entities_in_session(&host)
                .unwrap()
                .contains(&group)
        );
        requests.push(request);
    }
    assert!(
        requests[0]
            .ids
            .values()
            .all(|id| !requests[1].ids.values().any(|other| id == other))
    );
    let saved = host.document().clone();
    let mut bytes = Vec::new();
    project_file::write(
        &ProjectFile::new(saved.clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    drop(authoring);
    drop(definition);
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    let reopened = Session::new(reopened.project.document).unwrap();
    assert_eq!(reopened.document(), &saved);
    assert_eq!(
        architecture_instances(reopened.document(), id)
            .unwrap()
            .len(),
        2
    );
    let mut private = host
        .document()
        .get(requests[0].ids[&blueprint.construction])
        .unwrap()
        .clone();
    if let EntityData::Extension { definition, .. } = &mut private.data {
        definition.data["height"] = serde_json::to_value(length(9999.)).unwrap();
    }
    assert!(
        host.document()
            .edited(&[Mutation::Replace(private)])
            .is_err()
    );
    host.undo(host.revision(), actor()).unwrap();
    assert_eq!(
        architecture_instances(host.document(), id).unwrap().len(),
        1
    );
    host.redo(host.revision(), actor()).unwrap();
    assert_eq!(host.document(), &saved);
}
#[test]
fn real_layered_wall_context_can_change_while_private_construction_stays_protected() {
    let (mut authoring, mut blueprint, _) = wall();
    let height = EntityId::new();
    blueprint.dimensions.push(ArchitectureDimensionSlot {
        id: height,
        name: "Height".into(),
        target: ArchitectureDimensionTarget::WallHeight,
    });
    let definition = author(&mut authoring, &blueprint, vec![]);
    let (mut host, id) = consumer(&definition);
    let mut requests = vec![];
    for (end, high) in [(3000., 2500.), (6000., 3100.)] {
        let mut inputs = ArchitectureInputs::default();
        inputs.dimensions.insert(height, length(high));
        let request = ArchitecturePlaceRequest::new(
            host.document(),
            id,
            "Layered wall",
            inputs,
            wall_context(end),
        )
        .unwrap();
        let prepared =
            prepare_architecture_place(&host, host.revision(), actor(), &request).unwrap();
        host.commit(prepared.prepared).unwrap();
        let ArchitectureOutput::Wall { feature, body } = request.output else {
            unreachable!()
        };
        let wall = read_wall(host.document(), feature).unwrap();
        assert_eq!(wall.output.body, body);
        let construction = read_wall_construction(host.document(), wall.wall_type).unwrap();
        let WallTypeConstruction::Layered(construction) = construction else {
            unreachable!()
        };
        assert_eq!(construction.height, length(high));
        assert_eq!(
            construction.layers.layers[0].construction,
            WallLayerKind::Solid {
                thickness: length(120.),
                material: request.ids[&definition
                    .template
                    .entities()
                    .find(|e| matches!(e.data, EntityData::Material { .. }) && e.name == "Brick")
                    .unwrap()
                    .id]
            }
        );
        let mut context = wall.clone();
        context.path[1][0] = length(end + 500.);
        context.base_z = length(150.);
        let prepared =
            prepare_wall_edit(&host, host.revision(), actor(), feature, &context).unwrap();
        host.commit(prepared.prepared).unwrap();
        assert_eq!(read_wall(host.document(), feature).unwrap(), context);
        context.height = Some(length(9000.));
        assert!(prepare_wall_edit(&host, host.revision(), actor(), feature, &context).is_err());
        requests.push(request);
    }
    assert_eq!(
        architecture_instances(host.document(), id).unwrap().len(),
        2
    );
    let mut invalid = ArchitecturePlaceRequest::new(
        host.document(),
        id,
        "Wall",
        ArchitectureInputs::default(),
        wall_context(3000.),
    )
    .unwrap();
    invalid.ids = requests[0].ids.clone();
    assert!(prepare_architecture_place(&host, host.revision(), actor(), &invalid).is_err());
    assert!(
        ArchitecturePlaceRequest::new(
            host.document(),
            id,
            "Wrong role",
            ArchitectureInputs::default(),
            fence_context(3000.)
        )
        .is_err()
    );
}

#[test]
fn architectural_pin_never_hides_physical_sample_geometry_from_native_validation() {
    let (mut session, blueprint, width, _) = fence();
    let mut definition = author(&mut session, &blueprint, vec![width]);
    let extra = Entity::new(
        "Stray sample",
        EntityData::Part {
            features: vec![],
            material: None,
            tags: Default::default(),
        },
    );
    definition.template = definition
        .template
        .edited(&[Mutation::Insert(extra)])
        .unwrap();
    assert!(
        definition
            .validate()
            .unwrap_err()
            .to_string()
            .contains("canonical construction source")
    );
}

#[test]
fn exposed_architectural_edits_reconcile_members_preserve_context_and_cancel_atomically() {
    let (mut authoring, mut blueprint, width, _) = fence();
    let spacing = EntityId::new();
    blueprint.dimensions.push(ArchitectureDimensionSlot {
        id: spacing,
        name: "Spacing".into(),
        target: ArchitectureDimensionTarget::FenceSpacing,
    });
    let definition = author(&mut authoring, &blueprint, vec![width]);
    let (mut host, id) = consumer(&definition);
    let request = ArchitecturePlaceRequest::new(
        host.document(),
        id,
        "Boundary",
        ArchitectureInputs::default(),
        fence_context(4500.),
    )
    .unwrap();
    let placed = prepare_architecture_place(&host, host.revision(), actor(), &request).unwrap();
    host.commit(placed.prepared).unwrap();
    let ArchitectureOutput::Fence { run, .. } = request.output else {
        unreachable!()
    };
    let before = host.document().clone();
    let previous = read_fence_run(&before, run).unwrap();
    let old: BTreeMap<_, _> = fence_members(&before, run)
        .unwrap()
        .into_iter()
        .map(|(k, f, b)| (k, (f, b)))
        .collect();
    let mut inputs = ArchitectureInputs::default();
    inputs.dimensions.insert(spacing, length(1000.));
    inputs
        .dimensions
        .insert(blueprint.dimensions[0].id, length(2400.));
    let edit = ArchitectureEditRequest {
        record: request.record,
        inputs,
    };
    let cancelled = prepare_architecture_edit(&host, host.revision(), actor(), &edit).unwrap();
    assert_eq!(
        fence_members(cancelled.prepared.document(), run)
            .unwrap()
            .len(),
        11
    );
    assert_eq!(host.document(), &before);
    drop(cancelled);
    let changed = prepare_architecture_edit(&host, host.revision(), actor(), &edit).unwrap();
    assert_eq!(
        read_fence_run(changed.prepared.document(), run).unwrap(),
        previous
    );
    let record = read_architecture_instance(changed.prepared.document(), request.record).unwrap();
    assert_eq!(record.ids, request.ids);
    assert_eq!(record.pin, definition.pin().unwrap());
    let members = fence_members(changed.prepared.document(), run).unwrap();
    for (key, feature, body) in &members {
        if let Some(old) = old.get(key) {
            assert_eq!((*feature, *body), *old);
        }
    }
    host.commit(changed.prepared).unwrap();
    let after = host.document().clone();
    host.undo(host.revision(), actor()).unwrap();
    assert_eq!(host.document(), &before);
    host.redo(host.revision(), actor()).unwrap();
    assert_eq!(host.document(), &after);
    let mut invalid = edit.clone();
    invalid
        .inputs
        .dimensions
        .insert(EntityId::new(), length(1000.));
    assert!(prepare_architecture_edit(&host, host.revision(), actor(), &invalid).is_err());
    assert!(prepare_architecture_edit(&host, 0, actor(), &edit).is_err());
    assert_eq!(host.document(), &after);
    let mut raw = host.document().get(run).unwrap().clone();
    let mut source = read_fence_run(host.document(), run).unwrap();
    source.overrides.post_height = Some(length(9999.));
    raw.data = source.entity_data().unwrap();
    assert!(host.document().edited(&[Mutation::Replace(raw)]).is_err());
}
