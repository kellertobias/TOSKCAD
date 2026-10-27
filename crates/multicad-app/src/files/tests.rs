use super::*;
use multicad_core::{document::*, project_file::*};
use multicad_kernel::Solid;
use std::{collections::BTreeSet, sync::Arc};
fn project(directory: &Path) -> ProjectFile {
    let material = Entity::new(
        "Steel",
        EntityData::Material {
            density_kg_m3: Some(7850.),
            color_rgba: [130, 150, 175, 255],
            section_hatch: None,
        },
    );
    let feature = Entity::new(
        "Imported exact solid",
        EntityData::Feature {
            definition: Definition {
                type_id: "future.import.step".into(),
                schema_version: 1,
                data: serde_json::json!({"preserve":true}),
            },
            inputs: vec![],
            suppressed: false,
        },
    );
    let part = Entity::new(
        "Panel",
        EntityData::Part {
            features: vec![feature.id],
            material: Some(material.id),
            tags: BTreeSet::from(["fabricate".into()]),
        },
    );
    let doc = Document::new("Native file test")
        .unwrap()
        .edited(&[
            Mutation::Insert(material),
            Mutation::Insert(feature.clone()),
            Mutation::Insert(part),
        ])
        .unwrap();
    let mut project = ProjectFile::new(doc).unwrap();
    let step = directory.join("real-solid.step");
    Solid::box_named([20., 30., 8.], feature.id, Default::default())
        .unwrap()
        .write_step(&step)
        .unwrap();
    let payload = Payload::new(
        "kernel.source.step",
        "model/step",
        Some(feature.id),
        std::fs::read(step).unwrap(),
    );
    project
        .resources
        .payloads
        .insert(payload.descriptor.id, payload);
    let pin = Payload::pinned_symbol(
        EntityId::new(),
        "door-v1",
        &ProjectFile::new(Document::new("Door pin").unwrap()).unwrap(),
    )
    .unwrap();
    project.resources.payloads.insert(pin.descriptor.id, pin);
    project
}
#[test]
fn native_roundtrip_preserves_actual_step_and_pinned_symbol_bytes_across_source_edits() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = ProjectStore::new(dir.path().join("recovery"));
    let mut project = project(dir.path());
    let path = dir.path().join("native.multicad");
    let saved = store
        .save_project(&path, &project, SaveMode::Create)
        .unwrap();
    let loaded = store.open_project(&path).unwrap();
    assert_eq!(loaded.file.document, project.document);
    assert_eq!(loaded.file.resources.payloads, project.resources.payloads);
    assert_eq!(loaded.stamp, Some(saved.stamp));
    let resources = loaded.file.resources.clone();
    project.document.rename("Edited project").unwrap();
    let edited = ProjectFile::with_resources(project.document.clone(), resources).unwrap();
    store
        .save_project(&path, &edited, SaveMode::Replace(saved.stamp))
        .unwrap();
    let reopened = store.open_project(&path).unwrap();
    assert_eq!(reopened.file.resources.payloads, project.resources.payloads);
    let raw = reopened
        .file
        .resources
        .payloads
        .values()
        .find(|p| p.descriptor.kind == "kernel.source.step")
        .unwrap();
    let exact = dir.path().join("reopened.step");
    std::fs::write(&exact, &raw.bytes).unwrap();
    assert!(
        (Solid::read_step(&exact, Default::default())
            .unwrap()
            .volume_mm3()
            .unwrap()
            - 4800.)
            .abs()
            < 1e-7
    );
    assert_eq!(
        reopened
            .file
            .resources
            .payloads
            .values()
            .find(|p| p.descriptor.symbol_pin.is_some())
            .unwrap()
            .embedded_symbol()
            .unwrap()
            .project
            .document
            .name(),
        "Door pin"
    );
}
#[test]
fn failed_and_conflicting_saves_preserve_the_last_valid_project() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("native.multicad");
    let mut store = ProjectStore::new(dir.path().join("recovery"));
    let mut project = project(dir.path());
    let original = store
        .save_project(&path, &project, SaveMode::Create)
        .unwrap();
    let bytes = std::fs::read(&path).unwrap();
    project.document.rename("Changed source").unwrap();
    assert!(
        atomic_save_with(&path, &project, SaveMode::Replace(original.stamp), || Err(
            StorageError::Message("Injected write interruption".into())
        ))
        .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert!(matches!(
        store.save_project(&path, &project, SaveMode::Create),
        Err(StorageError::AlreadyExists(_))
    ));
    let latest = store
        .save_project(&path, &project, SaveMode::Replace(original.stamp))
        .unwrap();
    let saved = std::fs::read(&path).unwrap();
    project
        .document
        .rename("Other window stale source")
        .unwrap();
    assert!(matches!(
        store.save_project(&path, &project, SaveMode::Replace(original.stamp)),
        Err(StorageError::Changed)
    ));
    assert_eq!(std::fs::read(&path).unwrap(), saved);
    assert!(
        store
            .save_project(
                &dir.path().join("missing/file.multicad"),
                &project,
                SaveMode::Create
            )
            .is_err()
    );
    let payload = project.resources.payloads.values_mut().next().unwrap();
    payload.descriptor.schema_version = 0;
    assert!(
        store
            .save_project(&path, &project, SaveMode::Replace(latest.stamp))
            .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), saved);
}
#[test]
fn recovery_is_discoverable_and_restoration_cannot_replace_the_original_silently() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("native.multicad");
    let mut store = ProjectStore::new(dir.path().join("recovery"));
    let mut project = project(dir.path());
    store
        .save_project(&path, &project, SaveMode::Create)
        .unwrap();
    let original = std::fs::read(&path).unwrap();
    project.document.rename("Unsaved recovery edit").unwrap();
    let recovery = store.write_recovery(&project, 7, Some(&path)).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), original);
    let listed = store.recoveries().unwrap();
    assert_eq!(listed.entries.len(), 1);
    assert_eq!(listed.entries[0].info.id, recovery.info.id);
    assert_eq!(listed.entries[0].info.source_revision, 7);
    assert!(
        store
            .open_project(&store.recovery_path(recovery.info.id))
            .is_err()
    );
    let restored = store.restore_recovery(recovery.info.id).unwrap();
    assert!(restored.stamp.is_none());
    assert_eq!(restored.file.document, project.document);
    assert_eq!(restored.file.resources.payloads, project.resources.payloads);
    assert_eq!(std::fs::read(&path).unwrap(), original);
    let restored_path = dir.path().join("recovered-copy.multicad");
    store
        .save_project(&restored_path, &restored.file, SaveMode::Create)
        .unwrap();
    assert!(
        store
            .open_project(&restored_path)
            .unwrap()
            .file
            .recovery
            .is_none()
    );
    assert_eq!(std::fs::read(&path).unwrap(), original);
}
#[test]
fn native_open_and_explicit_source_import_are_distinct() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = ProjectStore::new(dir.path().join("recovery"));
    let project = project(dir.path());
    let json = dir.path().join("legacy.multicad.json");
    std::fs::write(&json, serde_json::to_vec(&project.document).unwrap()).unwrap();
    assert!(
        store
            .open_project(&json)
            .unwrap_err()
            .to_string()
            .contains("Import source snapshot")
    );
    let imported = store.import_source(&json).unwrap();
    assert_eq!(imported.file.document, project.document);
    assert!(imported.stamp.is_none());
    let native = dir.path().join("native.multicad");
    store
        .save_project(&native, &project, SaveMode::Create)
        .unwrap();
    assert!(
        store
            .import_source(&native)
            .unwrap_err()
            .to_string()
            .contains("Open project")
    );
    assert!(
        store
            .save_project(&json, &project, SaveMode::Create)
            .is_err()
    );
}
#[test]
fn cooperating_writers_lock_and_interrupted_process_do_not_destroy_existing_source() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("native.multicad");
    let mut store = ProjectStore::new(dir.path().join("recovery"));
    let project = project(dir.path());
    let saved = store
        .save_project(&path, &project, SaveMode::Create)
        .unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(dir.path().join(".native.multicad.multicad-save.lock"))
        .unwrap();
    lock.lock().unwrap();
    assert!(
        store
            .save_project(&path, &project, SaveMode::Replace(saved.stamp))
            .is_err()
    );
    drop(lock);
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "files::native::tests::interrupted_writer_child",
            "--ignored",
            "--nocapture",
        ])
        .env("MULTICAD_INTERRUPTED_SAVE_TEST", &path)
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(73));
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    store
        .save_project(&path, &project, SaveMode::Replace(saved.stamp))
        .unwrap();
}
#[test]
#[ignore = "Subprocess helper: the parent test supplies the fixture and checks the exit interruption"]
fn interrupted_writer_child() {
    let path = PathBuf::from(
        std::env::var_os("MULTICAD_INTERRUPTED_SAVE_TEST")
            .expect("Parent must provide the fixture path"),
    );
    let store = ProjectStore::new(path.parent().unwrap().join("recovery"));
    let mut opened = store.open(&path, false).unwrap();
    opened.file.document.rename("Interrupted revision").unwrap();
    let _ = atomic_save_with(
        &path,
        &opened.file,
        SaveMode::Replace(opened.stamp.unwrap()),
        || std::process::exit(73),
    );
    panic!("The writer should have exited before publication");
}
#[test]
fn shared_payload_ownership_does_not_copy_bytes_for_a_save_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let snapshot = project.clone();
    for (id, payload) in project.resources.payloads {
        assert!(Arc::ptr_eq(
            &payload.bytes,
            &snapshot.resources.payloads[&id].bytes
        ));
    }
}
