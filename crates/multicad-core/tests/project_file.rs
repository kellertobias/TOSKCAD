use multicad_core::{document::*, project_file::*};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
fn project() -> ProjectFile {
    let mut edits = Vec::new();
    let parameter = Entity::new(
        "width",
        EntityData::Parameter {
            expression: "600".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    edits.push(Mutation::Insert(parameter));
    let mut instance_ids = vec![];
    for (name, color, density) in [
        ("Birch", [190, 150, 105, 255], 700.),
        ("Steel", [150, 170, 190, 255], 7850.),
    ] {
        let material = Entity::new(
            name,
            EntityData::Material {
                density_kg_m3: Some(density),
                color_rgba: color,
                section_hatch: None,
            },
        );
        let feature = Entity::new(
            format!("{name} source"),
            EntityData::Feature {
                definition: Definition {
                    type_id: "unavailable.vendor.nurbs".into(),
                    schema_version: 17,
                    data: serde_json::json!({"vendor":{"opaque_knots":[0,0,1,1],"policy":"preserve"}}),
                },
                inputs: vec![],
                suppressed: false,
            },
        );
        let part = Entity::new(
            format!("{name} part"),
            EntityData::Part {
                features: vec![feature.id],
                material: Some(material.id),
                tags: BTreeSet::from(["maker-space".into()]),
            },
        );
        let instance = Entity::new(
            format!("{name} instance"),
            EntityData::Instance {
                component: part.id,
                placement: Placement {
                    translation_mm: [20., 10., 5.],
                    ..Default::default()
                },
            },
        );
        instance_ids.push(instance.id);
        edits.extend(
            [material, feature, part, instance]
                .into_iter()
                .map(Mutation::Insert),
        );
    }
    edits.push(Mutation::Insert(Entity::new(
        "Assembly",
        EntityData::Assembly {
            instances: instance_ids,
        },
    )));
    let mut project = ProjectFile::new(
        Document::new("Mixed workshop")
            .unwrap()
            .edited(&edits)
            .unwrap(),
    )
    .unwrap();
    let opaque = Payload::new(
        "vendor.future.geometry",
        "application/octet-stream",
        None,
        vec![0, 1, 255, 0, 7],
    );
    project
        .resources
        .payloads
        .insert(opaque.descriptor.id, opaque);
    let symbol = ProjectFile::new(Document::new("Door definition").unwrap()).unwrap();
    let pin = Payload::pinned_symbol(EntityId::new(), "revision-42", &symbol).unwrap();
    project.resources.payloads.insert(pin.descriptor.id, pin);
    project.resources.metadata.insert(
        "future_manifest_fields".into(),
        serde_json::json!({"must":"survive","number":13}),
    );
    let key = project
        .cache_key("display.mesh", "occt-7.9.3/render-1", 1, BTreeMap::new())
        .unwrap();
    let cache = DerivedCache::new(
        Payload::new(
            "display.mesh",
            "application/octet-stream",
            None,
            vec![9, 8, 7],
        ),
        key,
    );
    project.resources.caches.insert(cache.descriptor.id, cache);
    project
}
fn encode(project: &ProjectFile) -> Vec<u8> {
    let mut bytes = Vec::new();
    write(project, &mut bytes, FileLimits::default()).unwrap();
    bytes
}
fn decode(bytes: &[u8]) -> Result<OpenedProject, String> {
    read(
        &mut &*bytes,
        FileLimits::default(),
        &supported_capabilities(),
    )
}
#[test]
fn mixed_material_source_unknown_payloads_and_pinned_symbols_roundtrip() {
    let project = project();
    let bytes = encode(&project);
    let loaded = decode(&bytes).unwrap();
    assert_eq!(loaded.project.document, project.document);
    assert_eq!(
        loaded.project.resources.payloads,
        project.resources.payloads
    );
    assert_eq!(
        loaded.project.resources.metadata,
        project.resources.metadata
    );
    let pin = loaded
        .project
        .resources
        .payloads
        .values()
        .find(|p| p.descriptor.symbol_pin.is_some())
        .unwrap();
    assert_eq!(
        pin.embedded_symbol().unwrap().project.document.name(),
        "Door definition"
    );
    let key = loaded
        .project
        .cache_key("display.mesh", "occt-7.9.3/render-1", 1, BTreeMap::new())
        .unwrap();
    assert_eq!(
        loaded
            .project
            .resources
            .caches
            .values()
            .next()
            .unwrap()
            .validated(&key)
            .unwrap(),
        [9, 8, 7]
    );
    assert!(
        loaded
            .diagnostics
            .iter()
            .all(|d| matches!(d, ProjectDiagnostic::CacheNeedsValidation { .. }))
    );
    let summary = summary(
        &mut bytes.as_slice(),
        FileLimits::default(),
        &supported_capabilities(),
    )
    .unwrap();
    assert_eq!(summary.document, project.document.id());
    assert_eq!(summary.expected_file_bytes, bytes.len() as u64);
}
#[test]
fn caches_require_matching_source_producer_schema_and_external_payload_dependencies() {
    let project = project();
    let cache = project.resources.caches.values().next().unwrap();
    let key = project
        .cache_key("display.mesh", "occt-7.9.3/render-1", 1, BTreeMap::new())
        .unwrap();
    let mut changed = key.clone();
    changed.producer_version = "next".into();
    assert_eq!(
        cache.validated(&changed),
        Err(RebuildReason::ProducerChanged)
    );
    changed = key.clone();
    changed.schema_version = 2;
    assert_eq!(cache.validated(&changed), Err(RebuildReason::SchemaChanged));
    let mut edited = project.clone();
    edited.document.rename("Edited source").unwrap();
    assert_eq!(
        cache.validated(
            &edited
                .cache_key("display.mesh", "occt-7.9.3/render-1", 1, BTreeMap::new())
                .unwrap()
        ),
        Err(RebuildReason::SourceChanged)
    );
    edited = project.clone();
    let payload = edited
        .resources
        .payloads
        .values_mut()
        .find(|p| p.descriptor.symbol_pin.is_none())
        .unwrap();
    payload.bytes = Arc::from([10u8, 20]);
    assert_eq!(
        cache.validated(
            &edited
                .cache_key("display.mesh", "occt-7.9.3/render-1", 1, BTreeMap::new())
                .unwrap()
        ),
        Err(RebuildReason::DependenciesChanged)
    );
    changed = key;
    changed.dependencies.insert(
        "linked.external.step".into(),
        ContentHash::of(b"new-revision"),
    );
    assert_eq!(
        cache.validated(&changed),
        Err(RebuildReason::DependenciesChanged)
    );
}
#[test]
fn checksums_limits_future_versions_and_capabilities_fail_without_a_project() {
    let project = project();
    let bytes = encode(&project);
    for position in [0, 25, bytes.len() - 4] {
        let mut bad = bytes.clone();
        bad[position] ^= 1;
        assert!(
            decode(&bad).is_err(),
            "Critical corruption was accepted at {position}"
        );
    }
    let mut bad = bytes.clone();
    bad[8..12].copy_from_slice(&999u32.to_le_bytes());
    assert!(
        decode(&bad)
            .unwrap_err()
            .contains("Unsupported project format")
    );
    let mut bad = bytes.clone();
    bad[12..20].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(decode(&bad).is_err());
    assert!(decode(&bytes[..bytes.len() - 1]).is_err());
    let mut bad = bytes.clone();
    bad.push(0);
    assert!(decode(&bad).is_err());
    assert!(
        read(
            &mut bytes.as_slice(),
            FileLimits {
                maximum_file_bytes: 100,
                ..Default::default()
            },
            &supported_capabilities()
        )
        .is_err()
    );
    assert!(
        read(
            &mut bytes.as_slice(),
            FileLimits::default(),
            &BTreeSet::new()
        )
        .is_err()
    );
    assert!(decode(b"{\"schema_version\":1}").is_err());
}
#[test]
fn corrupt_optional_cache_is_discarded_with_an_explicit_rebuild_diagnostic() {
    let project = project();
    let mut bytes = encode(&project);
    *bytes.last_mut().unwrap() ^= 1;
    let loaded = decode(&bytes).unwrap();
    assert_eq!(loaded.project.document, project.document);
    assert!(loaded.project.resources.caches.is_empty());
    assert!(matches!(
        loaded.diagnostics.as_slice(),
        [ProjectDiagnostic::CacheCorrupt { .. }]
    ));
}
#[test]
fn native_format_one_migrates_without_regenerating_identities_or_opaque_payloads() {
    let mut project = project();
    project.resources.caches.clear();
    let records:Vec<_>=project.resources.payloads.values().map(|p|serde_json::json!({"descriptor":p.descriptor,"chunk":{"length":p.bytes.len(),"sha256":ContentHash::of(&p.bytes)}})).collect();
    let manifest=serde_json::to_vec(&serde_json::json!({"document":project.document,"required_capabilities":project.capabilities(),"payloads":records,"caches":[],"recovery":null,"future_manifest_fields":{"must":"survive","number":13}})).unwrap();
    let mut bytes = b"MCADPRJ\0".to_vec();
    bytes.extend(1u32.to_le_bytes());
    bytes.extend((manifest.len() as u64).to_le_bytes());
    bytes.extend(ContentHash::of(&manifest).0);
    bytes.extend(manifest);
    for payload in project.resources.payloads.values() {
        bytes.extend(payload.bytes.iter());
    }
    let opened = decode(&bytes).unwrap();
    assert_eq!(opened.original_format, 1);
    assert_eq!(opened.project.document, project.document);
    assert_eq!(
        opened.project.resources.payloads,
        project.resources.payloads
    );
    assert!(matches!(
        opened.diagnostics.as_slice(),
        [ProjectDiagnostic::Migrated { from: 1, to: 2 }]
    ));
    let current = decode(&encode(&opened.project)).unwrap();
    assert_eq!(current.original_format, 2);
    assert_eq!(current.project.document, project.document);
    assert_eq!(
        current.project.resources.payloads,
        project.resources.payloads
    );
}
#[test]
fn offset_plane_semantics_are_mandatory_and_old_cache_keys_cannot_match() {
    let base = Entity::new(
        "Base plane",
        EntityData::Plane {
            origin_mm: [0.; 3],
            normal: [0., 0., 1.],
            anchor: None,
        },
    );
    let offset = Entity::new(
        "Offset plane",
        EntityData::Plane {
            origin_mm: [0., 0., 20.],
            normal: [0., 0., 1.],
            anchor: Some(Link::new(base.id, "offset_plane")),
        },
    );
    let project = ProjectFile::new(
        Document::new("Offset source")
            .unwrap()
            .edited(&[Mutation::Insert(base), Mutation::Insert(offset)])
            .unwrap(),
    )
    .unwrap();
    assert!(project.capabilities().contains("core.plane.offset.1"));
    let bytes = encode(&project);
    assert!(
        read(
            &mut bytes.as_slice(),
            FileLimits::default(),
            &BTreeSet::from(["core.document.1".into()])
        )
        .unwrap_err()
        .contains("core.plane.offset.1")
    );
    let expected = project
        .cache_key("geometry", "1", 1, BTreeMap::new())
        .unwrap();
    let mut old = expected.clone();
    old.dependencies.remove("multicad.core.semantics");
    let cache = DerivedCache::new(
        Payload::new(
            "geometry.display",
            "application/octet-stream",
            None,
            vec![1, 2],
        ),
        old,
    );
    assert_eq!(
        cache.validated(&expected),
        Err(RebuildReason::DependenciesChanged)
    );
}
