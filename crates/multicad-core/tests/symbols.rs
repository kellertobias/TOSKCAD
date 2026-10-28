use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::{Dimension, source::Source},
    document::{Document, Entity, EntityData, EntityId, Mutation, Placement},
    model::*,
    parameters::{self, BoundExpression, Unit},
    sketch::Datum,
    symbols::*,
};
use std::collections::BTreeMap;
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn apply(s: &mut Session, edits: Vec<Mutation>) {
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Fixture edit".into(),
        edits,
    })
    .unwrap();
}
fn author_fixture(kind: SymbolKind) -> (Session, SymbolDefinition, EntityId, EntityId) {
    let mut s = Session::new(Document::new("Library source").unwrap()).unwrap();
    let p = Entity::new(
        "width",
        EntityData::Parameter {
            expression: "80".into(),
            unit: "mm".into(),
            exposed: false,
            inputs: vec![],
        },
    );
    let width = p.id;
    apply(&mut s, vec![Mutation::Insert(p)]);
    let derived = Entity::new(
        "depth",
        EntityData::Parameter {
            expression: "=$width / 4".into(),
            unit: "mm".into(),
            exposed: false,
            inputs: vec![],
        },
    );
    apply(&mut s, vec![Mutation::Insert(derived)]);
    let primitive = PrimitiveRequest::new(
        "Door",
        PrimitiveParameters::Box {
            size: [
                Dimension::Expression {
                    expression: BoundExpression::bind(s.document(), "=$width", "mm", None).unwrap(),
                },
                length(20.),
                Dimension::Expression {
                    expression: BoundExpression::bind(s.document(), "=$depth", "mm", None).unwrap(),
                },
            ],
            frame: Default::default(),
        },
    );
    s.commit(
        prepare_primitive(&s, s.revision(), actor(), &primitive)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let (sketch, edits) = Source::create(Datum::Xy, "Plan").unwrap();
    apply(&mut s, edits);
    let mut source = Source::read(s.document(), sketch).unwrap().0;
    source.add_rectangle([0., 0.], [80., 20.]).unwrap();
    let edits = source.replace(s.document(), sketch).unwrap();
    apply(&mut s, edits);
    let request = SymbolAuthorRequest::new(
        "Door",
        primitive.body,
        kind,
        vec![width],
        Some(SymbolPlan { sketch }),
    );
    let before = s.document().clone();
    let preview = prepare_author(&s, s.revision(), actor(), &request).unwrap();
    assert_eq!(s.document(), &before);
    s.commit(preview.prepared).unwrap();
    let def = read_definition(s.document(), request.definition).unwrap();
    assert_eq!(def.template.get(width).unwrap(), before.get(width).unwrap());
    (s, def, width, sketch)
}
fn host(def: &SymbolDefinition) -> (Session, EntityId) {
    let mut s = Session::new(Document::new("Host").unwrap()).unwrap();
    let p = prepare_import(&s, 0, actor(), def).unwrap();
    let id = p.definition;
    s.commit(p.prepared).unwrap();
    (s, id)
}
fn value(id: EntityId, n: f64) -> BTreeMap<EntityId, SymbolValue> {
    BTreeMap::from([(
        id,
        SymbolValue {
            value: n,
            unit: "mm".into(),
        },
    )])
}
#[test]
fn isolated_variants_expose_only_author_choices_and_reopen_with_fixed_ids() {
    let (library, def, width, plan) = author_fixture(SymbolKind::Dynamic);
    let (mut s, id) = host(&def);
    let unchanged = library.document().clone();
    let a = SymbolPlaceRequest::new(
        s.document(),
        id,
        "Door A",
        value(width, 100.),
        Placement::default(),
    )
    .unwrap();
    let b = SymbolPlaceRequest::new(
        s.document(),
        id,
        "Door B",
        value(width, 200.),
        Placement {
            translation_mm: [300., 0., 0.],
            ..Default::default()
        },
    )
    .unwrap();
    for r in [&a, &b] {
        let p = prepare_place(&s, s.revision(), actor(), r).unwrap();
        assert_eq!(p.plans[&r.instance], r.ids[&plan]);
        s.commit(p.prepared).unwrap();
    }
    validate(s.document()).unwrap();
    assert_eq!(library.document(), &unchanged);
    let params = parameters::evaluate(s.document()).unwrap();
    assert_eq!(params.value(a.ids[&width], Unit::Millimeter).unwrap(), 100.);
    assert_eq!(params.value(b.ids[&width], Unit::Millimeter).unwrap(), 200.);
    assert_ne!(a.ids[&plan], b.ids[&plan]);
    let local = Source::from_definition(match &def.template.get(plan).unwrap().data {
        EntityData::Sketch { definition, .. } => definition,
        _ => unreachable!(),
    })
    .unwrap();
    assert_ne!(
        a.ids[&local.sketch.points[0].id],
        b.ids[&local.sketch.points[0].id]
    );
    let before = s.document().clone();
    let p = prepare_edit(
        &s,
        s.revision(),
        actor(),
        &SymbolEditRequest {
            record: a.instance,
            overrides: value(width, 120.),
        },
    )
    .unwrap();
    s.commit(p.prepared).unwrap();
    assert_eq!(read_instance(s.document(), a.instance).unwrap().ids, a.ids);
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &before);
    s.redo(s.revision(), actor()).unwrap();
    let reopened: Document =
        serde_json::from_slice(&serde_json::to_vec(s.document()).unwrap()).unwrap();
    validate(&reopened).unwrap();
    assert_eq!(reopened, *s.document());
    assert_eq!(
        read_definition(&reopened, id).unwrap().pin().unwrap(),
        def.pin().unwrap()
    );
}
#[test]
fn static_private_source_ownership_and_invalid_override_failures_are_neutral() {
    let (_, def, width, _) = author_fixture(SymbolKind::Static);
    let (mut s, id) = host(&def);
    let mut r = SymbolPlaceRequest::new(
        s.document(),
        id,
        "Static",
        value(width, 90.),
        Placement::default(),
    )
    .unwrap();
    let before = s.document().clone();
    assert!(prepare_place(&s, s.revision(), actor(), &r).is_err());
    assert_eq!(s.document(), &before);
    r.overrides.clear();
    s.commit(
        prepare_place(&s, s.revision(), actor(), &r)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let mut bad = s.document().get(r.ids[&width]).unwrap().clone();
    if let EntityData::Parameter { expression, .. } = &mut bad.data {
        *expression = "123".into();
    }
    assert!(
        s.document()
            .edited(&[Mutation::Replace(bad.clone())])
            .is_err()
    );
    let original = s.document().clone();
    assert!(
        s.apply(Request {
            expected_revision: s.revision(),
            actor: actor(),
            label: "Forbidden private edit".into(),
            edits: vec![Mutation::Replace(bad)]
        })
        .is_err()
    );
    assert_eq!(s.document(), &original);
    let owned = owned_entities(s.document()).unwrap();
    assert!(owned.contains(&r.ids[&width]));
    assert!(!owned.contains(&r.instance));
    let mut moved = s.document().get(r.instance).unwrap().clone();
    if let EntityData::Instance { placement, .. } = &mut moved.data {
        placement.translation_mm = [1., 2., 3.];
    }
    validate(&s.document().edited(&[Mutation::Replace(moved)]).unwrap()).unwrap();
}
#[test]
fn explicit_upgrade_preserves_maps_values_and_undo_and_reports_conflicts() {
    let (_, def, width, _) = author_fixture(SymbolKind::Dynamic);
    let (mut s, id) = host(&def);
    let r = SymbolPlaceRequest::new(
        s.document(),
        id,
        "Door",
        value(width, 150.),
        Placement::default(),
    )
    .unwrap();
    s.commit(
        prepare_place(&s, s.revision(), actor(), &r)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let mut next = def.clone();
    next.revision = "2".into();
    next.name = "Door revised".into();
    let mut parameter = next.template.get(width).unwrap().clone();
    if let EntityData::Parameter { expression, .. } = &mut parameter.data {
        *expression = "90".into();
    }
    next.template = next
        .template
        .edited(&[Mutation::Replace(parameter)])
        .unwrap();
    let report = compare_update(s.document(), id, &next).unwrap();
    assert_eq!(report.affected_instances, vec![r.instance]);
    assert_eq!(report.changed_parameters, vec![width]);
    let request = SymbolUpdateRequest::new(s.document(), id, next.clone()).unwrap();
    let before = s.document().clone();
    let prepared = prepare_update(&s, s.revision(), actor(), &request).unwrap();
    assert_eq!(s.document(), &before);
    s.commit(prepared.prepared).unwrap();
    assert_eq!(read_instance(s.document(), r.instance).unwrap().ids, r.ids);
    assert_eq!(
        parameters::evaluate(s.document())
            .unwrap()
            .value(r.ids[&width], Unit::Millimeter)
            .unwrap(),
        150.
    );
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &before);
    s.redo(s.revision(), actor()).unwrap();
    next.revision = "3".into();
    next.exposed.clear();
    assert_eq!(
        compare_update(s.document(), id, &next)
            .unwrap()
            .conflicts
            .len(),
        1
    );
    let mut update = SymbolUpdateRequest::new(s.document(), id, next).unwrap();
    assert!(prepare_update(&s, s.revision(), actor(), &update).is_err());
    update
        .instance_overrides
        .get_mut(&r.record)
        .unwrap()
        .clear();
    let ready = prepare_update(&s, s.revision(), actor(), &update).unwrap();
    s.commit(ready.prepared).unwrap();
    assert_eq!(
        parameters::evaluate(s.document())
            .unwrap()
            .value(r.ids[&width], Unit::Millimeter)
            .unwrap(),
        90.
    );
}

#[test]
fn packaging_preserves_uuid_looking_user_text_and_inline_hatches_without_rekeying_tags() {
    let (mut library, def, width, _) = author_fixture(SymbolKind::Dynamic);
    let mut part = library.document().get(def.root).unwrap().clone();
    let unrelated = EntityId::new();
    let tags = std::collections::BTreeSet::from([
        width.to_string(),
        format!("custom/{width}/tag"),
        unrelated.to_string(),
    ]);
    part.name = width.to_string();
    part.description = format!("Literal user annotation /{width}/ unchanged");
    let hatch = multicad_core::document::Definition {
        type_id: "materials.hatch.inline".into(),
        schema_version: 1,
        data: serde_json::to_value(multicad_core::materials::HatchStyle::default()).unwrap(),
    };
    let material = Entity::new(
        "Wood",
        EntityData::Material {
            density_kg_m3: Some(600.),
            color_rgba: [80, 90, 100, 255],
            section_hatch: Some(hatch),
        },
    );
    if let EntityData::Part {
        tags: current,
        material: assigned,
        ..
    } = &mut part.data
    {
        *current = tags.clone();
        *assigned = Some(material.id);
    }
    let original = part.clone();
    apply(
        &mut library,
        vec![Mutation::Insert(material.clone()), Mutation::Replace(part)],
    );
    let author = SymbolAuthorRequest::new(
        "Tagged door",
        def.root,
        SymbolKind::Dynamic,
        vec![width],
        None,
    );
    let preview = prepare_author(&library, library.revision(), actor(), &author).unwrap();
    library.commit(preview.prepared).unwrap();
    let def = read_definition(library.document(), author.definition).unwrap();
    let (mut host, id) = host(&def);
    let place = SymbolPlaceRequest::new(
        host.document(),
        id,
        "Tagged instance",
        value(width, 101.),
        Placement::default(),
    )
    .unwrap();
    assert!(!place.ids.contains_key(&unrelated));
    host.commit(
        prepare_place(&host, host.revision(), actor(), &place)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let mapped = host.document().get(place.ids[&def.root]).unwrap();
    assert_eq!(mapped.name, original.name);
    assert_eq!(mapped.description, original.description);
    if let EntityData::Part {
        tags: actual,
        material: assigned,
        ..
    } = &mapped.data
    {
        assert_eq!(actual, &tags);
        assert_eq!(*assigned, Some(place.ids[&material.id]));
    } else {
        panic!("Expected materialized Part");
    }
    assert_eq!(
        host.document().get(place.ids[&material.id]).unwrap().data,
        material.data
    );
}

#[test]
fn explicit_upgrade_never_reuses_a_removed_source_identity_for_new_content() {
    let (_, def, width, _) = author_fixture(SymbolKind::Dynamic);
    let (mut s, id) = host(&def);
    let placed = SymbolPlaceRequest::new(
        s.document(),
        id,
        "Door",
        value(width, 120.),
        Placement::default(),
    )
    .unwrap();
    s.commit(
        prepare_place(&s, s.revision(), actor(), &placed)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let old_feature = match &def.template.get(def.root).unwrap().data {
        EntityData::Part { features, .. } => features[0],
        _ => unreachable!(),
    };
    let new_root = EntityId::new();
    let new_feature = EntityId::new();
    let mut part = def.template.get(def.root).unwrap().clone();
    part.id = new_root;
    if let EntityData::Part { features, .. } = &mut part.data {
        *features = vec![new_feature];
    }
    let mut source = read_primitive(&def.template, old_feature).unwrap();
    source.output.body = new_root;
    let mut feature = def.template.get(old_feature).unwrap().clone();
    feature.id = new_feature;
    if let EntityData::Feature { definition, .. } = &mut feature.data {
        *definition = source.definition().unwrap();
    }
    let mut incoming = def.clone();
    incoming.revision = "2".into();
    incoming.root = new_root;
    incoming.template = incoming
        .template
        .edited(&[
            Mutation::Remove(def.root),
            Mutation::Remove(old_feature),
            Mutation::Insert(part),
            Mutation::Insert(feature),
        ])
        .unwrap();
    let good = SymbolUpdateRequest::new(s.document(), id, incoming).unwrap();
    let mut forged = good.clone();
    forged
        .maps
        .get_mut(&placed.record)
        .unwrap()
        .insert(new_root, placed.ids[&def.root]);
    let before = s.document().clone();
    assert!(prepare_update(&s, s.revision(), actor(), &forged).is_err());
    assert_eq!(s.document(), &before);
    let preview = prepare_update(&s, s.revision(), actor(), &good).unwrap();
    s.commit(preview.prepared).unwrap();
    let current = read_instance(s.document(), placed.instance).unwrap();
    assert_eq!(current.ids[&width], placed.ids[&width]);
    assert_ne!(current.ids[&new_root], placed.ids[&def.root]);
    assert!(s.document().get(placed.ids[&def.root]).is_none());
}

#[test]
fn derived_private_parameter_bindings_normalize_atomically_without_weakening_ownership() {
    let (_, def, width, _) = author_fixture(SymbolKind::Dynamic);
    let depth = def
        .template
        .entities()
        .find(|e| e.name == "depth")
        .unwrap()
        .id;
    let (mut s, id) = host(&def);
    let request = SymbolPlaceRequest::new(
        s.document(),
        id,
        "Derived door",
        value(width, 100.),
        Placement::default(),
    )
    .unwrap();
    s.commit(
        prepare_place(&s, s.revision(), actor(), &request)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let evaluated = parameters::evaluate(s.document()).unwrap();
    assert_eq!(
        evaluated
            .value(request.ids[&depth], Unit::Millimeter)
            .unwrap(),
        25.
    );
    let before = s.document().clone();
    let mut tampered = s.document().get(request.ids[&depth]).unwrap().clone();
    if let EntityData::Parameter { inputs, .. } = &mut tampered.data {
        inputs.clear();
    }
    assert!(s.document().edited(&[Mutation::Replace(tampered)]).is_err());
    assert_eq!(s.document(), &before);
    s.commit(
        prepare_edit(
            &s,
            s.revision(),
            actor(),
            &SymbolEditRequest {
                record: request.record,
                overrides: value(width, 120.),
            },
        )
        .unwrap()
        .prepared,
    )
    .unwrap();
    assert_eq!(
        parameters::evaluate(s.document())
            .unwrap()
            .value(request.ids[&depth], Unit::Millimeter)
            .unwrap(),
        30.
    );
    let reopened: Document =
        serde_json::from_slice(&serde_json::to_vec(s.document()).unwrap()).unwrap();
    validate(&reopened).unwrap();
    assert_eq!(
        parameters::evaluate(&reopened)
            .unwrap()
            .value(request.ids[&depth], Unit::Millimeter)
            .unwrap(),
        30.
    );
}
