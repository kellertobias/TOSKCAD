//! Interim canonical source snapshots; not the complete native project container.
use multicad_core::{document::Document, parameters};
use std::{
    io::{Read, Write},
    path::Path,
};
#[path = "files/native.rs"]
mod native;
pub use native::ProjectStore;
fn read_source(path: &Path) -> Result<Document, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("Source snapshot exceeds 16 MiB".into());
    }
    if bytes.starts_with(b"MCADPRJ\0") {
        return Err(
            "This is a native project. Use Open project instead of Import source snapshot.".into(),
        );
    }
    let doc: Document = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    Ok(parameters::evaluate(&doc)
        .map_err(|e| e.to_string())?
        .document)
}
#[allow(
    dead_code,
    reason = "The legacy adapter is constructed by acceptance examples, not the production executable"
)]
pub struct SnapshotStore;
impl multicad_ui::application::DocumentStorage for SnapshotStore {
    fn save(&mut self, path: &Path, document: &Document) -> Result<(), String> {
        parameters::evaluate(document).map_err(|e| e.to_string())?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let bytes = serde_json::to_vec_pretty(document).map_err(|e| e.to_string())?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        temporary.write_all(&bytes).map_err(|e| e.to_string())?;
        temporary.as_file().sync_all().map_err(|e| e.to_string())?;
        temporary.persist(path).map_err(|e| e.to_string())?;
        Ok(())
    }
    fn load(&mut self, path: &Path) -> Result<Document, String> {
        read_source(path)
    }
    fn save_project(
        &mut self,
        path: &Path,
        project: &multicad_core::project_file::ProjectFile,
        mode: multicad_ui::storage::SaveMode,
    ) -> Result<multicad_ui::storage::SavedProject, multicad_ui::storage::StorageError> {
        ProjectStore::default().save_project(path, project, mode)
    }
    fn open_project(
        &mut self,
        path: &Path,
    ) -> Result<multicad_ui::storage::LoadedProject, multicad_ui::storage::StorageError> {
        ProjectStore::default().open_project(path)
    }
    fn write_recovery(
        &mut self,
        project: &multicad_core::project_file::ProjectFile,
        revision: u64,
        original: Option<&Path>,
    ) -> Result<multicad_ui::storage::RecoveryEntry, multicad_ui::storage::StorageError> {
        ProjectStore::default().write_recovery(project, revision, original)
    }
    fn recoveries(
        &mut self,
    ) -> Result<multicad_ui::storage::RecoveryListing, multicad_ui::storage::StorageError> {
        ProjectStore::default().recoveries()
    }
    fn restore_recovery(
        &mut self,
        id: multicad_core::document::EntityId,
    ) -> Result<multicad_ui::storage::LoadedProject, multicad_ui::storage::StorageError> {
        ProjectStore::default().restore_recovery(id)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use multicad_ui::application::DocumentStorage;
    #[test]
    fn source_snapshot_roundtrip_and_failed_save_do_not_erase_valid_data() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("project.multicad.json");
        let document = Document::new("Project").unwrap();
        let mut store = SnapshotStore;
        store.save(&path, &document).unwrap();
        assert_eq!(store.load(&path).unwrap(), document);
        assert!(
            store
                .save(&directory.path().join("missing/file"), &document)
                .is_err()
        );
        assert_eq!(store.load(&path).unwrap(), document);
        std::fs::write(&path, b"{broken").unwrap();
        assert!(store.load(&path).is_err());
    }
}
