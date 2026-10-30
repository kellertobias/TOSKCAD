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
    };
    let document = document
        .edited(&info::edits(&document, &wanted).unwrap())
        .unwrap();
    ProjectFile::new(document).unwrap()
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

#[test]
fn explicit_registration_persists_across_restart_and_survives_source_moves() {
    let root = tempfile::tempdir().unwrap();
    let mut projects = ProjectStore::new(root.path().join("recovery"));
    let path = root.path().join("bracket.multicad");
    let mut bracket = project("Wall bracket", "Holds a shelf");
    let stamp = save(&mut projects, &path, &bracket, SaveMode::Create);
    let library_root = root.path().join("library");
    let mut library = LibraryFileStore::new(library_root.clone());
    assert!(
        library.load().unwrap().is_empty(),
        "saving never registers a file"
    );

    let (receipt, entries) = library.register(&request(&path, stamp, &bracket)).unwrap();
    assert_eq!(receipt.outcome, Registration::Added);
    assert!(receipt.message().contains("Added “Wall bracket”"));
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(entry.name, "Wall bracket");
    assert_eq!(entry.description, "Holds a shelf");
    assert_eq!(entry.preview.as_ref().unwrap().png(), ICON);
    assert!(entry.snapshot.starts_with(&library_root));
    assert!(entry.problem.is_none());

    // Same saved content again: a defined, visible no-op, not a duplicate.
    let (again, entries) = library.register(&request(&path, stamp, &bracket)).unwrap();
    assert_eq!(again.outcome, Registration::Unchanged);
    assert!(again.message().contains("nothing changed"));
    assert_eq!(entries.len(), 1);

    // Edit and save: re-registering updates the one entry for this document.
    let renamed = FileInfo {
        name: "Wall bracket v2".into(),
        description: "Holds a deeper shelf".into(),
        preview: None,
    };
    bracket = ProjectFile::new(
        bracket
            .document
            .edited(&info::edits(&bracket.document, &renamed).unwrap())
            .unwrap(),
    )
    .unwrap();
    let stamp = save(&mut projects, &path, &bracket, SaveMode::Replace(stamp));
    let (updated, entries) = library.register(&request(&path, stamp, &bracket)).unwrap();
    assert_eq!(updated.outcome, Registration::Updated);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, entry.id, "same entry identity");
    assert_eq!(entries[0].name, "Wall bracket v2");
    assert!(entries[0].preview.is_none());

    // The original disappears: the managed copy keeps the entry usable, and a
    // new store instance (application restart / another project) sees it.
    fs::remove_file(&path).unwrap();
    let entries = LibraryFileStore::new(library_root.clone()).load().unwrap();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].problem.is_none());
    let opened = projects.open_project(&entries[0].snapshot).unwrap();
    assert_eq!(opened.file.document.name(), "Wall bracket v2");
    assert_eq!(
        entries[0]
            .source_path
            .as_ref()
            .unwrap()
            .file_name()
            .unwrap(),
        "bracket.multicad"
    );
}

#[test]
fn a_moved_save_as_copy_with_the_same_document_reports_its_previous_source() {
    let root = tempfile::tempdir().unwrap();
    let mut projects = ProjectStore::new(root.path().join("recovery"));
    let first = root.path().join("a.multicad");
    let second = root.path().join("b.multicad");
    let mut part = project("Part", "");
    let stamp = save(&mut projects, &first, &part, SaveMode::Create);
    let mut library = LibraryFileStore::new(root.path().join("library"));
    library.register(&request(&first, stamp, &part)).unwrap();
    part = ProjectFile::new(part.document.edited(&[]).unwrap()).unwrap();
    let mut info = info::read(&part.document).unwrap();
    info.description = "Changed copy".into();
    part = ProjectFile::new(
        part.document
            .edited(&info::edits(&part.document, &info).unwrap())
            .unwrap(),
    )
    .unwrap();
    let stamp = save(&mut projects, &second, &part, SaveMode::Create);
    let (receipt, entries) = library.register(&request(&second, stamp, &part)).unwrap();
    assert_eq!(receipt.outcome, Registration::Updated);
    assert_eq!(entries.len(), 1);
    assert!(
        receipt
            .previous_source
            .as_ref()
            .unwrap()
            .ends_with("a.multicad")
    );
    assert!(receipt.message().contains("previously registered from"));
}

#[test]
fn missing_changed_and_unwritable_registrations_fail_without_partial_entries() {
    let root = tempfile::tempdir().unwrap();
    let mut projects = ProjectStore::new(root.path().join("recovery"));
    let path = root.path().join("part.multicad");
    let part = project("Part", "");
    let stamp = save(&mut projects, &path, &part, SaveMode::Create);
    let mut library = LibraryFileStore::new(root.path().join("library"));

    // Changed on disk by someone else.
    let other = project("Other", "");
    let mut external = ProjectStore::new(root.path().join("recovery"));
    let _ = save(&mut external, &path, &other, SaveMode::Replace(stamp));
    let error = library.register(&request(&path, stamp, &part)).unwrap_err();
    assert!(error.contains("changed on disk"), "{error}");

    // Missing source.
    fs::remove_file(&path).unwrap();
    let error = library.register(&request(&path, stamp, &part)).unwrap_err();
    assert!(error.contains("missing"), "{error}");
    assert!(library.load().unwrap().is_empty());

    // Storage-write failure: the library root is a regular file.
    let stamp = save(&mut projects, &path, &part, SaveMode::Create);
    let blocked = root.path().join("blocked");
    fs::write(&blocked, b"not a directory").unwrap();
    let error = LibraryFileStore::new(blocked)
        .register(&request(&path, stamp, &part))
        .unwrap_err();
    assert!(error.contains("library folder"), "{error}");
}

#[test]
fn damaged_copies_and_index_are_reported_and_cards_read_file_metadata() {
    let root = tempfile::tempdir().unwrap();
    let mut projects = ProjectStore::new(root.path().join("recovery"));
    let path = root.path().join("card.multicad");
    let part = project("Card part", "Shown on the home screen");
    let stamp = save(&mut projects, &path, &part, SaveMode::Create);
    let mut library = LibraryFileStore::new(root.path().join("library"));

    let card = library.card(&path).unwrap();
    assert_eq!(card.info.name, "Card part");
    assert_eq!(card.info.description, "Shown on the home screen");
    assert_eq!(card.info.preview.unwrap().png(), ICON);
    assert!(library.card(&root.path().join("gone.multicad")).is_err());

    let (_, entries) = library.register(&request(&path, stamp, &part)).unwrap();
    fs::write(&entries[0].snapshot, b"truncated").unwrap();
    let entries = library.load().unwrap();
    assert!(
        entries[0]
            .problem
            .as_ref()
            .unwrap()
            .contains("Add to Library again")
    );
    // Re-registering the unchanged original repairs the damaged copy.
    let (receipt, entries) = library.register(&request(&path, stamp, &part)).unwrap();
    assert_eq!(receipt.outcome, Registration::Updated);
    assert!(entries[0].problem.is_none());

    fs::write(root.path().join("library/library.json"), b"{broken").unwrap();
    assert!(library.load().unwrap_err().contains("damaged"));
}

#[test]
fn preview_files_must_be_bounded_pngs() {
    let root = tempfile::tempdir().unwrap();
    let mut library = LibraryFileStore::new(root.path().join("library"));
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
    assert!(
        library
            .read_preview(&root.path().join("gone.png"))
            .unwrap_err()
            .contains("no longer exists")
    );
}

#[test]
fn default_library_lives_beside_user_recovery_data_not_in_the_bundle() {
    let store = LibraryFileStore::default();
    let recovery = ProjectStore::default().recovery_directory;
    assert_eq!(store.root, recovery.parent().unwrap().join("library"));
    let executable = std::env::current_exe().unwrap();
    assert!(!store.root.starts_with(executable.parent().unwrap()));
}
