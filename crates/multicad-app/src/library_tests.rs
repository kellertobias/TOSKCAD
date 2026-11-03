use super::*;
use crate::files::ProjectStore;
use multicad_core::{
    document::Document,
    project_file::{
        ProjectFile,
        info::{self, FileInfo},
    },
};
use multicad_ui::storage::{DocumentStorage, FileStamp, SaveMode};

const ICON: &[u8] = include_bytes!("../../../assets/branding/app/AppIcon.png");

fn project(name: &str, description: &str) -> ProjectFile {
    let document = Document::new("Untitled").unwrap();
    let wanted = FileInfo {
        name: name.into(),
        description: description.into(),
        preview: Some(PreviewImage::from_png(ICON.to_vec()).unwrap()),
        capture_on_save: true,
    };
    let document = document
        .edited(&info::edits(&document, &wanted).unwrap())
        .unwrap();
    ProjectFile::new(document).unwrap()
}
fn describe(project: &ProjectFile, name: &str, description: &str) -> ProjectFile {
    let mut wanted = info::read(&project.document).unwrap();
    wanted.name = name.into();
    wanted.description = description.into();
    ProjectFile::new(
        project
            .document
            .edited(&info::edits(&project.document, &wanted).unwrap())
            .unwrap(),
    )
    .unwrap()
}
fn save(store: &mut ProjectStore, path: &Path, project: &ProjectFile, mode: SaveMode) -> FileStamp {
    store.save_project(path, project, mode).unwrap().stamp
}
fn request(path: &Path, stamp: FileStamp, project: &ProjectFile) -> RegisterRequest {
    RegisterRequest {
        path: path.into(),
        stamp,
        document: project.document.id(),
    }
}
fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap()
}

#[test]
fn included_definitions_share_browsing_but_never_enter_or_change_the_original_file_index() {
    use multicad_ui::library::LibraryOrigin;
    let root = tempfile::tempdir().unwrap();
    let bundled = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let library_root = root.path().join("library");
    let mut library =
        LibraryFileStore::new(library_root.clone()).with_bundled(Some(bundled.clone()));
    let included = library.load().unwrap();
    assert_eq!(included.len(), 57);
    assert!(included.iter().all(|e| e.origin == LibraryOrigin::Bundled));
    assert!(
        !library_root.join(INDEX).exists(),
        "included entries are not silently registered"
    );
    let mut projects = ProjectStore::new(root.path().join("recovery"));
    let path = root.path().join("own.tcd");
    let own = project("Own part", "Original location remains authoritative");
    let stamp = save(&mut projects, &path, &own, SaveMode::Create);
    let original = fs::read(&path).unwrap();
    let (receipt, entries) = library.register(&request(&path, stamp, &own)).unwrap();
    assert_eq!(entries.len(), included.len() + 1);
    assert_eq!(
        entries
            .iter()
            .filter(|e| e.origin == LibraryOrigin::ListedOriginal)
            .count(),
        1
    );
    assert_eq!(
        decode(&fs::read(library_root.join(INDEX)).unwrap())
            .unwrap()
            .len(),
        1
    );
    let index = fs::read(library_root.join(INDEX)).unwrap();
    assert!(library.remove(included[0].id).is_err());
    assert_eq!(fs::read(library_root.join(INDEX)).unwrap(), index);
    let source = projects.open_project(&included[0].path).unwrap();
    assert!(
        library
            .register(&request(
                &included[0].path,
                source.stamp.unwrap(),
                &source.file
            ))
            .unwrap_err()
            .contains("already in Library")
    );
    assert_eq!(fs::read(library_root.join(INDEX)).unwrap(), index);
    assert_eq!(
        library.remove(receipt.entry.id).unwrap().len(),
        included.len()
    );
    assert_eq!(fs::read(path).unwrap(), original);
    assert!(
        decode(&fs::read(library_root.join(INDEX)).unwrap())
            .unwrap()
            .is_empty()
    );
    assert!(
        fs::read_dir(library_root).unwrap().all(|entry| entry
            .unwrap()
            .path()
            .extension()
            .is_none_or(|e| e != "tcd"))
    );
}

#[test]
fn a_broken_included_catalog_does_not_hide_registered_originals() {
    let root = tempfile::tempdir().unwrap();
    let bad = root.path().join("bad-catalog");
    fs::create_dir_all(&bad).unwrap();
    fs::write(bad.join("library.json"), b"not JSON").unwrap();
    let mut library = LibraryFileStore::new(root.path().join("library")).with_bundled(Some(bad));
    let mut projects = ProjectStore::new(root.path().join("recovery"));
    let own = project("Own part", "Still available");
    let path = root.path().join("own.tcd");
    let stamp = save(&mut projects, &path, &own, SaveMode::Create);
    let (_, entries) = library.register(&request(&path, stamp, &own)).unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].name, "Own part");
    assert!(entries[0].problem.is_none());
    assert!(
        entries[1]
            .problem
            .as_ref()
            .unwrap()
            .contains("Invalid bundled Library manifest")
    );
}

#[test]
fn listing_records_the_original_location_and_reads_metadata_live() {
    let root = tempfile::tempdir().unwrap();
    let mut projects = ProjectStore::new(root.path().join("recovery"));
    let path = root.path().join("bracket.multicad");
    let mut bracket = project("Wall bracket", "Holds a shelf");
    let stamp = save(&mut projects, &path, &bracket, SaveMode::Create);
    let original = fs::read(&path).unwrap();
    let library_root = root.path().join("library");
    let mut library = LibraryFileStore::new(library_root.clone());
    assert!(
        library.load().unwrap().is_empty(),
        "saving never lists a file"
    );

    let (receipt, entries) = library.register(&request(&path, stamp, &bracket)).unwrap();
    assert_eq!(receipt.outcome, Registration::Added);
    assert!(receipt.message().contains("Listed “Wall bracket”"));
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, canonical(&path));
    assert_eq!(entries[0].preview.as_ref().unwrap().png(), ICON);
    assert_eq!(
        fs::read(&path).unwrap(),
        original,
        "the file is never written"
    );
    let names: Vec<_> = fs::read_dir(&library_root)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert!(
        names
            .iter()
            .all(|n| n == "library.json" || n == ".library.json.lock"),
        "no copies: {names:?}"
    );

    // Same file again: visible no-op, no duplicate.
    let (again, entries) = library.register(&request(&path, stamp, &bracket)).unwrap();
    assert_eq!(again.outcome, Registration::AlreadyListed);
    assert!(again.message().contains("already listed"));
    assert_eq!(entries.len(), 1);

    // Later edits to the original show up without re-adding (live read).
    bracket = describe(&bracket, "Wall bracket v2", "Deeper shelf");
    save(&mut projects, &path, &bracket, SaveMode::Replace(stamp));
    let entries = LibraryFileStore::new(library_root).load().unwrap();
    assert_eq!(entries.len(), 1, "a new store instance (restart) sees it");
    assert_eq!(entries[0].name, "Wall bracket v2");
    assert_eq!(entries[0].description, "Deeper shelf");
    assert!(entries[0].problem.is_none());
}

#[test]
fn moved_or_deleted_files_stay_listed_as_missing_and_can_be_relocated_or_removed() {
    let root = tempfile::tempdir().unwrap();
    let mut projects = ProjectStore::new(root.path().join("recovery"));
    let first = root.path().join("a.multicad");
    let part = project("Part", "Original");
    let stamp = save(&mut projects, &first, &part, SaveMode::Create);
    let mut library = LibraryFileStore::new(root.path().join("library"));
    library.register(&request(&first, stamp, &part)).unwrap();

    let moved = root.path().join("moved.multicad");
    fs::rename(&first, &moved).unwrap();
    let entries = library.load().unwrap();
    assert_eq!(entries.len(), 1, "never dropped silently");
    assert_eq!(entries[0].path, canonical(root.path()).join("a.multicad"));
    assert_eq!(entries[0].name, "Part", "last known name");
    let problem = entries[0].problem.as_ref().unwrap();
    assert!(
        problem.contains("Missing") && problem.contains("Add to Library"),
        "{problem}"
    );

    // Re-adding from the new location updates the entry and says so.
    let (receipt, entries) = library.register(&request(&moved, stamp, &part)).unwrap();
    assert!(matches!(receipt.outcome, Registration::Relocated { .. }));
    assert!(
        receipt.message().contains("instead of"),
        "{}",
        receipt.message()
    );
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, canonical(&moved));
    assert!(entries[0].problem.is_none());

    // A Save As copy keeps the document id: listing it relocates too.
    let copy = root.path().join("copy.multicad");
    let stamp = save(&mut projects, &copy, &part, SaveMode::Create);
    let (receipt, entries) = library.register(&request(&copy, stamp, &part)).unwrap();
    assert!(matches!(receipt.outcome, Registration::Relocated { .. }));
    assert_eq!(entries.len(), 1);

    // Another project now at the listed path is reported, not shown as ours.
    let other = project("Other", "");
    let copy_stamp = projects.open_project(&copy).unwrap().stamp.unwrap();
    save(&mut projects, &copy, &other, SaveMode::Replace(copy_stamp));
    let entries = library.load().unwrap();
    assert!(
        entries[0]
            .problem
            .as_ref()
            .unwrap()
            .contains("different project")
    );

    let entries = library.remove(entries[0].id).unwrap();
    assert!(entries.is_empty());
    assert!(copy.exists(), "removing a listing never touches the file");
    assert!(library.remove(EntityId::new()).is_err());
}

#[test]
fn missing_changed_and_unwritable_registrations_fail_without_partial_entries() {
    let root = tempfile::tempdir().unwrap();
    let mut projects = ProjectStore::new(root.path().join("recovery"));
    let path = root.path().join("part.multicad");
    let part = project("Part", "");
    let stamp = save(&mut projects, &path, &part, SaveMode::Create);
    let mut library = LibraryFileStore::new(root.path().join("library"));

    let other = project("Other", "");
    let mut external = ProjectStore::new(root.path().join("recovery"));
    let _ = save(&mut external, &path, &other, SaveMode::Replace(stamp));
    let error = library.register(&request(&path, stamp, &part)).unwrap_err();
    assert!(error.contains("changed on disk"), "{error}");

    fs::remove_file(&path).unwrap();
    let error = library.register(&request(&path, stamp, &part)).unwrap_err();
    assert!(error.contains("missing"), "{error}");
    assert!(library.load().unwrap().is_empty());

    let stamp = save(&mut projects, &path, &part, SaveMode::Create);
    let blocked = root.path().join("blocked");
    fs::write(&blocked, b"not a directory").unwrap();
    let error = LibraryFileStore::new(blocked)
        .register(&request(&path, stamp, &part))
        .unwrap_err();
    assert!(error.contains("library folder"), "{error}");
}

#[test]
fn an_index_from_the_snapshot_build_migrates_to_original_locations() {
    let root = tempfile::tempdir().unwrap();
    let mut projects = ProjectStore::new(root.path().join("recovery"));
    let path = root.path().join("old.multicad");
    let part = project("Old listing", "");
    save(&mut projects, &path, &part, SaveMode::Create);
    let library_root = root.path().join("library");
    fs::create_dir_all(library_root.join("entries")).unwrap();
    fs::write(library_root.join("entries/x.multicad"), b"old copy").unwrap();
    let old = json!({
        "application": "multicad.library",
        "version": 1,
        "entries": [
            {"id": EntityId::new().to_string(), "document": part.document.id().to_string(),
             "name": "Old listing", "description": "", "file": "entries/x.multicad",
             "source_path": path.to_string_lossy(), "content": "00".repeat(32),
             "registered_unix_ms": 1},
            {"id": EntityId::new().to_string(), "document": EntityId::new().to_string(),
             "name": "No source", "description": "", "file": "entries/y.multicad",
             "source_path": null, "content": "00".repeat(32), "registered_unix_ms": 1}
        ]
    });
    fs::write(library_root.join("library.json"), old.to_string()).unwrap();
    let mut library = LibraryFileStore::new(library_root.clone());
    let entries = library.load().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, path);
    assert!(entries[0].problem.is_none());
    assert!(
        library_root.join("entries/x.multicad").exists(),
        "old copies are left untouched"
    );
    fs::write(library_root.join("library.json"), b"{broken").unwrap();
    assert!(library.load().unwrap_err().contains("damaged"));
}

#[test]
fn cards_and_preview_files() {
    let root = tempfile::tempdir().unwrap();
    let mut projects = ProjectStore::new(root.path().join("recovery"));
    let path = root.path().join("card.multicad");
    let part = project("Card part", "Shown on the home screen");
    save(&mut projects, &path, &part, SaveMode::Create);
    let mut library = LibraryFileStore::new(root.path().join("library"));
    let card = library.card(&path).unwrap();
    assert_eq!(card.info.name, "Card part");
    assert_eq!(card.info.preview.unwrap().png(), ICON);
    assert!(library.card(&root.path().join("gone.multicad")).is_err());

    let png = root.path().join("shot.png");
    fs::write(&png, ICON).unwrap();
    assert_eq!(library.read_preview(&png).unwrap().width(), 256);
    let text = root.path().join("notes.png");
    fs::write(&text, b"hello").unwrap();
    assert!(
        library
            .read_preview(&text)
            .unwrap_err()
            .contains("not a PNG")
    );
    let large = root.path().join("large.png");
    fs::write(&large, vec![0u8; info::MAX_PREVIEW_BYTES + 1]).unwrap();
    assert!(library.read_preview(&large).unwrap_err().contains("KiB"));
}

#[test]
fn default_library_lives_beside_user_recovery_data_not_in_the_bundle() {
    let store = LibraryFileStore::default();
    let recovery = ProjectStore::default().recovery_directory;
    assert_eq!(store.root, recovery.parent().unwrap().join("library"));
    let executable = std::env::current_exe().unwrap();
    assert!(!store.root.starts_with(executable.parent().unwrap()));
}
