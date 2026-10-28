//! User-local launch metadata. The persistent lock inode is separate from the
//! atomically replaced JSON file, so independent instances share one lock.
use multicad_ui::recent_projects::{RecentProject, RecentStore, RecentView, normalize};
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAX_BYTES: usize = 128 * 1024;
const APPLICATION: &str = "multicad.recent-projects";

pub struct RecentFileStore {
    path: PathBuf,
}
impl Default for RecentFileStore {
    fn default() -> Self {
        let recovery = crate::files::ProjectStore::default().recovery_directory;
        Self::new(
            recovery
                .parent()
                .unwrap_or(&recovery)
                .join("recent-projects.json"),
        )
    }
}
fn invalid(message: impl std::fmt::Display) -> String {
    format!("Recent-project metadata: {message}")
}
fn fields(value: &Value, expected: &[&str]) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid("expected an object"))?;
    if object.len() != expected.len() || expected.iter().any(|key| !object.contains_key(*key)) {
        return Err(invalid("unknown or missing fields"));
    }
    Ok(())
}
fn validate(entry: &RecentProject) -> Result<&str, String> {
    entry.validate()?;
    let path = entry
        .path
        .to_str()
        .ok_or_else(|| invalid("project path is not UTF-8"))?;
    if path.chars().any(char::is_control) {
        return Err(invalid("project path contains control characters"));
    }
    Ok(path)
}
fn decode(bytes: &[u8]) -> Result<Vec<RecentProject>, String> {
    if bytes.len() > MAX_BYTES {
        return Err(invalid("file exceeds 128 KiB"));
    }
    let value: Value = serde_json::from_slice(bytes).map_err(invalid)?;
    fields(&value, &["application", "version", "projects"])?;
    let version = value["version"].as_u64();
    if value["application"].as_str() != Some(APPLICATION) || !matches!(version, Some(1 | 2)) {
        return Err(invalid("foreign application or unsupported version"));
    }
    let projects = value["projects"]
        .as_array()
        .ok_or_else(|| invalid("projects must be an array"))?;
    if projects.len() > 1024 {
        return Err(invalid("project list exceeds its budget"));
    }
    let mut entries = Vec::with_capacity(projects.len());
    for project in projects {
        let view = if version == Some(1) {
            fields(project, &["path", "title", "last_opened_unix_ms"])?;
            None
        } else {
            fields(project, &["path", "title", "last_opened_unix_ms", "view"])?;
            if project["view"].is_null() {
                None
            } else {
                fields(&project["view"], &["mode", "workspace"])?;
                let mode = project["view"]["mode"]
                    .as_str()
                    .and_then(|name| {
                        multicad_ui::registry::Mode::ALL
                            .into_iter()
                            .find(|m| m.title() == name)
                    })
                    .ok_or_else(|| invalid("unknown workspace mode"))?;
                let workspace = project["view"]["workspace"]
                    .as_str()
                    .ok_or_else(|| invalid("workspace must be a string"))?
                    .into();
                Some(RecentView { mode, workspace })
            }
        };
        let entry = RecentProject {
            view,
            path: PathBuf::from(
                project["path"]
                    .as_str()
                    .ok_or_else(|| invalid("path must be a string"))?,
            ),
            title: project["title"]
                .as_str()
                .ok_or_else(|| invalid("title must be a string"))?
                .into(),
            last_opened_unix_ms: project["last_opened_unix_ms"]
                .as_u64()
                .ok_or_else(|| invalid("timestamp must be an unsigned integer"))?,
        };
        validate(&entry)?;
        entries.push(entry);
    }
    normalize(entries)
}
fn encode(entries: &[RecentProject]) -> Result<Vec<u8>, String> {
    let mut projects = Vec::with_capacity(entries.len());
    for entry in entries {
        let path = validate(entry)?;
        let view = entry
            .view
            .as_ref()
            .map(|v| json!({"mode":v.mode.title(),"workspace":v.workspace}));
        projects.push(json!({"path":path,"title":entry.title,"last_opened_unix_ms":entry.last_opened_unix_ms,"view":view}));
    }
    let bytes = serde_json::to_vec_pretty(
        &json!({"application":APPLICATION,"version":2,"projects":projects}),
    )
    .map_err(invalid)?;
    if bytes.len() > MAX_BYTES {
        return Err(invalid("file exceeds 128 KiB"));
    }
    Ok(bytes)
}
impl RecentFileStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
    fn parent(&self) -> &Path {
        self.path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
    }
    fn lock(&self) -> Result<File, String> {
        let name = self
            .path
            .file_name()
            .ok_or_else(|| invalid("choose a metadata filename"))?;
        fs::create_dir_all(self.parent()).map_err(invalid)?;
        let mut lock_name = std::ffi::OsString::from(".");
        lock_name.push(name);
        lock_name.push(".lock");
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.parent().join(lock_name))
            .map_err(invalid)?;
        // Blocking is confined to the dedicated metadata worker. Keep the lock
        // file: deleting it can strand another process on the previous inode.
        lock.lock().map_err(invalid)?;
        Ok(lock)
    }
    fn read_locked(&self) -> Result<Vec<RecentProject>, String> {
        let metadata = match fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(invalid(e)),
        };
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(invalid("expected a regular metadata file"));
        }
        if metadata.len() > MAX_BYTES as u64 {
            return Err(invalid("file exceeds 128 KiB"));
        }
        let mut bytes = vec![];
        File::open(&self.path)
            .map_err(invalid)?
            .take(MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(invalid)?;
        decode(&bytes)
    }
}
impl RecentStore for RecentFileStore {
    fn load(&mut self) -> Result<Vec<RecentProject>, String> {
        let _lock = self.lock()?;
        self.read_locked()
    }
    fn record(&mut self, entry: &RecentProject) -> Result<Vec<RecentProject>, String> {
        validate(entry)?;
        let _lock = self.lock()?;
        let mut entries = self.read_locked()?;
        // A stale asynchronous receipt must not replace a newer visit. Equal
        // timestamps permit the current successful receipt to refresh a title.
        if entries.iter().any(|old| {
            old.path == entry.path && old.last_opened_unix_ms > entry.last_opened_unix_ms
        }) {
            return Ok(entries);
        }
        let mut entry = entry.clone();
        if entry.view.is_none() {
            entry.view = entries
                .iter()
                .find(|old| old.path == entry.path)
                .and_then(|old| old.view.clone());
        }
        entries.retain(|old| old.path != entry.path);
        entries.push(entry);
        let entries = normalize(entries)?;
        let bytes = encode(&entries)?;
        let mut temporary = tempfile::NamedTempFile::new_in(self.parent()).map_err(invalid)?;
        temporary.write_all(&bytes).map_err(invalid)?;
        temporary.flush().map_err(invalid)?;
        temporary.as_file().sync_all().map_err(invalid)?;
        temporary.persist(&self.path).map_err(invalid)?;
        #[cfg(unix)]
        File::open(self.parent())
            .and_then(|directory| directory.sync_all())
            .map_err(|e| {
                invalid(format!(
                    "saved, but directory durability could not be confirmed: {e}"
                ))
            })?;
        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };

    fn entry(root: &Path, index: usize, timestamp: u64) -> RecentProject {
        RecentProject {
            path: root.join(format!("project-{index}.multicad")),
            title: format!("Project {index}"),
            last_opened_unix_ms: timestamp,
            view: None,
        }
    }
    #[test]
    fn restart_preserves_recents_without_touching_canonical_project_bytes() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("state/recents.json");
        let mut store = RecentFileStore::new(path.clone());
        assert!(store.load().unwrap().is_empty());
        assert!(!path.exists());
        let project = entry(root.path(), 1, 100);
        let source = b"MCADPRJ\0actual source bytes are never metadata";
        fs::write(&project.path, source).unwrap();
        assert_eq!(store.record(&project).unwrap(), vec![project.clone()]);
        assert_eq!(fs::read(&project.path).unwrap(), source);
        assert_eq!(
            RecentFileStore::new(path.clone()).load().unwrap(),
            vec![project]
        );
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 2);
        let expected = crate::files::ProjectStore::default()
            .recovery_directory
            .parent()
            .unwrap()
            .join("recent-projects.json");
        assert_eq!(RecentFileStore::default().path, expected);
    }
    #[test]
    fn order_deduplication_timestamps_and_budgets_preserve_unrelated_entries() {
        let root = tempfile::tempdir().unwrap();
        let mut store = RecentFileStore::new(root.path().join("recent.json"));
        for index in 0..25 {
            store
                .record(&entry(root.path(), index, index as u64))
                .unwrap();
        }
        let values = store.load().unwrap();
        assert_eq!(values.len(), 20);
        assert_eq!(
            values
                .iter()
                .map(|v| v.last_opened_unix_ms)
                .collect::<Vec<_>>(),
            (5..25).rev().collect::<Vec<_>>()
        );
        let mut latest = entry(root.path(), 24, 1);
        latest.title = "Stale title".into();
        assert_eq!(store.record(&latest).unwrap(), values);
        latest.last_opened_unix_ms = 24;
        latest.title = "Current title".into();
        let values = store.record(&latest).unwrap();
        assert_eq!(values.len(), 20);
        assert_eq!(values[0], latest);
        assert_eq!(values.iter().filter(|v| v.path == latest.path).count(), 1);
        let huge = RecentProject {
            path: PathBuf::from(format!("/{}", "a".repeat(8192))),
            ..latest
        };
        let bytes = fs::read(&store.path).unwrap();
        assert!(store.record(&huge).is_err());
        assert_eq!(fs::read(&store.path).unwrap(), bytes);
    }
    #[test]
    fn malformed_foreign_and_oversized_metadata_fail_without_overwriting_any_bytes() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("recent.json");
        let valid = entry(root.path(), 1, 100);
        let encoded = encode(std::slice::from_ref(&valid)).unwrap();
        let mut foreign: Value = serde_json::from_slice(&encoded).unwrap();
        foreign["application"] = json!("another.application");
        let mut unknown: Value = serde_json::from_slice(&encoded).unwrap();
        unknown["extra"] = json!(true);
        let mut bad_path: Value = serde_json::from_slice(&encoded).unwrap();
        bad_path["projects"][0]["path"] = json!("relative.multicad");
        let mut bad_timestamp: Value = serde_json::from_slice(&encoded).unwrap();
        bad_timestamp["projects"][0]["last_opened_unix_ms"] = json!(-1);
        for bytes in [
            b"{broken".to_vec(),
            serde_json::to_vec(&foreign).unwrap(),
            serde_json::to_vec(&unknown).unwrap(),
            serde_json::to_vec(&bad_path).unwrap(),
            serde_json::to_vec(&bad_timestamp).unwrap(),
            vec![b' '; MAX_BYTES + 1],
            b"MCADPRJ\0native project bytes".to_vec(),
        ] {
            fs::write(&path, &bytes).unwrap();
            let mut store = RecentFileStore::new(path.clone());
            assert!(store.load().is_err());
            assert!(store.record(&valid).is_err());
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
    }
    #[test]
    fn invalid_entry_does_not_create_or_replace_metadata() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("state/recent.json");
        let mut invalid = entry(root.path(), 1, 1);
        invalid.title = "\n".into();
        assert!(RecentFileStore::new(path.clone()).record(&invalid).is_err());
        assert!(!path.parent().unwrap().exists());
        invalid.title = "Valid title".into();
        invalid.path = PathBuf::from("/invalid\0path");
        assert!(RecentFileStore::new(path.clone()).record(&invalid).is_err());
        assert!(!path.parent().unwrap().exists());
    }

    const CHILD_PATH: &str = "MULTICAD_RECENTS_TEST_PATH";
    const CHILD_INDEX: &str = "MULTICAD_RECENTS_TEST_INDEX";
    #[test]
    fn process_writer() {
        let Some(path) = std::env::var_os(CHILD_PATH) else {
            return;
        };
        let path = PathBuf::from(path);
        let index: usize = std::env::var(CHILD_INDEX).unwrap().parse().unwrap();
        let root = path.parent().unwrap().to_path_buf();
        File::create_new(root.join(format!("ready-{index}"))).unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        while !root.join("start").exists() {
            assert!(
                Instant::now() < deadline,
                "Parent did not release concurrent writers"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        let mut store = RecentFileStore::new(path);
        for offset in 0..2 {
            store
                .record(&entry(
                    &root,
                    index * 2 + offset,
                    1000 + (index * 2 + offset) as u64,
                ))
                .unwrap();
        }
    }
    #[test]
    fn independent_process_writers_merge_under_one_persistent_advisory_lock() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("recent.json");
        let mut store = RecentFileStore::new(path.clone());
        let existing = entry(root.path(), 100, 100);
        store.record(&existing).unwrap();
        for index in 0..16 {
            fs::write(
                entry(root.path(), index, 0).path,
                format!("Canonical project {index}"),
            )
            .unwrap();
        }
        let test = format!(
            "{}::process_writer",
            module_path!().split_once("::").unwrap().1
        );
        let mut children = (0..8)
            .map(|index| {
                Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", &test, "--nocapture"])
                    .env(CHILD_PATH, &path)
                    .env(CHILD_INDEX, index.to_string())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap()
            })
            .collect::<Vec<_>>();
        let deadline = Instant::now() + Duration::from_secs(20);
        while (0..8).any(|i| !root.path().join(format!("ready-{i}")).exists()) {
            if Instant::now() >= deadline {
                for child in &mut children {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                panic!("Recent-project child initialization timed out");
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        File::create_new(root.path().join("start")).unwrap();
        while children
            .iter_mut()
            .any(|child| child.try_wait().unwrap().is_none())
        {
            if Instant::now() >= deadline {
                for child in &mut children {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                panic!("Recent-project child writer timed out");
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        for child in children {
            let result = child.wait_with_output().unwrap();
            assert!(result.status.success(), "Child writer failed: {result:?}");
        }
        let values = store.load().unwrap();
        assert_eq!(values.len(), 17);
        assert!(values.contains(&existing));
        for index in 0..16 {
            assert!(
                values
                    .iter()
                    .any(|v| v.path == entry(root.path(), index, 0).path)
            );
            assert_eq!(
                fs::read_to_string(entry(root.path(), index, 0).path).unwrap(),
                format!("Canonical project {index}")
            );
        }
    }
    #[test]
    fn workspace_metadata_roundtrips_and_legacy_history_migrates_without_losing_visits() {
        let root = tempfile::tempdir().unwrap();
        let mut store = RecentFileStore::new(root.path().join("recent.json"));
        let mut first = entry(root.path(), 0, 1);
        first.view = Some(RecentView {
            mode: multicad_ui::registry::Mode::Model,
            workspace: "Sheet metal".into(),
        });
        store.record(&first).unwrap();
        assert_eq!(store.load().unwrap(), vec![first.clone()]);
        let mut visit = first.clone();
        visit.view = None;
        visit.last_opened_unix_ms = 2;
        let persisted = store.record(&visit).unwrap();
        assert_eq!(
            persisted[0].view, first.view,
            "Opening before history is read must retain its saved view"
        );
        let mut legacy = serde_json::from_slice::<Value>(&fs::read(&store.path).unwrap()).unwrap();
        legacy["version"] = json!(1);
        legacy["projects"][0]
            .as_object_mut()
            .unwrap()
            .remove("view");
        fs::write(&store.path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        assert_eq!(store.load().unwrap()[0].view, None);
        first.last_opened_unix_ms = 3;
        store.record(&first).unwrap();
        let migrated = serde_json::from_slice::<Value>(&fs::read(&store.path).unwrap()).unwrap();
        assert_eq!(migrated["version"], json!(2));
        assert_eq!(store.load().unwrap()[0], first);
        let mut invalid = migrated;
        invalid["projects"][0]["view"]["mode"] = json!("Unknown");
        let bytes = serde_json::to_vec(&invalid).unwrap();
        fs::write(&store.path, &bytes).unwrap();
        assert!(store.record(&visit).is_err());
        assert_eq!(fs::read(&store.path).unwrap(), bytes);
    }
}
