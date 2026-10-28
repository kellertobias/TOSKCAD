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
        atomic_save_with(&path, &project, SaveMode::Replace(original.stamp), |_| Err(
            StorageError::Message("Injected write interruption".into())
        ))
        .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let conflict = store.save_project(&path, &project, SaveMode::Create);
    assert!(
        matches!(conflict, Err(StorageError::AlreadyExists(_))),
        "Unexpected create conflict result: {conflict:?}"
    );
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
        |_| std::process::exit(73),
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

#[test]
fn external_replacement_during_serialization_is_preserved_and_cannot_be_acknowledged_as_our_save() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("external-race.multicad");
    let mut store = ProjectStore::new(dir.path().join("recovery"));
    let mut ours = project(dir.path());
    let original = store.save_project(&path, &ours, SaveMode::Create).unwrap();
    ours.document.rename("Our pending edit").unwrap();
    let mut external = ours.clone();
    external.document.rename("External writer source").unwrap();
    let mut bytes = vec![];
    project_file::write(&external, &mut bytes, FileLimits::default()).unwrap();
    let result = atomic_save_with(&path, &ours, SaveMode::Replace(original.stamp), |_| {
        std::fs::write(&path, &bytes)?;
        Ok(())
    });
    assert!(matches!(result, Err(StorageError::Changed)));
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(
        store.open_project(&path).unwrap().file.document,
        external.document
    );
}

fn retained_lock_alias_release(inject_error: bool) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("retained-lock.multicad");
    let lock_path = dir
        .path()
        .join(".retained-lock.multicad.multicad-save.lock");
    let mut store = ProjectStore::new(dir.path().join("recovery"));
    let mut file = ProjectFile::new(Document::new("Original source").unwrap()).unwrap();
    let saved = store.save_project(&path, &file, SaveMode::Create).unwrap();
    let before = std::fs::read(&path).unwrap();
    file.document.rename("Updated source").unwrap();
    let mut retained = None;
    let result = atomic_save_with(&path, &file, SaveMode::Replace(saved.stamp), |lock| {
        retained = Some(lock.try_clone()?);
        let independent = OpenOptions::new().read(true).write(true).open(&lock_path)?;
        assert!(
            matches!(
                independent.try_lock(),
                Err(std::fs::TryLockError::WouldBlock)
            ),
            "The real save must retain exclusion through publication"
        );
        if inject_error {
            return Err(StorageError::Message(
                "Injected pre-publication failure".into(),
            ));
        }
        Ok(())
    });
    let retained = retained.expect("A duplicate of the real save lock stays open across return");
    let next = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock_path)
        .unwrap();
    next.try_lock().expect("The returned save must explicitly unlock even while an inherited or cloned descriptor remains open");
    next.unlock().unwrap();
    assert!(lock_path.is_file(), "The persistent lock inode must remain");
    if inject_error {
        assert!(
            matches!(result,Err(StorageError::Message(ref error)) if error=="Injected pre-publication failure")
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(
            store.open_project(&path).unwrap().file.document.name(),
            "Original source"
        );
    } else {
        let saved = result.unwrap();
        let opened = store.open_project(&path).unwrap();
        assert_eq!(opened.stamp, Some(saved.stamp));
        assert_eq!(opened.file.document, file.document);
        assert_ne!(std::fs::read(&path).unwrap(), before);
    }
    drop(retained);
}

#[test]
fn successful_save_releases_with_a_retained_lock_alias() {
    retained_lock_alias_release(false);
}
#[test]
fn failed_save_releases_with_a_retained_lock_alias() {
    retained_lock_alias_release(true);
}
#[test]
fn serialization_failure_preserves_source_and_releases_before_another_save() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("serialization.multicad");
    let mut store = ProjectStore::new(dir.path().join("recovery"));
    let file = ProjectFile::new(Document::new("Valid source").unwrap()).unwrap();
    let saved = store.save_project(&path, &file, SaveMode::Create).unwrap();
    let before = std::fs::read(&path).unwrap();
    let mut invalid = file.clone();
    invalid
        .resources
        .required_capabilities
        .insert(String::new());
    let error = store
        .save_project(&path, &invalid, SaveMode::Replace(saved.stamp))
        .unwrap_err();
    assert!(
        matches!(error,StorageError::Message(ref message) if message.contains("capability")),
        "Unexpected serialization failure: {error:?}"
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let next = OpenOptions::new()
        .read(true)
        .write(true)
        .open(
            dir.path()
                .join(".serialization.multicad.multicad-save.lock"),
        )
        .unwrap();
    next.try_lock()
        .expect("Serialization errors must release the acquired save lock");
    next.unlock().unwrap();
    store
        .save_project(&path, &file, SaveMode::Replace(saved.stamp))
        .unwrap();
    assert_eq!(
        store.open_project(&path).unwrap().file.document,
        file.document
    );
}

#[test]
fn automatic_recovery_reuses_only_its_session_tab_slot_after_more_than_512_updates() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = ProjectStore::new(dir.path().join("recovery"));
    let mut file = ProjectFile::new(Document::new("Automatic recovery").unwrap()).unwrap();
    let manual = store.write_recovery(&file, 0, None).unwrap();
    let manual_path = store.recovery_path(manual.info.id);
    let manual_bytes = std::fs::read(&manual_path).unwrap();
    let other_session = store
        .write_recovery_slot(&file, 1, None, EntityId::new())
        .unwrap();
    let other_path = store.recovery_path(other_session.info.id);
    let other_bytes = std::fs::read(&other_path).unwrap();
    let slot = EntityId::new();
    for revision in 0..=513 {
        file.document
            .rename(format!("Latest revision {revision}"))
            .unwrap();
        let entry = store
            .write_recovery_slot(&file, revision, None, slot)
            .unwrap();
        assert_eq!(entry.info.id, slot);
        assert_eq!(entry.info.slot, Some(slot));
    }
    let entries = store.recoveries().unwrap().entries;
    assert_eq!(entries.len(), 3);
    let latest = entries.iter().find(|e| e.info.id == slot).unwrap();
    assert_eq!(latest.info.source_revision, 513);
    assert_eq!(
        store.restore_recovery(slot).unwrap().file.document.name(),
        "Latest revision 513"
    );
    assert_eq!(std::fs::read(&manual_path).unwrap(), manual_bytes);
    assert_eq!(std::fs::read(&other_path).unwrap(), other_bytes);
    // Even a wrongly reused slot cannot overwrite another document or a manual checkpoint.
    let before = std::fs::read(store.recovery_path(slot)).unwrap();
    let foreign = ProjectFile::new(Document::new("Another project").unwrap()).unwrap();
    assert!(store.write_recovery_slot(&foreign, 1, None, slot).is_err());
    assert!(
        store
            .write_recovery_slot(&file, 514, None, manual.info.id)
            .is_err()
    );
    assert_eq!(std::fs::read(store.recovery_path(slot)).unwrap(), before);
    assert_eq!(std::fs::read(&manual_path).unwrap(), manual_bytes);
}
#[test]
fn discovery_retains_the_newest_512_checked_summaries_independently_of_directory_order() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = ProjectStore::new(dir.path().join("recovery"));
    std::fs::create_dir_all(&store.recovery_directory).unwrap();
    let mut file = ProjectFile::new(Document::new("Recovery discovery").unwrap()).unwrap();
    let mut latest = EntityId::new();
    // Deliberately reversed insertion order. More than 512 independent manual snapshots remain on disk.
    for timestamp in (0..=520).rev() {
        let id = EntityId::new();
        if timestamp == 520 {
            latest = id;
        }
        file.recovery = Some(RecoveryInfo {
            id,
            slot: None,
            created_unix_ms: timestamp,
            source_revision: timestamp,
            original_path: None,
        });
        let mut bytes = vec![];
        project_file::write(&file, &mut bytes, FileLimits::default()).unwrap();
        std::fs::write(store.recovery_path(id), bytes).unwrap();
    }
    let listed = store.recoveries().unwrap();
    assert_eq!(listed.entries.len(), 512);
    assert_eq!(listed.entries[0].info.id, latest);
    assert_eq!(listed.entries.last().unwrap().info.source_revision, 9);
    assert!(listed.diagnostics.iter().any(|d| d.contains("521")));
    assert_eq!(
        std::fs::read_dir(&store.recovery_directory)
            .unwrap()
            .count(),
        521
    );
    assert!(store.restore_recovery(latest).is_ok());
}
