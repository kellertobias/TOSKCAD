use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Mutation, Placement},
    model::*,
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    symbols::{self, SymbolAuthorRequest, SymbolKind, SymbolPlaceRequest, SymbolValue},
};
use std::collections::BTreeMap;
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn session() -> Session {
    Session::new(Document::new("Mechanical part source").unwrap()).unwrap()
}
fn washer() -> MechanicalPartParameters {
    MechanicalPartParameters::Washer {
        bore_diameter: length(3.2),
        outside_diameter: length(7.),
        thickness: length(0.5),
        frame: Default::default(),
    }
}

#[test]
fn hardware_attribution_keeps_assigned_material_and_retained_source_distinct() {
    let s = session();
    let mut part = MechanicalPartDefinition {
        output: BodyOutput {
            body: EntityId::new(),
        },
        parameters: washer(),
        provenance: Some(MechanicalPartProvenance {
            catalog_id: "test-only".into(),
            revision: "fixture-1".into(),
            designation: "Test ring".into(),
            source_url: "https://example.invalid/fixture".into(),
            reference_geometry: washer(),
            catalog_facts: Some(MechanicalPartCatalogFacts {
                material: Some("Steel".into()),
                ..Default::default()
            }),
        }),
    };
    let unassigned = part.attribution(s.parameters(), None, None).unwrap();
    assert_eq!(
        unassigned.basis,
        MechanicalAttributionBasis::ReferenceDimensions
    );
    assert_eq!(unassigned.material_match, None);
    assert_eq!(unassigned.actual_material, None);
    assert_eq!(
        unassigned.reference.unwrap().material.as_deref(),
        Some("Steel")
    );

    let changed = part
        .attribution(s.parameters(), Some("Brass"), None)
        .unwrap();
    assert_eq!(changed.basis, MechanicalAttributionBasis::CustomDerived);
    assert_eq!(changed.dimensions_match, Some(true));
    assert_eq!(changed.material_match, Some(false));
    assert_eq!(changed.actual_material.as_deref(), Some("Brass"));
    assert_eq!(changed.reference.unwrap().designation, "Test ring");

    let reference = part.provenance.as_mut().unwrap();
    reference.catalog_facts = None;
    let retained = part
        .attribution(s.parameters(), Some("Brass"), Some("Brass"))
        .unwrap();
    assert_eq!(
        retained.basis,
        MechanicalAttributionBasis::ReferenceDimensions
    );
    assert_eq!(retained.material_match, Some(true));
    assert!(retained.reference.unwrap().material.is_none());
    let assigned = part
        .attribution(s.parameters(), Some("Brass"), None)
        .unwrap();
    assert_eq!(assigned.basis, MechanicalAttributionBasis::CustomDerived);
    assert_eq!(assigned.material_match, None);

    let MechanicalPartParameters::Washer { thickness, .. } = &mut part.parameters else {
        panic!()
    };
    *thickness = length(1.);
    let changed = part.attribution(s.parameters(), None, None).unwrap();
    assert_eq!(changed.basis, MechanicalAttributionBasis::CustomDerived);
    assert_eq!(changed.dimensions_match, Some(false));
    assert_eq!(changed.reference.unwrap().revision, "fixture-1");
}

#[test]
fn library_cards_describe_pinned_defaults_not_changed_authoring_geometry() {
    let mut s = session();
    let mut request = MechanicalPartRequest::new("Reference washer", washer());
    request.provenance = Some(MechanicalPartProvenance {
        catalog_id: "test-only".into(),
        revision: "fixture-1".into(),
        designation: "Test ring".into(),
        source_url: "https://example.invalid/fixture".into(),
        reference_geometry: washer(),
        catalog_facts: None,
    });
    let prepared = prepare_mechanical_part(&s, s.revision(), actor(), &request).unwrap();
    s.commit(prepared.prepared).unwrap();
    let cards = mechanical_part_cards(s.document());
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].symbol, None);
    assert!(cards[0].matches_reference_dimensions);
    assert_eq!(cards[0].dimensions_mm["Bore"], 3.2);
    assert_eq!(cards[0].dimensions_mm["Outside diameter"], 7.);
    assert_eq!(cards[0].dimensions_mm["Thickness"], 0.5);
    assert!(
        !cards[0]
            .dimensions_mm
            .contains_key("Nominal thread diameter")
    );
    let author = SymbolAuthorRequest::new(
        "Reusable ring",
        request.body,
        SymbolKind::Static,
        vec![],
        None,
    );
    let prepared = symbols::prepare_author(&s, s.revision(), actor(), &author).unwrap();
    s.commit(prepared.prepared).unwrap();
    let definition = symbols::read_definition(s.document(), author.definition).unwrap();
    let mut changed = washer();
    let MechanicalPartParameters::Washer { thickness, .. } = &mut changed else {
        panic!()
    };
    *thickness = length(1.);
    let prepared =
        prepare_mechanical_part_edit(&s, s.revision(), actor(), request.feature, &changed).unwrap();
    s.commit(prepared.prepared).unwrap();
    let live = read_mechanical_part(s.document(), request.feature).unwrap();
    assert!(!live.matches_reference_dimensions(s.parameters()).unwrap());
    let cards = mechanical_part_cards(s.document());
    assert_eq!(cards.len(), 1, "do not duplicate author and template");
    assert_eq!(
        cards[0].symbol,
        Some((definition.symbol, definition.revision))
    );
    assert_eq!(cards[0].dimensions_mm["Thickness"], 0.5);
    assert!(cards[0].matches_reference_dimensions);
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(s.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let card = project_file::card(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(card.mechanical_parts, cards);
    assert_eq!(card.symbol_definitions, 1);
}
fn add_width(s: &mut Session) -> EntityId {
    let p = Entity::new(
        "outside",
        EntityData::Parameter {
            expression: "7".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let id = p.id;
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Dimension".into(),
        edits: vec![Mutation::Insert(p)],
    })
    .unwrap();
    id
}
#[test]
fn annular_dimensions_and_nut_thread_share_one_axis_and_height() {
    let s = session();
    let ResolvedMechanicalPart::Washer {
        bore_radius_mm,
        outside_radius_mm,
        thickness_mm,
        ..
    } = washer().resolve(s.parameters()).unwrap()
    else {
        panic!()
    };
    assert_eq!(
        (bore_radius_mm, outside_radius_mm, thickness_mm),
        (1.6, 3.5, 0.5)
    );
    let bearing = MechanicalPartParameters::BearingEnvelope {
        bore_diameter: length(10.),
        outside_diameter: length(30.),
        width: length(9.),
        frame: Default::default(),
    };
    let ResolvedMechanicalPart::BearingEnvelope {
        bore_radius_mm,
        outside_radius_mm,
        width_mm,
        ..
    } = bearing.resolve(s.parameters()).unwrap()
    else {
        panic!()
    };
    assert_eq!((bore_radius_mm, outside_radius_mm, width_mm), (5., 15., 9.));
    let nut = MechanicalPartParameters::HexNut {
        across_flats: length(5.5),
        thread: Box::new(ThreadParameters {
            major_diameter: length(3.),
            pitch: length(0.5),
            length: length(2.4),
            ..Default::default()
        }),
    };
    let ResolvedMechanicalPart::HexNut {
        across_flats_mm,
        thread,
    } = nut.resolve(s.parameters()).unwrap()
    else {
        panic!()
    };
    assert_eq!(across_flats_mm, 5.5);
    assert_eq!(thread.length_mm, 2.4);
    assert_eq!(thread.major_radius_mm, 1.5);
    assert_eq!(thread.representation, ThreadRepresentation::Modeled);
}
#[test]
fn preparation_history_reopen_and_tampered_dependencies_preserve_canonical_ownership() {
    let mut s = session();
    let width = add_width(&mut s);
    let mut p = washer();
    let MechanicalPartParameters::Washer {
        outside_diameter, ..
    } = &mut p
    else {
        panic!()
    };
    *outside_diameter = Dimension::Expression {
        expression: BoundExpression::bind(s.document(), "=$outside", "mm", None).unwrap(),
    };
    let r = MechanicalPartRequest::new("Parametric washer", p);
    let before = s.document().clone();
    let prepared = prepare_mechanical_part(&s, s.revision(), actor(), &r).unwrap();
    assert_eq!(s.document(), &before);
    s.commit(prepared.prepared).unwrap();
    let committed = s.document().clone();
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &before);
    s.redo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &committed);
    let mut p = r.parameters.clone();
    let MechanicalPartParameters::Washer { thickness, .. } = &mut p else {
        panic!()
    };
    *thickness = length(0.8);
    let edit = prepare_mechanical_part_edit(&s, s.revision(), actor(), r.feature, &p).unwrap();
    s.commit(edit.prepared).unwrap();
    assert_eq!(
        read_mechanical_part(s.document(), r.feature)
            .unwrap()
            .output
            .body,
        r.body
    );
    let mut parameter = s.document().get(width).unwrap().clone();
    let EntityData::Parameter { expression, .. } = &mut parameter.data else {
        panic!()
    };
    *expression = "9".into();
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Resize".into(),
        edits: vec![Mutation::Replace(parameter)],
    })
    .unwrap();
    let ResolvedMechanicalPart::Washer {
        outside_radius_mm, ..
    } = read_mechanical_part(s.document(), r.feature)
        .unwrap()
        .parameters
        .resolve(s.parameters())
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(outside_radius_mm, 4.5);
    let file = ProjectFile::new(s.document().clone()).unwrap();
    assert!(file.capabilities().contains(MECHANICAL_PART_CAPABILITY));
    let mut bytes = vec![];
    project_file::write(&file, &mut bytes, Default::default()).unwrap();
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document;
    assert_eq!(&reopened, s.document());
    let mut damaged = reopened.get(r.feature).unwrap().clone();
    let EntityData::Feature { inputs, .. } = &mut damaged.data else {
        panic!()
    };
    inputs.clear();
    assert!(reopened.edited(&[Mutation::Replace(damaged)]).is_err());
    assert!(prepare_mechanical_part(&s, s.revision() - 1, actor(), &r).is_err());
}
#[test]
fn invalid_rings_nut_walls_units_and_nonliteral_reference_facts_are_refused() {
    let s = session();
    for (bore, outside, thickness) in [(3., 3., 0.5), (3., 2., 0.5), (0., 7., 0.5), (3., 7., 0.)] {
        let p = MechanicalPartParameters::Washer {
            bore_diameter: length(bore),
            outside_diameter: length(outside),
            thickness: length(thickness),
            frame: Default::default(),
        };
        assert!(
            prepare_mechanical_part(&s, 0, actor(), &MechanicalPartRequest::new("Invalid", p))
                .is_err()
        );
    }
    let p = MechanicalPartParameters::HexNut {
        across_flats: length(3.1),
        thread: Box::new(ThreadParameters {
            major_diameter: length(3.),
            pitch: length(0.5),
            length: length(2.4),
            ..Default::default()
        }),
    };
    assert!(p.resolve(s.parameters()).is_err());
    let mut request = MechanicalPartRequest::new("Wrong unit", washer());
    let MechanicalPartParameters::Washer { thickness, .. } = &mut request.parameters else {
        panic!()
    };
    *thickness = Dimension::Literal {
        value: 1.,
        unit: "deg".into(),
    };
    assert!(prepare_mechanical_part(&s, 0, actor(), &request).is_err());
    let mut s = session();
    add_width(&mut s);
    let mut facts = washer();
    let MechanicalPartParameters::Washer {
        outside_diameter, ..
    } = &mut facts
    else {
        panic!()
    };
    *outside_diameter = Dimension::Expression {
        expression: BoundExpression::bind(s.document(), "=$outside", "mm", None).unwrap(),
    };
    let mut request = MechanicalPartRequest::new("Closed facts", washer());
    request.provenance = Some(MechanicalPartProvenance {
        catalog_id: "test".into(),
        revision: "1".into(),
        designation: "Fixture".into(),
        source_url: "https://example.invalid/fixture".into(),
        reference_geometry: facts,
        catalog_facts: None,
    });
    assert!(prepare_mechanical_part(&s, s.revision(), actor(), &request).is_err());
}
#[test]
fn pinned_variants_remap_real_parameters_but_never_uuid_looking_catalog_prose() {
    let mut library = session();
    let width = add_width(&mut library);
    let mut p = washer();
    let MechanicalPartParameters::Washer {
        outside_diameter, ..
    } = &mut p
    else {
        panic!()
    };
    *outside_diameter = Dimension::Expression {
        expression: BoundExpression::bind(library.document(), "=$outside", "mm", None).unwrap(),
    };
    let mut request = MechanicalPartRequest::new("Reference washer", p);
    let provenance = MechanicalPartProvenance {
        catalog_id: request.feature.to_string(),
        revision: width.to_string(),
        designation: EntityId::nil().to_string(),
        source_url: format!(
            "https://example.invalid/catalog/{}/{}",
            request.body,
            EntityId::nil()
        ),
        reference_geometry: washer(),
        catalog_facts: Some(MechanicalPartCatalogFacts {
            nominal_fastener_diameter_mm: Some(3.),
            series: Some(width.to_string()),
            material: Some(request.feature.to_string()),
            grade: Some(EntityId::nil().to_string()),
        }),
    };
    request.provenance = Some(provenance.clone());
    let p = prepare_mechanical_part(&library, library.revision(), actor(), &request).unwrap();
    library.commit(p.prepared).unwrap();
    let authored = SymbolAuthorRequest::new(
        "Washer",
        request.body,
        SymbolKind::Dynamic,
        vec![width],
        None,
    );
    let p = symbols::prepare_author(&library, library.revision(), actor(), &authored).unwrap();
    library.commit(p.prepared).unwrap();
    let definition = symbols::read_definition(library.document(), authored.definition).unwrap();
    let pinned = definition.pin().unwrap();
    let mut host = session();
    let p = symbols::prepare_import(&host, 0, actor(), &definition).unwrap();
    let imported = p.definition;
    host.commit(p.prepared).unwrap();
    let placement = SymbolPlaceRequest::new(
        host.document(),
        imported,
        "Larger washer",
        BTreeMap::from([(
            width,
            SymbolValue {
                value: 10.,
                unit: "mm".into(),
            },
        )]),
        Placement::default(),
    )
    .unwrap();
    let default_draft = symbols::configured_hardware_parts(&definition, &BTreeMap::new()).unwrap();
    assert_eq!(default_draft.len(), 1);
    assert_eq!(
        default_draft[0].attribution.basis,
        MechanicalAttributionBasis::ReferenceDimensions
    );
    let draft = symbols::configured_hardware_parts(&definition, &placement.overrides).unwrap();
    assert_eq!(draft.len(), 1);
    assert_eq!(
        draft[0].attribution.basis,
        MechanicalAttributionBasis::CustomDerived
    );
    assert_eq!(draft[0].retained, default_draft[0].current);
    assert_eq!(draft[0].attribution.actual_material, None);
    assert_eq!(definition.pin().unwrap(), pinned);
    let p = symbols::prepare_place(&host, host.revision(), actor(), &placement).unwrap();
    host.commit(p.prepared).unwrap();
    let record = symbols::read_instance(host.document(), placement.instance).unwrap();
    assert_eq!(record.pin, pinned);
    let mapped_feature = record.ids[&request.feature];
    assert_ne!(mapped_feature, request.feature);
    let part = read_mechanical_part(host.document(), mapped_feature).unwrap();
    assert_eq!(part.provenance, Some(provenance));
    assert_eq!(
        part.parameters.resolve(host.parameters()).unwrap(),
        draft[0].current
    );
    let ResolvedMechanicalPart::Washer {
        outside_radius_mm, ..
    } = part.parameters.resolve(host.parameters()).unwrap()
    else {
        panic!()
    };
    assert_eq!(outside_radius_mm, 5.);
    assert_eq!(
        read_mechanical_part(library.document(), request.feature)
            .unwrap()
            .parameters
            .resolve(library.parameters())
            .unwrap(),
        washer().resolve(library.parameters()).unwrap()
    );
}

#[test]
fn source_dimension_matching_ignores_placement_but_detects_customized_dimensions() {
    let s = session();
    let reference = washer();
    let mut p = reference.clone();
    let MechanicalPartParameters::Washer {
        outside_diameter,
        frame,
        ..
    } = &mut p
    else {
        panic!()
    };
    *outside_diameter = Dimension::Literal {
        value: 0.7,
        unit: "cm".into(),
    };
    frame.origin = [length(40.), length(20.), length(30.)];
    frame.u = [0., 1., 0.];
    frame.v = [0., 0., 1.];
    let mut source = MechanicalPartDefinition {
        output: BodyOutput {
            body: EntityId::new(),
        },
        parameters: p,
        provenance: Some(MechanicalPartProvenance {
            catalog_id: "test/reference".into(),
            revision: "1".into(),
            designation: "Dimensional fixture".into(),
            source_url: "https://example.invalid/reference".into(),
            reference_geometry: reference,
            catalog_facts: None,
        }),
    };
    assert!(source.matches_reference_dimensions(s.parameters()).unwrap());
    let MechanicalPartParameters::Washer { thickness, .. } = &mut source.parameters else {
        panic!()
    };
    *thickness = length(0.8);
    assert!(!source.matches_reference_dimensions(s.parameters()).unwrap());
    let nut = MechanicalPartParameters::HexNut {
        across_flats: length(5.5),
        thread: Box::new(ThreadParameters {
            major_diameter: length(3.),
            pitch: length(0.5),
            length: length(2.4),
            ..Default::default()
        }),
    };
    source.parameters = nut.clone();
    source.provenance.as_mut().unwrap().reference_geometry = nut;
    let MechanicalPartParameters::HexNut { thread, .. } = &mut source.parameters else {
        panic!()
    };
    thread.representation = ThreadRepresentation::CosmeticEnvelope;
    assert!(source.matches_reference_dimensions(s.parameters()).unwrap());
    let MechanicalPartParameters::HexNut { thread, .. } = &mut source.parameters else {
        panic!()
    };
    thread.radial_allowance = length(0.1);
    assert!(!source.matches_reference_dimensions(s.parameters()).unwrap());
}
