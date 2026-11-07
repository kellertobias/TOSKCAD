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
        assert_eq!(entries.len(), 172);
        let mut bearings = 0;
        let mut washers = BTreeSet::new();
        let mut screws = BTreeSet::new();
        let mut profile_screws = BTreeSet::new();
        let profile_curation: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../library/machine-screws-curation.json"
        ))
        .unwrap();
        let mut wood_screws = BTreeSet::new();
        let wood_curation: serde_json::Value =
            serde_json::from_slice(include_bytes!("../../../library/wood-screws-curation.json"))
                .unwrap();
        let mut nuts = BTreeSet::new();
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
            assert_eq!(entry.mechanical_parts.len(), 1);
            let card = &entry.mechanical_parts[0];
            assert!(card.matches_reference_dimensions);
            if card.family == "Machine screw" {
                let facts = card.catalog_facts.as_ref().unwrap();
                let nominal = facts.nominal_fastener_diameter_mm.unwrap();
                assert_eq!(card.dimensions_mm["Nominal thread diameter"], nominal);
                assert!(
                    card.dimensions_mm["Thread length"] <= card.dimensions_mm["Underhead length"]
                );
                let series = facts.series.as_deref().unwrap();
                if let Some(family) =
                    series.strip_suffix("; source dimensions with explicit engineering profile")
                {
                    let source = profile_curation["profile_entries"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|source| {
                            source["family"] == family
                                && source["nominal_mm"].as_f64() == Some(nominal)
                        })
                        .unwrap();
                    assert_eq!(
                        facts.material.as_deref(),
                        source["source_material"].as_str()
                    );
                    assert_eq!(facts.grade.as_deref(), source["source_grade"].as_str());
                    let length = source["length_mm"].as_f64().unwrap();
                    let key = if source["length_convention"] == "overall" {
                        "Overall length"
                    } else {
                        "Underhead length"
                    };
                    assert_eq!(card.dimensions_mm[key], length);
                    assert!(card.problem.is_none());
                    assert!(card.symbol.is_some());
                    if source["head"]["kind"] == "countersunk"
                        || source["drive"]["kind"] != "hex_socket"
                    {
                        assert!(!card.profile_details.is_empty());
                    }
                    assert_eq!(
                        card.source_url.as_deref(),
                        source["verification_url"].as_str()
                    );
                    assert!(profile_screws.insert((family.to_owned(), (nominal * 1000.) as u32)));
                } else {
                    let socket_cap = card.representation.starts_with("Socket-cap head");
                    assert_eq!(facts.material.as_deref(), Some("Steel"));
                    assert_eq!(
                        facts.grade.as_deref(),
                        Some(if socket_cap { "12.9" } else { "8.8" })
                    );
                    screws.insert((
                        (nominal * 1000.) as u32,
                        (card.dimensions_mm["Underhead length"] * 1000.) as u32,
                        socket_cap,
                    ));
                }
            } else if card.family == "Wood screw envelope" {
                let facts = card.catalog_facts.as_ref().unwrap();
                let nominal = facts.nominal_fastener_diameter_mm.unwrap();
                let underhead = card.dimensions_mm["Underhead stock-envelope length"];
                let source = wood_curation["entries"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|source| {
                        source["nominal_mm"].as_f64() == Some(nominal)
                            && source["underhead_length_mm"].as_f64() == Some(underhead)
                    })
                    .unwrap();
                assert_eq!(card.dimensions_mm["Nominal wood diameter"], nominal);
                assert_eq!(
                    card.dimensions_mm["Head across flats"],
                    source["head"]["across_flats_max_mm"].as_f64().unwrap()
                );
                assert_eq!(
                    card.dimensions_mm["Head height"],
                    source["head"]["height_nominal_mm"].as_f64().unwrap()
                );
                assert!(
                    (card.dimensions_mm["Minimum tip-inclusive thread extent"] - 0.6 * underhead)
                        .abs()
                        < 1e-10
                );
                assert!(
                    (card.dimensions_mm["Maximum possible plain length"] - 0.4 * underhead).abs()
                        < 1e-10
                );
                assert!(
                    card.representation
                        .contains("Simplified wood stock envelope")
                );
                assert!(card.representation.contains("manufacturing unavailable"));
                assert!(!card.designation.as_ref().unwrap().starts_with('M'));
                assert!(!card.dimensions_mm.contains_key("Nominal thread diameter"));
                assert!(facts.material.is_none() && facts.grade.is_none());
                assert!(card.symbol.is_some());
                assert!(wood_screws.insert(((nominal * 1000.) as u32, (underhead * 1000.) as u32)));
            } else if card.family == "Hex nut" {
                let facts = card.catalog_facts.as_ref().unwrap();
                let nominal = facts.nominal_fastener_diameter_mm.unwrap();
                let series = facts.series.as_deref().unwrap();
                assert!(matches!(series, "DIN 934:1987" | "ISO 4032:2012"));
                assert_eq!(card.dimensions_mm["Nominal thread diameter"], nominal);
                assert_eq!(card.representation, "Modeled internal thread");
                assert!(card.problem.is_none());
                assert!(card.symbol.is_some());
                assert!(facts.material.is_none());
                assert!(facts.grade.is_none());
                assert!(card.designation.as_ref().unwrap().contains(series));
                assert!(card.source_url.as_ref().unwrap().starts_with("https://"));
                if nominal == 10. {
                    let expected = if series == "DIN 934:1987" {
                        (17., 8.)
                    } else {
                        (16., 8.4)
                    };
                    assert_eq!(
                        (
                            card.dimensions_mm["Across flats"],
                            card.dimensions_mm["Height"]
                        ),
                        expected
                    );
                }
                assert!(
                    nuts.insert((series.to_owned(), (nominal * 1000.) as u32)),
                    "Duplicate nut family/nominal card"
                );
            } else if card.family == "Plain washer" {
                let facts = card.catalog_facts.as_ref().unwrap();
                washers.insert((facts.nominal_fastener_diameter_mm.unwrap() * 1000.0) as u32);
                assert_eq!(
                    facts.series.as_deref(),
                    Some("normal (BOLTS plainwasher1, curated revision 1)")
                );
                assert!(facts.material.is_none());
                assert!(facts.grade.is_none());
            } else {
                bearings += 1;
                assert_eq!(card.family, "Bearing mounting envelope");
                assert!(card.catalog_facts.is_none());
            }
        }
        assert_eq!((bearings, plants), (4, 2));
        assert_eq!(screws.len(), 19);
        assert_eq!(profile_screws.len(), 52);
        assert_eq!(wood_screws.len(), 44);
        assert_eq!(nuts.len(), 19);
        for (series, expected) in [
            (
                "DIN 934:1987",
                vec![2000, 2500, 3000, 3500, 4000, 5000, 6000, 7000, 8000, 10000],
            ),
            (
                "ISO 4032:2012",
                vec![2000, 2500, 3000, 3500, 4000, 5000, 6000, 8000, 10000],
            ),
        ] {
            assert_eq!(
                nuts.iter()
                    .filter(|(family, _)| family == series)
                    .map(|(_, nominal)| *nominal)
                    .collect::<BTreeSet<_>>(),
                expected.into_iter().collect()
            );
        }
        assert_eq!(equipment.len(), 22);
        assert_eq!(
            screws.iter().map(|s| s.0).collect::<BTreeSet<_>>(),
            [2000, 2500, 3000, 4000, 5000, 6000, 8000, 10000].into()
        );
        assert_eq!(
            washers,
            [2000, 2500, 3000, 3500, 4000, 5000, 6000, 7000, 8000, 10000].into()
        );
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
