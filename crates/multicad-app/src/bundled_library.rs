//! Read-only suppliers for the single installation Library. Geometry remains
//! in checked native projects; this manifest never supplies runtime dimensions.
use multicad_core::{
    document::EntityId,
    project_file::{self, ContentHash, FileLimits},
};
use multicad_ui::library::{LibraryEntry, LibraryOrigin};
use serde::Deserialize;
use std::{
    collections::BTreeSet,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_PROJECT_BYTES: u64 = 32 * 1024 * 1024;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    format: String,
    version: u32,
    revision: String,
    entries: Vec<Entry>,
    requires: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    file: String,
    document: EntityId,
    sha256: String,
}
fn bounded(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    if !std::fs::symlink_metadata(path)
        .map_err(|e| e.to_string())?
        .is_file()
    {
        return Err("Expected a regular file, not a link or folder".into());
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("Bundled library file exceeds its size limit".into());
    }
    Ok(bytes)
}
fn relative(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && !value.contains(['\\', ':'])
        && value
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}
pub fn locate() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    locate_at(&executable.canonicalize().unwrap_or(executable))
}
fn locate_at(executable: &Path) -> Option<PathBuf> {
    let directory = executable.parent()?;
    let mut candidates = vec![
        directory.join("../Resources/library"),
        directory.join("resources/library"),
    ];
    if cfg!(debug_assertions) {
        candidates.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library"));
    }
    candidates
        .into_iter()
        .find(|root| root.join("library.json").is_file())
        .and_then(|root| root.canonicalize().ok())
}
pub fn load(root: &Path) -> Result<Vec<LibraryEntry>, String> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let manifest: Manifest =
        serde_json::from_slice(&bounded(&root.join("library.json"), MAX_MANIFEST_BYTES)?)
            .map_err(|e| format!("Invalid bundled Library manifest: {e}"))?;
    if manifest.format != "toskcad.bundled-library"
        || manifest.version != 1
        || manifest.revision.is_empty()
        || manifest.revision.len() > 100
        || manifest.entries.is_empty()
        || manifest.entries.len() > 256
        || manifest.requires.len() > 256
    {
        return Err("Unsupported bundled Library manifest or entry limit".into());
    }
    for required in &manifest.requires {
        if !relative(required) {
            return Err("Bundled Library requirement escapes its folder".into());
        }
        let path = root
            .join(required)
            .canonicalize()
            .map_err(|e| format!("Missing bundled Library requirement {required}: {e}"))?;
        if !path.starts_with(&root) || !path.is_file() {
            return Err("Bundled Library requirement escapes its folder or is not a file".into());
        }
    }
    let mut identities = BTreeSet::new();
    let mut files = BTreeSet::new();
    let mut entries = Vec::new();
    for source in manifest.entries {
        if !relative(&source.file)
            || source.file.contains('/')
            || !source.file.ends_with(".tcd")
            || source.document.is_nil()
            || !identities.insert(source.document)
            || !files.insert(source.file.clone())
        {
            return Err("Bundled Library has a duplicate identity, invalid file or path".into());
        }
        let mut entry = LibraryEntry {
            origin: LibraryOrigin::Bundled,
            id: source.document,
            document: source.document,
            path: root.join(&source.file),
            name: source.file.clone(),
            description: String::new(),
            preview: None,
            registered_unix_ms: 0,
            symbol_definitions: 0,
            mechanical_parts: Vec::new(),
            problem: None,
        };
        let card = (|| {
            let bytes = bounded(&entry.path, MAX_PROJECT_BYTES)?;
            if ContentHash::of(&bytes).to_string() != source.sha256 {
                return Err("Bundled project checksum differs from the installed manifest".into());
            }
            let card = project_file::card(
                &mut bytes.as_slice(),
                FileLimits::default(),
                &project_file::supported_capabilities(),
            )?;
            if card.document != source.document || card.symbol_definitions == 0 {
                return Err(
                    "Bundled project identity or reusable definitions do not match its manifest"
                        .into(),
                );
            }
            if let Some(error) = card.info_error {
                return Err(error);
            }
            Ok(card)
        })();
        match card {
            Ok(card) => {
                entry.name = card.info.name;
                entry.description = card.info.description;
                entry.preview = card.info.preview;
                entry.symbol_definitions = card.symbol_definitions;
                entry.mechanical_parts = card.mechanical_parts;
            }
            Err(error) => {
                entry.problem = Some(format!(
                    "Included hardware unavailable: {error}. Rebuild or restore the application bundle."
                ))
            }
        }
        entries.push(entry);
    }
    Ok(entries)
}
pub fn problem(root: &Path, error: String) -> LibraryEntry {
    LibraryEntry {
        origin: LibraryOrigin::Bundled,
        id: EntityId::parse("cad20000-0000-4000-8000-000000000001")
            .expect("fixed catalog issue identity"),
        document: EntityId::nil(),
        path: root.join("library.json"),
        name: "Included hardware".into(),
        description: String::new(),
        preview: None,
        registered_unix_ms: 0,
        symbol_definitions: 0,
        mechanical_parts: Vec::new(),
        problem: Some(format!(
            "Included hardware unavailable: {error}. Rebuild or restore the application bundle."
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
        let manifest: Manifest =
            serde_json::from_slice(&std::fs::read(source.join("library.json")).unwrap()).unwrap();
        for file in std::iter::once("library.json".to_string())
            .chain(manifest.entries.into_iter().map(|e| e.file))
            .chain(manifest.requires)
        {
            let destination = directory.path().join(&file);
            std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
            std::fs::copy(source.join(file), destination).unwrap();
        }
        directory
    }
    fn mutate(root: &Path, change: impl FnOnce(&mut serde_json::Value)) {
        let path = root.join("library.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        change(&mut value);
        std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    }
    #[test]
    fn bundled_cards_use_checked_projects_and_do_not_evaluate_native_geometry() {
        let directory = fixture();
        let entries = load(directory.path()).unwrap();
        assert_eq!(entries.len(), 27);
        let mut bearings = 0;
        let mut plants = 0;
        let mut equipment = std::collections::BTreeSet::new();
        for entry in entries {
            assert_eq!(entry.origin, LibraryOrigin::Bundled);
            assert!(entry.problem.is_none());
            assert!(entry.preview.is_some());
            assert_eq!(entry.symbol_definitions, 1);
            if entry.mechanical_parts.is_empty() {
                if entry.name.starts_with("Generic ") {
                    plants += 1;
                } else {
                    equipment.insert(entry.name);
                }
                continue;
            }
            bearings += 1;
            assert_eq!(entry.mechanical_parts.len(), 1);
            assert!(entry.mechanical_parts[0].matches_reference_dimensions);
            assert_eq!(
                entry.mechanical_parts[0].family,
                "Bearing mounting envelope"
            );
        }
        assert_eq!((bearings, plants), (4, 2));
        assert_eq!(
            equipment,
            multicad_core::equipment::Kind::ALL
                .into_iter()
                .map(|kind| kind.preset().name.to_string())
                .collect()
        );
    }
    #[test]
    fn damaged_projects_remain_visible_and_manifest_escapes_or_duplicates_are_refused() {
        let directory = fixture();
        let path = directory.path().join("bearing-open-608.tcd");
        std::fs::write(&path, b"damaged native project").unwrap();
        let entries = load(directory.path()).unwrap();
        assert!(entries[0].problem.as_ref().unwrap().contains("checksum"));
        assert_eq!(entries[0].symbol_definitions, 0);
        assert!(entries[1..].iter().all(|e| e.problem.is_none()));
        mutate(directory.path(), |m| {
            m["entries"][0]["file"] = "../outside.tcd".into()
        });
        assert!(
            load(directory.path())
                .unwrap_err()
                .contains("invalid file or path")
        );
        let directory = fixture();
        mutate(directory.path(), |m| {
            m["entries"][1]["document"] = m["entries"][0]["document"].clone()
        });
        assert!(load(directory.path()).unwrap_err().contains("duplicate"));
        let directory = fixture();
        mutate(directory.path(), |m| {
            m["requires"] = serde_json::json!(["../COPYING.LESSER"])
        });
        assert!(load(directory.path()).unwrap_err().contains("escapes"));
        let directory = fixture();
        std::fs::remove_file(directory.path().join("sources/bolts/COPYING.LESSER")).unwrap();
        assert!(
            load(directory.path())
                .unwrap_err()
                .contains("Missing bundled Library requirement")
        );
    }
    #[test]
    fn catalog_follows_macos_and_portable_layouts_before_development_fallback() {
        for (executable, relative) in [
            (
                "MultiCAD.app/Contents/MacOS/multicad",
                "MultiCAD.app/Contents/Resources/library",
            ),
            ("multicad.exe", "resources/library"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(dir.path().join(executable).parent().unwrap()).unwrap();
            std::fs::write(dir.path().join(executable), b"locator fixture").unwrap();
            let catalog = dir.path().join(relative);
            std::fs::create_dir_all(&catalog).unwrap();
            std::fs::write(catalog.join("library.json"), b"marker").unwrap();
            assert_eq!(
                locate_at(&dir.path().join(executable)),
                Some(catalog.canonicalize().unwrap())
            );
        }
    }
}
