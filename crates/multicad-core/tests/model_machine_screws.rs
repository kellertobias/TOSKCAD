use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, Mutation},
    model::*,
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    symbols::{self, SymbolAuthorRequest, SymbolKind},
};

fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn session() -> Session {
    Session::new(Document::new("Machine screw contract").unwrap()).unwrap()
}
fn hex_screw() -> MechanicalPartParameters {
    let mut value = default_machine_screw();
    let MechanicalPartParameters::MachineScrew { head, drive, .. } = &mut value else {
        unreachable!()
    };
    *head = ScrewHead::ExternalHex {
        across_flats: length(5.5),
        height: length(2.),
    };
    *drive = ScrewDrive::ExternalHex;
    value
}
fn provenance(reference_geometry: MechanicalPartParameters) -> MechanicalPartProvenance {
    MechanicalPartProvenance {
        catalog_id: "test-only-screw".into(),
        revision: "fixture-1".into(),
        designation: "Test-only M3 dimensional reference".into(),
        source_url: "https://example.invalid/screw".into(),
        reference_geometry,
        catalog_facts: None,
    }
}
fn insert(s: &mut Session, parameters: MechanicalPartParameters) -> MechanicalPartRequest {
    let request = MechanicalPartRequest::new("Machine screw", parameters);
    let p = prepare_mechanical_part(s, s.revision(), actor(), &request).unwrap();
    s.commit(p.prepared).unwrap();
    request
}
fn encoded(s: &Session) -> Vec<u8> {
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(s.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    bytes
}

#[test]
fn both_head_drive_pairs_resolve_full_and_partial_threads_without_rewriting_authored_lengths() {
    let s = session();
    for parameters in [default_machine_screw(), hex_screw()] {
        for b in [10., 4.] {
            let mut parameters = parameters.clone();
            let MechanicalPartParameters::MachineScrew { thread, .. } = &mut parameters else {
                unreachable!()
            };
            thread.length = length(b);
            thread.frame.origin = [length(1.), length(2.), length(3.)];
            let saved = parameters.clone();
            let ResolvedMechanicalPart::MachineScrew {
                underhead_length_mm,
                shank_radius_mm,
                head,
                drive,
                thread,
                ..
            } = parameters.resolve(s.parameters()).unwrap()
            else {
                unreachable!()
            };
            assert_eq!(underhead_length_mm, 10.);
            assert_eq!(shank_radius_mm, 1.5);
            assert_eq!(thread.length_mm, b);
            assert_eq!(thread.pitch_mm, 0.5);
            assert_eq!(thread.frame.origin, [1., 2., 3.]);
            assert_eq!(thread.representation, ThreadRepresentation::Modeled);
            match (head, drive) {
                (
                    ResolvedScrewHead::SocketCap {
                        radius_mm,
                        height_mm,
                    },
                    ResolvedScrewDrive::HexSocket {
                        across_flats_mm,
                        depth_mm,
                    },
                ) => {
                    assert_eq!(
                        (radius_mm, height_mm, across_flats_mm, depth_mm),
                        (2.84, 3., 2.55, 1.3)
                    );
                }
                (
                    ResolvedScrewHead::ExternalHex {
                        across_flats_mm,
                        height_mm,
                    },
                    ResolvedScrewDrive::ExternalHex,
                ) => assert_eq!((across_flats_mm, height_mm), (5.5, 2.)),
                _ => panic!("Unsupported head/drive pair resolved"),
            }
            assert_eq!(parameters, saved);
        }
    }
}

#[test]
fn invalid_screw_lengths_heads_sockets_units_and_pitch_are_atomic_for_create_and_edit() {
    let mut s = session();
    let request = insert(&mut s, default_machine_screw());
    let before = s.document().clone();
    let revision = s.revision();
    let replacements = [
        (
            "/underhead_length",
            serde_json::to_value(length(0.)).unwrap(),
        ),
        (
            "/underhead_length",
            serde_json::to_value(length(9.)).unwrap(),
        ),
        (
            "/shank_diameter",
            serde_json::to_value(length(-1.)).unwrap(),
        ),
        ("/head/diameter", serde_json::to_value(length(3.)).unwrap()),
        ("/head/height", serde_json::to_value(length(0.)).unwrap()),
        (
            "/drive/across_flats",
            serde_json::to_value(length(5.5)).unwrap(),
        ),
        ("/drive/depth", serde_json::to_value(length(3.)).unwrap()),
        ("/drive/depth", serde_json::to_value(length(-1.)).unwrap()),
        ("/thread/length", serde_json::to_value(length(0.)).unwrap()),
        ("/thread/pitch", serde_json::to_value(length(0.)).unwrap()),
        (
            "/head/height",
            serde_json::to_value(Dimension::Literal {
                value: 3.,
                unit: "deg".into(),
            })
            .unwrap(),
        ),
        ("/drive", serde_json::json!({"kind":"external_hex"})),
    ];
    for (path, replacement) in replacements {
        let mut value = serde_json::to_value(default_machine_screw()).unwrap();
        *value.pointer_mut(path).unwrap() = replacement;
        let parameters: MechanicalPartParameters = serde_json::from_value(value).unwrap();
        assert!(
            parameters.resolve(s.parameters()).is_err(),
            "Invalid case {path} accepted"
        );
        let bad = MechanicalPartRequest::new("Invalid screw", parameters.clone());
        assert!(prepare_mechanical_part(&s, revision, actor(), &bad).is_err());
        assert!(
            prepare_mechanical_part_edit(&s, revision, actor(), request.feature, &parameters)
                .is_err()
        );
        assert_eq!(s.document(), &before);
        assert_eq!(s.revision(), revision);
    }
    let mut wrong_pair = hex_screw();
    let MechanicalPartParameters::MachineScrew { drive, .. } = &mut wrong_pair else {
        unreachable!()
    };
    *drive = ScrewDrive::HexSocket {
        across_flats: length(2.),
        depth: length(1.),
    };
    assert!(
        prepare_mechanical_part_edit(&s, revision, actor(), request.feature, &wrong_pair).is_err()
    );
    assert_eq!(s.document(), &before);
}

#[test]
fn every_screw_dimension_binds_dependencies_but_catalog_references_remain_literal() {
    for external in [false, true] {
        let paths: &[&str] = if external {
            &[
                "/underhead_length",
                "/shank_diameter",
                "/head/across_flats",
                "/head/height",
                "/thread/length",
                "/thread/pitch",
                "/thread/major_diameter",
            ]
        } else {
            &[
                "/underhead_length",
                "/shank_diameter",
                "/head/diameter",
                "/head/height",
                "/drive/across_flats",
                "/drive/depth",
                "/thread/length",
                "/thread/pitch",
                "/thread/major_diameter",
            ]
        };
        for path in paths {
            let mut s = session();
            let original = if external {
                hex_screw()
            } else {
                default_machine_screw()
            };
            let mut data = serde_json::to_value(&original).unwrap();
            let value = data.pointer(path).unwrap()["value"].as_f64().unwrap();
            let parameter = Entity::new(
                "input",
                EntityData::Parameter {
                    expression: value.to_string(),
                    unit: "mm".into(),
                    exposed: true,
                    inputs: vec![],
                },
            );
            let id = parameter.id;
            s.apply(Request {
                expected_revision: 0,
                actor: actor(),
                label: "Input".into(),
                edits: vec![Mutation::Insert(parameter)],
            })
            .unwrap();
            let dimension = Dimension::Expression {
                expression: BoundExpression::bind(s.document(), "=$input", "mm", None).unwrap(),
            };
            *data.pointer_mut(path).unwrap() = serde_json::to_value(dimension).unwrap();
            let parameters: MechanicalPartParameters = serde_json::from_value(data).unwrap();
            let mut request = MechanicalPartRequest::new("Bound screw", parameters.clone());
            request.provenance = Some(provenance(original));
            let prepared = prepare_mechanical_part(&s, s.revision(), actor(), &request).unwrap();
            s.commit(prepared.prepared).unwrap();
            let EntityData::Feature { inputs, .. } =
                &s.document().get(request.feature).unwrap().data
            else {
                unreachable!()
            };
            assert!(
                inputs.iter().any(|link| link.target == id),
                "Missing dependency for {path}"
            );
            let source = read_mechanical_part(s.document(), request.feature).unwrap();
            assert!(source.matches_reference_dimensions(s.parameters()).unwrap());
            let before = s.document().clone();
            let mut invalid = MechanicalPartRequest::new("Nonliteral source", parameters.clone());
            invalid.provenance = Some(provenance(parameters));
            assert!(
                prepare_mechanical_part(&s, s.revision(), actor(), &invalid).is_err(),
                "Nonliteral catalog field {path} accepted"
            );
            assert_eq!(s.document(), &before);
        }
    }
}

#[test]
fn screw_reference_matching_uses_physical_and_discrete_intent_while_ignoring_pose_and_display() {
    let s = session();
    let original = default_machine_screw();
    let mut source = MechanicalPartDefinition {
        output: BodyOutput {
            body: multicad_core::document::EntityId::new(),
        },
        parameters: original.clone(),
        provenance: Some(provenance(original)),
    };
    assert!(source.matches_reference_dimensions(s.parameters()).unwrap());
    let MechanicalPartParameters::MachineScrew { thread, .. } = &mut source.parameters else {
        unreachable!()
    };
    thread.frame.origin = [length(1.), length(2.), length(3.)];
    thread.representation = ThreadRepresentation::CosmeticEnvelope;
    assert!(source.matches_reference_dimensions(s.parameters()).unwrap());
    let reference = serde_json::to_value(default_machine_screw()).unwrap();
    for (path, value) in [
        ("/underhead_length", 11.),
        ("/shank_diameter", 2.9),
        ("/head/diameter", 5.8),
        ("/head/height", 3.1),
        ("/drive/across_flats", 2.4),
        ("/drive/depth", 1.4),
        ("/thread/length", 9.),
        ("/thread/pitch", 0.35),
    ] {
        let mut data = reference.clone();
        *data.pointer_mut(path).unwrap() = serde_json::to_value(length(value)).unwrap();
        source.parameters = serde_json::from_value(data).unwrap();
        assert!(
            !source.matches_reference_dimensions(s.parameters()).unwrap(),
            "Reference matched changed {path}"
        );
    }
    source.parameters = hex_screw();
    assert!(!source.matches_reference_dimensions(s.parameters()).unwrap());
    source.parameters = default_machine_screw();
    let MechanicalPartParameters::MachineScrew { thread, .. } = &mut source.parameters else {
        unreachable!()
    };
    thread.hand = ThreadHand::Left;
    assert!(!source.matches_reference_dimensions(s.parameters()).unwrap());
}

#[test]
fn screw_edits_preserve_identities_and_reference_through_history_and_independent_saved_reopen() {
    let mut s = session();
    let before = s.document().clone();
    let mut request = MechanicalPartRequest::new("Sourced fixture", default_machine_screw());
    request.provenance = Some(provenance(default_machine_screw()));
    let p = prepare_mechanical_part(&s, 0, actor(), &request).unwrap();
    s.commit(p.prepared).unwrap();
    let created = s.document().clone();
    let mut edited = hex_screw();
    let MechanicalPartParameters::MachineScrew { thread, .. } = &mut edited else {
        unreachable!()
    };
    thread.length = length(4.);
    let p =
        prepare_mechanical_part_edit(&s, s.revision(), actor(), request.feature, &edited).unwrap();
    s.commit(p.prepared).unwrap();
    let source = read_mechanical_part(s.document(), request.feature).unwrap();
    assert_eq!(source.output.body, request.body);
    assert_eq!(source.provenance, request.provenance);
    assert_eq!(source.parameters, edited);
    let saved = s.document().clone();
    assert!(prepare_mechanical_part_edit(&s, 0, actor(), request.feature, &edited).is_err());
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &created);
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &before);
    s.redo(s.revision(), actor()).unwrap();
    s.redo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &saved);
    let bytes = encoded(&s);
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    let independent = Session::new(reopened.document).unwrap();
    assert_eq!(independent.document(), &saved);
    assert_eq!(independent.revision(), 0);
    assert_eq!(
        read_mechanical_part(independent.document(), request.feature).unwrap(),
        source
    );
}

fn assert_reader_gate(s: &Session) {
    assert!(
        ProjectFile::new(s.document().clone())
            .unwrap()
            .capabilities()
            .contains(MACHINE_SCREW_CAPABILITY)
    );
    let bytes = encoded(s);
    let supported = project_file::supported_capabilities();
    assert_eq!(
        project_file::read(&mut bytes.as_slice(), Default::default(), &supported)
            .unwrap()
            .project
            .document,
        *s.document()
    );
    let mut older = supported;
    older.remove(MACHINE_SCREW_CAPABILITY);
    assert!(
        project_file::read(&mut bytes.as_slice(), Default::default(), &older)
            .unwrap_err()
            .contains(MACHINE_SCREW_CAPABILITY)
    );
}
#[test]
fn screw_reader_capability_covers_direct_suppressed_template_only_and_preserves_legacy_families() {
    let mut s = session();
    let request = insert(&mut s, default_machine_screw());
    assert_reader_gate(&s);
    let mut suppressed_session = Session::new(s.document().clone()).unwrap();
    let mut suppressed_entity = suppressed_session
        .document()
        .get(request.feature)
        .unwrap()
        .clone();
    let EntityData::Feature { suppressed, .. } = &mut suppressed_entity.data else {
        unreachable!()
    };
    *suppressed = true;
    suppressed_session
        .apply(Request {
            expected_revision: 0,
            actor: actor(),
            label: "Suppress standalone screw".into(),
            edits: vec![Mutation::Replace(suppressed_entity)],
        })
        .unwrap();
    assert_reader_gate(&suppressed_session);
    let author = SymbolAuthorRequest::new(
        "Reusable screw",
        request.body,
        SymbolKind::Static,
        vec![],
        None,
    );
    let p = symbols::prepare_author(&s, s.revision(), actor(), &author).unwrap();
    s.commit(p.prepared).unwrap();
    let definition = symbols::read_definition(s.document(), author.definition).unwrap();
    let mut host = session();
    let p = symbols::prepare_import(&host, 0, actor(), &definition).unwrap();
    host.commit(p.prepared).unwrap();
    assert!(
        !host
            .document()
            .entities()
            .any(|e| matches!(e.data, EntityData::Feature { .. }))
    );
    assert_reader_gate(&host);
    for parameters in [
        MechanicalPartParameters::Washer {
            bore_diameter: length(3.2),
            outside_diameter: length(7.),
            thickness: length(0.5),
            frame: Default::default(),
        },
        MechanicalPartParameters::BearingEnvelope {
            bore_diameter: length(8.),
            outside_diameter: length(22.),
            width: length(7.),
            frame: Default::default(),
        },
        MechanicalPartParameters::HexNut {
            across_flats: length(13.),
            thread: Box::default(),
        },
    ] {
        let mut old = session();
        insert(&mut old, parameters);
        assert!(
            !ProjectFile::new(old.document().clone())
                .unwrap()
                .capabilities()
                .contains(MACHINE_SCREW_CAPABILITY)
        );
    }
}

#[test]
fn pinned_screw_head_overrides_remap_real_bindings_and_preserve_uuid_looking_reference_prose() {
    use multicad_core::{
        document::Placement,
        symbols::{SymbolPlaceRequest, SymbolValue},
    };
    use std::collections::BTreeMap;
    let mut library = session();
    let parameter = Entity::new(
        "head",
        EntityData::Parameter {
            expression: "5.68".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let parameter_id = parameter.id;
    library
        .apply(Request {
            expected_revision: 0,
            actor: actor(),
            label: "Head input".into(),
            edits: vec![Mutation::Insert(parameter)],
        })
        .unwrap();
    let mut parameters = default_machine_screw();
    let MechanicalPartParameters::MachineScrew {
        head: ScrewHead::SocketCap { diameter, .. },
        ..
    } = &mut parameters
    else {
        unreachable!()
    };
    *diameter = Dimension::Expression {
        expression: BoundExpression::bind(library.document(), "=$head", "mm", None).unwrap(),
    };
    let mut request = MechanicalPartRequest::new("Pinned screw", parameters);
    let mut reference = provenance(default_machine_screw());
    reference.catalog_id = request.feature.to_string();
    reference.revision = parameter_id.to_string();
    reference.designation = request.body.to_string();
    reference.source_url = format!("https://example.invalid/{}", parameter_id);
    request.provenance = Some(reference.clone());
    let p = prepare_mechanical_part(&library, library.revision(), actor(), &request).unwrap();
    library.commit(p.prepared).unwrap();
    let author = SymbolAuthorRequest::new(
        "Adjustable screw",
        request.body,
        SymbolKind::Dynamic,
        vec![parameter_id],
        None,
    );
    let p = symbols::prepare_author(&library, library.revision(), actor(), &author).unwrap();
    library.commit(p.prepared).unwrap();
    let definition = symbols::read_definition(library.document(), author.definition).unwrap();
    let pin = definition.pin().unwrap();
    let mut host = session();
    let p = symbols::prepare_import(&host, 0, actor(), &definition).unwrap();
    let imported = p.definition;
    host.commit(p.prepared).unwrap();
    let placement = SymbolPlaceRequest::new(
        host.document(),
        imported,
        "Custom head",
        BTreeMap::from([(
            parameter_id,
            SymbolValue {
                value: 6.,
                unit: "mm".into(),
            },
        )]),
        Placement::default(),
    )
    .unwrap();
    let p = symbols::prepare_place(&host, host.revision(), actor(), &placement).unwrap();
    host.commit(p.prepared).unwrap();
    let instance = symbols::read_instance(host.document(), placement.instance).unwrap();
    assert_eq!(instance.pin, pin);
    let mapped = instance.ids[&request.feature];
    assert_ne!(mapped, request.feature);
    let source = read_mechanical_part(host.document(), mapped).unwrap();
    assert_eq!(source.provenance, Some(reference));
    assert_eq!(source.output.body, instance.ids[&request.body]);
    assert!(
        !source
            .matches_reference_dimensions(host.parameters())
            .unwrap()
    );
    let ResolvedMechanicalPart::MachineScrew {
        head: ResolvedScrewHead::SocketCap { radius_mm, .. },
        ..
    } = source.parameters.resolve(host.parameters()).unwrap()
    else {
        unreachable!()
    };
    assert_eq!(radius_mm, 3.);
    let EntityData::Feature { inputs, .. } = &host.document().get(mapped).unwrap().data else {
        unreachable!()
    };
    assert!(
        inputs
            .iter()
            .any(|link| link.target == instance.ids[&parameter_id])
    );
    assert!(!inputs.iter().any(|link| link.target == parameter_id));
    assert_eq!(
        symbols::read_definition(library.document(), author.definition).unwrap(),
        definition
    );
}
