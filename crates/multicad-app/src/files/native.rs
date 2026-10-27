use multicad_core::{
    document::{Document, EntityId},
    project_file::{self, ContentHash, FileLimits, ProjectFile, RecoveryInfo},
};
use multicad_ui::storage::{
    DocumentStorage, FileStamp, LoadedProject, RecoveryEntry, RecoveryListing, SaveMode,
    SavedProject, StorageError,
};
use std::{
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
    fn recovery_path(&self, id: EntityId) -> PathBuf {
        self.recovery_directory
            .join(format!("{id}.recovery.multicad"))
    }
}
fn stamp(file: &mut File) -> Result<FileStamp, StorageError> {
    let mut header = [0u8; 52];
    file.read_exact(&mut header)?;
    if &header[..8] != b"MCADPRJ\0" {
        return Err(StorageError::Message(
            "This is not a native MultiCAD project. Use Import source snapshot for JSON.".into(),
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
    let existing = existing_stamp(path)?;
    match (mode, existing) {
        (SaveMode::Create, Some(stamp)) => return Err(StorageError::AlreadyExists(stamp)),
        (SaveMode::Replace(expected), Some(current)) if expected == current => {}
        (SaveMode::Replace(_), _) => return Err(StorageError::Changed),
        (SaveMode::Create, None) => {}
    }
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    project_file::write(project, &mut temporary, FileLimits::default())
        .map_err(StorageError::Message)?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    before_publish()?;
    match mode {
        SaveMode::Create => temporary
            .persist_noclobber(path)
            .map_err(|e| StorageError::Message(e.to_string()))?,
        SaveMode::Replace(_) => temporary
            .persist(path)
            .map_err(|e| StorageError::Message(e.to_string()))?,
    };
    let warnings = directory_sync_warnings(parent);
    let stamp = existing_stamp(path)?
        .ok_or_else(|| StorageError::Message("Saved project is missing".into()))?;
    drop(lock);
    Ok(SavedProject { stamp, warnings })
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
        std::fs::create_dir_all(&self.recovery_directory)?;
        let created_unix_ms = u64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| StorageError::Message(e.to_string()))?
                .as_millis(),
        )
        .map_err(|e| StorageError::Message(e.to_string()))?;
        let info = RecoveryInfo {
            id: EntityId::new(),
            created_unix_ms,
            source_revision: revision,
            original_path: original.map(|p| p.to_string_lossy().into_owned()),
        };
        let mut snapshot = project.clone();
        snapshot.recovery = Some(info.clone());
        atomic_save(&self.recovery_path(info.id), &snapshot, SaveMode::Create)?;
        Ok(RecoveryEntry {
            info,
            document: project.document.id(),
            name: project.document.name().into(),
        })
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
        let mut examined = 0;
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
            examined += 1;
            if examined > 512 {
                listing.diagnostics.push(
                    "Recovery listing is limited to 512 files; older snapshots remain on disk."
                        .into(),
                );
                break;
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
                Ok(entry) => listing.entries.push(entry),
                Err(error) => listing
                    .diagnostics
                    .push(format!("A recovery snapshot could not be listed: {error}")),
            }
        }
        listing
            .entries
            .sort_by_key(|e| std::cmp::Reverse(e.info.created_unix_ms));
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
