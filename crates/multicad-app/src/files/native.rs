use multicad_core::{
    document::{Document, EntityId},
    project_file::{self, ContentHash, FileLimits, ProjectFile, RecoveryInfo},
};
use multicad_ui::storage::{
    DocumentStorage, FileStamp, LoadedProject, RecoveryEntry, RecoveryListing, SaveMode,
    SavedProject, StorageError,
};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub struct ProjectStore {
    pub recovery_directory: PathBuf,
}
impl Default for ProjectStore {
    fn default() -> Self {
        let base = if cfg!(target_os = "macos") {
            std::env::var_os("HOME").map(|p| PathBuf::from(p).join("Library/Application Support"))
        } else if cfg!(target_os = "windows") {
            std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
        } else {
            std::env::var_os("XDG_STATE_HOME")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/state")))
        };
        Self::new(
            base.unwrap_or_else(std::env::temp_dir)
                .join("MultiCAD/recovery"),
        )
    }
}
impl ProjectStore {
    pub fn new(recovery_directory: PathBuf) -> Self {
        Self { recovery_directory }
    }
    fn open(&self, path: &Path, allow_recovery: bool) -> Result<LoadedProject, StorageError> {
        let mut file = File::open(path)?;
        let stamp = stamp(&mut file)?;
        file.seek(SeekFrom::Start(0))?;
        let opened = project_file::read(
            &mut BufReader::new(file),
            FileLimits::default(),
            &project_file::supported_capabilities(),
        )
        .map_err(StorageError::Message)?;
        if opened.project.recovery.is_some() && !allow_recovery {
            return Err(StorageError::Message(
                "This is a recovery snapshot. Use Recover projects to open an unsaved copy.".into(),
            ));
        }
        Ok(LoadedProject{file:opened.project,stamp:Some(stamp),diagnostics:opened.diagnostics.into_iter().map(|d|match d {
            project_file::ProjectDiagnostic::Migrated{from,to}=>format!("Project format {from} migrated to {to}; Save writes the current format."),
            project_file::ProjectDiagnostic::CacheNeedsValidation{..}=>"A cached result requires its provider to validate versions and dependencies before reuse.".into(),
            project_file::ProjectDiagnostic::CacheSourceChanged{..}=>"A cached result belongs to older source and requires rebuilding.".into(),
            project_file::ProjectDiagnostic::CacheCorrupt{..}=>"A corrupt optional cache was discarded; source is intact and requires rebuilding.".into(),
        }).collect()})
    }
    fn write_snapshot(
        &self,
        project: &ProjectFile,
        revision: u64,
        original: Option<&Path>,
        slot: Option<EntityId>,
    ) -> Result<RecoveryEntry, StorageError> {
        std::fs::create_dir_all(&self.recovery_directory)?;
        let created_unix_ms = u64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| StorageError::Message(e.to_string()))?
                .as_millis(),
        )
        .map_err(|e| StorageError::Message(e.to_string()))?;
        let info = RecoveryInfo {
            id: slot.unwrap_or_default(),
            slot,
            created_unix_ms,
            source_revision: revision,
            original_path: original.map(|p| p.to_string_lossy().into_owned()),
        };
        let mut snapshot = project.clone();
        snapshot.recovery = Some(info.clone());
        let path = self.recovery_path(info.id);
        let mode = match (slot, existing_stamp(&path)?) {
            (Some(_), Some(stamp)) => {
                let mut previous = File::open(&path)?;
                let summary = project_file::summary(
                    &mut previous,
                    FileLimits::default(),
                    &project_file::supported_capabilities(),
                )
                .map_err(StorageError::Message)?;
                if summary.document != project.document.id()
                    || summary
                        .recovery
                        .as_ref()
                        .is_none_or(|r| r.id != info.id || r.slot != slot)
                {
                    return Err(StorageError::Message("The automatic recovery slot belongs to another document. It was left unchanged.".into()));
                }
                SaveMode::Replace(stamp)
            }
            _ => SaveMode::Create,
        };
        atomic_save(&path, &snapshot, mode)?;
        Ok(RecoveryEntry {
            info,
            document: project.document.id(),
            name: project.document.name().into(),
        })
    }
    fn recovery_path(&self, id: EntityId) -> PathBuf {
        self.recovery_directory
            .join(format!("{id}.recovery.multicad"))
    }
}
fn stamp(file: &mut File) -> Result<FileStamp, StorageError> {
    file.seek(SeekFrom::Start(0))?;
    let mut header = [0u8; 52];
    file.read_exact(&mut header)?;
    if &header[..8] != b"MCADPRJ\0" {
        return Err(StorageError::Message(
            "This is not a native ToskCAD project. Use Import source snapshot for JSON.".into(),
        ));
    }
    Ok(FileStamp {
        manifest: ContentHash(header[20..52].try_into().expect("fixed header")),
        length: file.metadata()?.len(),
    })
}
fn existing_stamp(path: &Path) -> Result<Option<FileStamp>, StorageError> {
    match File::open(path) {
        Ok(mut file) => stamp(&mut file).map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}
fn atomic_save(
    path: &Path,
    project: &ProjectFile,
    mode: SaveMode,
) -> Result<SavedProject, StorageError> {
    atomic_save_with(path, project, mode, || Ok(()))
}
fn atomic_save_with(
    path: &Path,
    project: &ProjectFile,
    mode: SaveMode,
    before_publish: impl FnOnce() -> Result<(), StorageError>,
) -> Result<SavedProject, StorageError> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| StorageError::Message("Choose a project filename".into()))?;
    let lock_path = parent.join(format!(".{}.multicad-save.lock", name.to_string_lossy()));
    // A persistent lock inode avoids races between processes waiting on the same
    // path. OS locks release automatically if a writer exits unexpectedly.
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)?;
    lock.try_lock().map_err(|e| {
        StorageError::Message(format!("Another process is saving this project: {e}"))
    })?;
    check_destination(path, mode)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    project_file::write(project, &mut temporary, FileLimits::default())
        .map_err(StorageError::Message)?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    before_publish()?;
    // Serialization can be long. Detect a noncooperating writer again before
    // publication; the advisory lock serializes cooperating MultiCAD writers.
    check_destination(path, mode)?;
    let mut published = match mode {
        SaveMode::Create => temporary
            .persist_noclobber(path)
            .map_err(|e| StorageError::Message(e.to_string()))?,
        SaveMode::Replace(_) => temporary
            .persist(path)
            .map_err(|e| StorageError::Message(e.to_string()))?,
    };
    let warnings = directory_sync_warnings(parent);
    // Read our published inode, not a new open that could observe another
    // writer's replacement and incorrectly acknowledge its stamp as our save.
    let stamp = stamp(&mut published)?;
    drop(lock);
    Ok(SavedProject { stamp, warnings })
}
fn check_destination(path: &Path, mode: SaveMode) -> Result<(), StorageError> {
    match (mode, existing_stamp(path)?) {
        (SaveMode::Create, Some(stamp)) => Err(StorageError::AlreadyExists(stamp)),
        (SaveMode::Replace(expected), Some(current)) if expected == current => Ok(()),
        (SaveMode::Replace(_), _) => Err(StorageError::Changed),
        (SaveMode::Create, None) => Ok(()),
    }
}
fn directory_sync_warnings(parent: &Path) -> Vec<String> {
    #[cfg(unix)]
    if let Err(error) = File::open(parent).and_then(|directory| directory.sync_all()) {
        return vec![format!(
            "The project was saved, but directory durability could not be confirmed: {error}"
        )];
    }
    #[cfg(not(unix))]
    let _ = parent;
    Vec::new()
}
impl DocumentStorage for ProjectStore {
    fn save(&mut self, _path: &Path, _document: &Document) -> Result<(), String> {
        Err(
            "Use native project Save; source JSON writing is only an explicit fixture operation"
                .into(),
        )
    }
    fn load(&mut self, path: &Path) -> Result<Document, String> {
        super::read_source(path)
    }
    fn save_project(
        &mut self,
        path: &Path,
        project: &ProjectFile,
        mode: SaveMode,
    ) -> Result<SavedProject, StorageError> {
        if path.extension().is_none_or(|e| e != "multicad") {
            return Err(StorageError::Message(
                "Native projects use the .multicad extension. Choose Save As for a source import."
                    .into(),
            ));
        }
        let mut project = project.clone();
        project.recovery = None;
        atomic_save(path, &project, mode)
    }
    fn open_project(&mut self, path: &Path) -> Result<LoadedProject, StorageError> {
        self.open(path, false)
    }
    fn write_recovery(
        &mut self,
        project: &ProjectFile,
        revision: u64,
        original: Option<&Path>,
    ) -> Result<RecoveryEntry, StorageError> {
        self.write_snapshot(project, revision, original, None)
    }
    fn write_recovery_slot(
        &mut self,
        project: &ProjectFile,
        revision: u64,
        original: Option<&Path>,
        slot: EntityId,
    ) -> Result<RecoveryEntry, StorageError> {
        self.write_snapshot(project, revision, original, Some(slot))
    }
    fn recoveries(&mut self) -> Result<RecoveryListing, StorageError> {
        let entries = match std::fs::read_dir(&self.recovery_directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(RecoveryListing::default());
            }
            Err(error) => return Err(error.into()),
        };
        let mut listing = RecoveryListing::default();
        let mut retained = BTreeMap::new();
        let mut valid_count = 0usize;
        let mut invalid_count = 0usize;
        for entry in entries {
            let entry = entry?;
            if !entry.file_type()?.is_file()
                || !entry
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".recovery.multicad")
            {
                continue;
            }
            let result = (|| {
                let mut file = File::open(entry.path())?;
                let summary = project_file::summary(
                    &mut file,
                    FileLimits::default(),
                    &project_file::supported_capabilities(),
                )
                .map_err(StorageError::Message)?;
                let info = summary.recovery.ok_or_else(|| {
                    StorageError::Message("File is not a recovery snapshot".into())
                })?;
                if entry.path() != self.recovery_path(info.id)
                    || file.metadata()?.len() != summary.expected_file_bytes
                {
                    return Err(StorageError::Message(
                        "Recovery filename or length does not match its manifest".into(),
                    ));
                }
                Ok(RecoveryEntry {
                    info,
                    document: summary.document,
                    name: summary.name,
                })
            })();
            match result {
                Ok(entry) => {
                    valid_count += 1;
                    retained.insert((entry.info.created_unix_ms, entry.info.id), entry);
                    if retained.len() > 512 {
                        retained.pop_first();
                    }
                }
                Err(error) => {
                    invalid_count += 1;
                    if listing.diagnostics.len() < 32 {
                        listing
                            .diagnostics
                            .push(format!("A recovery snapshot could not be listed: {error}"));
                    }
                }
            }
        }
        listing.entries = retained.into_values().rev().collect();
        if valid_count > 512 {
            listing.diagnostics.push(format!("Showing the 512 newest of {valid_count} recovery snapshots. Other snapshots remain on disk."));
        }
        if invalid_count > 32 {
            listing.diagnostics.push(format!("{invalid_count} recovery files could not be listed; showing the first 32 diagnostics."));
        }
        Ok(listing)
    }
    fn restore_recovery(&mut self, id: EntityId) -> Result<LoadedProject, StorageError> {
        let mut loaded = self.open(&self.recovery_path(id), true)?;
        if loaded.file.recovery.as_ref().is_none_or(|r| r.id != id) {
            return Err(StorageError::Message(
                "Recovery identity does not match the selected snapshot".into(),
            ));
        }
        loaded.stamp = None;
        loaded
            .diagnostics
            .push("Recovery opened as an unsaved copy. The original file was not replaced.".into());
        Ok(loaded)
    }
}
#[cfg(test)]
#[path = "tests.rs"]
mod tests;
