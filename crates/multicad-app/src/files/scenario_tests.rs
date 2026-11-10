//! Release regression coverage over the four acceptance scenario projects:
//! every project round-trips through the desktop's native store and its
//! recovery path with an identical document, re-saves byte-identically,
//! migrates from format 1, is rejected with a clear message by a reader that
//! lacks a capability it needs, and fails cleanly when corrupted or
//! truncated.
#[path = "../../examples/makerspace_review.rs"]
#[allow(dead_code)]
mod makerspace;

#[path = "../../examples/cabinetry_table_review.rs"]
#[allow(dead_code)]
mod cabinetry_table;

#[path = "../../examples/tool_case_review.rs"]
#[allow(dead_code)]
mod tool_case;

#[path = "../../examples/control_desk_review.rs"]
#[allow(dead_code)]
mod control_desk;

use super::*;
use multicad_core::document::Document;
use std::collections::BTreeSet;

fn encode(project: &ProjectFile) -> Vec<u8> {
    let mut bytes = Vec::new();
    project_file::write(project, &mut bytes, FileLimits::default()).unwrap();
    bytes
}
fn decode(bytes: &[u8], supported: &BTreeSet<String>) -> Result<ProjectFile, String> {
    project_file::read(&mut &bytes[..], FileLimits::default(), supported).map(|o| o.project)
}

/// Opening records the manifest's (derived) capability set as the required
/// set; everything else is the authored project unchanged.
fn same_resources(opened: &ProjectFile, saved: &ProjectFile) {
    assert_eq!(opened.capabilities(), saved.capabilities());
    assert_eq!(opened.resources.required_capabilities, saved.capabilities());
    assert_eq!(opened.resources.payloads, saved.resources.payloads);
    assert_eq!(opened.resources.caches, saved.resources.caches);
    assert_eq!(opened.resources.metadata, saved.resources.metadata);
}

/// Full release round trip for one scenario's canonical document.
fn check_scenario(document: &Document) {
    let dir = tempfile::tempdir().unwrap();
    let project = ProjectFile::new(document.clone()).unwrap();
    assert_eq!(
        &project.document, document,
        "saving must not rewrite source"
    );
    let bytes = encode(&project);

    // Desktop store: save, open, re-save byte-identically.
    let mut store = ProjectStore::new(dir.path().join("recovery"));
    let path = dir.path().join("scenario.multicad");
    store
        .save_project(&path, &project, SaveMode::Create)
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let opened = store.open_project(&path).unwrap();
    assert!(opened.diagnostics.is_empty(), "{:?}", opened.diagnostics);
    assert_eq!(&opened.file.document, document);
    same_resources(&opened.file, &project);
    let resaved = dir.path().join("resaved.multicad");
    store
        .save_project(&resaved, &opened.file, SaveMode::Create)
        .unwrap();
    assert_eq!(std::fs::read(&resaved).unwrap(), bytes);

    // Recovery: a snapshot is listed, refused by plain Open, restored as an
    // unsaved copy with the identical document, and leaves the original alone.
    let entry = store.write_recovery(&project, 42, Some(&path)).unwrap();
    let listed = store.recoveries().unwrap();
    assert_eq!(listed.entries.len(), 1);
    assert_eq!(listed.entries[0].info, entry.info);
    assert_eq!(listed.entries[0].document, document.id());
    assert!(
        store
            .open_project(&store.recovery_path(entry.info.id))
            .unwrap_err()
            .to_string()
            .contains("recovery snapshot")
    );
    let restored = store.restore_recovery(entry.info.id).unwrap();
    assert!(restored.stamp.is_none());
    assert_eq!(&restored.file.document, document);
    same_resources(&restored.file, &project);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    // The automatic session slot round-trips the same way.
    let slot = EntityId::new();
    store
        .write_recovery_slot(&project, 43, Some(&path), slot)
        .unwrap();
    assert_eq!(
        &store.restore_recovery(slot).unwrap().file.document,
        document
    );

    // Format 1 (source inside the manifest) migrates to the same document.
    let manifest = serde_json::to_vec(&serde_json::json!({
        "document": document,
        "required_capabilities": project.capabilities(),
        "recovery": null,
    }))
    .unwrap();
    let mut v1 = b"MCADPRJ\0".to_vec();
    v1.extend(1u32.to_le_bytes());
    v1.extend((manifest.len() as u64).to_le_bytes());
    v1.extend(ContentHash::of(&manifest).0);
    v1.extend(&manifest);
    let migrated = project_file::read(
        &mut v1.as_slice(),
        FileLimits::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(&migrated.project.document, document);
    assert_eq!(
        migrated.diagnostics,
        [project_file::ProjectDiagnostic::Migrated { from: 1, to: 2 }]
    );

    check_compatibility(&project, &bytes);
    check_corruption(&bytes);
}

/// An older reader rejects capabilities and formats it does not implement.
fn check_compatibility(project: &ProjectFile, bytes: &[u8]) {
    let supported = project_file::supported_capabilities();
    let required = project.capabilities();
    assert!(required.is_subset(&supported));
    for capability in &required {
        let mut older = supported.clone();
        older.remove(capability);
        let error = decode(bytes, &older).unwrap_err();
        assert!(
            error.contains("unsupported core capabilities") && error.contains(capability.as_str()),
            "{capability}: {error}"
        );
    }
    // A newer writer's capability is named, never ignored.
    let mut newer = project.clone();
    newer
        .resources
        .required_capabilities
        .insert("future.kernel.99".into());
    let error = decode(&encode(&newer), &supported).unwrap_err();
    assert_eq!(
        error,
        "Project requires unsupported core capabilities: future.kernel.99"
    );
    // A newer container format is rejected before its manifest is trusted.
    let mut future = bytes.to_vec();
    future[8..12].copy_from_slice(&3u32.to_le_bytes());
    assert_eq!(
        decode(&future, &supported).unwrap_err(),
        "Unsupported project format 3; this build supports 1 through 2"
    );
}

/// Truncated, bit-flipped and extended files fail with an error, not a panic
/// or a partially decoded document.
fn check_corruption(bytes: &[u8]) {
    let supported = project_file::supported_capabilities();
    let manifest_end = 52 + u64::from_le_bytes(bytes[12..20].try_into().unwrap()) as usize;
    let mut cuts: BTreeSet<usize> = (0..=52).collect();
    cuts.extend((0..64).map(|i| i * bytes.len() / 64));
    cuts.extend([
        manifest_end - 1,
        manifest_end,
        manifest_end + 1,
        bytes.len() - 1,
    ]);
    for cut in cuts {
        assert!(
            decode(&bytes[..cut], &supported).is_err(),
            "truncated at {cut} of {}",
            bytes.len()
        );
    }
    let flip = |offset: usize| {
        let mut copy = bytes.to_vec();
        copy[offset] ^= 0x20;
        decode(&copy, &supported)
    };
    assert!(
        flip(3)
            .unwrap_err()
            .contains("not a native ToskCAD project")
    );
    assert!(flip(manifest_end - 2).unwrap_err().contains("checksum"));
    // Source chunk: start, middle and end are hash-checked.
    for offset in [
        manifest_end,
        (manifest_end + bytes.len()) / 2,
        bytes.len() - 1,
    ] {
        assert!(flip(offset).is_err(), "flipped source byte {offset}");
    }
    let mut extended = bytes.to_vec();
    extended.push(0);
    assert_eq!(
        decode(&extended, &supported).unwrap_err(),
        "Project has unexpected trailing data"
    );
}

#[test]
fn makerspace_project_round_trips_recovers_and_rejects_damage() {
    check_scenario(makerspace::build().unwrap().session.document());
}
#[test]
fn cabinetry_table_project_round_trips_recovers_and_rejects_damage() {
    check_scenario(cabinetry_table::build().unwrap().session.document());
}
#[test]
fn tool_case_project_round_trips_recovers_and_rejects_damage() {
    check_scenario(tool_case::build().unwrap().session.document());
}
#[test]
fn tool_case_slide_pattern_reopens_and_rebuilds_exact_cutter_members() {
    use multicad_core::{
        commands::Session,
        document::{EntityData, Mutation},
        model,
        references::Origin,
    };
    use multicad_model::{Compiler, GeometryBackend};

    let parameters = tool_case::rail_parameters();
    let pitch = parameters[0].id;
    let document = Document::new("Slide pattern")
        .unwrap()
        .edited(&parameters.map(Mutation::Insert))
        .unwrap();
    let mut session = Session::new(document).unwrap();
    let (_, slide) =
        tool_case::slide_member(&mut session, "Tool case slide", tool_case::PLY).unwrap();
    check_scenario(session.document());

    let compiler = Compiler {
        omit_display: true,
        ..Default::default()
    };
    let edit_pitch = |value: &str| {
        let mut parameter = session.document().get(pitch).unwrap().clone();
        let EntityData::Parameter { expression, .. } = &mut parameter.data else {
            unreachable!("slide pitch is a project parameter");
        };
        *expression = value.into();
        session
            .document()
            .edited(&[Mutation::Replace(parameter)])
            .unwrap()
    };
    for (pitch, expected) in [("96", [29., 125., 221.]), ("80", [29., 109., 189.])] {
        let project = encode(&ProjectFile::new(edit_pitch(pitch)).unwrap());
        let document = decode(&project, &project_file::supported_capabilities())
            .unwrap()
            .document;
        let pattern = model::read_boolean(&document, slide).unwrap().tools[0].producer;
        let compiled = compiler
            .compile_with_backend(&document, session.revision(), GeometryBackend::Rust, None)
            .unwrap();
        let rail = compiled.solid(slide).unwrap();
        assert!(rail.uses_rust_geometry());
        assert_eq!(rail.solid_count().unwrap(), 1);
        assert_eq!(compiled.solid(pattern).unwrap().solid_count().unwrap(), 3);
        for index in 0..3 {
            assert!(
                compiled.snapshot().shapes[&pattern]
                    .topology
                    .elements
                    .iter()
                    .any(|element| {
                        element.origins.contains(&Origin {
                            source: pattern,
                            role: format!("member/{index}"),
                        })
                    })
            );
        }
        let [thickness, length, height] = tool_case::SLIDE;
        let radius = tool_case::RAIL_HOLE_R;
        let volume = thickness * (length * height - 3. * std::f64::consts::PI * radius * radius);
        assert!((rail.volume_mm3().unwrap() - volume).abs() < 1e-7);
        for x in [tool_case::PLY, tool_case::PLY + thickness] {
            assert_eq!(
                tool_case::hole_centers(&compiled, slide, x, radius),
                expected.map(|y| [y, tool_case::HOLE_Z]),
            );
        }
        #[cfg(feature = "occt")]
        {
            let native = compiler
                .compile_with_backend(
                    &document,
                    session.revision(),
                    GeometryBackend::OpenCascade,
                    None,
                )
                .unwrap();
            let native_rail = native.solid(slide).unwrap();
            assert!((native_rail.volume_mm3().unwrap() - volume).abs() < 1e-7);
            for x in [tool_case::PLY, tool_case::PLY + thickness] {
                assert_eq!(
                    tool_case::hole_centers(&native, slide, x, radius),
                    tool_case::hole_centers(&compiled, slide, x, radius),
                );
            }
        }
    }
    let overlapping = edit_pitch("2");
    let error = compiler
        .compile_with_backend(
            &overlapping,
            session.revision(),
            GeometryBackend::Rust,
            None,
        )
        .err()
        .expect("overlapping tools need general curved Boolean support");
    assert_eq!(error.entity, Some(slide));
    assert!(
        error
            .message
            .contains("touches stock boundaries or other cuts")
    );
}

#[test]
fn control_desk_project_round_trips_recovers_and_rejects_damage() {
    let sources = tempfile::tempdir().unwrap();
    check_scenario(
        control_desk::build(sources.path())
            .unwrap()
            .session
            .document(),
    );
}

#[test]
fn control_desk_rust_board_and_linked_step_reopen_without_the_source_files() {
    use multicad_core::{
        commands::{Actor, Channel, Session},
        model::imports::prepare_kicad_import,
    };
    use multicad_model::{
        Compiler, GeometryBackend,
        imports::{KicadOptions, read_kicad_file_with_backend},
    };

    let backend = GeometryBackend::Rust;
    let sources = tempfile::tempdir().unwrap();
    let board_path =
        control_desk::write_sources_for(sources.path(), control_desk::BOARD_A, backend).unwrap();
    let imported = read_kicad_file_with_backend(
        &board_path,
        &KicadOptions {
            components: Some(control_desk::COMPONENTS_A.map(String::from).to_vec()),
            ..Default::default()
        },
        backend,
    )
    .unwrap();
    let mut session = Session::new(Document::new("Control Desk board sources").unwrap()).unwrap();
    let prepared = prepare_kicad_import(&session, 0, Actor::local(Channel::Ui), &imported).unwrap();
    assert!(
        prepared
            .unsupported
            .iter()
            .any(|item| item.contains("2 vias"))
    );
    assert!(
        prepared
            .unsupported
            .iter()
            .any(|item| item.contains("oval drill"))
    );
    assert_eq!(prepared.components.len(), 5);
    assert_eq!(
        prepared
            .components
            .iter()
            .filter(|(_, _, model)| model.is_some())
            .count(),
        4
    );
    assert_eq!(prepared.missing_models.len(), 1);
    assert_eq!(prepared.missing_models[0].0, "ENC1");
    let parts = prepared
        .model
        .outputs
        .iter()
        .map(|part| part.feature)
        .collect::<Vec<_>>();
    session.commit(prepared.model.prepared).unwrap();
    let snapshot = |document, revision| {
        let model = Compiler {
            omit_display: true,
            ..Default::default()
        }
        .compile_with_backend(document, revision, backend, None)
        .unwrap();
        parts
            .iter()
            .map(|&id| {
                let solid = model.solid(id).unwrap();
                (
                    solid.solid_count().unwrap(),
                    solid.volume_mm3().unwrap(),
                    solid.bounds().unwrap(),
                )
            })
            .collect::<Vec<_>>()
    };
    let before = snapshot(session.document(), session.revision());
    assert_eq!(before.len(), 6);
    assert!(
        before
            .iter()
            .all(|(count, volume, _)| *count == 1 && *volume > 0.)
    );
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("control-desk-board.tcd");
    let mut store = ProjectStore::new(scratch.path().join("recovery"));
    let project = ProjectFile::new(session.document().clone()).unwrap();
    store
        .save_project(&path, &project, SaveMode::Create)
        .unwrap();
    sources.close().unwrap();
    let reopened = store.open_project(&path).unwrap();
    assert_eq!(reopened.file.document, project.document);
    assert_eq!(snapshot(&reopened.file.document, 0), before);
}
