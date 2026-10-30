//! The installation library on disk: one per-user writable directory next to
//! recovery data (never the application bundle). Registration copies the
//! saved project's exact bytes into `entries/<entry>.multicad`, so entries
//! keep working when the original moves, changes or disappears. The index is
//! replaced atomically under a persistent lock shared by all instances.
use multicad_core::{
    document::EntityId,
    project_file::{self, ContentHash, FileLimits, ProjectCard, info::PreviewImage},
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
const INDEX: &str = "library.json";
const MAX_INDEX_BYTES: u64 = 4 * 1024 * 1024;
const MAX_ENTRIES: usize = 4096;

pub struct LibraryFileStore {
    root: PathBuf,
}
impl Default for LibraryFileStore {
    fn default() -> Self {
        let recovery = crate::files::ProjectStore::default().recovery_directory;
        Self::new(recovery.parent().unwrap_or(&recovery).join("library"))
    }
}
fn failure(message: impl std::fmt::Display) -> String {
    format!("{message}")
}
fn storage(context: &str, error: impl std::fmt::Display) -> String {
    format!("{context}: {error}. Check that the library folder is writable and has free space.")
}
/// The stored index record; previews and symbol counts come from snapshots.
struct Record {
    id: EntityId,
    document: EntityId,
    name: String,
    description: String,
    file: String,
    source_path: Option<String>,
    content: ContentHash,
    registered_unix_ms: u64,
}
fn hex(hash: &ContentHash) -> String {
    hash.to_string()
}
fn parse_hex(text: &str) -> Result<ContentHash, String> {
    if text.len() != 64 || !text.is_ascii() {
        return Err("invalid content hash".into());
    }
    let mut bytes = [0u8; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16)
            .map_err(|_| "invalid content hash".to_string())?;
    }
    Ok(ContentHash(bytes))
}
fn decode(bytes: &[u8]) -> Result<Vec<Record>, String> {
    let invalid = |m: &str| format!("Library index is damaged ({m})");
    let value: Value = serde_json::from_slice(bytes).map_err(|e| invalid(&e.to_string()))?;
    if value["application"].as_str() != Some(APPLICATION) || value["version"].as_u64() != Some(1) {
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
        let file = text(entry, "file")?;
        // Snapshots always live directly inside entries/.
        if !file.starts_with("entries/")
            || file[8..].contains(['/', '\\'])
            || file.contains("..")
            || !file.ends_with(".multicad")
        {
            return Err(invalid("snapshot path escapes the library"));
        }
        records.push(Record {
            id: EntityId::parse(&text(entry, "id")?).map_err(|e| invalid(&e.to_string()))?,
            document: EntityId::parse(&text(entry, "document")?)
                .map_err(|e| invalid(&e.to_string()))?,
            name: text(entry, "name")?,
            description: text(entry, "description")?,
            file,
            source_path: entry["source_path"].as_str().map(Into::into),
            content: parse_hex(&text(entry, "content")?).map_err(|e| invalid(&e))?,
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
                "name": r.name,
                "description": r.description,
                "file": r.file,
                "source_path": r.source_path,
                "content": hex(&r.content),
                "registered_unix_ms": r.registered_unix_ms,
            })
        })
        .collect();
    serde_json::to_vec_pretty(&json!({"application":APPLICATION,"version":1,"entries":entries}))
        .map_err(failure)
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
        Self { root }
    }
    fn lock(&self) -> Result<File, String> {
        fs::create_dir_all(self.root.join("entries"))
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
    fn persist(&self, path: &Path, bytes: &[u8], context: &str) -> Result<(), String> {
        let mut temporary =
            tempfile::NamedTempFile::new_in(&self.root).map_err(|e| storage(context, e))?;
        temporary
            .write_all(bytes)
            .map_err(|e| storage(context, e))?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|e| storage(context, e))?;
        temporary.persist(path).map_err(|e| storage(context, e))?;
        #[cfg(unix)]
        if let Some(parent) = path.parent() {
            File::open(parent)
                .and_then(|d| d.sync_all())
                .map_err(|e| storage(context, e))?;
        }
        Ok(())
    }
    fn entry(&self, record: &Record) -> LibraryEntry {
        let snapshot = self.root.join(&record.file);
        let card = File::open(&snapshot)
            .map_err(|e| e.to_string())
            .and_then(|f| card_of(&mut BufReader::new(f)));
        let (preview, symbol_definitions, problem) = match card {
            Ok(card) => (card.info.preview, card.symbol_definitions, None),
            Err(error) => (
                None,
                0,
                Some(format!(
                    "The library copy is missing or damaged ({error}). Open the original file and choose Add to Library again."
                )),
            ),
        };
        LibraryEntry {
            id: record.id,
            document: record.document,
            name: record.name.clone(),
            description: record.description.clone(),
            preview,
            snapshot,
            source_path: record.source_path.as_ref().map(PathBuf::from),
            content: record.content,
            registered_unix_ms: record.registered_unix_ms,
            symbol_definitions,
            problem,
        }
    }
    fn entries(&self, records: &[Record]) -> Vec<LibraryEntry> {
        let mut entries: Vec<_> = records.iter().map(|r| self.entry(r)).collect();
        entries.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then(a.id.cmp(&b.id))
        });
        entries
    }
    /// Read and check the saved project exactly as it is on disk.
    fn saved_bytes(request: &RegisterRequest) -> Result<Vec<u8>, String> {
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
        Ok(bytes)
    }
}
impl LibraryStore for LibraryFileStore {
    fn load(&mut self) -> Result<Vec<LibraryEntry>, String> {
        let _lock = self.lock()?;
        let records = self.read_index()?;
        Ok(self.entries(&records))
    }
    fn register(
        &mut self,
        request: &RegisterRequest,
    ) -> Result<(RegisterReceipt, Vec<LibraryEntry>), String> {
        let bytes = Self::saved_bytes(request)?;
        let card = card_of(&mut bytes.as_slice())?;
        let content = ContentHash::of(&bytes);
        let source_path = request
            .path
            .canonicalize()
            .unwrap_or_else(|_| request.path.clone())
            .to_string_lossy()
            .into_owned();
        let _lock = self.lock()?;
        let mut records = self.read_index()?;
        let existing = records.iter().position(|r| r.document == request.document);
        if let Some(index) = existing {
            let record = &records[index];
            let snapshot_intact = read_limited(
                &self.root.join(&record.file),
                FileLimits::default().maximum_file_bytes,
            )
            .is_ok_and(|old| ContentHash::of(&old) == record.content);
            if record.content == content && snapshot_intact {
                let entry = self.entry(record);
                return Ok((
                    RegisterReceipt {
                        outcome: Registration::Unchanged,
                        entry,
                        previous_source: None,
                    },
                    self.entries(&records),
                ));
            }
        } else if records.len() >= MAX_ENTRIES {
            return Err(format!(
                "The library holds its maximum of {MAX_ENTRIES} entries"
            ));
        }
        let (id, outcome, previous_source) = match existing {
            Some(index) => {
                let previous = records[index].source_path.clone();
                (
                    records[index].id,
                    Registration::Updated,
                    previous.filter(|p| *p != source_path).map(PathBuf::from),
                )
            }
            None => (EntityId::new(), Registration::Added, None),
        };
        let file = format!("entries/{id}.multicad");
        // Write the snapshot before the index: a failure leaves the previous
        // registration (or none) intact.
        self.persist(
            &self.root.join(&file),
            &bytes,
            "Cannot write the library copy",
        )?;
        let record = Record {
            id,
            document: request.document,
            name: card.info.name.clone(),
            description: card.info.description.clone(),
            file,
            source_path: Some(source_path),
            content,
            registered_unix_ms: now_ms(),
        };
        match existing {
            Some(index) => records[index] = record,
            None => records.push(record),
        }
        let index = encode(&records)?;
        self.persist(
            &self.root.join(INDEX),
            &index,
            "Cannot write the library index",
        )?;
        let entry = self.entry(
            records
                .iter()
                .find(|r| r.id == id)
                .expect("registered record"),
        );
        Ok((
            RegisterReceipt {
                outcome,
                entry,
                previous_source,
            },
            self.entries(&records),
        ))
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
