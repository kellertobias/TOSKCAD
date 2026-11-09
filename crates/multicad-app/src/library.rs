//! The installation library on disk: one per-user writable index next to
//! recovery data (never the application bundle). Read-only included projects
//! merge into the same browser without entering that index. Add to Library lists the
//! saved project at its original absolute location; nothing is copied and
//! the project file is never written. Browsing reads name, description and
//! preview live from each listed file. The index is replaced atomically under
//! a persistent lock shared by all application instances.
use multicad_core::{
    document::EntityId,
    project_file::{self, FileLimits, ProjectCard, info::PreviewImage},
};
use multicad_ui::library::{
    LibraryEntry, LibraryStore, RegisterReceipt, RegisterRequest, Registration,
};
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::{BufReader, Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const APPLICATION: &str = "multicad.library";
const VERSION: u64 = 2;
const INDEX: &str = "library.json";
const MAX_INDEX_BYTES: u64 = 4 * 1024 * 1024;
const MAX_ENTRIES: usize = 4096;

pub struct LibraryFileStore {
    root: PathBuf,
    bundled: Option<PathBuf>,
}
impl Default for LibraryFileStore {
    fn default() -> Self {
        let recovery = crate::files::ProjectStore::default().recovery_directory;
        Self::new(recovery.parent().unwrap_or(&recovery).join("library"))
            .with_bundled(crate::bundled_library::locate())
    }
}
fn storage(context: &str, error: impl std::fmt::Display) -> String {
    format!("{context}: {error}. Check that the library folder is writable and has free space.")
}
/// One listing. Name and description are the last values read from the file,
/// shown when it is missing.
struct Record {
    id: EntityId,
    document: EntityId,
    path: String,
    name: String,
    description: String,
    registered_unix_ms: u64,
    tags: Vec<String>,
}
fn decode(bytes: &[u8]) -> Result<Vec<Record>, String> {
    let invalid = |m: &str| format!("Library index is damaged ({m})");
    let value: Value = serde_json::from_slice(bytes).map_err(|e| invalid(&e.to_string()))?;
    let version = value["version"].as_u64();
    if value["application"].as_str() != Some(APPLICATION) || !matches!(version, Some(1 | 2)) {
        return Err(invalid("foreign application or unsupported version"));
    }
    let entries = value["entries"]
        .as_array()
        .ok_or_else(|| invalid("entries must be an array"))?;
    if entries.len() > MAX_ENTRIES {
        return Err(invalid("too many entries"));
    }
    let text = |v: &Value, key: &str| -> Result<String, String> {
        v[key]
            .as_str()
            .map(Into::into)
            .ok_or_else(|| invalid(&format!("{key} must be a string")))
    };
    let mut records = Vec::with_capacity(entries.len());
    for entry in entries {
        // Version 1 (an unreleased development build) kept managed copies and
        // recorded the original as `source_path`. Migrate to that original
        // location; the old copies are ignored and left untouched on disk.
        let path = if version == Some(1) {
            match entry["source_path"].as_str() {
                Some(path) => path.to_string(),
                None => continue,
            }
        } else {
            text(entry, "path")?
        };
        if !Path::new(&path).is_absolute() || path.chars().any(char::is_control) {
            return Err(invalid("listed paths must be absolute"));
        }
        records.push(Record {
            id: EntityId::parse(&text(entry, "id")?).map_err(|e| invalid(&e.to_string()))?,
            document: EntityId::parse(&text(entry, "document")?)
                .map_err(|e| invalid(&e.to_string()))?,
            path,
            name: text(entry, "name")?,
            description: text(entry, "description")?,
            tags: match entry.get("tags") {
                None => Vec::new(),
                Some(tags) => multicad_ui::library::library_tags(
                    serde_json::from_value::<Vec<String>>(tags.clone())
                        .map_err(|e| invalid(&e.to_string()))?,
                )
                .map_err(|e| invalid(&e))?,
            },
            registered_unix_ms: entry["registered_unix_ms"]
                .as_u64()
                .ok_or_else(|| invalid("registered_unix_ms must be an integer"))?,
        });
    }
    Ok(records)
}
fn encode(records: &[Record]) -> Result<Vec<u8>, String> {
    let entries: Vec<_> = records
        .iter()
        .map(|r| {
            json!({
                "id": r.id.to_string(),
                "document": r.document.to_string(),
                "path": r.path,
                "name": r.name,
                "description": r.description,
                "registered_unix_ms": r.registered_unix_ms,
                "tags": r.tags,
            })
        })
        .collect();
    serde_json::to_vec_pretty(
        &json!({"application":APPLICATION,"version":VERSION,"entries":entries}),
    )
    .map_err(|e| e.to_string())
}
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}
fn read_limited(path: &Path, limit: u64) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(std::io::Error::other("file exceeds its size limit"));
    }
    Ok(bytes)
}
fn card_of(reader: &mut impl Read) -> Result<ProjectCard, String> {
    project_file::card(
        reader,
        FileLimits::default(),
        &project_file::supported_capabilities(),
    )
}
impl LibraryFileStore {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            bundled: None,
        }
    }
    pub fn with_bundled(mut self, root: Option<PathBuf>) -> Self {
        self.bundled = root;
        self
    }
    fn merged(&self, records: &[Record]) -> Vec<LibraryEntry> {
        let mut entries = Self::entries(records);
        if let Some(root) = &self.bundled {
            match crate::bundled_library::load(root) {
                Ok(bundled) => entries.extend(bundled),
                Err(error) => entries.push(crate::bundled_library::problem(root, error)),
            }
        }
        entries
    }
    fn lock(&self) -> Result<File, String> {
        fs::create_dir_all(&self.root)
            .map_err(|e| storage("Cannot create the library folder", e))?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.root.join(".library.json.lock"))
            .map_err(|e| storage("Cannot lock the library", e))?;
        lock.lock()
            .map_err(|e| storage("Cannot lock the library", e))?;
        Ok(lock)
    }
    fn read_index(&self) -> Result<Vec<Record>, String> {
        let path = self.root.join(INDEX);
        match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(storage("Cannot read the library index", e)),
            Ok(m) if !m.is_file() => {
                return Err("Library index is not a regular file".into());
            }
            Ok(_) => {}
        }
        let bytes = read_limited(&path, MAX_INDEX_BYTES)
            .map_err(|e| storage("Cannot read the library index", e))?;
        decode(&bytes)
    }
    fn write_index(&self, records: &[Record]) -> Result<(), String> {
        let context = "Cannot write the library index";
        let bytes = encode(records)?;
        let mut temporary =
            tempfile::NamedTempFile::new_in(&self.root).map_err(|e| storage(context, e))?;
        temporary
            .write_all(&bytes)
            .map_err(|e| storage(context, e))?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|e| storage(context, e))?;
        temporary
            .persist(self.root.join(INDEX))
            .map_err(|e| storage(context, e))?;
        #[cfg(unix)]
        File::open(&self.root)
            .and_then(|d| d.sync_all())
            .map_err(|e| storage(context, e))?;
        Ok(())
    }
    /// The live view of one listing, read from the original file.
    fn entry(record: &Record) -> LibraryEntry {
        let path = PathBuf::from(&record.path);
        let mut entry = LibraryEntry {
            origin: multicad_ui::library::LibraryOrigin::ListedOriginal,
            id: record.id,
            document: record.document,
            path: path.clone(),
            name: record.name.clone(),
            description: record.description.clone(),
            preview: None,
            registered_unix_ms: record.registered_unix_ms,
            tags: record.tags.clone(),
            symbol_definitions: 0,
            mechanical_parts: Vec::new(),
            problem: None,
        };
        let card = match File::open(&path) {
            Ok(file) => card_of(&mut BufReader::new(file)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                entry.problem = Some(format!(
                    "Missing: no file at {} (moved or deleted). Open it from its new location and choose Add to Library, or remove this entry.",
                    path.display()
                ));
                return entry;
            }
            Err(e) => Err(e.to_string()),
        };
        match card {
            Ok(card) if card.document != record.document => {
                entry.problem = Some(format!(
                    "{} now holds a different project. Re-add the original from its new location, or remove this entry.",
                    path.display()
                ));
            }
            Ok(card) => {
                entry.name = card.info.name;
                entry.description = card.info.description;
                entry.preview = card.info.preview;
                entry.symbol_definitions = card.symbol_definitions;
                entry.mechanical_parts = card.mechanical_parts;
            }
            Err(error) => {
                entry.problem = Some(format!(
                    "Cannot read {} ({error}). Fix or re-save the file, or remove this entry.",
                    path.display()
                ));
            }
        }
        entry
    }
    fn entries(records: &[Record]) -> Vec<LibraryEntry> {
        let mut entries: Vec<_> = records.iter().map(Self::entry).collect();
        entries.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then(a.id.cmp(&b.id))
        });
        entries
    }
    /// Check that the file on disk is exactly the project this window saved.
    fn saved_card(request: &RegisterRequest) -> Result<ProjectCard, String> {
        let limits = FileLimits::default();
        let bytes = match read_limited(&request.path, limits.maximum_file_bytes) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(format!(
                    "The saved file is missing at {}. Save the project again, then choose Add to Library.",
                    request.path.display()
                ));
            }
            Err(e) => return Err(format!("Cannot read {}: {e}", request.path.display())),
        };
        if bytes.len() < 52
            || &bytes[..8] != b"MCADPRJ\0"
            || bytes[20..52] != request.stamp.manifest.0
            || bytes.len() as u64 != request.stamp.length
        {
            return Err(
                "The file changed on disk since this window opened or saved it. Reopen it or use Save As, then choose Add to Library."
                    .into(),
            );
        }
        let opened = project_file::read(
            &mut bytes.as_slice(),
            limits,
            &project_file::supported_capabilities(),
        )?;
        if opened.project.recovery.is_some() {
            return Err("Recovery snapshots cannot be added to the library".into());
        }
        if opened.project.document.id() != request.document {
            return Err("The file on disk is a different project. Save this project first.".into());
        }
        card_of(&mut bytes.as_slice())
    }
}
impl LibraryStore for LibraryFileStore {
    fn load(&mut self) -> Result<Vec<LibraryEntry>, String> {
        let _lock = self.lock()?;
        let records = self.read_index()?;
        Ok(self.merged(&records))
    }
    fn register(
        &mut self,
        request: &RegisterRequest,
    ) -> Result<(RegisterReceipt, Vec<LibraryEntry>), String> {
        if self.bundled.as_ref().is_some_and(|root| {
            request
                .path
                .canonicalize()
                .ok()
                .zip(root.canonicalize().ok())
                .is_some_and(|(path, root)| path.starts_with(root))
        }) {
            return Err("Included hardware is already in Library. Save your own copy before adding a separate listing.".into());
        }
        let card = Self::saved_card(request)?;
        let path = request
            .path
            .canonicalize()
            .unwrap_or_else(|_| request.path.clone())
            .to_string_lossy()
            .into_owned();
        let _lock = self.lock()?;
        let mut records = self.read_index()?;
        let existing = records.iter().position(|r| r.document == request.document);
        let outcome = match existing {
            Some(index) if records[index].path == path => Registration::AlreadyListed,
            Some(index) => Registration::Relocated {
                previous: PathBuf::from(&records[index].path),
            },
            None if records.len() >= MAX_ENTRIES => {
                return Err(format!(
                    "The library holds its maximum of {MAX_ENTRIES} entries"
                ));
            }
            None => Registration::Added,
        };
        let id = existing.map_or_else(EntityId::new, |index| records[index].id);
        let record = Record {
            id,
            document: request.document,
            path: path.clone(),
            name: card.info.name.clone(),
            description: card.info.description.clone(),
            tags: existing.map_or_else(Vec::new, |index| records[index].tags.clone()),
            registered_unix_ms: existing
                .map_or_else(now_ms, |index| records[index].registered_unix_ms),
        };
        // A listing at this location for another document is stale: that
        // file was replaced by this project.
        records.retain(|r| r.document == request.document || r.path != path);
        match records.iter().position(|r| r.document == request.document) {
            Some(index) => records[index] = record,
            None => records.push(record),
        }
        // Also refreshes the last known name and description of an
        // already-listed file.
        self.write_index(&records)?;
        let entry = Self::entry(records.iter().find(|r| r.id == id).expect("listed record"));
        Ok((RegisterReceipt { outcome, entry }, self.merged(&records)))
    }
    fn remove(&mut self, entry: EntityId) -> Result<Vec<LibraryEntry>, String> {
        let _lock = self.lock()?;
        let mut records = self.read_index()?;
        let before = records.len();
        records.retain(|r| r.id != entry);
        if records.len() == before {
            return Err("That library entry no longer exists".into());
        }
        self.write_index(&records)?;
        Ok(self.merged(&records))
    }
    fn set_tags(&mut self, entry: EntityId, tags: Vec<String>) -> Result<Vec<String>, String> {
        let tags = multicad_ui::library::library_tags(tags)?;
        let _lock = self.lock()?;
        let mut records = self.read_index()?;
        let record = records
            .iter_mut()
            .find(|record| record.id == entry)
            .ok_or("That Library listing no longer exists")?;
        if record.tags != tags {
            record.tags.clone_from(&tags);
            self.write_index(&records)?;
        }
        Ok(tags)
    }
    fn card(&mut self, path: &Path) -> Result<ProjectCard, String> {
        let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        card_of(&mut BufReader::new(file))
    }
    fn read_preview(&mut self, path: &Path) -> Result<PreviewImage, String> {
        let limit = project_file::info::MAX_PREVIEW_BYTES as u64;
        let bytes = read_limited(path, limit).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                format!("The image {} no longer exists", path.display())
            } else {
                format!(
                    "Cannot use {} as a preview ({e}). Choose a PNG of at most {} KiB.",
                    path.display(),
                    limit / 1024
                )
            }
        })?;
        PreviewImage::from_png(bytes)
    }
}

#[cfg(test)]
#[path = "library_tests.rs"]
mod tests;
