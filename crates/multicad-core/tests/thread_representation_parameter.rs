use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, Entity, EntityData, EntityId, Mutation, Placement},
    model::*,
    project_file::{self, ProjectFile},
    symbols::{
        self, SymbolAuthorRequest, SymbolEditRequest, SymbolKind, SymbolPlaceRequest, SymbolValue,
    },
};
use std::collections::BTreeMap;

fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn session() -> Session {
    Session::new(Document::new("Representation binding").unwrap()).unwrap()
}
fn parameter(s: &mut Session, value: &str, unit: &str) -> EntityId {
    let entity = Entity::new(
        format!("representation_choice_{}", s.revision()),
        EntityData::Parameter {
            expression: value.into(),
            unit: unit.into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let id = entity.id;
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Representation input".into(),
        edits: vec![Mutation::Insert(entity)],
    })
    .unwrap();
    id
}
fn bound(id: EntityId, literal: ThreadRepresentation) -> ThreadParameters {
    ThreadParameters {
        representation_parameter: Some(id),
        representation: literal,
        ..Default::default()
    }
}
fn nut(thread: ThreadParameters) -> MechanicalPartParameters {
    MechanicalPartParameters::HexNut {
        across_flats: length(13.),
        thread: Box::new(ThreadParameters {
            length: length(4.),
            ..thread
        }),
    }
}
fn mode(part: &MechanicalPartParameters, s: &Session) -> ThreadRepresentation {
    match part.resolve(s.parameters()).unwrap() {
        ResolvedMechanicalPart::HexNut { thread, .. }
        | ResolvedMechanicalPart::MachineScrew { thread, .. } => thread.representation,
        _ => unreachable!(),
    }
}
fn overrides(id: EntityId, value: f64) -> BTreeMap<EntityId, SymbolValue> {
    BTreeMap::from([(
        id,
        SymbolValue {
            value,
            unit: "count".into(),
        },
    )])
}

#[test]
fn exact_count_binding_overrides_opposite_literal_in_direct_nut_and_screw_recipes() {
    for (value, expected, dormant) in [
        (
            "0",
            ThreadRepresentation::CosmeticEnvelope,
            ThreadRepresentation::Modeled,
        ),
        (
            "1",
            ThreadRepresentation::Modeled,
            ThreadRepresentation::CosmeticEnvelope,
        ),
    ] {
        let mut s = session();
        let id = parameter(&mut s, value, "count");
        let thread = bound(id, dormant);
        assert_eq!(
            thread.resolved_representation(s.parameters()).unwrap(),
            expected
        );
        assert_eq!(
            thread
                .resolve(s.parameters(), ThreadMode::NewExternal)
                .unwrap()
                .representation,
            expected
        );
        let mut screw = default_machine_screw();
        let MechanicalPartParameters::MachineScrew { thread: nested, .. } = &mut screw else {
            unreachable!()
        };
        nested.representation_parameter = Some(id);
        nested.representation = dormant;
        for part in [nut(thread.clone()), screw] {
            assert_eq!(mode(&part, &s), expected);
            let request = MechanicalPartRequest::new("Bound hardware", part);
            let before = s.document().clone();
            let prepared = prepare_mechanical_part(&s, s.revision(), actor(), &request).unwrap();
            assert_eq!(s.document(), &before);
            s.commit(prepared.prepared).unwrap();
            let EntityData::Feature { inputs, .. } =
                &s.document().get(request.feature).unwrap().data
            else {
                unreachable!()
            };
            assert!(inputs.iter().any(|link| link.target == id));
            assert_eq!(
                cosmetic_thread_in_history(s.document(), request.feature).unwrap(),
                (expected == ThreadRepresentation::CosmeticEnvelope).then_some(request.feature)
            );
        }
        let request = ThreadRequest::new("Bound thread", thread);
        s.commit(
            prepare_thread(&s, s.revision(), actor(), &request)
                .unwrap()
                .prepared,
        )
        .unwrap();
        let EntityData::Feature { inputs, .. } = &s.document().get(request.feature).unwrap().data
        else {
            unreachable!()
        };
        assert!(inputs.iter().any(|link| link.target == id));
        assert_eq!(
            cosmetic_thread_in_history(s.document(), request.feature).unwrap(),
            (expected == ThreadRepresentation::CosmeticEnvelope).then_some(request.feature)
        );
    }
}

#[test]
fn invalid_bindings_never_fall_back_or_commit_partial_hardware() {
    let mut s = session();
    let primitive = PrimitiveRequest::new(
        "Not a parameter",
        PrimitiveParameters::Box {
            size: [length(1.), length(1.), length(1.)],
            frame: Default::default(),
        },
    );
    s.commit(
        prepare_primitive(&s, s.revision(), actor(), &primitive)
            .unwrap()
            .prepared,
    )
    .unwrap();
    // Fractions, negative counts and nonfinite expressions already fail the
    // canonical parameter transaction, before any recipe can bind them.
    for expression in ["0.5", "-1", "1 / 0"] {
        let before = s.document().clone();
        let entity = Entity::new(
            "invalid_count",
            EntityData::Parameter {
                expression: expression.into(),
                unit: "count".into(),
                exposed: true,
                inputs: vec![],
            },
        );
        assert!(
            s.apply(Request {
                expected_revision: s.revision(),
                actor: actor(),
                label: "Invalid count".into(),
                edits: vec![Mutation::Insert(entity)]
            })
            .is_err()
        );
        assert_eq!(s.document(), &before);
    }
    let mut invalid = vec![EntityId::nil(), EntityId::new(), primitive.body];
    for (value, unit) in [
        ("2", "count"),
        ("1.000000000000001", "count"),
        ("0", "mm"),
        ("1", "ratio"),
    ] {
        invalid.push(parameter(&mut s, value, unit));
    }
    let before = s.document().clone();
    let revision = s.revision();
    for id in invalid {
        let thread = bound(id, ThreadRepresentation::Modeled);
        assert!(
            thread.resolved_representation(s.parameters()).is_err(),
            "Invalid binding {id} used its literal fallback"
        );
        assert!(
            prepare_thread(
                &s,
                revision,
                actor(),
                &ThreadRequest::new("Invalid thread", thread.clone())
            )
            .is_err()
        );
        for part in [nut(thread.clone()), {
            let mut screw = default_machine_screw();
            let MechanicalPartParameters::MachineScrew { thread: nested, .. } = &mut screw else {
                unreachable!()
            };
            nested.representation_parameter = Some(id);
            screw
        }] {
            assert!(
                prepare_mechanical_part(
                    &s,
                    revision,
                    actor(),
                    &MechanicalPartRequest::new("Invalid hardware", part)
                )
                .is_err()
            );
        }
        assert_eq!(s.document(), &before);
        assert_eq!(s.revision(), revision);
    }
}

#[test]
fn unbound_legacy_wire_preserves_literal_and_omits_optional_binding() {
    let s = session();
    for representation in [
        ThreadRepresentation::Modeled,
        ThreadRepresentation::CosmeticEnvelope,
    ] {
        let original = ThreadParameters {
            representation,
            ..Default::default()
        };
        let wire = serde_json::to_value(&original).unwrap();
        assert!(wire.get("representation_parameter").is_none());
        let decoded: ThreadParameters = serde_json::from_value(wire).unwrap();
        assert_eq!(decoded, original);
        assert_eq!(
            decoded.resolved_representation(s.parameters()).unwrap(),
            representation
        );
    }
}

#[test]
fn one_pin_two_instance_choices_remap_dependencies_and_switch_independently_through_history_and_reopen()
 {
    let mut library = session();
    let choice = parameter(&mut library, "0", "count");
    let literal_reference = nut(ThreadParameters::default());
    let mut request = MechanicalPartRequest::new(
        "Referenced M8 nut",
        nut(bound(choice, ThreadRepresentation::Modeled)),
    );
    request.provenance = Some(MechanicalPartProvenance {
        catalog_id: "independent-test-nut".into(),
        revision: "1".into(),
        designation: "M8 dimensional fixture".into(),
        source_url: "https://example.invalid/nut".into(),
        reference_geometry: literal_reference,
        catalog_facts: Some(MechanicalPartCatalogFacts {
            nominal_fastener_diameter_mm: Some(8.),
            ..Default::default()
        }),
    });
    library
        .commit(
            prepare_mechanical_part(&library, library.revision(), actor(), &request)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let author = SymbolAuthorRequest::new(
        "Configurable nut",
        request.body,
        SymbolKind::Dynamic,
        vec![choice],
        None,
    );
    library
        .commit(
            symbols::prepare_author(&library, library.revision(), actor(), &author)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let definition = symbols::read_definition(library.document(), author.definition).unwrap();
    assert!(
        definition.template.get(choice).is_some(),
        "Extraction lost the binding's parameter dependency"
    );
    let original_library = library.document().clone();
    let mut s = session();
    let import = symbols::prepare_import(&s, 0, actor(), &definition).unwrap();
    let embedded = import.definition;
    s.commit(import.prepared).unwrap();
    let a = SymbolPlaceRequest::new(
        s.document(),
        embedded,
        "Envelope nut",
        overrides(choice, 0.),
        Placement::default(),
    )
    .unwrap();
    let b = SymbolPlaceRequest::new(
        s.document(),
        embedded,
        "Modeled nut",
        overrides(choice, 1.),
        Placement {
            translation_mm: [30., 0., 0.],
            ..Default::default()
        },
    )
    .unwrap();
    for placement in [&a, &b] {
        s.commit(
            symbols::prepare_place(&s, s.revision(), actor(), placement)
                .unwrap()
                .prepared,
        )
        .unwrap();
    }
    for (placement, expected) in [
        (&a, ThreadRepresentation::CosmeticEnvelope),
        (&b, ThreadRepresentation::Modeled),
    ] {
        let source = read_mechanical_part(s.document(), placement.ids[&request.feature]).unwrap();
        let MechanicalPartParameters::HexNut { thread, .. } = &source.parameters else {
            unreachable!()
        };
        assert_eq!(
            thread.representation_parameter,
            Some(placement.ids[&choice])
        );
        assert_ne!(placement.ids[&choice], choice);
        assert_eq!(mode(&source.parameters, &s), expected);
        assert!(source.matches_reference_dimensions(s.parameters()).unwrap());
        assert_eq!(source.provenance, request.provenance);
        assert_eq!(
            symbols::read_instance(s.document(), placement.instance)
                .unwrap()
                .pin,
            definition.pin().unwrap()
        );
    }
    let before = s.document().clone();
    let b_before = symbols::read_instance(s.document(), b.instance).unwrap();
    let b_entities: Vec<_> = b
        .ids
        .values()
        .map(|id| s.document().get(*id).unwrap().clone())
        .collect();
    let edit = SymbolEditRequest {
        record: a.instance,
        overrides: overrides(choice, 1.),
    };
    let canceled = symbols::prepare_edit(&s, s.revision(), actor(), &edit).unwrap();
    drop(canceled);
    assert_eq!(s.document(), &before);
    s.commit(
        symbols::prepare_edit(&s, s.revision(), actor(), &edit)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let after = s.document().clone();
    assert_eq!(
        symbols::read_instance(s.document(), a.instance)
            .unwrap()
            .ids,
        a.ids
    );
    assert_eq!(
        symbols::read_instance(s.document(), b.instance).unwrap(),
        b_before
    );
    for entity in b_entities {
        assert_eq!(s.document().get(entity.id), Some(&entity));
    }
    assert_eq!(
        mode(
            &read_mechanical_part(s.document(), a.ids[&request.feature])
                .unwrap()
                .parameters,
            &s
        ),
        ThreadRepresentation::Modeled
    );
    assert!(symbols::prepare_edit(&s, 0, actor(), &edit).is_err());
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &before);
    s.redo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &after);
    let mut bytes = Vec::new();
    project_file::write(
        &ProjectFile::new(after.clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    let independent = Session::new(reopened.document).unwrap();
    assert_eq!(independent.document(), &after);
    assert_eq!(
        mode(
            &read_mechanical_part(independent.document(), a.ids[&request.feature])
                .unwrap()
                .parameters,
            &independent
        ),
        ThreadRepresentation::Modeled
    );
    assert_eq!(library.document(), &original_library);
}

#[test]
fn end_treatment_support_checks_effective_mode_instead_of_dormant_literal() {
    for (value, dormant, accepted) in [
        ("0", ThreadRepresentation::Modeled, false),
        ("1", ThreadRepresentation::CosmeticEnvelope, true),
    ] {
        let mut s = session();
        let id = parameter(&mut s, value, "count");
        let thread = ThreadParameters {
            end_treatments: Some(ThreadEndTreatments {
                start: ThreadStartTreatment::Chamfer45 {
                    radial_depth: length(0.2),
                },
                end: ThreadEndTreatment::FlatCut,
            }),
            ..bound(id, dormant)
        };
        let before = s.document().clone();
        let result = prepare_thread(
            &s,
            s.revision(),
            actor(),
            &ThreadRequest::new("Treated thread", thread),
        );
        assert_eq!(result.is_ok(), accepted);
        assert_eq!(s.document(), &before);
    }
}
