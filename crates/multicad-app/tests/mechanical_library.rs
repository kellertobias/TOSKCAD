#[allow(dead_code)]
#[path = "../examples/mechanical_library.rs"]
mod generator;
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityData, Placement},
    model::{self, MECHANICAL_PART_TYPE},
    project_file::{self, ProjectFile},
    symbols::{self, SymbolPlaceRequest, SymbolValue},
};
use multicad_kernel::{GeometryPolicy, Solid};
use multicad_model::Compiler;
use std::{collections::BTreeMap, f64::consts::PI, path::Path};
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn bytes(project: &ProjectFile) -> Vec<u8> {
    let mut bytes = Vec::new();
    project_file::write(project, &mut bytes, Default::default()).unwrap();
    bytes
}
fn open(bytes: &[u8]) -> ProjectFile {
    let mut reader = bytes;
    project_file::read(
        &mut reader,
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
}
#[test]
fn shipped_bearing_defaults_reproduce_exactly_and_match_independent_native_dimensions() {
    let curation = generator::curated().unwrap();
    // Independently checked manufacturer d/D/B triples, not dimensions read
    // back from the generator's own recipe or from a rendered picture.
    let expected = [
        ("608", 8., 22., 7.),
        ("6000", 10., 26., 8.),
        ("6200", 10., 30., 9.),
        ("6300", 10., 35., 11.),
    ];
    let dir = tempfile::tempdir().unwrap();
    for (index, (designation, bore, outside, width)) in expected.into_iter().enumerate() {
        assert_eq!(curation.entries[index].designation, designation);
        let generated = generator::build(&curation, index).unwrap();
        let shipped = std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../library")
                .join(&curation.entries[index].file),
        )
        .unwrap();
        assert_eq!(
            bytes(&generated),
            shipped,
            "regeneration must preserve identities and the pin"
        );
        assert_eq!(bytes(&generator::build(&curation, index).unwrap()), shipped);
        let project = open(&shipped);
        let info = project_file::info::read(&project.document).unwrap();
        assert!(info.preview.is_some());
        assert!(info.description.contains("fillets are omitted"));
        let definitions = symbols::definitions(&project.document).unwrap();
        assert_eq!(definitions.len(), 1);
        assert_eq!(definitions[0].1.exposed.len(), 3);
        let card = model::mechanical_part_cards(&project.document);
        assert_eq!(card.len(), 1);
        assert!(card[0].matches_reference_dimensions);
        let built = Compiler::default().compile(&project.document, 0).unwrap();
        let solid = built.solid(card[0].feature).unwrap();
        assert_eq!(solid.solid_count().unwrap(), 1);
        let expected_volume = PI * (outside * outside - bore * bore) * width / 4.;
        assert!((solid.volume_mm3().unwrap() - expected_volume).abs() < 1e-6);
        let bounds = solid.bounds().unwrap();
        assert!((bounds.min[0] + outside / 2.).abs() < 1e-6);
        assert!((bounds.max[1] - outside / 2.).abs() < 1e-6);
        assert!((bounds.max[2] - width).abs() < 1e-6);
        let path = dir.path().join(format!("{designation}.step"));
        solid.write_step(&path).unwrap();
        let reopened = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
        assert_eq!(reopened.solid_count().unwrap(), 1);
        assert!((reopened.volume_mm3().unwrap() - expected_volume).abs() < 1e-6);
    }
}
#[test]
fn configured_bearing_places_one_physical_leaf_and_retains_its_pin_offline() {
    let curation = generator::curated().unwrap();
    let library = generator::build(&curation, 0).unwrap();
    let (_, definition) = symbols::definitions(&library.document)
        .unwrap()
        .pop()
        .unwrap();
    let pin = definition.pin().unwrap();
    let width = definition
        .template
        .entities()
        .find(|e| e.name == "width")
        .unwrap()
        .id;
    let outside = definition
        .template
        .entities()
        .find(|e| e.name == "outside")
        .unwrap()
        .id;
    let feature = definition.template.entities().find(|e| matches!(&e.data,EntityData::Feature{definition,..} if definition.type_id==MECHANICAL_PART_TYPE)).unwrap().id;
    let mut session = Session::new(Document::new("Bearing host").unwrap()).unwrap();
    let imported = symbols::prepare_import(&session, 0, actor(), &definition).unwrap();
    let embedded = imported.definition;
    session.commit(imported.prepared).unwrap();
    let before = session.document().clone();
    let invalid = SymbolPlaceRequest::new(
        session.document(),
        embedded,
        "Invalid bearing",
        BTreeMap::from([(
            outside,
            SymbolValue {
                value: 7.,
                unit: "mm".into(),
            },
        )]),
        Default::default(),
    )
    .unwrap();
    if let Ok(candidate) = symbols::prepare_place(&session, session.revision(), actor(), &invalid) {
        assert!(
            Compiler::default()
                .compile(candidate.prepared.document(), session.revision())
                .is_err()
        );
    }
    assert_eq!(session.document(), &before);
    let request = SymbolPlaceRequest::new(
        session.document(),
        embedded,
        "Wider bearing",
        BTreeMap::from([(
            width,
            SymbolValue {
                value: 14.,
                unit: "mm".into(),
            },
        )]),
        Placement {
            translation_mm: [12., 18., 24.],
            ..Default::default()
        },
    )
    .unwrap();
    let prepared = symbols::prepare_place(&session, session.revision(), actor(), &request).unwrap();
    Compiler::default()
        .compile(prepared.prepared.document(), session.revision())
        .unwrap();
    session.commit(prepared.prepared).unwrap();
    let record = symbols::read_instance(session.document(), request.instance).unwrap();
    assert_eq!(record.pin, pin);
    let source = model::read_mechanical_part(session.document(), record.ids[&feature]).unwrap();
    assert!(
        !source
            .matches_reference_dimensions(session.parameters())
            .unwrap()
    );
    let reopened = open(&bytes(
        &ProjectFile::new(session.document().clone()).unwrap(),
    ));
    let built = Compiler::default().compile(&reopened.document, 0).unwrap();
    assert_eq!(
        built.snapshot().leaf_occurrences.len(),
        1,
        "no extra physical definition Part"
    );
    let leaf = built.snapshot().leaf_occurrences.values().next().unwrap();
    let solid = built.leaf_solid(&leaf.path).unwrap();
    assert!((solid.volume_mm3().unwrap() - PI * (11f64.powi(2) - 4f64.powi(2)) * 14.).abs() < 1e-6);
    assert!((leaf.bounds.min[0] - 1.).abs() < 1e-6);
    assert!((leaf.bounds.max[2] - 38.).abs() < 1e-6);
    assert_eq!(
        symbols::read_instance(&reopened.document, request.instance)
            .unwrap()
            .pin,
        pin
    );
    assert_eq!(definition.pin().unwrap(), pin);
}
