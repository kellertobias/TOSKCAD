//! Check the actual bundled profile pin through canonical placement and reopen.
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityData, Placement},
    materials::{ProfileShape, StockKind, part_properties},
    model,
    project_file::{self, ContentHash, ProjectFile},
    symbols::{self, SymbolEditRequest, SymbolKind, SymbolPlaceRequest, SymbolValue},
};
use multicad_model::Compiler;
use std::{collections::BTreeMap, fs, path::Path};

fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}

#[test]
fn bundled_line_8_member_retains_its_pin_and_edits_one_physical_length() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let name = "profile-item-line8-40x40.tcd";
    let bytes = fs::read(root.join(name)).unwrap();
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("library.json")).unwrap()).unwrap();
    let entry = manifest["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["file"] == name)
        .unwrap();
    assert_eq!(entry["sha256"], ContentHash::of(&bytes).to_string());
    let capabilities = project_file::supported_capabilities();
    let bundled = project_file::read(&mut bytes.as_slice(), Default::default(), &capabilities)
        .unwrap()
        .project;
    assert_eq!(entry["document"], bundled.document.id().to_string());
    let info = project_file::info::read(&bundled.document).unwrap();
    assert!(info.preview.is_some());
    assert!(info.description.contains("0.0.026.03"));
    let definitions = symbols::definitions(&bundled.document).unwrap();
    let [(definition_id, definition)] = definitions.as_slice() else {
        panic!("Expected one bundled profile definition");
    };
    assert_eq!(definition.kind, SymbolKind::Dynamic);
    let [length] = definition.exposed.as_slice() else {
        panic!("Expected exactly one Length input");
    };
    let parameter = definition.template.get(*length).unwrap();
    assert_eq!(parameter.name, "Length");
    assert!(
        matches!(&parameter.data, EntityData::Parameter { expression, unit, exposed: true, .. }
        if expression == "160" && unit == "mm")
    );
    let feature = model::member_feature(&definition.template, definition.root).unwrap();
    let source = model::read_member(&definition.template, feature).unwrap();
    let stock = model::read_profile_stock(&definition.template, source.stock).unwrap();
    assert!(matches!(&stock.specification, StockKind::Profile {
        shape: ProfileShape::ItemLine8_40x40, designation, width_mm: 40., height_mm: 40.,
        length_mm: 6000., wall_mm: None
    } if designation == "item 0.0.026.03"));
    let material = definition.template.get(stock.material).unwrap();
    assert_eq!(material.name, "Natural anodized aluminum");
    assert!(matches!(
        material.data,
        EntityData::Material {
            density_kg_m3: None,
            ..
        }
    ));

    let mut receiving = Session::new(Document::new("Receiving frame").unwrap()).unwrap();
    let empty = receiving.document().clone();
    let overrides = |value| {
        BTreeMap::from([(
            *length,
            SymbolValue {
                value,
                unit: "mm".into(),
            },
        )])
    };
    let request = SymbolPlaceRequest::from_definition(
        *definition_id,
        definition,
        "Installed Line 8 member",
        overrides(240.),
        Placement {
            translation_mm: [7., 11., 13.],
            ..Default::default()
        },
    )
    .unwrap();
    let draft =
        symbols::prepare_library_place(&receiving, 0, actor(), definition, &request, None).unwrap();
    assert_eq!(receiving.document(), &empty);
    receiving.commit(draft.prepared).unwrap();
    let compiler = Compiler {
        omit_display: true,
        ..Default::default()
    };
    let physical = |document: &Document, revision, span| {
        let model = compiler.compile(document, revision).unwrap();
        let mut leaves = model.snapshot().leaf_occurrences.iter();
        let (path, leaf) = leaves.next().unwrap();
        assert_eq!(path.root, request.instance);
        assert_eq!(leaf.component, request.ids[&definition.root]);
        assert_eq!(leaf.producer, request.ids[&feature]);
        assert!(leaves.next().is_none());
        assert_eq!(model.leaf_solid(path).unwrap().solid_count().unwrap(), 1);
        for (actual, expected) in leaf.bounds.min.into_iter().chain(leaf.bounds.max).zip([
            7.,
            -9.,
            -7.,
            7. + span,
            31.,
            33.,
        ]) {
            assert!((actual - expected).abs() < 1e-5, "{actual} != {expected}");
        }
        let (_, properties) = part_properties(document, leaf.component).unwrap().unwrap();
        assert_eq!(properties.stock.unwrap().id, request.ids[&source.stock]);
        assert_eq!(
            properties.material.unwrap().id,
            request.ids[&stock.material]
        );
    };
    physical(receiving.document(), 1, 240.);
    let placed = symbols::read_instance(receiving.document(), request.record).unwrap();
    assert_eq!(placed.pin, definition.pin().unwrap());

    let draft = symbols::prepare_edit(
        &receiving,
        1,
        actor(),
        &SymbolEditRequest {
            record: request.record,
            overrides: overrides(325.),
        },
    )
    .unwrap();
    receiving.commit(draft.prepared).unwrap();
    physical(receiving.document(), 2, 325.);
    let edited = symbols::read_instance(receiving.document(), request.record).unwrap();
    assert_eq!(edited.pin, placed.pin);
    assert_eq!(edited.ids, placed.ids);
    assert_eq!(edited.instance, placed.instance);
    assert_eq!(
        symbols::read_definition(receiving.document(), request.definition).unwrap(),
        *definition
    );

    let mut saved = Vec::new();
    let project = ProjectFile::new(receiving.document().clone()).unwrap();
    project_file::write(&project, &mut saved, Default::default()).unwrap();
    let reopened = project_file::read(&mut saved.as_slice(), Default::default(), &capabilities)
        .unwrap()
        .project;
    assert_eq!(reopened.document, *receiving.document());
    physical(&reopened.document, 0, 325.);
    assert_eq!(fs::read(root.join(name)).unwrap(), bytes);
}
