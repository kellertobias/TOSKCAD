use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, Entity, EntityData, EntityId, Mutation, Placement},
    model::*,
    project_file::{self, ProjectFile},
    symbols::{self, SymbolAuthorRequest, SymbolKind, SymbolPlaceRequest, SymbolValue},
};
use std::collections::BTreeMap;

fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn session() -> Session {
    Session::new(Document::new("Representation reader gate").unwrap()).unwrap()
}
fn fixture(kind: u8, bound: bool) -> (Session, EntityId, EntityId, EntityId) {
    let mut s = session();
    let parameter = Entity::new(
        "thread_mode",
        EntityData::Parameter {
            expression: "0".into(),
            unit: "count".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let choice = parameter.id;
    s.apply(Request {
        expected_revision: 0,
        actor: actor(),
        label: "Mode input".into(),
        edits: vec![Mutation::Insert(parameter)],
    })
    .unwrap();
    let thread = ThreadParameters {
        representation_parameter: bound.then_some(choice),
        ..Default::default()
    };
    let (feature, body) = if kind == 0 {
        let request = ThreadRequest::new("Thread", thread);
        s.commit(
            prepare_thread(&s, s.revision(), actor(), &request)
                .unwrap()
                .prepared,
        )
        .unwrap();
        (request.feature, request.body)
    } else {
        let parameters = if kind == 1 {
            MechanicalPartParameters::HexNut {
                across_flats: length(13.),
                thread: Box::new(ThreadParameters {
                    length: length(4.),
                    ..thread
                }),
            }
        } else {
            let mut screw = default_machine_screw();
            let MechanicalPartParameters::MachineScrew { thread: nested, .. } = &mut screw else {
                unreachable!()
            };
            nested.representation_parameter = bound.then_some(choice);
            screw
        };
        let request = MechanicalPartRequest::new("Hardware", parameters);
        s.commit(
            prepare_mechanical_part(&s, s.revision(), actor(), &request)
                .unwrap()
                .prepared,
        )
        .unwrap();
        (request.feature, request.body)
    };
    (s, feature, body, choice)
}
fn check_reader(document: &Document, required: bool) {
    let project = ProjectFile::new(document.clone()).unwrap();
    assert_eq!(
        project
            .capabilities()
            .contains(THREAD_REPRESENTATION_PARAMETER_CAPABILITY),
        required
    );
    let mut bytes = Vec::new();
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let supported = project_file::supported_capabilities();
    assert_eq!(
        project_file::read(&mut bytes.as_slice(), Default::default(), &supported)
            .unwrap()
            .project
            .document,
        *document
    );
    let mut older = supported;
    assert!(older.remove(THREAD_REPRESENTATION_PARAMETER_CAPABILITY));
    let read = project_file::read(&mut bytes.as_slice(), Default::default(), &older);
    if required {
        assert!(
            read.unwrap_err()
                .contains(THREAD_REPRESENTATION_PARAMETER_CAPABILITY)
        );
    } else {
        assert_eq!(read.unwrap().project.document, *document);
    }
}

#[test]
fn direct_and_nested_bound_threads_gate_old_readers_even_when_suppressed() {
    for kind in 0..3 {
        let (mut s, feature, _, _) = fixture(kind, true);
        check_reader(s.document(), true);
        let mut entity = s.document().get(feature).unwrap().clone();
        let EntityData::Feature { suppressed, .. } = &mut entity.data else {
            unreachable!()
        };
        *suppressed = true;
        s.apply(Request {
            expected_revision: s.revision(),
            actor: actor(),
            label: "Suppress source".into(),
            edits: vec![Mutation::Replace(entity)],
        })
        .unwrap();
        check_reader(s.document(), true);
    }
}

#[test]
fn template_only_and_receiving_private_sources_retain_the_reader_gate() {
    for kind in 0..3 {
        let (mut library, feature, body, choice) = fixture(kind, true);
        let author = SymbolAuthorRequest::new(
            "Configurable thread",
            body,
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
        let mut host = session();
        let import = symbols::prepare_import(&host, 0, actor(), &definition).unwrap();
        let embedded = import.definition;
        host.commit(import.prepared).unwrap();
        assert!(
            !host
                .document()
                .entities()
                .any(|entity| matches!(entity.data, EntityData::Feature { .. }))
        );
        check_reader(host.document(), true);
        let place = SymbolPlaceRequest::new(
            host.document(),
            embedded,
            "Modeled receiving instance",
            BTreeMap::from([(
                choice,
                SymbolValue {
                    value: 1.,
                    unit: "count".into(),
                },
            )]),
            Placement::default(),
        )
        .unwrap();
        host.commit(
            symbols::prepare_place(&host, host.revision(), actor(), &place)
                .unwrap()
                .prepared,
        )
        .unwrap();
        assert_ne!(place.ids[&choice], choice);
        let source = host.document().get(place.ids[&feature]).unwrap();
        let EntityData::Feature { inputs, .. } = &source.data else {
            unreachable!()
        };
        assert!(inputs.iter().any(|link| link.target == place.ids[&choice]));
        check_reader(host.document(), true);
    }
}

#[test]
fn legacy_unbound_recipes_do_not_add_a_capability_or_refuse_old_readers() {
    for kind in 0..3 {
        let (s, feature, _, _) = fixture(kind, false);
        check_reader(s.document(), false);
        let EntityData::Feature { definition, .. } = &s.document().get(feature).unwrap().data
        else {
            unreachable!()
        };
        let path = if kind == 0 {
            "/parameters/representation_parameter"
        } else {
            "/parameters/thread/representation_parameter"
        };
        assert!(definition.data.pointer(path).is_none());
    }
}

#[test]
fn raw_retained_reference_field_presence_requires_the_reader_without_legitimizing_invalid_reference_geometry()
 {
    let (s, feature, _, choice) = fixture(1, false);
    for value in [
        serde_json::Value::Null,
        serde_json::to_value(choice).unwrap(),
    ] {
        let mut raw = s.document().clone();
        let mut entity = raw.get(feature).unwrap().clone();
        let EntityData::Feature { definition, .. } = &mut entity.data else {
            unreachable!()
        };
        let mut reference = serde_json::to_value(
            read_mechanical_part(s.document(), feature)
                .unwrap()
                .parameters,
        )
        .unwrap();
        reference["thread"]["representation_parameter"] = value.clone();
        definition.data["provenance"] = serde_json::json!({
            "catalog_id": "reader-fixture", "revision": "1",
            "designation": "M8 dimensional fixture", "source_url": "https://example.invalid/nut",
            "reference_geometry": reference
        });
        if value.is_null() {
            raw.replace(entity).unwrap();
            assert!(
                required_capabilities(&raw).contains(THREAD_REPRESENTATION_PARAMETER_CAPABILITY)
            );
            check_reader(&raw, true);
        } else {
            // Both typed edits and the public serde boundary validate source.
            // Do not manufacture an invalid canonical Document to test scanning.
            assert!(
                s.document()
                    .edited(&[Mutation::Replace(entity.clone())])
                    .is_err()
            );
            let mut wire = serde_json::to_value(s.document()).unwrap();
            wire["entities"][feature.to_string()] = serde_json::to_value(entity.clone()).unwrap();
            assert!(serde_json::from_value::<Document>(wire).is_err());
            assert!(raw.replace(entity).is_err());
            assert_eq!(raw, *s.document());
        }
    }
}
