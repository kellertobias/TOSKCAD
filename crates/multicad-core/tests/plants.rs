use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, EntityData, EntityId, Mutation, Placement},
    parameters::{self, Unit},
    plants::{self, PlantInstance, PlantKind, SizeBasis},
    symbols::{self, SymbolPlaceRequest, SymbolValue},
};
use std::collections::BTreeMap;
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn fixture(kind: PlantKind) -> (Session, SymbolPlaceRequest) {
    let definition = plants::generic_definition(kind).unwrap();
    let template = plants::template_for(&definition.template, definition.root)
        .unwrap()
        .unwrap();
    assert_eq!(definition.plan.as_ref().unwrap().sketch, template.plan);
    let mut s = Session::new(Document::new("Garden").unwrap()).unwrap();
    let imported = symbols::prepare_import(&s, 0, actor(), &definition).unwrap();
    let id = imported.definition;
    s.commit(imported.prepared).unwrap();
    let request = SymbolPlaceRequest::new(
        s.document(),
        id,
        "Plant A",
        BTreeMap::from([(
            template.spread_parameter,
            SymbolValue {
                value: 2500.,
                unit: "mm".into(),
            },
        )]),
        Placement {
            translation_mm: [300., 400., 20.],
            ..Default::default()
        },
    )
    .unwrap();
    s.commit(
        symbols::prepare_place(&s, s.revision(), actor(), &request)
            .unwrap()
            .prepared,
    )
    .unwrap();
    (s, request)
}
#[test]
fn linked_template_is_captured_remapped_and_restored_with_the_same_pin() {
    for kind in [PlantKind::Tree, PlantKind::Shrub] {
        let (mut s, placed) = fixture(kind);
        let before = s.document().clone();
        let resolved = plants::resolve(s.document(), s.parameters(), placed.record)
            .unwrap()
            .unwrap();
        assert_eq!(resolved.template.kind, kind);
        assert_eq!(resolved.spread_mm, 2500.);
        assert_eq!(resolved.plan, placed.ids[&resolved.template.plan]);
        assert_eq!(
            parameters::evaluate(s.document())
                .unwrap()
                .value(
                    placed.ids[&resolved.template.spread_parameter],
                    Unit::Millimeter
                )
                .unwrap(),
            2500.
        );
        let label = placed.instance.to_string();
        let settings = PlantInstance {
            symbol_instance: placed.record,
            size_basis: SizeBasis::Current,
            label: Some(label.clone()),
            plan_visible: false,
        };
        s.apply(Request {
            expected_revision: s.revision(),
            actor: actor(),
            label: "Plant meaning".into(),
            edits: vec![Mutation::Insert(settings.entity(EntityId::new()).unwrap())],
        })
        .unwrap();
        let changed = s.document().clone();
        let resolved = plants::resolve(s.document(), s.parameters(), placed.instance)
            .unwrap()
            .unwrap();
        assert_eq!(resolved.label.as_deref(), Some(label.as_str()));
        assert!(!resolved.plan_visible);
        assert_eq!(resolved.size_basis, SizeBasis::Current);
        s.undo(s.revision(), actor()).unwrap();
        assert_eq!(s.document(), &before);
        s.redo(s.revision(), actor()).unwrap();
        assert_eq!(s.document(), &changed);
        let reopened: Document =
            serde_json::from_slice(&serde_json::to_vec(s.document()).unwrap()).unwrap();
        reopened.validate().unwrap();
        assert_eq!(reopened, changed);
        assert_eq!(
            symbols::read_instance(&reopened, placed.record)
                .unwrap()
                .pin,
            symbols::read_instance(&before, placed.record).unwrap().pin
        );
        assert!(plants::required_capability(&reopened));
    }
}
#[test]
fn invalid_settings_are_rejected_without_a_revision_or_document_change() {
    let (mut s, placed) = fixture(PlantKind::Tree);
    for target in [placed.instance, placed.definition, EntityId::new()] {
        let before = s.document().clone();
        let revision = s.revision();
        let settings = PlantInstance {
            symbol_instance: target,
            size_basis: SizeBasis::PlannedMature,
            label: None,
            plan_visible: true,
        };
        assert!(
            s.apply(Request {
                expected_revision: revision,
                actor: actor(),
                label: "Invalid settings".into(),
                edits: vec![Mutation::Insert(settings.entity(EntityId::new()).unwrap())]
            })
            .is_err()
        );
        assert_eq!(s.document(), &before);
        assert_eq!(s.revision(), revision);
    }
    let value = PlantInstance {
        symbol_instance: placed.record,
        size_basis: SizeBasis::Current,
        label: None,
        plan_visible: true,
    };
    let before = s.document().clone();
    assert!(
        s.apply(Request {
            expected_revision: s.revision(),
            actor: actor(),
            label: "Duplicate settings".into(),
            edits: vec![
                Mutation::Insert(value.entity(EntityId::new()).unwrap()),
                Mutation::Insert(value.entity(EntityId::new()).unwrap())
            ]
        })
        .is_err()
    );
    assert_eq!(s.document(), &before);
    for label in ["".to_string(), "\n".to_string(), "a".repeat(201)] {
        assert!(
            PlantInstance {
                label: Some(label),
                ..value.clone()
            }
            .definition()
            .is_err()
        );
    }
}
#[test]
fn template_dimensions_require_length_and_duplicate_ownership_is_rejected() {
    let definition = plants::generic_definition(PlantKind::Shrub).unwrap();
    let template = plants::template_for(&definition.template, definition.root)
        .unwrap()
        .unwrap();
    let mut parameter = definition
        .template
        .get(template.spread_parameter)
        .unwrap()
        .clone();
    if let EntityData::Parameter { unit, .. } = &mut parameter.data {
        *unit = "deg".into();
    }
    assert!(
        definition
            .template
            .edited(&[Mutation::Replace(parameter)])
            .is_err()
    );
    assert!(
        definition
            .template
            .edited(&[Mutation::Insert(template.entity(EntityId::new()).unwrap())])
            .is_err()
    );
    let mut s = Session::new(Document::new("Imported only").unwrap()).unwrap();
    s.commit(
        symbols::prepare_import(&s, 0, actor(), &definition)
            .unwrap()
            .prepared,
    )
    .unwrap();
    assert!(plants::required_capability(s.document()));
}
#[test]
fn placement_and_dimension_settings_edit_are_single_cancellable_transactions() {
    let mut s = Session::new(Document::new("Garden transaction").unwrap()).unwrap();
    let request = plants::PlantPlaceRequest::new(
        &s,
        plants::generic_definition(PlantKind::Tree).unwrap(),
        "Oak placeholder".into(),
        6000.,
        4000.,
        Placement::default(),
    )
    .unwrap();
    let before = s.document().clone();
    let prepared = plants::prepare_place(&s, 0, actor(), &request).unwrap();
    assert_eq!(s.document(), &before);
    s.commit(prepared).unwrap();
    assert_eq!(s.revision(), 1);
    let plant = plants::resolve(s.document(), s.parameters(), request.placement.record)
        .unwrap()
        .unwrap();
    assert_eq!(plant.spread_mm, 4000.);
    let mut settings = request.settings.clone();
    settings.label = Some("T1".into());
    settings.size_basis = SizeBasis::Current;
    let prepared = plants::prepare_edit(
        &s,
        1,
        actor(),
        request.placement.record,
        5000.,
        3000.,
        settings.clone(),
    )
    .unwrap();
    assert_eq!(
        plants::resolve(s.document(), s.parameters(), request.placement.record)
            .unwrap()
            .unwrap()
            .spread_mm,
        4000.
    );
    s.commit(prepared).unwrap();
    assert_eq!(s.revision(), 2);
    let plant = plants::resolve(s.document(), s.parameters(), request.placement.record)
        .unwrap()
        .unwrap();
    assert_eq!(plant.spread_mm, 3000.);
    assert_eq!(plant.label.as_deref(), Some("T1"));
    let current = s.document().clone();
    for size in [0., -1., f64::NAN, 1e8] {
        assert!(
            plants::prepare_edit(
                &s,
                2,
                actor(),
                request.placement.record,
                5000.,
                size,
                settings.clone()
            )
            .is_err()
        );
        assert_eq!(s.document(), &current);
    }
    assert!(plants::prepare_place(&s, 0, actor(), &request).is_err());
    s.undo(2, actor()).unwrap();
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &before);
}
#[test]
fn generic_presets_have_repeatable_pins_and_copy_retains_literal_settings() {
    for kind in [PlantKind::Tree, PlantKind::Shrub] {
        let a = plants::generic_definition(kind).unwrap();
        let b = plants::generic_definition(kind).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.pin().unwrap(), b.pin().unwrap());
    }
    let (mut s, placed) = fixture(PlantKind::Tree);
    let label = placed.instance.to_string();
    let settings = PlantInstance {
        symbol_instance: placed.record,
        size_basis: SizeBasis::Current,
        label: Some(label.clone()),
        plan_visible: false,
    };
    s.commit(
        plants::prepare_edit(
            &s,
            s.revision(),
            actor(),
            placed.record,
            7000.,
            3500.,
            settings,
        )
        .unwrap(),
    )
    .unwrap();
    let request = multicad_core::model::InstanceCopyRequest::new(
        multicad_core::model::InstanceCopySource::Instance {
            instance: placed.instance,
        },
        Default::default(),
    );
    s.commit(
        multicad_core::model::prepare_instance_copies(
            &s,
            s.revision(),
            actor(),
            std::slice::from_ref(&request),
        )
        .unwrap()
        .prepared,
    )
    .unwrap();
    let copied = plants::resolve(s.document(), s.parameters(), request.instance)
        .unwrap()
        .unwrap();
    assert_eq!(copied.label.as_deref(), Some(label.as_str()));
    assert_eq!(copied.size_basis, SizeBasis::Current);
    assert!(!copied.plan_visible);
    assert_eq!(copied.spread_mm, 3500.);
}
#[test]
fn absolute_planar_edit_keeps_prior_drivers_and_commits_pose_name_and_dimensions_once() {
    let (mut s, placed) = fixture(PlantKind::Tree);
    let before = s.document().clone();
    let settings = PlantInstance {
        symbol_instance: placed.record,
        size_basis: SizeBasis::Current,
        label: Some("T1".into()),
        plan_visible: true,
    };
    let movement = plants::planar_move(
        s.document(),
        s.parameters(),
        placed.record,
        [1000., 2000., 300.],
        90.,
    )
    .unwrap();
    let request = plants::PlantEditRequest {
        record: placed.record,
        height_mm: 8000.,
        spread_mm: 5000.,
        settings,
        name: "Tree relocated".into(),
        move_by: movement,
    };
    s.commit(plants::prepare_edit_request(&s, s.revision(), actor(), &request).unwrap())
        .unwrap();
    let pose =
        multicad_core::model::resolve_instance_pose(s.document(), placed.instance, s.parameters())
            .unwrap();
    assert_eq!(pose.translation_mm, [1000., 2000., 300.]);
    assert_eq!(pose.rotation, [[0., -1., 0.], [1., 0., 0.], [0., 0., 1.]]);
    assert_eq!(
        s.document().get(placed.instance).unwrap().name,
        "Tree relocated"
    );
    let revision = s.revision();
    s.undo(revision, actor()).unwrap();
    assert_eq!(s.document(), &before);
}
