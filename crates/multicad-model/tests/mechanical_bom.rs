//! The BOM reports native physical hardware leaves and their retained source.
#[path = "../../multicad-mcp/tests/support/screw_connection.rs"]
mod fixture;

use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, Entity, EntityData, EntityId, Mutation, Placement},
    materials::{Assignment, prepare_assignment},
    model::*,
    project_file::{self, ProjectFile},
    symbols::{self, SymbolAuthorRequest, SymbolKind, SymbolPlaceRequest, SymbolValue},
};
use multicad_model::{
    Compiler, Model,
    schedules::{BomOptions, Supply, bom_schedule},
};
use std::collections::BTreeMap;

fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn compiler() -> Compiler {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
}
fn native(session: &Session) -> Model {
    compiler()
        .compile(session.document(), session.revision())
        .unwrap()
}
fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 2e-6,
        "expected {expected}, got {actual}"
    );
}
fn read(mut bytes: &[u8]) -> Document {
    project_file::read(
        &mut bytes,
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document
}
fn reopen(session: &Session) -> Session {
    let mut bytes = Vec::new();
    project_file::write(
        &ProjectFile::new(session.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    Session::new(read(&bytes)).unwrap()
}

fn assign_material(session: &mut Session, part: EntityId, name: &str, density: f64) -> EntityId {
    let material = Entity::new(
        name,
        EntityData::Material {
            density_kg_m3: Some(density),
            color_rgba: [128, 128, 128, 255],
            section_hatch: None,
        },
    );
    let material_id = material.id;
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Add material".into(),
            edits: vec![Mutation::Insert(material)],
        })
        .unwrap();
    let assignment = prepare_assignment(
        session,
        session.revision(),
        actor(),
        &[part],
        &Assignment {
            material: Some(material_id),
            stock: None,
            appearance: None,
            hatch: None,
        },
    )
    .unwrap();
    session.commit(assignment).unwrap();
    material_id
}

#[test]
fn pinned_bearing_keeps_reference_while_two_physical_rows_report_live_dimensions() {
    let library_bytes = include_bytes!("../../../library/bearing-open-608.tcd");
    let source = read(library_bytes);
    let [(.., definition)] = symbols::definitions(&source)
        .unwrap()
        .try_into()
        .ok()
        .unwrap();
    let width = definition
        .exposed
        .iter()
        .copied()
        .find(|id| {
            definition
                .template
                .get(*id)
                .unwrap()
                .name
                .to_ascii_lowercase()
                .contains("width")
        })
        .expect("shipped 608 exposes width");
    let expected_pin = definition.pin().unwrap();
    let mut session = Session::new(Document::new("Two 608 occurrences").unwrap()).unwrap();
    let mut requests = Vec::new();
    let embedded = EntityId::new();
    for (x, configured) in [(0., false), (40., true)] {
        let overrides = if configured {
            BTreeMap::from([(
                width,
                SymbolValue {
                    value: 14.,
                    unit: "mm".into(),
                },
            )])
        } else {
            BTreeMap::new()
        };
        let request = SymbolPlaceRequest::from_definition(
            embedded,
            &definition,
            if configured {
                "Configured 608"
            } else {
                "Reference 608"
            },
            overrides,
            Placement {
                translation_mm: [x, 0., 0.],
                ..Default::default()
            },
        )
        .unwrap();
        let placement = symbols::prepare_library_place(
            &session,
            session.revision(),
            actor(),
            &definition,
            &request,
            None,
        )
        .unwrap();
        session.commit(placement.prepared).unwrap();
        requests.push(request);
    }
    let before = session.document().clone();
    let model = native(&session);
    let bom = model
        .bom(session.document(), &BomOptions::default())
        .unwrap();
    assert_eq!(bom.rows.len(), 2);
    assert_eq!(model.snapshot().leaf_occurrences.len(), 2);
    for (request, width, x, basis) in [
        (
            &requests[0],
            7.,
            0.,
            MechanicalAttributionBasis::ReferenceDimensions,
        ),
        (
            &requests[1],
            14.,
            40.,
            MechanicalAttributionBasis::CustomDerived,
        ),
    ] {
        let row = bom
            .rows
            .iter()
            .find(|row| row.part == request.ids[&definition.root])
            .unwrap();
        let hardware = row.hardware.as_ref().unwrap();
        let pin = hardware.library.as_ref().unwrap();
        assert_eq!(
            (pin.record, pin.instance, pin.pin),
            (request.record, request.instance, expected_pin)
        );
        assert_eq!(pin.template_part, definition.root);
        assert_eq!(request.ids[&pin.template_feature], hardware.feature);
        assert_eq!(hardware.attribution.basis, basis);
        assert_eq!(hardware.attribution.dimensions_match, Some(width == 7.));
        assert!(
            hardware
                .attribution
                .reference
                .as_ref()
                .unwrap()
                .designation
                .contains("608")
        );
        assert_eq!(row.quantity, 1);
        assert_eq!(row.occurrences[0].root, request.instance);
        assert_eq!(hardware.occurrences[0].path, row.occurrences[0]);
        assert!(hardware.occurrences[0].connection.is_none());
        close(hardware.occurrences[0].mount.unwrap().origin_mm[0], x);
        assert_eq!(hardware.final_producer, hardware.feature);
        assert_eq!(hardware.final_solid_count, 1);
        let dimensions = |recipe: &ResolvedMechanicalPart| match recipe {
            ResolvedMechanicalPart::BearingEnvelope {
                bore_radius_mm,
                outside_radius_mm,
                width_mm,
                ..
            } => (2. * bore_radius_mm, 2. * outside_radius_mm, *width_mm),
            _ => panic!("608 must resolve a bearing envelope"),
        };
        assert_eq!(dimensions(&hardware.resolved_recipe), (8., 22., width));
        assert_eq!(
            dimensions(hardware.reference_recipe.as_ref().unwrap()),
            (8., 22., 7.)
        );
        close(
            hardware.final_bounds.max[2] - hardware.final_bounds.min[2],
            width,
        );
        close(
            hardware.final_volume_mm3,
            std::f64::consts::PI * (121. - 16.) * width,
        );
    }
    let schedule = bom_schedule(&bom);
    let csv = schedule.csv();
    assert!(csv.contains("Custom / derived"));
    assert!(csv.contains("Bearing Ø8/22 × 14 mm"));
    assert!(csv.contains("Bearing Ø8/22 × 7 mm"));
    assert!(csv.contains("608"));
    schedule.check_table(&schedule.table([0., 0.])).unwrap();
    assert_eq!(session.document(), &before);
    let independent = reopen(&session);
    let reopened = native(&independent)
        .bom(independent.document(), &BomOptions::default())
        .unwrap();
    assert_eq!(reopened.source, bom.source);
    assert_eq!(reopened.rows, bom.rows);
}

#[test]
fn repeated_assembly_leaves_share_one_part_row_and_keep_effective_material_separate() {
    let mut session = Session::new(Document::new("Repeated washers").unwrap()).unwrap();
    let recipe = MechanicalPartParameters::Washer {
        bore_diameter: length(4.3),
        outside_diameter: length(9.),
        thickness: length(0.8),
        frame: Default::default(),
    };
    let mut washer = MechanicalPartRequest::new("Assigned washer", recipe.clone());
    washer.provenance = Some(MechanicalPartProvenance {
        catalog_id: "test.washer".into(),
        revision: "1".into(),
        designation: "Washer reference".into(),
        source_url: "https://example.invalid/washer".into(),
        reference_geometry: recipe,
        catalog_facts: Some(MechanicalPartCatalogFacts {
            material: Some("Reference steel".into()),
            ..Default::default()
        }),
    });
    let prepared = prepare_mechanical_part(&session, 0, actor(), &washer).unwrap();
    session.commit(prepared.prepared).unwrap();
    let material_id = assign_material(&mut session, washer.body, "Assigned aluminum", 2700.);
    let mut members = Vec::new();
    for x in [0., 20.] {
        let placement = ComponentPlacementRequest::new(
            washer.body,
            NumericTransform {
                translation: [length(x), length(0.), length(0.)],
                ..Default::default()
            },
        );
        let prepared =
            prepare_place_component(&session, session.revision(), actor(), &placement).unwrap();
        session.commit(prepared.prepared).unwrap();
        members.push(placement.instance);
    }
    let assembly = AssemblyRequest::new("Reusable washer pair", members.clone(), vec![washer.body]);
    let prepared = prepare_assembly(&session, session.revision(), actor(), &assembly).unwrap();
    session.commit(prepared.prepared).unwrap();
    let placement = ComponentPlacementRequest::new(
        assembly.assembly,
        NumericTransform {
            translation: [length(100.), length(0.), length(0.)],
            ..Default::default()
        },
    );
    let prepared =
        prepare_place_component(&session, session.revision(), actor(), &placement).unwrap();
    session.commit(prepared.prepared).unwrap();

    let model = native(&session);
    let bom = model
        .bom(session.document(), &BomOptions::default())
        .unwrap();
    let [row] = bom.rows.as_slice() else {
        panic!("definition Part must not add a physical row")
    };
    assert_eq!(
        (row.part, row.quantity, row.supply),
        (washer.body, 2, Supply::Manufactured)
    );
    assert_eq!(
        row.material,
        Some((material_id, "Assigned aluminum".into()))
    );
    let hardware = row.hardware.as_ref().unwrap();
    assert_eq!(
        hardware.attribution.basis,
        MechanicalAttributionBasis::CustomDerived
    );
    assert_eq!(hardware.attribution.dimensions_match, Some(true));
    assert_eq!(hardware.attribution.material_match, Some(false));
    assert_eq!(
        hardware.attribution.actual_material.as_deref(),
        Some("Assigned aluminum")
    );
    assert_eq!(
        hardware
            .attribution
            .reference
            .as_ref()
            .unwrap()
            .material
            .as_deref(),
        Some("Reference steel")
    );
    for (member, x) in members.into_iter().zip([100., 120.]) {
        let occurrence = hardware
            .occurrences
            .iter()
            .find(|leaf| leaf.path.members == [member])
            .unwrap();
        assert_eq!(occurrence.path.root, placement.instance);
        close(occurrence.mount.unwrap().origin_mm[0], x);
    }
    let csv = bom_schedule(&bom).csv();
    assert!(csv.contains("Assigned aluminum"));
    assert!(csv.contains("Reference steel"));
    assert!(csv.contains("Custom / derived"));

    let scoped = compiler()
        .compile_sources(session.document(), session.revision(), &[washer.feature])
        .unwrap();
    assert!(
        scoped
            .bom(session.document(), &BomOptions::default())
            .is_err()
    );
    let mut changed = session.document().get(material_id).unwrap().clone();
    changed.name = "New material label".into();
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Rename material".into(),
            edits: vec![Mutation::Replace(changed)],
        })
        .unwrap();
    assert!(
        model
            .bom(session.document(), &BomOptions::default())
            .is_err()
    );
}

#[test]
fn connection_generated_and_library_hardware_use_the_same_physical_bom_roles() {
    let (document, mut request, _) = fixture::fixture();
    let stock = request.receivers[0].source.output.body;
    let nut = request
        .hardware
        .iter_mut()
        .find(|row| row.role == ScrewConnectionHardwareRole::Nut)
        .unwrap();
    let ScrewConnectionHardwareSource::Generated { part } = &nut.source else {
        panic!()
    };
    let mut author = Session::new(Document::new("Pinned connection nut").unwrap()).unwrap();
    let prepared = prepare_mechanical_part(&author, 0, actor(), part).unwrap();
    author.commit(prepared.prepared).unwrap();
    let template_material = assign_material(&mut author, part.body, "Pinned nut steel", 7800.);
    let symbol = SymbolAuthorRequest::new(
        "Modeled M3 nut",
        part.body,
        SymbolKind::Static,
        Vec::new(),
        None,
    );
    let prepared = symbols::prepare_author(&author, author.revision(), actor(), &symbol).unwrap();
    author.commit(prepared.prepared).unwrap();
    let definition = symbols::read_definition(author.document(), symbol.definition).unwrap();
    let pin = definition.pin().unwrap();
    let placement = SymbolPlaceRequest::from_definition(
        EntityId::new(),
        &definition,
        "Pinned nut",
        BTreeMap::new(),
        Placement::default(),
    )
    .unwrap();
    let expected_record = placement.record;
    let mapped_material = placement.ids[&template_material];
    nut.instance = placement.instance;
    nut.source = ScrewConnectionHardwareSource::Library {
        definition: Box::new(definition),
        placement: Box::new(placement),
    };
    let mut session = Session::new(document).unwrap();
    let prepared = prepare_screw_connection(&session, 0, actor(), &request).unwrap();
    session.commit(prepared.prepared).unwrap();
    let model = native(&session);
    assert_eq!(model.snapshot().screw_connection_checks.len(), 1);
    let bom = model
        .bom(session.document(), &BomOptions::default())
        .unwrap();
    assert_eq!(bom.rows.len(), 3);
    assert!(
        bom.rows
            .iter()
            .find(|row| row.part == stock)
            .unwrap()
            .hardware
            .is_none()
    );
    for role in &request.hardware {
        let row = bom
            .rows
            .iter()
            .find(|row| {
                row.occurrences
                    .iter()
                    .any(|path| path.root == role.instance)
            })
            .unwrap();
        assert_eq!(row.quantity, 1);
        let hardware = row.hardware.as_ref().unwrap();
        let occurrence = &hardware.occurrences[0];
        let owner = occurrence.connection.as_ref().unwrap();
        assert_eq!(
            (owner.connection, owner.role_id, owner.role, owner.instance),
            (request.connection, role.role_id, role.role, role.instance)
        );
        let leaf = model.leaf_snapshot(&occurrence.path).unwrap();
        assert_eq!(row.part, leaf.component);
        assert_eq!(
            occurrence.mount.unwrap().origin_mm,
            leaf.pose.translation_mm
        );
        if role.role == ScrewConnectionHardwareRole::Nut {
            let library = hardware.library.as_ref().unwrap();
            assert_eq!((library.record, library.pin), (expected_record, pin));
            assert_eq!(
                row.material,
                Some((mapped_material, "Pinned nut steel".into()))
            );
            assert_eq!(
                hardware.attribution.actual_material.as_deref(),
                Some("Pinned nut steel")
            );
            assert_eq!(
                hardware.attribution.template_material.as_deref(),
                Some("Pinned nut steel")
            );
            assert_eq!(hardware.attribution.material_match, Some(true));
        } else {
            assert!(hardware.library.is_none());
        }
    }
}

#[test]
fn bom_follows_retained_solid_links_and_skips_obsolete_evaluated_hardware() {
    let mut session = Session::new(Document::new("Retained hardware lineage").unwrap()).unwrap();
    let washer = MechanicalPartRequest::new(
        "Washer source",
        MechanicalPartParameters::Washer {
            bore_diameter: length(4.3),
            outside_diameter: length(9.),
            thickness: length(0.8),
            frame: Default::default(),
        },
    );
    let prepared = prepare_mechanical_part(&session, 0, actor(), &washer).unwrap();
    session.commit(prepared.prepared).unwrap();
    let moved = TransformRequest::new(
        BodyInput {
            body: washer.body,
            producer: washer.feature,
        },
        NumericTransform {
            translation: [10., 10., 1.].map(length),
            ..Default::default()
        },
    );
    let prepared = prepare_transform(&session, session.revision(), actor(), &moved).unwrap();
    session.commit(prepared.prepared).unwrap();
    let stock = PrimitiveRequest::new(
        "Separate stock",
        PrimitiveParameters::Box {
            size: [20., 20., 4.].map(length),
            frame: Default::default(),
        },
    );
    let prepared = prepare_primitive(&session, session.revision(), actor(), &stock).unwrap();
    session.commit(prepared.prepared).unwrap();
    let mut cut = BooleanRequest::new(
        Operation::Cut,
        BodyInput {
            body: stock.body,
            producer: stock.feature,
        },
        vec![BodyInput {
            body: washer.body,
            producer: moved.feature,
        }],
    );
    cut.keep_tools = true;
    let prepared = prepare_boolean(&session, session.revision(), actor(), &cut).unwrap();
    session.commit(prepared.prepared).unwrap();

    let model = native(&session);
    let bom = model
        .bom(session.document(), &BomOptions::default())
        .unwrap();
    let row = bom.rows.iter().find(|row| row.part == washer.body).unwrap();
    let hardware = row.hardware.as_ref().unwrap();
    assert_eq!(
        (hardware.feature, hardware.final_producer),
        (washer.feature, moved.feature)
    );
    close(hardware.final_bounds.min[2], 1.);
    assert!(hardware.occurrences[0].mount.is_none());
    assert!(
        bom.rows
            .iter()
            .find(|row| row.part == stock.body)
            .unwrap()
            .hardware
            .is_none()
    );

    // Retain the earlier native washer for the other Part's frozen Boolean tool
    // while making an independent, typed solid the washer Part's current head.
    let replacement = PrimitiveDefinition {
        output: BodyOutput { body: washer.body },
        parameters: PrimitiveParameters::Box {
            size: [3., 4., 5.].map(length),
            frame: Default::default(),
        },
    };
    let replacement = Entity::new(
        "Independent replacement",
        EntityData::Feature {
            definition: replacement.definition().unwrap(),
            inputs: replacement.inputs().unwrap(),
            suppressed: false,
        },
    );
    let replacement_id = replacement.id;
    let mut part = session.document().get(washer.body).unwrap().clone();
    let EntityData::Part { features, .. } = &mut part.data else {
        panic!()
    };
    features.push(replacement_id);
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Replace current solid".into(),
            edits: vec![Mutation::Insert(replacement), Mutation::Replace(part)],
        })
        .unwrap();

    let model = native(&session);
    let snapshot = model.snapshot();
    assert!(snapshot.shapes.contains_key(&washer.feature));
    assert!(snapshot.shapes.contains_key(&moved.feature));
    assert_eq!(snapshot.bodies[&washer.body], replacement_id);
    close(snapshot.shapes[&replacement_id].volume_mm3, 60.);
    let bom = model
        .bom(session.document(), &BomOptions::default())
        .unwrap();
    assert_eq!(bom.rows.len(), 2);
    assert!(bom.rows.iter().all(|row| row.hardware.is_none()));
}
