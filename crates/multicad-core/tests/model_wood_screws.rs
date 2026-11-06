use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Mutation},
    model::*,
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    symbols::{self, SymbolAuthorRequest, SymbolKind, SymbolPlaceRequest},
};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn session() -> Session {
    Session::new(Document::new("Wood screw contract").unwrap()).unwrap()
}
fn wood(p: &mut MechanicalPartParameters) -> &mut WoodScrewParameters {
    let MechanicalPartParameters::WoodScrew { parameters } = p else {
        unreachable!()
    };
    parameters
}
fn insert(s: &mut Session, p: MechanicalPartParameters) -> MechanicalPartRequest {
    let r = MechanicalPartRequest::new("Wood 6 × 40 envelope", p);
    let prepared = prepare_mechanical_part(s, s.revision(), actor(), &r).unwrap();
    s.commit(prepared.prepared).unwrap();
    r
}
fn reference() -> MechanicalPartProvenance {
    MechanicalPartProvenance {
        catalog_id: "fixture-wood-d6-l40".into(),
        revision: "1".into(),
        designation: "Wood 6 × 40 source reference".into(),
        source_url: "https://example.invalid/wood-reference".into(),
        reference_geometry: default_wood_screw(),
        catalog_facts: None,
    }
}
#[test]
fn wood_is_distinct_flat_source_with_reference_bounds_not_a_metric_thread() {
    let s = session();
    let p = default_wood_screw();
    assert!(p.thread_parameters().is_none());
    let j = serde_json::to_value(&p).unwrap();
    assert_eq!(j["kind"], "wood_screw");
    assert!(j.get("parameters").is_none());
    for key in [
        "thread",
        "shank_diameter",
        "underhead_length",
        "fit",
        "pitch_diameter",
        "thread_start",
        "actual_thread_length",
        "representation_parameter",
    ] {
        assert!(j.get(key).is_none(), "unexpected {key}");
    }
    assert_eq!(j["point"], "manufacturer_unspecified");
    assert_eq!(j["representation"], "simplified_envelope");
    assert_eq!(
        serde_json::from_value::<MechanicalPartParameters>(j.clone()).unwrap(),
        p
    );
    let ResolvedMechanicalPart::WoodScrew { parameters: r } = p.resolve(s.parameters()).unwrap()
    else {
        unreachable!()
    };
    assert_eq!(r.underhead_length_mm, 40.);
    assert_eq!(r.overall_length_mm, 44.);
    assert_eq!(r.nominal_diameter_mm, 6.);
    assert_eq!(r.reference_profile.pitch_mm, 2.6);
    assert_eq!(r.reference_profile.core_diameter_mm, 4.2);
    assert!((r.reference_profile.included_angle_deg - 60.).abs() < 1e-12);
    assert_eq!(r.minimum_tip_inclusive_thread_extent_mm, 24.);
    assert_eq!(r.maximum_possible_plain_length_mm, 16.);
    let mut overall = default_wood_screw();
    wood(&mut overall).length_input = ScrewLengthInput::Overall { value: length(44.) };
    let ResolvedMechanicalPart::WoodScrew { parameters: q } =
        overall.resolve(s.parameters()).unwrap()
    else {
        unreachable!()
    };
    assert_eq!(q.underhead_length_mm, r.underhead_length_mm);
    assert_eq!(q.authored_length_mm, 44.);
    assert_eq!(q.length_convention, ScrewLengthConvention::Overall);
    for (key, value) in [
        ("shank_diameter", serde_json::to_value(length(6.)).unwrap()),
        ("representation", serde_json::json!("modeled")),
        ("point", serde_json::json!("sharp_cone")),
    ] {
        let mut invalid = j.clone();
        invalid[key] = value;
        assert!(serde_json::from_value::<MechanicalPartParameters>(invalid).is_err());
    }
    // A reference pitch does not manufacture an exact threaded span.
    let mut p = default_wood_screw();
    wood(&mut p).reference_profile.pitch = length(100.);
    assert!(p.resolve(s.parameters()).is_ok());
}
#[test]
fn wood_invalid_units_dimensions_minimum_and_unsupported_geometry_are_atomic() {
    let s = session();
    let before = s.document().clone();
    let mut bad = vec![];
    for (field, value) in [
        ("nominal_diameter", 0.),
        ("reference_profile/pitch", 0.),
        ("reference_profile/core_diameter", 6.),
        ("reference_profile/included_angle", 0.),
        ("reference_profile/included_angle", 180.),
        ("minimum_tip_inclusive_thread_extent", 23.9),
        ("minimum_tip_inclusive_thread_extent", 40.1),
        ("head/across_flats", 5.9),
        ("head/height", 0.),
    ] {
        let mut v = serde_json::to_value(default_wood_screw()).unwrap();
        v.pointer_mut(&format!("/{field}")).unwrap()["value"] = serde_json::json!(value);
        bad.push(serde_json::from_value(v).unwrap());
    }
    let mut p = default_wood_screw();
    wood(&mut p).reference_profile.included_angle = length(60.);
    bad.push(p);
    let mut p = default_wood_screw();
    wood(&mut p).length_input = ScrewLengthInput::Overall { value: length(4.) };
    bad.push(p);
    let mut p = default_wood_screw();
    wood(&mut p).head = ScrewHead::SocketCap {
        diameter: length(10.),
        height: length(4.),
    };
    bad.push(p);
    let mut p = default_wood_screw();
    wood(&mut p).drive = ScrewDrive::HexSocket {
        across_flats: length(4.),
        depth: length(2.),
    };
    bad.push(p);
    for parameters in bad {
        let request = MechanicalPartRequest::new("Refuse", parameters);
        assert!(prepare_mechanical_part(&s, 0, actor(), &request).is_err());
        assert_eq!(s.document(), &before);
        assert_eq!(s.revision(), 0);
        assert!(!s.can_undo());
    }
}
#[test]
fn wood_sources_bind_length_and_minimum_and_keep_literal_reference_history_and_reopen() {
    let mut s = session();
    let mut p = default_wood_screw();
    let parameter = Entity::new(
        "wood_length",
        EntityData::Parameter {
            expression: "40".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let id = parameter.id;
    s.apply(Request {
        expected_revision: 0,
        actor: actor(),
        label: "Wood length".into(),
        edits: vec![Mutation::Insert(parameter)],
    })
    .unwrap();
    wood(&mut p).length_input = ScrewLengthInput::Underhead {
        value: Dimension::Expression {
            expression: BoundExpression::bind(s.document(), "=$wood_length", "mm", None).unwrap(),
        },
    };
    wood(&mut p).minimum_tip_inclusive_thread_extent = Dimension::Expression {
        expression: BoundExpression::bind(s.document(), "=$wood_length * 0.6", "mm", None).unwrap(),
    };
    let mut request = MechanicalPartRequest::new("Bound wood envelope", p.clone());
    request.provenance = Some(reference());
    let prepared = prepare_mechanical_part(&s, s.revision(), actor(), &request).unwrap();
    assert!(s.document().get(request.feature).is_none());
    s.commit(prepared.prepared).unwrap();
    let source = read_mechanical_part(s.document(), request.feature).unwrap();
    assert!(source.matches_reference_dimensions(s.parameters()).unwrap());
    let EntityData::Feature { inputs, .. } = &s.document().get(request.feature).unwrap().data
    else {
        unreachable!()
    };
    assert!(inputs.iter().any(|link| link.target == id));
    let created = s.document().clone();
    let mut changed = p.clone();
    wood(&mut changed).nominal_diameter = length(6.2);
    let prepared =
        prepare_mechanical_part_edit(&s, s.revision(), actor(), request.feature, &changed).unwrap();
    s.commit(prepared.prepared).unwrap();
    let configured = read_mechanical_part(s.document(), request.feature).unwrap();
    assert_eq!(configured.output.body, request.body);
    assert_eq!(configured.provenance, request.provenance);
    assert!(
        !configured
            .matches_reference_dimensions(s.parameters())
            .unwrap()
    );
    assert_eq!(
        wood(&mut configured.parameters.clone()).length_input,
        wood(&mut p.clone()).length_input
    );
    let saved = s.document().clone();
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &created);
    s.redo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &saved);
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(saved.clone()).unwrap(),
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
    assert_eq!(independent.revision(), 0);
    assert_eq!(independent.document(), &saved);
    assert_eq!(
        read_mechanical_part(independent.document(), request.feature).unwrap(),
        configured
    );
    let mut invalid = MechanicalPartRequest::new("Invalid bound reference", p.clone());
    invalid.provenance = Some(reference());
    invalid.provenance.as_mut().unwrap().reference_geometry = p;
    assert!(prepare_mechanical_part(&s, s.revision(), actor(), &invalid).is_err());
}
#[test]
fn neutral_manufacturing_history_distinguishes_wood_cosmetic_and_unrelated_modeled_parts() {
    let mut s = session();
    let w = insert(&mut s, default_wood_screw());
    assert!(has_manufacturing_limitations(s.document()).unwrap());
    assert!(!has_cosmetic_threads(s.document()).unwrap());
    assert_eq!(
        cosmetic_thread_in_history(s.document(), w.feature).unwrap(),
        None
    );
    let limit = manufacturing_limitation_in_history(s.document(), w.feature)
        .unwrap()
        .unwrap();
    assert_eq!(limit.feature, w.feature);
    assert_eq!(limit.kind, ManufacturingLimitationKind::SimplifiedWoodScrew);
    assert!(limit.message().contains("point"));
    assert!(!limit.message().contains("switch"));
    let transform = TransformRequest::new(
        BodyInput {
            body: w.body,
            producer: w.feature,
        },
        NumericTransform {
            translation: [length(2.), length(3.), length(4.)],
            ..Default::default()
        },
    );
    let p = prepare_transform(&s, s.revision(), actor(), &transform).unwrap();
    s.commit(p.prepared).unwrap();
    assert_eq!(
        manufacturing_limitation_in_history(s.document(), transform.feature).unwrap(),
        Some(limit)
    );
    let solid = PrimitiveRequest::new(
        "Unrelated modeled box",
        PrimitiveParameters::Box {
            size: [length(1.), length(2.), length(3.)],
            frame: Default::default(),
        },
    );
    let p = prepare_primitive(&s, s.revision(), actor(), &solid).unwrap();
    s.commit(p.prepared).unwrap();
    assert_eq!(
        manufacturing_limitation_in_history(s.document(), solid.feature).unwrap(),
        None
    );
    let mut machine = default_machine_screw();
    let MechanicalPartParameters::MachineScrew { thread, .. } = &mut machine else {
        unreachable!()
    };
    thread.representation = ThreadRepresentation::CosmeticEnvelope;
    let m = insert(&mut s, machine);
    let cosmetic = manufacturing_limitation_in_history(s.document(), m.feature)
        .unwrap()
        .unwrap();
    assert_eq!(cosmetic.kind, ManufacturingLimitationKind::CosmeticThread);
    assert!(cosmetic.message().contains("cosmetic") && cosmetic.message().contains("Modeled"));
    assert_eq!(
        cosmetic_thread_in_history(s.document(), m.feature).unwrap(),
        Some(m.feature)
    );
    assert!(has_cosmetic_threads(s.document()).unwrap());
    assert!(manufacturing_limitation_in_history(s.document(), EntityId::new()).is_err());
}
#[test]
fn reader_cards_and_private_symbol_sources_keep_wood_semantics_without_machine_caps() {
    use multicad_core::document::Placement;
    let mut s = session();
    let w = insert(&mut s, default_wood_screw());
    let cards = mechanical_part_cards(s.document());
    assert_eq!(cards.len(), 1);
    assert!(cards[0].family.contains("Wood"));
    assert!(
        cards[0]
            .representation
            .contains("manufacturing unavailable")
    );
    assert!(!format!("{:?}", cards[0]).contains("M6"));
    assert!(!cards[0].dimensions_mm.contains_key("Thread length"));
    let author = SymbolAuthorRequest::new(
        "Wood envelope pin",
        w.body,
        SymbolKind::Static,
        vec![],
        None,
    );
    let p = symbols::prepare_author(&s, s.revision(), actor(), &author).unwrap();
    s.commit(p.prepared).unwrap();
    let definition = symbols::read_definition(s.document(), author.definition).unwrap();
    let mut host = session();
    let p = symbols::prepare_import(&host, 0, actor(), &definition).unwrap();
    let imported = p.definition;
    host.commit(p.prepared).unwrap();
    assert!(
        !has_manufacturing_limitations(host.document()).unwrap(),
        "Template-only wood must not blanket-block unrelated manufacturing"
    );
    fn reader(document: &Document) {
        let project = ProjectFile::new(document.clone()).unwrap();
        let caps = project.capabilities();
        assert!(caps.contains(WOOD_SCREW_CAPABILITY));
        assert!(!caps.contains(MACHINE_SCREW_CAPABILITY));
        assert!(!caps.contains(MACHINE_SCREW_LENGTH_INPUT_CAPABILITY));
        assert!(!caps.contains(SCREW_PROFILES_CAPABILITY));
        let mut bytes = vec![];
        project_file::write(&project, &mut bytes, Default::default()).unwrap();
        let mut old = project_file::supported_capabilities();
        old.remove(WOOD_SCREW_CAPABILITY);
        assert!(
            project_file::read(&mut bytes.as_slice(), Default::default(), &old)
                .unwrap_err()
                .contains(WOOD_SCREW_CAPABILITY)
        );
        assert_eq!(
            project_file::read(
                &mut bytes.as_slice(),
                Default::default(),
                &project_file::supported_capabilities()
            )
            .unwrap()
            .project
            .document,
            *document
        );
    }
    reader(s.document());
    reader(host.document());
    let placed = SymbolPlaceRequest::new(
        host.document(),
        imported,
        "Private wood envelope",
        Default::default(),
        Placement::default(),
    )
    .unwrap();
    let p = symbols::prepare_place(&host, host.revision(), actor(), &placed).unwrap();
    host.commit(p.prepared).unwrap();
    let instance = symbols::read_instance(host.document(), placed.instance).unwrap();
    let mapped = instance.ids[&w.feature];
    assert_ne!(mapped, w.feature);
    assert_eq!(
        manufacturing_limitation_in_history(host.document(), mapped)
            .unwrap()
            .unwrap()
            .kind,
        ManufacturingLimitationKind::SimplifiedWoodScrew
    );
    assert!(!has_cosmetic_threads(host.document()).unwrap());
    reader(host.document());
    let mut entity = s.document().get(w.feature).unwrap().clone();
    let EntityData::Feature { suppressed, .. } = &mut entity.data else {
        unreachable!()
    };
    *suppressed = true;
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Suppressed wood source".into(),
        edits: vec![Mutation::Replace(entity)],
    })
    .unwrap();
    reader(s.document());
}
