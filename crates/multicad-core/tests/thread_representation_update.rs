use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, Entity, EntityData, EntityId, Mutation, Placement},
    model::*,
    symbols::*,
};
use std::collections::BTreeMap;
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn session() -> Session {
    Session::new(Document::new("Representation updates").unwrap()).unwrap()
}
fn values(id: EntityId, n: f64) -> BTreeMap<EntityId, SymbolValue> {
    BTreeMap::from([(
        id,
        SymbolValue {
            value: n,
            unit: "count".into(),
        },
    )])
}
fn definition() -> (SymbolDefinition, EntityId, EntityId) {
    let mut s = session();
    let parameter = Entity::new(
        "mode",
        EntityData::Parameter {
            expression: "0".into(),
            unit: "count".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let id = parameter.id;
    s.apply(Request {
        expected_revision: 0,
        actor: actor(),
        label: "Mode".into(),
        edits: vec![Mutation::Insert(parameter)],
    })
    .unwrap();
    let request = MechanicalPartRequest::new(
        "Nut",
        MechanicalPartParameters::HexNut {
            across_flats: length(13.),
            thread: Box::new(ThreadParameters {
                length: length(4.),
                representation_parameter: Some(id),
                ..Default::default()
            }),
        },
    );
    s.commit(
        prepare_mechanical_part(&s, s.revision(), actor(), &request)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let author = SymbolAuthorRequest::new(
        "Configurable nut",
        request.body,
        SymbolKind::Dynamic,
        vec![id],
        None,
    );
    s.commit(
        prepare_author(&s, s.revision(), actor(), &author)
            .unwrap()
            .prepared,
    )
    .unwrap();
    (
        read_definition(s.document(), author.definition).unwrap(),
        id,
        request.feature,
    )
}
fn role(
    def: &SymbolDefinition,
    feature: EntityId,
    parameter: Option<EntityId>,
    revision: &str,
) -> SymbolDefinition {
    let mut replacement = def.clone();
    replacement.revision = revision.into();
    let mut source = read_mechanical_part(&def.template, feature).unwrap();
    let MechanicalPartParameters::HexNut { thread, .. } = &mut source.parameters else {
        unreachable!()
    };
    thread.representation_parameter = parameter;
    let mut entity = def.template.get(feature).unwrap().clone();
    let EntityData::Feature {
        definition, inputs, ..
    } = &mut entity.data
    else {
        unreachable!()
    };
    *definition = source.definition().unwrap();
    *inputs = source.inputs().unwrap();
    replacement.template = def.template.edited(&[Mutation::Replace(entity)]).unwrap();
    replacement.validate().unwrap();
    replacement
}
fn host(def: &SymbolDefinition) -> (Session, EntityId) {
    let mut s = session();
    let import = prepare_import(&s, 0, actor(), def).unwrap();
    let embedded = import.definition;
    s.commit(import.prepared).unwrap();
    (s, embedded)
}
fn place(
    s: &mut Session,
    embedded: EntityId,
    overrides: BTreeMap<EntityId, SymbolValue>,
) -> SymbolPlaceRequest {
    let request = SymbolPlaceRequest::new(
        s.document(),
        embedded,
        "Instance",
        overrides,
        Placement::default(),
    )
    .unwrap();
    s.commit(
        prepare_place(s, s.revision(), actor(), &request)
            .unwrap()
            .prepared,
    )
    .unwrap();
    request
}
#[test]
fn representation_overrides_reject_nonbinary_counts_before_placement_or_edit_commit() {
    let (def, id, _) = definition();
    let (mut s, embedded) = host(&def);
    let placed = place(&mut s, embedded, values(id, 0.));
    let before = s.document().clone();
    for n in [2., 0.5, -1., 1.000000000000001] {
        let result = SymbolPlaceRequest::new(
            s.document(),
            embedded,
            "Invalid",
            values(id, n),
            Placement::default(),
        )
        .and_then(|request| {
            prepare_place(&s, s.revision(), actor(), &request)
                .map_err(|e| multicad_core::document::DocumentError::Invalid(e.to_string()))
        });
        assert!(result.is_err());
        assert!(
            prepare_edit(
                &s,
                s.revision(),
                actor(),
                &SymbolEditRequest {
                    record: placed.instance,
                    overrides: values(id, n)
                }
            )
            .is_err()
        );
        assert_eq!(s.document(), &before);
    }
}
#[test]
fn unchanged_typed_role_retains_two_independent_overrides_and_private_ids() {
    let (def, id, feature) = definition();
    let (mut s, embedded) = host(&def);
    let a = place(&mut s, embedded, values(id, 0.));
    let b = place(&mut s, embedded, values(id, 1.));
    let replacement = role(&def, feature, Some(id), "2");
    let update = SymbolUpdateRequest::new(s.document(), embedded, replacement.clone()).unwrap();
    assert!(
        compare_update_request(s.document(), &update)
            .unwrap()
            .conflicts
            .is_empty()
    );
    let before = s.document().clone();
    s.commit(
        prepare_update(&s, s.revision(), actor(), &update)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let after = s.document().clone();
    for (request, n) in [(&a, 0.), (&b, 1.)] {
        let instance = read_instance(s.document(), request.instance).unwrap();
        assert_eq!(instance.ids, request.ids);
        assert_eq!(instance.overrides, values(id, n));
        assert_eq!(instance.pin, replacement.pin().unwrap());
        let source = read_mechanical_part(s.document(), instance.ids[&feature]).unwrap();
        let MechanicalPartParameters::HexNut { thread, .. } = source.parameters else {
            unreachable!()
        };
        assert_eq!(
            thread.resolved_representation(s.parameters()).unwrap(),
            ThreadRepresentation::from_parameter_value(n).unwrap()
        );
    }
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &before);
    s.redo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &after);
}
#[test]
fn removing_or_adding_role_requires_explicit_reset_of_an_existing_override() {
    let (bound, id, feature) = definition();
    let unbound = role(&bound, feature, None, "2");
    for (old, incoming) in [
        (&bound, &unbound),
        (&unbound, &role(&bound, feature, Some(id), "3")),
    ] {
        let (mut s, embedded) = host(old);
        let instance = place(&mut s, embedded, values(id, 1.));
        let before = s.document().clone();
        let mut update =
            SymbolUpdateRequest::new(s.document(), embedded, incoming.clone()).unwrap();
        let report = compare_update_request(s.document(), &update).unwrap();
        assert!(
            report
                .conflicts
                .iter()
                .any(|conflict| conflict.parameter == id
                    && conflict.instance == instance.instance
                    && conflict.reason.contains("representation role changed"))
        );
        assert!(
            prepare_update(&s, s.revision(), actor(), &update).is_err(),
            "Direct preparation must enforce the comparison conflict"
        );
        assert_eq!(s.document(), &before);
        // Even rewriting the same retained ID to another valid number is not a reset.
        update
            .instance_overrides
            .insert(instance.record, values(id, 0.));
        assert!(prepare_update(&s, s.revision(), actor(), &update).is_err());
        update
            .instance_overrides
            .get_mut(&instance.record)
            .unwrap()
            .remove(&id);
        assert!(
            compare_update_request(s.document(), &update)
                .unwrap()
                .conflicts
                .is_empty()
        );
        s.commit(
            prepare_update(&s, s.revision(), actor(), &update)
                .unwrap()
                .prepared,
        )
        .unwrap();
        assert_eq!(
            read_instance(s.document(), instance.instance).unwrap().ids,
            instance.ids
        );
        assert!(
            read_instance(s.document(), instance.instance)
                .unwrap()
                .overrides
                .is_empty()
        );
    }
}
#[test]
fn newly_bound_role_accepts_an_explicit_choice_when_no_old_override_exists() {
    let (bound, id, feature) = definition();
    let unbound = role(&bound, feature, None, "2");
    let incoming = role(&bound, feature, Some(id), "3");
    let (mut s, embedded) = host(&unbound);
    let instance = place(&mut s, embedded, BTreeMap::new());
    let mut update = SymbolUpdateRequest::new(s.document(), embedded, incoming).unwrap();
    update
        .instance_overrides
        .insert(instance.record, values(id, 1.));
    assert!(
        compare_update_request(s.document(), &update)
            .unwrap()
            .conflicts
            .is_empty()
    );
    s.commit(
        prepare_update(&s, s.revision(), actor(), &update)
            .unwrap()
            .prepared,
    )
    .unwrap();
    assert_eq!(
        read_instance(s.document(), instance.instance)
            .unwrap()
            .overrides,
        values(id, 1.)
    );
}
