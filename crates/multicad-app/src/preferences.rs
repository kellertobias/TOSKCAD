//! User-local application preferences file (CAD-187). It lives beside the
//! recent-project metadata, never inside a project, and is replaced
//! atomically under a persistent lock shared by every instance.
use multicad_ui::{
    preferences::{LengthUnit, PreferenceStore, Preferences, WorkbenchPreference},
    registry::Mode,
};
use serde_json::{Map, Value, json};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAX_BYTES: usize = 64 * 1024;
const APPLICATION: &str = "multicad.preferences";
const VERSION: u64 = 1;

pub struct PreferenceFileStore {
    path: PathBuf,
}
impl Default for PreferenceFileStore {
    fn default() -> Self {
        let recovery = crate::files::ProjectStore::default().recovery_directory;
        Self::new(
            recovery
                .parent()
                .unwrap_or(&recovery)
                .join("preferences.json"),
        )
    }
}
fn invalid(message: impl std::fmt::Display) -> String {
    format!("Preferences file: {message}")
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
fn decode(bytes: &[u8]) -> Result<Preferences, String> {
    if bytes.len() > MAX_BYTES {
        return Err(invalid("file exceeds 64 KiB"));
    }
    let value: Value = serde_json::from_slice(bytes).map_err(invalid)?;
    fields(
        &value,
        &[
            "application",
            "version",
            "length_unit",
            "workbenches",
            "title_logo",
        ],
    )?;
    if value["application"].as_str() != Some(APPLICATION)
        || value["version"].as_u64() != Some(VERSION)
    {
        return Err(invalid("foreign application or unsupported version"));
    }
    let length_unit = value["length_unit"]
        .as_str()
        .and_then(LengthUnit::from_symbol)
        .ok_or_else(|| invalid("length unit must be \"mm\" or \"in\""))?;
    let modes = value["workbenches"]
        .as_object()
        .ok_or_else(|| invalid("workbenches must be an object"))?;
    let mut workbenches = BTreeMap::new();
    for (name, entries) in modes {
        let mode = Mode::ALL
            .into_iter()
            .find(|mode| mode.title() == name)
            .ok_or_else(|| invalid(format!("unknown mode {name:?}")))?;
        let entries = entries
            .as_array()
            .ok_or_else(|| invalid("workbench list must be an array"))?;
        let mut list = Vec::with_capacity(entries.len());
        for entry in entries {
            fields(entry, &["title", "visible"])?;
            list.push(WorkbenchPreference {
                title: entry["title"]
                    .as_str()
                    .ok_or_else(|| invalid("workbench title must be a string"))?
                    .into(),
                visible: entry["visible"]
                    .as_bool()
                    .ok_or_else(|| invalid("workbench visibility must be a boolean"))?,
            });
        }
        workbenches.insert(mode, list);
    }
    let title_logo = value["title_logo"]
        .as_bool()
        .ok_or_else(|| invalid("title_logo must be a boolean"))?;
    let preferences = Preferences {
        title_logo,
        length_unit,
        workbenches,
    };
    preferences.validate().map_err(invalid)?;
    Ok(preferences)
}
fn encode(preferences: &Preferences) -> Result<Vec<u8>, String> {
    preferences.validate().map_err(invalid)?;
    let mut modes = Map::new();
    for (mode, entries) in &preferences.workbenches {
        modes.insert(
            mode.title().into(),
            entries
                .iter()
                .map(|e| json!({"title": e.title, "visible": e.visible}))
                .collect(),
        );
    }
    let bytes = serde_json::to_vec_pretty(&json!({
        "application": APPLICATION,
        "version": VERSION,
        "length_unit": preferences.length_unit.symbol(),
        "workbenches": modes,
        "title_logo": preferences.title_logo,
    }))
    .map_err(invalid)?;
    if bytes.len() > MAX_BYTES {
        return Err(invalid("file exceeds 64 KiB"));
    }
    Ok(bytes)
}
impl PreferenceFileStore {
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
            .ok_or_else(|| invalid("choose a preferences filename"))?;
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
        lock.lock().map_err(invalid)?;
        Ok(lock)
    }
}
impl PreferenceStore for PreferenceFileStore {
    fn load(&mut self) -> Result<Option<Preferences>, String> {
        let metadata = match fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(invalid(e)),
        };
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(invalid("expected a regular file"));
        }
        let _lock = self.lock()?;
        let mut bytes = vec![];
        File::open(&self.path)
            .map_err(invalid)?
            .take(MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(invalid)?;
        decode(&bytes).map(Some)
    }
    fn save(&mut self, preferences: &Preferences) -> Result<(), String> {
        let bytes = encode(preferences)?;
        let _lock = self.lock()?;
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
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_ui::registry::Registry;
    #[test]
    fn preferences_survive_restart_through_the_file_store() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("state/preferences.json");
        let registry = Registry::default();
        let mut store = PreferenceFileStore::new(path.clone());
        assert_eq!(store.load().unwrap(), None);
        let mut preferences = Preferences::defaults(&registry);
        preferences.length_unit = LengthUnit::Inch;
        preferences.title_logo = false;
        preferences.set_visible(Mode::Model, "Wood", false).unwrap();
        preferences
            .move_workbench(Mode::Export, "Render", -2)
            .unwrap();
        store.save(&preferences).unwrap();
        // A new process reads exactly the saved preferences back.
        let mut restarted = PreferenceFileStore::new(path.clone());
        let mut loaded = restarted.load().unwrap().unwrap();
        loaded.normalize(&registry);
        assert_eq!(loaded, preferences);
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"length_unit\": \"in\""), "{text}");
        // Restore defaults overwrites the stored choice.
        let defaults = Preferences::defaults(&registry);
        restarted.save(&defaults).unwrap();
        assert_eq!(
            PreferenceFileStore::new(path).load().unwrap(),
            Some(defaults)
        );
    }
    #[test]
    fn foreign_corrupt_or_out_of_range_files_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("preferences.json");
        for bytes in [
            &b"not json"[..],
            br#"{"application":"other","version":1,"length_unit":"mm","workbenches":{},"title_logo":true}"#,
            br#"{"application":"multicad.preferences","version":2,"length_unit":"mm","workbenches":{},"title_logo":true}"#,
            br#"{"application":"multicad.preferences","version":1,"length_unit":"cm","workbenches":{},"title_logo":true}"#,
            br#"{"application":"multicad.preferences","version":1,"length_unit":"mm","workbenches":{"Woodshop":[]},"title_logo":true}"#,
            br#"{"application":"multicad.preferences","version":1,"length_unit":"mm","workbenches":{"Model":[{"title":"Wood"}]},"title_logo":true}"#,
            br#"{"application":"multicad.preferences","version":1,"length_unit":"mm","workbenches":{},"title_logo":true,"extra":1}"#,
            br#"{"application":"multicad.preferences","version":1,"length_unit":"mm","workbenches":{}}"#,
            br#"{"application":"multicad.preferences","version":1,"length_unit":"mm","workbenches":{},"title_logo":"no"}"#,
        ] {
            fs::write(&path, bytes).unwrap();
            assert!(
                PreferenceFileStore::new(path.clone()).load().is_err(),
                "{}",
                String::from_utf8_lossy(bytes)
            );
        }
        fs::write(&path, vec![b' '; MAX_BYTES + 1]).unwrap();
        assert!(PreferenceFileStore::new(path).load().is_err());
    }
}
