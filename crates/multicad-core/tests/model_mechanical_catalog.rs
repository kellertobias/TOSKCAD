use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, EntityData, Mutation},
    model::*,
    project_file::{self, ProjectFile},
    symbols::{self, SymbolAuthorRequest, SymbolKind},
};

fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn session() -> Session {
    Session::new(Document::new("Catalog fact regression").unwrap()).unwrap()
}
fn washer() -> MechanicalPartParameters {
    MechanicalPartParameters::Washer {
        bore_diameter: length(3.2),
        outside_diameter: length(7.),
        thickness: length(0.5),
        frame: Default::default(),
    }
}
fn provenance(parameters: MechanicalPartParameters) -> MechanicalPartProvenance {
    MechanicalPartProvenance {
        catalog_id: "catalog-fixture".into(),
        revision: "1".into(),
        designation: "M3 fixture".into(),
        source_url: "https://example.invalid/catalog".into(),
        reference_geometry: parameters,
        catalog_facts: None,
    }
}
fn facts() -> MechanicalPartCatalogFacts {
    MechanicalPartCatalogFacts {
        nominal_fastener_diameter_mm: Some(3.),
        series: Some("normal".into()),
        ..Default::default()
    }
}
fn add_washer(s: &mut Session) -> MechanicalPartRequest {
    let mut request = MechanicalPartRequest::new("M3 reference", washer());
    let mut source = provenance(washer());
    source.catalog_facts = Some(facts());
    request.provenance = Some(source);
    let prepared = prepare_mechanical_part(s, s.revision(), actor(), &request).unwrap();
    s.commit(prepared.prepared).unwrap();
    request
}
fn assert_reader_gate(s: &Session) {
    let project = ProjectFile::new(s.document().clone()).unwrap();
    assert!(
        project
            .capabilities()
            .contains(MECHANICAL_PART_CATALOG_FACTS_CAPABILITY)
    );
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let supported = project_file::supported_capabilities();
    let reopened = project_file::read(&mut &bytes[..], Default::default(), &supported).unwrap();
    assert_eq!(reopened.project.document, *s.document());
    let mut older = supported;
    older.remove(MECHANICAL_PART_CATALOG_FACTS_CAPABILITY);
    let error = project_file::read(&mut &bytes[..], Default::default(), &older).unwrap_err();
    assert!(
        error
            .to_string()
            .contains(MECHANICAL_PART_CATALOG_FACTS_CAPABILITY)
    );
}

#[test]
fn legacy_hardware_serialization_and_nominal_unknown_semantics_are_preserved() {
    let families = [
        washer(),
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
    ];
    for parameters in families {
        let source = provenance(parameters.clone());
        let value = serde_json::to_value(&source).unwrap();
        assert!(value.get("catalog_facts").is_none());
        let read: MechanicalPartProvenance = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(read).unwrap(), value);
        let mut s = session();
        let mut request = MechanicalPartRequest::new("Old hardware", parameters.clone());
        request.provenance = Some(source);
        let prepared = prepare_mechanical_part(&s, 0, actor(), &request).unwrap();
        s.commit(prepared.prepared).unwrap();
        let project = ProjectFile::new(s.document().clone()).unwrap();
        assert!(
            !project
                .capabilities()
                .contains(MECHANICAL_PART_CATALOG_FACTS_CAPABILITY)
        );
        let card = mechanical_part_cards(s.document()).remove(0);
        assert_eq!(
            card.catalog_facts, None,
            "A name or bore cannot supply a sourced nominal"
        );
        assert_eq!(
            read_mechanical_part(s.document(), request.feature)
                .unwrap()
                .parameters,
            parameters
        );
    }
}

#[test]
fn facts_are_independent_of_physical_bore_and_invalid_facts_cannot_commit() {
    let mut s = session();
    let request = add_washer(&mut s);
    let card = mechanical_part_cards(s.document()).remove(0);
    assert_eq!(card.dimensions_mm["Bore"], 3.2);
    assert_eq!(card.catalog_facts, Some(facts()));
    assert_eq!(card.catalog_facts.unwrap().material, None);
    let before = s.document().clone();
    let revision = s.revision();
    let mut invalid = vec![];
    for nominal in [0., -3., f64::NAN, f64::INFINITY] {
        invalid.push(MechanicalPartCatalogFacts {
            nominal_fastener_diameter_mm: Some(nominal),
            ..Default::default()
        });
    }
    for text in [
        "".into(),
        "   ".into(),
        "normal\nseries".into(),
        "x".repeat(4097),
    ] {
        for field in 0..3 {
            let mut value = facts();
            match field {
                0 => value.series = Some(text.clone()),
                1 => value.material = Some(text.clone()),
                _ => value.grade = Some(text.clone()),
            }
            invalid.push(value);
        }
    }
    for value in invalid {
        let mut bad = MechanicalPartRequest::new("Invalid facts", washer());
        bad.provenance = request.provenance.clone();
        bad.provenance.as_mut().unwrap().catalog_facts = Some(value);
        assert!(prepare_mechanical_part(&s, revision, actor(), &bad).is_err());
        assert_eq!(s.document(), &before);
        assert_eq!(s.revision(), revision);
    }
    assert!(
        serde_json::from_value::<MechanicalPartCatalogFacts>(
            serde_json::json!({"guessed_material": "steel"})
        )
        .is_err()
    );
}

#[test]
fn reader_capability_covers_direct_suppressed_and_template_only_sources() {
    let mut s = session();
    let request = add_washer(&mut s);
    assert_reader_gate(&s);
    let author = SymbolAuthorRequest::new(
        "Reusable washer",
        request.body,
        SymbolKind::Static,
        vec![],
        None,
    );
    let prepared = symbols::prepare_author(&s, s.revision(), actor(), &author).unwrap();
    s.commit(prepared.prepared).unwrap();
    let definition = symbols::read_definition(s.document(), author.definition).unwrap();
    let mut host = session();
    let prepared = symbols::prepare_import(&host, 0, actor(), &definition).unwrap();
    host.commit(prepared.prepared).unwrap();
    assert!(
        !host
            .document()
            .entities()
            .any(|e| matches!(e.data, EntityData::Feature { .. }))
    );
    assert_reader_gate(&host);
    let mut feature = s.document().get(request.feature).unwrap().clone();
    let EntityData::Feature { suppressed, .. } = &mut feature.data else {
        unreachable!()
    };
    *suppressed = true;
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Suppress source".into(),
        edits: vec![Mutation::Replace(feature)],
    })
    .unwrap();
    assert_reader_gate(&s);
}
