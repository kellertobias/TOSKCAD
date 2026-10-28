use multicad_core::document::*;
use serde_json::json;
use std::collections::BTreeSet;

fn definition(kind: &str, data: serde_json::Value) -> Definition {
    Definition {
        type_id: kind.into(),
        schema_version: 1,
        data,
    }
}
fn parameter(name: &str) -> Entity {
    Entity::new(
        name,
        EntityData::Parameter {
            expression: "120".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    )
}
fn plane() -> Entity {
    Entity::new(
        "XY",
        EntityData::Plane {
            origin_mm: [0.0; 3],
            normal: [0.0, 0.0, 1.0],
            anchor: None,
        },
    )
}
fn feature(name: &str, sketch: EntityId, height: f64) -> Entity {
    Entity::new(
        name,
        EntityData::Feature {
            definition: definition("solid.extrude", json!({"height_mm":height})),
            inputs: vec![Link::new(sketch, "profile")],
            suppressed: false,
        },
    )
}
fn material(name: &str, density: f64) -> Entity {
    Entity::new(
        name,
        EntityData::Material {
            density_kg_m3: Some(density),
            color_rgba: [100, 120, 150, 255],
            section_hatch: Some(definition(
                "drawing.hatch",
                json!({"pattern":"diagonal","angle_deg":45.0}),
            )),
        },
    )
}
fn part(name: &str, features: Vec<EntityId>, material: EntityId) -> Entity {
    Entity::new(
        name,
        EntityData::Part {
            features,
            material: Some(material),
            tags: BTreeSet::from(["fabricate".into()]),
        },
    )
}

#[test]
fn mixed_parts_shared_sketch_and_instances_preserve_ids_across_serialization_and_edits() {
    let mut document = Document::new("Mixed table").unwrap();
    let width = parameter("width");
    let xy = plane();
    let sketch = Entity::new(
        "Shared profile",
        EntityData::Sketch {
            plane: xy.id,
            definition: definition(
                "sketch.rectangle",
                json!({"width_binding":"width","depth_mm":80.0}),
            ),
            inputs: vec![Link::new(width.id, "width")],
        },
    );
    let wood = material("Plywood", 700.0);
    let steel = material("Steel", 7850.0);
    let first = feature("Panel extrusion", sketch.id, 18.0);
    let second = feature("Metal extrusion", sketch.id, 3.0);
    let panel = part("Panel", vec![first.id], wood.id);
    let metal = part("Bracket", vec![second.id], steel.id);
    let panel_instance = Entity::new(
        "Panel placement",
        EntityData::Instance {
            component: panel.id,
            placement: Placement::default(),
            placement_source: None,
        },
    );
    let metal_instance = Entity::new(
        "Bracket placement",
        EntityData::Instance {
            component: metal.id,
            placement: Placement {
                translation_mm: [600.0, 0.0, 0.0],
                ..Default::default()
            },
            placement_source: None,
        },
    );
    let assembly = Entity::new(
        "Table",
        EntityData::Assembly {
            instances: vec![panel_instance.id, metal_instance.id],
        },
    );
    let drawing = Entity::new(
        "Table drawing",
        EntityData::Drawing {
            definition: definition(
                "drawing.document",
                json!({"sheets":[{"size":"A3","author":"Test"}]}),
            ),
            inputs: vec![Link::new(assembly.id, "model")],
        },
    );
    let export = Entity::new(
        "Machining set",
        EntityData::ExportSet {
            definition: definition(
                "export.tag_query",
                json!({"all_tags":["fabricate"],"format":"step"}),
            ),
            inputs: vec![Link::new(assembly.id, "scope")],
        },
    );
    // Reference order deliberately differs from dependency order; IDs never
    // depend on array positions, display names or workspace ownership.
    for entity in [
        &drawing,
        &export,
        &assembly,
        &panel_instance,
        &metal_instance,
        &panel,
        &metal,
        &first,
        &second,
        &sketch,
        &wood,
        &steel,
        &xy,
        &width,
    ] {
        document.insert(entity.clone()).unwrap();
    }
    assert!(document.reference_diagnostics().is_empty());
    let order = document.topological_order().unwrap();
    let index = |id| order.iter().position(|i| *i == id).unwrap();
    assert!(index(width.id) < index(sketch.id));
    assert!(index(sketch.id) < index(first.id));
    assert!(index(first.id) < index(panel.id));
    assert!(index(panel.id) < index(assembly.id));
    assert!(index(assembly.id) < index(drawing.id));
    let ids: Vec<_> = document.entities().map(|e| e.id).collect();
    let serialized = serde_json::to_vec(&document).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("document-data.json");
    std::fs::write(&path, serialized).unwrap();
    let mut reopened: Document = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(reopened, document);
    let mut renamed = width.clone();
    renamed.name = "panel_width".into();
    reopened.replace(renamed).unwrap();
    assert_eq!(reopened.entities().map(|e| e.id).collect::<Vec<_>>(), ids);
    let affected = reopened.affected([width.id]).unwrap();
    assert!(
        affected.contains(&panel.id)
            && affected.contains(&metal.id)
            && affected.contains(&drawing.id)
    );
    assert!(!affected.contains(&wood.id));
    assert!(
        reopened
            .get(sketch.id)
            .unwrap()
            .references()
            .iter()
            .any(|link| link.target == width.id && link.role == "width")
    );
}

#[test]
fn feature_reordering_does_not_recreate_entities_or_depend_on_insertion_order() {
    let mut document = Document::new("History").unwrap();
    let xy = plane();
    let sketch = Entity::new(
        "Sketch",
        EntityData::Sketch {
            plane: xy.id,
            definition: definition("sketch.empty", json!({})),
            inputs: vec![],
        },
    );
    let a = feature("Feature", sketch.id, 10.0);
    let b = feature("Feature", sketch.id, 20.0);
    let stock = material("Stock", 600.0);
    let body = part("Body", vec![a.id, b.id], stock.id);
    let values = [xy, sketch, a, b, stock, body.clone()];
    for entity in &values {
        document.insert(entity.clone()).unwrap();
    }
    let mut reordered = body.clone();
    if let EntityData::Part { features, .. } = &mut reordered.data {
        features.reverse();
    }
    document.replace(reordered).unwrap();
    let before = document.topological_order().unwrap();
    let mut other = Document::new("Other insertion order").unwrap();
    for entity in values.iter().rev() {
        other
            .insert(document.get(entity.id).unwrap().clone())
            .unwrap();
    }
    assert_eq!(other.topological_order().unwrap(), before);
    assert_eq!(document.get(body.id).unwrap().id, body.id);
    assert!(document.insert(values[0].clone()).is_err());
}

#[test]
fn cyclic_edit_rolls_back_and_reports_an_actual_cycle() {
    let mut document = Document::new("Cycles").unwrap();
    let a = parameter("A");
    let mut b = parameter("B");
    if let EntityData::Parameter { inputs, .. } = &mut b.data {
        inputs.push(Link::new(a.id, "a"));
    }
    document.insert(a.clone()).unwrap();
    document.insert(b.clone()).unwrap();
    let before = document.clone();
    let mut cycle = a.clone();
    if let EntityData::Parameter { inputs, .. } = &mut cycle.data {
        inputs.push(Link::new(b.id, "b"));
    }
    let error = document.replace(cycle).unwrap_err();
    match error {
        DocumentError::Cycle(path) => {
            assert_eq!(path.first(), path.last());
            assert_eq!(path.len(), 3);
            assert!(path.contains(&a.id) && path.contains(&b.id));
        }
        _ => panic!("Expected cycle"),
    }
    assert_eq!(document, before);
}

#[test]
fn deleted_sources_remain_diagnosable_and_repair_restores_graph() {
    let mut document = Document::new("Repair").unwrap();
    let source = parameter("Width");
    let mut dependent = parameter("Double width");
    if let EntityData::Parameter { inputs, .. } = &mut dependent.data {
        inputs.push(Link::new(source.id, "width"));
    }
    document.insert(source.clone()).unwrap();
    document.insert(dependent.clone()).unwrap();
    document.remove(source.id).unwrap();
    let diagnostics = document.reference_diagnostics();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].owner, dependent.id);
    assert_eq!(diagnostics[0].target, source.id);
    assert_eq!(diagnostics[0].role, "width");
    assert_eq!(diagnostics[0].problem, ReferenceProblem::MissingEntity);
    assert_eq!(document.affected([source.id]).unwrap(), vec![dependent.id]);
    let roundtrip: Document =
        serde_json::from_slice(&serde_json::to_vec(&document).unwrap()).unwrap();
    assert_eq!(roundtrip.reference_diagnostics(), diagnostics);
    document.insert(source).unwrap();
    assert!(document.reference_diagnostics().is_empty());
}

#[test]
fn unavailable_module_payloads_are_preserved_and_bad_reference_kinds_explained() {
    let mut document = Document::new("Plugin").unwrap();
    let width = parameter("Width");
    let custom = Entity::new(
        "Custom feature",
        EntityData::Feature {
            definition: definition(
                "vendor.finger_joint",
                json!({"finger_count":13,"options":{"offset":1.25},"extension":["future",true]}),
            ),
            inputs: vec![Link::new(width.id, "width")],
            suppressed: false,
        },
    );
    let invalid_part = part("Bad material binding", vec![custom.id], width.id);
    for e in [&width, &custom, &invalid_part] {
        document.insert(e.clone()).unwrap();
    }
    let restored: Document =
        serde_json::from_slice(&serde_json::to_vec(&document).unwrap()).unwrap();
    assert_eq!(restored.get(custom.id), Some(&custom));
    assert!(matches!(
        &restored.reference_diagnostics()[0].problem,
        ReferenceProblem::WrongKind {
            actual: EntityKind::Parameter,
            ..
        }
    ));
}

#[test]
fn corrupt_identity_version_cycles_and_duplicate_json_keys_are_rejected() {
    let mut document = Document::new("Validation").unwrap();
    let value = parameter("Value");
    document.insert(value.clone()).unwrap();
    let original = serde_json::to_value(&document).unwrap();
    let mut bad = original.clone();
    bad["schema_version"] = json!(999);
    assert!(serde_json::from_value::<Document>(bad).is_err());
    let mut bad = original.clone();
    bad["entities"][value.id.to_string()]["id"] = json!(EntityId::new());
    assert!(serde_json::from_value::<Document>(bad).is_err());
    let mut bad = original.clone();
    bad["id"] = json!("00000000-0000-0000-0000-000000000000");
    assert!(serde_json::from_value::<Document>(bad).is_err());
    let mut bad = original.clone();
    bad["entities"][value.id.to_string()]["data"]["inputs"] =
        json!([{"target":value.id,"role":"self"}]);
    assert!(serde_json::from_value::<Document>(bad).is_err());
    let record = serde_json::to_string(&value).unwrap();
    let repeated = format!(
        "{{\"schema_version\":1,\"id\":\"{}\",\"name\":\"Duplicate\",\"entities\":{{\"{}\":{},\"{}\":{}}}}}",
        document.id(),
        value.id,
        record,
        value.id,
        record
    );
    assert!(serde_json::from_str::<Document>(&repeated).is_err());
    let mut invalid = plane();
    if let EntityData::Plane { normal, .. } = &mut invalid.data {
        *normal = [1e308; 3];
    }
    assert!(document.insert(invalid).is_err());
    let mut invalid = value.clone();
    invalid.name = " ".into();
    assert!(document.replace(invalid).is_err());
    assert_eq!(serde_json::to_value(&document).unwrap(), original);
}

#[test]
fn finite_source_coordinates_keep_exact_bits_through_json_round_trips() {
    let mut document = Document::new("Coordinate persistence").unwrap();
    let source = plane();
    document.insert(source.clone()).unwrap();
    let mut bits = 0xcad2026_u64;
    for _ in 0..128 {
        bits = bits.wrapping_mul(6364136223846793005).wrapping_add(1);
        let number = f64::from_bits(bits);
        if !number.is_finite() {
            continue;
        }
        let coordinates = [number, -0.0, number * 0.5];
        let mut next = source.clone();
        if let EntityData::Plane { origin_mm, .. } = &mut next.data {
            *origin_mm = coordinates;
        }
        document.replace(next).unwrap();
        let restored: Document =
            serde_json::from_slice(&serde_json::to_vec(&document).unwrap()).unwrap();
        let EntityData::Plane { origin_mm, .. } = &restored.get(source.id).unwrap().data else {
            panic!("expected plane")
        };
        assert_eq!(origin_mm.map(f64::to_bits), coordinates.map(f64::to_bits));
    }
}
