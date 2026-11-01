use multicad_core::{
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Mutation},
    model::{WallLayer, WallLayerKind, WallLayerReference, WallLayerSide, WallLayerStack, length},
    parameters::{self, BoundExpression},
};

fn fixture() -> (Document, WallLayerStack) {
    let material = Entity::new(
        "Brick / finish",
        EntityData::Material {
            density_kg_m3: None,
            color_rgba: [160, 100, 80, 255],
            section_hatch: None,
        },
    );
    let document = Document::new("Layer construction")
        .unwrap()
        .edited(&[Mutation::Insert(material.clone())])
        .unwrap();
    let mut layers = vec![WallLayer {
        id: EntityId::new(),
        name: "Exterior paint".into(),
        role: None,
        construction: WallLayerKind::Coating {
            material: material.id,
        },
    }];
    for (name, width) in [
        ("Insulation", 80.),
        ("Brick", 120.),
        ("Second insulation", 40.),
        ("Second brick", 100.),
        ("Interior lining", 12.5),
    ] {
        layers.push(WallLayer {
            id: EntityId::new(),
            name: name.into(),
            role: Some("User role".into()),
            construction: WallLayerKind::Solid {
                thickness: length(width),
                material: material.id,
            },
        });
    }
    (document, WallLayerStack { layers })
}
fn resolved(
    document: &Document,
    stack: &WallLayerStack,
) -> multicad_core::model::ResolvedWallLayerStack {
    stack
        .resolve(document, &parameters::evaluate(document).unwrap())
        .unwrap()
}

#[test]
fn requested_layer_order_keeps_repeated_materials_and_has_no_independent_total() {
    let (document, stack) = fixture();
    let result = resolved(&document, &stack);
    assert_eq!(result.thickness_mm, 352.5);
    assert_eq!(result.layers.len(), 6);
    assert_eq!(result.layers[0].thickness_mm, 0.);
    assert!(!result.layers[0].solid);
    let encoded = serde_json::to_value(&stack).unwrap();
    assert!(encoded.get("thickness").is_none());
    assert!(encoded.get("total_thickness").is_none());
    assert_eq!(
        serde_json::from_value::<WallLayerStack>(encoded.clone()).unwrap(),
        stack
    );
    let mut conflicting = encoded;
    conflicting["thickness"] = serde_json::json!(400.);
    assert!(serde_json::from_value::<WallLayerStack>(conflicting).is_err());
    assert_eq!(
        stack.inputs().len(),
        6,
        "Repeated materials retain per-layer canonical links"
    );
    assert!(
        stack
            .inputs()
            .windows(2)
            .all(|w| w[0].target == w[1].target && w[0].role != w[1].role)
    );
}

#[test]
fn cavities_occupy_real_width_without_acquiring_a_material_or_solid() {
    let (document, mut stack) = fixture();
    stack.layers.insert(
        3,
        WallLayer {
            id: EntityId::new(),
            name: "Air cavity".into(),
            role: None,
            construction: WallLayerKind::Cavity {
                thickness: length(30.),
            },
        },
    );
    let result = resolved(&document, &stack);
    assert_eq!(result.thickness_mm, 382.5);
    assert_eq!(result.layers[3].thickness_mm, 30.);
    assert_eq!(result.layers[3].material, None);
    assert!(!result.layers[3].solid);
    let mut encoded = serde_json::to_value(&stack).unwrap();
    encoded["layers"][3]["construction"]["material"] = serde_json::json!(EntityId::new());
    assert!(serde_json::from_value::<WallLayerStack>(encoded).is_err());
}

#[test]
fn reference_boundaries_and_flip_preserve_contiguous_physical_bands_and_layer_order() {
    let (document, stack) = fixture();
    let result = resolved(&document, &stack);
    let reference = WallLayerReference::Boundary {
        layer: stack.layers[2].id,
        side: WallLayerSide::Outside,
    };
    let original = result.bands(reference, false).unwrap();
    let flipped = result.bands(reference, true).unwrap();
    assert_eq!(original[2].outside_mm, 0.);
    assert_eq!(original[0].outside_mm, 80.);
    for i in 0..original.len() {
        assert_eq!(original[i].layer, stack.layers[i].id);
        assert_eq!(flipped[i].layer, original[i].layer);
        assert_eq!(flipped[i].outside_mm, -original[i].outside_mm);
        assert_eq!(flipped[i].inside_mm, -original[i].inside_mm);
        assert_eq!(
            (original[i].outside_mm - original[i].inside_mm).abs(),
            result.layers[i].thickness_mm
        );
        if i > 0 {
            assert_eq!(original[i - 1].inside_mm, original[i].outside_mm);
        }
    }
    for (reference, outside, inside) in [
        (WallLayerReference::Outside, 0., -352.5),
        (WallLayerReference::Center, 176.25, -176.25),
        (WallLayerReference::Inside, 352.5, 0.),
    ] {
        let bands = result.bands(reference, false).unwrap();
        assert_eq!(bands.first().unwrap().outside_mm, outside);
        assert_eq!(bands.last().unwrap().inside_mm, inside);
    }
    let inside = result
        .bands(
            WallLayerReference::Boundary {
                layer: stack.layers[2].id,
                side: WallLayerSide::Inside,
            },
            false,
        )
        .unwrap();
    assert_eq!(inside[2].inside_mm, 0.);
}

#[test]
fn reorder_duplicate_and_remove_preserve_identity_or_require_reference_repair() {
    let (document, mut stack) = fixture();
    let anchor = stack.layers[2].id;
    let reference = WallLayerReference::Boundary {
        layer: anchor,
        side: WallLayerSide::Outside,
    };
    let before = resolved(&document, &stack);
    stack.layers.swap(1, 2);
    assert_eq!(
        resolved(&document, &stack).bands(reference, false).unwrap()[1].outside_mm,
        0.
    );
    assert_eq!(
        resolved(&document, &stack).thickness_mm,
        before.thickness_mm
    );
    let mut duplicate = stack.layers[1].clone();
    duplicate.id = EntityId::new();
    stack.layers.push(duplicate);
    assert_eq!(resolved(&document, &stack).thickness_mm, 472.5);
    stack.layers.retain(|l| l.id != anchor);
    assert!(
        resolved(&document, &stack)
            .bands(reference, false)
            .unwrap_err()
            .to_string()
            .contains("removed")
    );
}

#[test]
fn parameter_bound_thickness_follows_identity_after_rename_and_converts_units() {
    let (mut document, mut stack) = fixture();
    let parameter = Entity::new(
        "insulation",
        EntityData::Parameter {
            expression: "8".into(),
            unit: "cm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    document = document
        .edited(&[Mutation::Insert(parameter.clone())])
        .unwrap();
    if let WallLayerKind::Solid { thickness, .. } = &mut stack.layers[1].construction {
        *thickness = Dimension::Expression {
            expression: BoundExpression::bind(&document, "=$insulation", "mm", None).unwrap(),
        };
    }
    assert_eq!(resolved(&document, &stack).thickness_mm, 352.5);
    let mut renamed = parameter.clone();
    renamed.name = "renamed_insulation".into();
    if let EntityData::Parameter { expression, .. } = &mut renamed.data {
        *expression = "10".into();
    }
    document = document.edited(&[Mutation::Replace(renamed)]).unwrap();
    assert_eq!(resolved(&document, &stack).thickness_mm, 372.5);
    assert!(stack.inputs().iter().any(|l| l.target == parameter.id));
}

#[test]
fn invalid_identities_units_materials_and_nonpositive_finite_layers_are_rejected() {
    let (document, stack) = fixture();
    let mut invalid = stack.clone();
    invalid.layers[2].id = invalid.layers[1].id;
    assert!(invalid.validate().is_err());
    invalid = stack.clone();
    invalid.layers[0].id = EntityId::nil();
    assert!(invalid.validate().is_err());
    invalid = stack.clone();
    invalid.layers[1].name = " ".into();
    assert!(invalid.validate().is_err());
    for width in [0., -1., f64::INFINITY, 1e9, 1e10] {
        invalid = stack.clone();
        if let WallLayerKind::Solid { thickness, .. } = &mut invalid.layers[1].construction {
            *thickness = length(width);
        }
        assert!(
            invalid
                .resolve(&document, &parameters::evaluate(&document).unwrap())
                .is_err()
        );
    }
    invalid = stack.clone();
    if let WallLayerKind::Solid { thickness, .. } = &mut invalid.layers[1].construction {
        *thickness = Dimension::Literal {
            value: 1.,
            unit: "deg".into(),
        };
    }
    assert!(invalid.validate().is_err());
    invalid = stack.clone();
    invalid.layers[1].construction = WallLayerKind::Solid {
        thickness: length(10.),
        material: document.id(),
    };
    assert!(
        invalid
            .resolve(&document, &parameters::evaluate(&document).unwrap())
            .is_err()
    );
    invalid = stack.clone();
    invalid
        .layers
        .retain(|l| !matches!(l.construction, WallLayerKind::Solid { .. }));
    assert!(invalid.validate().is_err());
}

#[test]
fn finite_finishes_are_not_inflated_and_coatings_do_not_accept_a_solid_thickness() {
    let (document, mut stack) = fixture();
    if let WallLayerKind::Solid { thickness, .. } = &mut stack.layers[1].construction {
        *thickness = length(0.00001);
    }
    assert_eq!(resolved(&document, &stack).layers[1].thickness_mm, 0.00001);
    let mut encoded = serde_json::to_value(&stack).unwrap();
    encoded["layers"][0]["construction"]["thickness"] = serde_json::to_value(length(10.)).unwrap();
    assert!(serde_json::from_value::<WallLayerStack>(encoded).is_err());
}

#[test]
fn layered_types_use_the_existing_catalog_and_explicit_file_capability() {
    use multicad_core::{
        commands::{Actor, Channel, Session},
        model::{self, LayeredWallTypeDefinition, WallTypeConstruction},
        project_file,
    };
    let (document, stack) = fixture();
    let mut session = Session::new(document).unwrap();
    let original = session.document().clone();
    let value = LayeredWallTypeDefinition {
        reference: WallLayerReference::Boundary {
            layer: stack.layers[2].id,
            side: WallLayerSide::Outside,
        },
        layers: stack,
        height: length(2700.),
    };
    let actor = || Actor::local(Channel::Ui);
    let (id, preview) =
        model::prepare_layered_wall_type(&session, 0, actor(), "Exterior construction", &value)
            .unwrap();
    assert_eq!(
        session.document(),
        &original,
        "Preparing a type is source-neutral"
    );
    session.commit(preview).unwrap();
    assert_eq!(
        model::wall_types(session.document())
            .map(|e| e.id)
            .collect::<Vec<_>>(),
        vec![id]
    );
    assert_eq!(
        model::read_wall_construction(session.document(), id).unwrap(),
        WallTypeConstruction::Layered(value.clone())
    );
    assert!(
        model::read_wall_type(session.document(), id).is_err(),
        "A uniform-only reader must never flatten a layered construction"
    );
    let project = project_file::ProjectFile::new(session.document().clone()).unwrap();
    assert!(
        project
            .capabilities()
            .contains(model::LAYERED_WALL_TYPE_CAPABILITY)
    );
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let supported = project_file::supported_capabilities();
    let opened = project_file::read(&mut bytes.as_slice(), Default::default(), &supported).unwrap();
    assert_eq!(
        model::read_wall_construction(&opened.project.document, id).unwrap(),
        WallTypeConstruction::Layered(value)
    );
    let mut old = supported;
    old.remove(model::LAYERED_WALL_TYPE_CAPABILITY);
    assert!(project_file::read(&mut bytes.as_slice(), Default::default(), &old).is_err());
    session.undo(session.revision(), actor()).unwrap();
    assert!(model::wall_types(session.document()).next().is_none());
    session.redo(session.revision(), actor()).unwrap();
    assert_eq!(model::wall_types(session.document()).count(), 1);
}

#[test]
fn uniform_type_wire_format_remains_schema_one_without_layer_capabilities() {
    use multicad_core::{
        commands::{Actor, Channel, Session},
        model::{self, Justification, WallTypeConstruction, WallTypeDefinition},
    };
    let mut session = Session::new(Document::new("Legacy uniform wall").unwrap()).unwrap();
    let value = WallTypeDefinition {
        thickness: length(200.),
        height: length(2500.),
        justification: Justification::Left,
        material: None,
    };
    let (id, preview) = model::prepare_wall_type(
        &session,
        0,
        Actor::local(Channel::Ui),
        "Existing wall type",
        &value,
    )
    .unwrap();
    session.commit(preview).unwrap();
    let EntityData::Extension { definition, .. } = &session.document().get(id).unwrap().data else {
        panic!("Expected type extension")
    };
    assert_eq!(definition.schema_version, 1);
    assert_eq!(definition.data, serde_json::to_value(&value).unwrap());
    assert_eq!(
        model::read_wall_construction(session.document(), id).unwrap(),
        WallTypeConstruction::Uniform(value)
    );
    assert!(
        !model::required_capabilities(session.document())
            .contains(model::LAYERED_WALL_TYPE_CAPABILITY)
    );
}

#[test]
fn type_edits_keep_layer_identity_and_cancel_history_and_invalid_changes_are_atomic() {
    use multicad_core::{
        commands::{Actor, Channel, Session},
        model::{self, LayeredWallTypeDefinition, WallTypeConstruction},
    };
    let (document, stack) = fixture();
    let mut session = Session::new(document).unwrap();
    let actor = || Actor::local(Channel::Ui);
    let mut value = LayeredWallTypeDefinition {
        reference: WallLayerReference::Center,
        layers: stack,
        height: length(2700.),
    };
    let (id, preview) =
        model::prepare_layered_wall_type(&session, 0, actor(), "Editable wall type", &value)
            .unwrap();
    session.commit(preview).unwrap();
    let original = session.document().clone();
    let reference_ids = value.layers.layers.iter().map(|l| l.id).collect::<Vec<_>>();
    value.layers.layers.swap(1, 2);
    drop(model::prepare_layered_wall_type_edit(&session, 1, actor(), id, &value).unwrap());
    assert_eq!(session.document(), &original);
    let preview = model::prepare_layered_wall_type_edit(&session, 1, actor(), id, &value).unwrap();
    session.commit(preview).unwrap();
    assert_eq!(
        model::read_wall_construction(session.document(), id).unwrap(),
        WallTypeConstruction::Layered(value.clone())
    );
    let mut sorted = value.layers.layers.iter().map(|l| l.id).collect::<Vec<_>>();
    sorted.sort();
    let mut expected = reference_ids;
    expected.sort();
    assert_eq!(sorted, expected);
    session.undo(2, actor()).unwrap();
    assert_eq!(session.document(), &original);
    session.redo(3, actor()).unwrap();
    let before = session.document().clone();
    value.reference = WallLayerReference::Boundary {
        layer: EntityId::new(),
        side: WallLayerSide::Inside,
    };
    assert!(model::prepare_layered_wall_type_edit(&session, 4, actor(), id, &value).is_err());
    assert_eq!(session.document(), &before);
    value.reference = WallLayerReference::Center;
    value.height = length(-20.);
    assert!(model::prepare_layered_wall_type_edit(&session, 4, actor(), id, &value).is_err());
    assert!(model::prepare_layered_wall_type_edit(&session, 0, actor(), id, &value).is_err());
    assert_eq!(session.document(), &before);
}

#[test]
fn forged_layer_source_links_and_conflicting_totals_are_refused_at_document_entry() {
    use multicad_core::{
        commands::{Actor, Channel, Session},
        model::{self, LayeredWallTypeDefinition},
    };
    let (document, stack) = fixture();
    let mut session = Session::new(document).unwrap();
    let value = LayeredWallTypeDefinition {
        reference: WallLayerReference::Center,
        layers: stack,
        height: length(2700.),
    };
    let (id, preview) = model::prepare_layered_wall_type(
        &session,
        0,
        Actor::local(Channel::Ui),
        "Checked construction",
        &value,
    )
    .unwrap();
    session.commit(preview).unwrap();
    let source = session.document().get(id).unwrap().clone();
    let mut forged = source.clone();
    if let EntityData::Extension { inputs, .. } = &mut forged.data {
        inputs.pop();
    }
    assert!(
        session
            .document()
            .edited(&[Mutation::Replace(forged)])
            .is_err()
    );
    let mut forged = source;
    if let EntityData::Extension { definition, .. } = &mut forged.data {
        definition.data["total_thickness"] = serde_json::json!(500.);
    }
    assert!(
        session
            .document()
            .edited(&[Mutation::Replace(forged)])
            .is_err()
    );
}

fn wall_request(wall_type: EntityId) -> multicad_core::model::WallRequest {
    use multicad_core::model::{BodyOutput, WallDefinition, WallLayerPlacement, WallRequest};
    WallRequest::new(
        "Layered L wall",
        WallDefinition {
            output: BodyOutput {
                body: EntityId::new(),
            },
            wall_type,
            layer_placement: Some(WallLayerPlacement::default()),
            path: vec![
                [length(0.), length(0.)],
                [length(4000.), length(0.)],
                [length(4000.), length(3000.)],
            ],
            closed: false,
            level: None,
            base_z: length(0.),
            thickness: None,
            height: None,
            justification: None,
            openings: vec![],
            cutouts: vec![],
        },
    )
}
#[test]
fn layered_l_band_plans_share_corners_and_conserve_the_exact_envelope_under_flip() {
    use multicad_core::{
        commands::{Actor, Channel, Session},
        model::{self, LayeredWallTypeDefinition, WallChange},
    };
    let (document, stack) = fixture();
    let mut session = Session::new(document).unwrap();
    let value = LayeredWallTypeDefinition {
        layers: stack,
        height: length(2700.),
        reference: WallLayerReference::Center,
    };
    let actor = || Actor::local(Channel::Ui);
    let (id, prepared) =
        model::prepare_layered_wall_type(&session, 0, actor(), "L construction", &value).unwrap();
    session.commit(prepared).unwrap();
    let mut request = wall_request(id);
    let area = |plan: &model::WallPlan| {
        let polygon = |points: &[[f64; 2]]| {
            (0..points.len())
                .map(|i| {
                    let a = points[i];
                    let b = points[(i + 1) % points.len()];
                    a[0] * b[1] - a[1] * b[0]
                })
                .sum::<f64>()
                .abs()
                / 2.
        };
        polygon(&plan.outer.points) - plan.holes.iter().map(|h| polygon(&h.points)).sum::<f64>()
    };
    for flipped in [false, true] {
        request.wall.layer_placement.as_mut().unwrap().flipped = flipped;
        let preview =
            model::prepare_wall_change(&session, 1, actor(), &[], &WallChange::Create(&request))
                .unwrap();
        let source = model::resolve_wall(
            preview.prepared.document(),
            session.parameters(),
            &request.wall,
        )
        .unwrap();
        let envelope = source.plan().unwrap();
        let layers = source.layers.as_ref().unwrap();
        let mut total = 0.;
        for band in &layers.bands {
            let (left, right) = (
                band.outside_mm.max(band.inside_mm),
                band.outside_mm.min(band.inside_mm),
            );
            if left > right {
                total += area(&source.plan_region(left, right).unwrap());
            }
        }
        assert!((area(&envelope) - total).abs() < 1e-6);
        assert!(
            source
                .plan_region(envelope.left_mm + 1., envelope.right_mm)
                .is_err()
        );
        assert_eq!(source.thickness_mm, 352.5);
        assert_eq!(session.revision(), 1, "Planning remains source-neutral");
    }
}

#[test]
fn layer_placement_and_material_authority_survive_source_history_and_file_roundtrip() {
    use multicad_core::{
        commands::{Actor, Channel, Session},
        model::{self, LayeredWallTypeDefinition, WallChange},
        project_file,
    };
    let (document, stack) = fixture();
    let mut session = Session::new(document).unwrap();
    let actor = || Actor::local(Channel::Ui);
    let value = LayeredWallTypeDefinition {
        layers: stack,
        height: length(2700.),
        reference: WallLayerReference::Outside,
    };
    let (id, prepared) =
        model::prepare_layered_wall_type(&session, 0, actor(), "Oriented construction", &value)
            .unwrap();
    session.commit(prepared).unwrap();
    let mut request = wall_request(id);
    request.wall.layer_placement.as_mut().unwrap().flipped = true;
    let preview =
        model::prepare_wall_change(&session, 1, actor(), &[], &WallChange::Create(&request))
            .unwrap();
    session.commit(preview.prepared).unwrap();
    assert_eq!(
        model::read_wall(session.document(), request.feature).unwrap(),
        request.wall
    );
    let project = project_file::ProjectFile::new(session.document().clone()).unwrap();
    assert!(
        project
            .capabilities()
            .contains(model::LAYERED_WALL_CAPABILITY)
    );
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let opened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(
        model::read_wall(&opened.project.document, request.feature).unwrap(),
        request.wall
    );
    let original = session.document().clone();
    let mut bad = request.wall.clone();
    bad.thickness = Some(length(500.));
    assert!(
        model::prepare_wall_change(
            &session,
            2,
            actor(),
            &[],
            &WallChange::Edit {
                feature: request.feature,
                wall: &bad
            }
        )
        .is_err()
    );
    bad = request.wall.clone();
    bad.layer_placement.as_mut().unwrap().reference = Some(WallLayerReference::Boundary {
        layer: EntityId::new(),
        side: WallLayerSide::Outside,
    });
    assert!(
        model::prepare_wall_change(
            &session,
            2,
            actor(),
            &[],
            &WallChange::Edit {
                feature: request.feature,
                wall: &bad
            }
        )
        .is_err()
    );
    assert_eq!(session.document(), &original);
    let mut part = session.document().get(request.body).unwrap().clone();
    if let EntityData::Part { material, .. } = &mut part.data {
        *material = value.layers.layers[0].construction.material();
    }
    assert!(
        session
            .document()
            .edited(&[Mutation::Replace(part)])
            .is_err()
    );
    session.undo(2, actor()).unwrap();
    assert!(model::walls(session.document()).next().is_none());
    session.redo(3, actor()).unwrap();
    assert_eq!(
        model::read_wall(session.document(), request.feature).unwrap(),
        request.wall
    );
}

#[test]
fn equipment_hosts_follow_real_material_faces_after_layer_reference_and_flip_edits() {
    use multicad_core::{
        commands::{Actor, Channel, Session},
        equipment::{Attachment, HostPlacement, WallSide},
        model::{self, LayeredWallTypeDefinition, WallChange},
    };
    let (document, mut stack) = fixture();
    stack.layers.insert(
        0,
        WallLayer {
            id: EntityId::new(),
            name: "Outside cavity datum".into(),
            role: None,
            construction: WallLayerKind::Cavity {
                thickness: length(20.),
            },
        },
    );
    let mut session = Session::new(document).unwrap();
    let actor = || Actor::local(Channel::Ui);
    let (floor, preview) = model::prepare_level(
        &session,
        0,
        actor(),
        "Floor",
        &model::LevelDefinition {
            elevation: length(0.),
            height: length(3000.),
            slab_thickness: length(200.),
        },
    )
    .unwrap();
    session.commit(preview).unwrap();
    let value = LayeredWallTypeDefinition {
        layers: stack,
        height: length(2700.),
        reference: WallLayerReference::Outside,
    };
    let (id, preview) =
        model::prepare_layered_wall_type(&session, 1, actor(), "Hosted construction", &value)
            .unwrap();
    session.commit(preview).unwrap();
    let mut request = wall_request(id);
    let preview =
        model::prepare_wall_change(&session, 2, actor(), &[], &WallChange::Create(&request))
            .unwrap();
    session.commit(preview.prepared).unwrap();
    let attachment = Attachment {
        owner: EntityId::new(),
        floor,
        placement: HostPlacement::Wall {
            wall: request.feature,
            segment: 0,
            point_count: 3,
            closed: false,
            side: WallSide::Left,
            along: length(1000.),
            standoff: length(5.),
            height: length(300.),
            rotation: model::angle(0.),
        },
    };
    assert_eq!(
        attachment
            .resolve(session.document(), session.parameters())
            .unwrap()
            .pose
            .translation_mm,
        [1000., -15., 300.],
        "An outside cavity is not a physical mounting face"
    );
    request.wall.layer_placement.as_mut().unwrap().flipped = true;
    let preview = model::prepare_wall_change(
        &session,
        3,
        actor(),
        &[],
        &WallChange::Edit {
            feature: request.feature,
            wall: &request.wall,
        },
    )
    .unwrap();
    session.commit(preview.prepared).unwrap();
    assert_eq!(
        attachment
            .resolve(session.document(), session.parameters())
            .unwrap()
            .pose
            .translation_mm,
        [1000., 377.5, 300.]
    );
}

#[test]
fn placement_fields_cannot_be_smuggled_into_legacy_wall_schema() {
    use multicad_core::model::WallDefinition;
    let mut source = wall_request(EntityId::new()).wall;
    let version_two = source.definition().unwrap();
    let mut downgraded = version_two.clone();
    downgraded.schema_version = 1;
    assert!(WallDefinition::from_definition(&downgraded).is_err());
    source.layer_placement = None;
    let legacy = source.definition().unwrap();
    assert_eq!(legacy.schema_version, 1);
    assert!(legacy.data.get("layer_placement").is_none());
    let mut misleading = legacy.clone();
    misleading.data["layer_placement"] = serde_json::Value::Null;
    assert!(WallDefinition::from_definition(&misleading).is_err());
    assert_eq!(WallDefinition::from_definition(&legacy).unwrap(), source);
}

#[test]
fn path_reversal_preserves_open_wall_side_and_refuses_silent_reference_rebinding() {
    use multicad_core::{
        commands::{Actor, Channel, Session},
        model::{self, LayeredWallTypeDefinition, NewType, NewTypeKind, WallChange},
    };
    let (document, stack) = fixture();
    let mut session = Session::new(document).unwrap();
    let actor = || Actor::local(Channel::Ui);
    let ty = NewType::new(
        "Reversible construction",
        NewTypeKind::LayeredWall(LayeredWallTypeDefinition {
            layers: stack,
            height: length(2700.),
            reference: WallLayerReference::Outside,
        }),
    );
    let mut request = wall_request(ty.id);
    let preview =
        model::prepare_wall_change(&session, 0, actor(), &[ty], &WallChange::Create(&request))
            .unwrap();
    session.commit(preview.prepared).unwrap();
    let old = model::resolve_wall(session.document(), session.parameters(), &request.wall).unwrap();
    let before = session.document().clone();
    request.wall.path.reverse();
    assert!(
        model::prepare_wall_change(
            &session,
            1,
            actor(),
            &[],
            &WallChange::Edit {
                feature: request.feature,
                wall: &request.wall
            }
        )
        .is_err()
    );
    assert_eq!(session.document(), &before);
    request.wall.layer_placement.as_mut().unwrap().flipped = true;
    let preview = model::prepare_wall_change(
        &session,
        1,
        actor(),
        &[],
        &WallChange::Edit {
            feature: request.feature,
            wall: &request.wall,
        },
    )
    .unwrap();
    let reversed = model::resolve_wall(
        preview.prepared.document(),
        session.parameters(),
        &request.wall,
    )
    .unwrap();
    let area = |wall: &model::ResolvedWall| {
        let plan = wall.plan().unwrap();
        (0..plan.outer.points.len())
            .map(|i| {
                let a = plan.outer.points[i];
                let b = plan.outer.points[(i + 1) % plan.outer.points.len()];
                a[0] * b[1] - a[1] * b[0]
            })
            .sum::<f64>()
            .abs()
            / 2.
    };
    assert!((area(&old) - area(&reversed)).abs() < 1e-6);
    drop(preview);
    let reference = Entity::new(
        "Dependent face sketch",
        EntityData::Extension {
            definition: multicad_core::document::Definition {
                type_id: "test.reference".into(),
                schema_version: 1,
                data: serde_json::json!({}),
            },
            inputs: vec![multicad_core::document::Link::new(request.feature, "host")],
        },
    );
    session
        .apply(multicad_core::commands::Request {
            expected_revision: 1,
            actor: actor(),
            label: "Add dependent reference".into(),
            edits: vec![Mutation::Insert(reference)],
        })
        .unwrap();
    assert!(
        model::prepare_wall_change(
            &session,
            2,
            actor(),
            &[],
            &WallChange::Edit {
                feature: request.feature,
                wall: &request.wall
            }
        )
        .is_err()
    );
}

#[test]
fn layered_equipment_anchors_in_voids_require_repair_but_lintels_and_the_opposite_face_remain_usable()
 {
    use multicad_core::{
        commands::{Actor, Channel, Session},
        equipment::{Attachment, HostPlacement, WallSide},
        model::{self, LayeredWallTypeDefinition, NewType, NewTypeKind, WallChange},
    };
    let (document, stack) = fixture();
    let mut session = Session::new(document).unwrap();
    let actor = || Actor::local(Channel::Ui);
    let (floor, p) = model::prepare_level(
        &session,
        0,
        actor(),
        "Ground",
        &model::LevelDefinition {
            elevation: length(0.),
            height: length(3000.),
            slab_thickness: length(200.),
        },
    )
    .unwrap();
    session.commit(p).unwrap();
    let ty = NewType::new(
        "Hosted layers",
        NewTypeKind::LayeredWall(LayeredWallTypeDefinition {
            layers: stack,
            height: length(2700.),
            reference: WallLayerReference::Outside,
        }),
    );
    let door = NewType::new(
        "Door",
        NewTypeKind::Opening(model::OpeningTypeDefinition {
            kind: model::OpeningKind::Door {
                hinge: model::Hinge::Start,
                swing: model::Swing::Left,
            },
            width: length(900.),
            height: length(2100.),
            sill: length(0.),
        }),
    );
    let mut request = wall_request(ty.id);
    request.wall.openings.push(model::Opening {
        id: EntityId::new(),
        opening_type: door.id,
        segment: 0,
        offset: length(500.),
        width: None,
        height: None,
        sill: None,
    });
    request.wall.cutouts.push(model::VerticalCutout {
        id: EntityId::new(),
        segment: 0,
        offset: length(1700.),
        width: length(400.),
        depth: Some(length(20.)),
        face: model::WallFace::Left,
    });
    let p = model::prepare_wall_change(
        &session,
        1,
        actor(),
        &[ty, door],
        &WallChange::Create(&request),
    )
    .unwrap();
    session.commit(p.prepared).unwrap();
    let anchor = |along, height, side| Attachment {
        owner: EntityId::new(),
        floor,
        placement: HostPlacement::Wall {
            wall: request.feature,
            segment: 0,
            point_count: 3,
            closed: false,
            side,
            along: length(along),
            standoff: length(0.),
            height: length(height),
            rotation: model::angle(0.),
        },
    };
    assert!(
        anchor(1000., 300., WallSide::Left)
            .resolve(session.document(), session.parameters())
            .unwrap_err()
            .to_string()
            .contains("reattach")
    );
    assert!(
        anchor(1000., 2300., WallSide::Left)
            .resolve(session.document(), session.parameters())
            .is_ok()
    );
    assert!(
        anchor(1900., 300., WallSide::Left)
            .resolve(session.document(), session.parameters())
            .is_err()
    );
    assert!(
        anchor(1900., 300., WallSide::Right)
            .resolve(session.document(), session.parameters())
            .is_ok()
    );
}

#[test]
fn shared_construction_edit_is_atomic_lists_users_and_retains_instance_overrides() {
    use multicad_core::{
        commands::{Actor, Channel, Session},
        model::{self, LayeredWallTypeDefinition, WallChange, WallConstructionEdit},
    };
    let (document, layers) = fixture();
    let mut session = Session::new(document).unwrap();
    let actor = || Actor::local(Channel::Ui);
    let value = LayeredWallTypeDefinition {
        layers,
        height: length(2700.),
        reference: WallLayerReference::Center,
    };
    let (ty, p) = model::prepare_layered_wall_type(&session, 0, actor(), "Shared", &value).unwrap();
    session.commit(p).unwrap();
    let first = wall_request(ty);
    let mut second = wall_request(ty);
    second.wall.height = Some(length(2800.));
    for request in [&first, &second] {
        let p = model::prepare_wall_change(
            &session,
            session.revision(),
            actor(),
            &[],
            &WallChange::Create(request),
        )
        .unwrap();
        session.commit(p.prepared).unwrap();
    }
    let users = model::wall_construction_users(session.document(), ty).unwrap();
    assert_eq!(users.len(), 2);
    assert!(users.iter().any(|u| u.body == first.body));
    assert!(users.iter().any(|u| u.body == second.body));
    let before = session.document().clone();
    let mut edited = value.clone();
    if let WallLayerKind::Solid { thickness, .. } = &mut edited.layers.layers[1].construction {
        *thickness = length(100.);
    }
    edited.height = length(3000.);
    let change = WallConstructionEdit {
        id: ty,
        name: "Shared revised".into(),
        value: edited,
    };
    let prepare = |session: &Session| {
        model::prepare_wall_construction_change(
            session,
            session.revision(),
            actor(),
            &[],
            &WallChange::Edit {
                feature: first.feature,
                wall: &first.wall,
            },
            Some(&change),
        )
        .unwrap()
    };
    let preview = prepare(&session);
    assert_eq!(session.document(), &before);
    for (wall, expected_height) in [(&first.wall, 3000.), (&second.wall, 2800.)] {
        let resolved = model::resolve_wall(
            preview.prepared.document(),
            preview.prepared.parameters(),
            wall,
        )
        .unwrap();
        assert_eq!(resolved.thickness_mm, 372.5);
        assert_eq!(resolved.height_mm, expected_height);
    }
    drop(preview);
    assert_eq!(
        session.document(),
        &before,
        "Cancel must not alter the shared type"
    );
    session.commit(prepare(&session).prepared).unwrap();
    let after = session.document().clone();
    assert_eq!(session.document().get(ty).unwrap().name, "Shared revised");
    assert_eq!(
        model::wall_construction_users(session.document(), ty).unwrap(),
        users
    );
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &before);
    session.redo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &after);
}

#[test]
fn removed_shared_boundary_can_be_repaired_atomically_but_other_walls_must_remain_valid() {
    use multicad_core::{
        commands::{Actor, Channel, Session},
        model::{self, LayeredWallTypeDefinition, WallChange, WallConstructionEdit},
    };
    let (document, layers) = fixture();
    let mut session = Session::new(document).unwrap();
    let actor = || Actor::local(Channel::Mcp);
    let removed = layers.layers[1].id;
    let value = LayeredWallTypeDefinition {
        layers,
        height: length(2700.),
        reference: WallLayerReference::Center,
    };
    let (ty, p) =
        model::prepare_layered_wall_type(&session, 0, actor(), "Referenced", &value).unwrap();
    session.commit(p).unwrap();
    let mut first = wall_request(ty);
    first.wall.layer_placement.as_mut().unwrap().reference = Some(WallLayerReference::Boundary {
        layer: removed,
        side: WallLayerSide::Outside,
    });
    let mut second = wall_request(ty);
    second.wall.layer_placement = first.wall.layer_placement.clone();
    for request in [&first, &second] {
        let p = model::prepare_wall_change(
            &session,
            session.revision(),
            actor(),
            &[],
            &WallChange::Create(request),
        )
        .unwrap();
        session.commit(p.prepared).unwrap();
    }
    let before = session.document().clone();
    let mut revised = value;
    revised.layers.layers.remove(1);
    let edit = WallConstructionEdit {
        id: ty,
        name: "Repaired".into(),
        value: revised,
    };
    let mut repaired = first.wall.clone();
    repaired.layer_placement.as_mut().unwrap().reference = None;
    let attempt = |session: &Session| {
        model::prepare_wall_construction_change(
            session,
            session.revision(),
            actor(),
            &[],
            &WallChange::Edit {
                feature: first.feature,
                wall: &repaired,
            },
            Some(&edit),
        )
    };
    assert!(
        attempt(&session).is_err(),
        "the other wall still refers to the removed boundary"
    );
    assert_eq!(session.document(), &before);
    let mut second_repaired = second.wall.clone();
    second_repaired.layer_placement.as_mut().unwrap().reference = None;
    let p = model::prepare_wall_edit(
        &session,
        session.revision(),
        actor(),
        second.feature,
        &second_repaired,
    )
    .unwrap();
    session.commit(p.prepared).unwrap();
    let p = attempt(&session).unwrap();
    assert_eq!(p.prepared.actor().channel, Channel::Mcp);
    session.commit(p.prepared).unwrap();
    let saved = model::read_wall(session.document(), first.feature).unwrap();
    assert_eq!(saved.layer_placement.unwrap().reference, None);
    assert!(model::read_wall_construction(session.document(), ty).is_ok());
}
