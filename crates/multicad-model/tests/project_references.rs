//! Pinned references to other native projects (CAD-185): the shipped welded
//! workbench is placed into another project as a reference, evaluates
//! natively at its new pose, survives save/reopen and copies elsewhere, and
//! reports a changed or missing source explicitly without losing its
//! pinned content.
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, EntityData, EntityId, Mutation, Placement},
    project_file::{self, ProjectFile},
    symbols::{self, ProjectReferenceRequest, ProjectSourceState},
};
use multicad_model::{
    Compiler, Model,
    project_references::{
        check_references, installed_example_dirs, open_project, prepare_reference_file,
        prepare_reference_refresh, reference_content, stored_path,
    },
};
use std::path::{Path, PathBuf};

fn actor() -> Actor {
    Actor::local(Channel::CommandLine)
}
fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}
fn workbench() -> PathBuf {
    examples().join("welded-workbench.tcd")
}
fn compile(session: &Session) -> Model {
    Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap()
}
fn save(document: &Document, path: &Path) {
    let project = ProjectFile::new(document.clone()).unwrap();
    let mut file = std::fs::File::create(path).unwrap();
    project_file::write(&project, &mut file, Default::default()).unwrap();
}
fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "multicad-project-references-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
fn placement(x: f64, y: f64, turn_quarter: bool) -> Placement {
    let h = std::f64::consts::FRAC_1_SQRT_2;
    Placement {
        translation_mm: [x, y, 0.],
        rotation_xyzw: if turn_quarter {
            [0., 0., h, h]
        } else {
            [0., 0., 0., 1.]
        },
    }
}
/// World bounds of every physical leaf below an outer Instance.
fn leaf_bounds(model: &Model, instance: EntityId) -> ([f64; 3], [f64; 3], usize) {
    let mut lo = [f64::MAX; 3];
    let mut hi = [f64::MIN; 3];
    let mut count = 0;
    for (path, leaf) in &model.snapshot().leaf_occurrences {
        if path.root != instance {
            continue;
        }
        count += 1;
        let b = &leaf.bounds;
        for i in 0..3 {
            lo[i] = lo[i].min(b.min[i]);
            hi[i] = hi[i].max(b.max[i]);
        }
    }
    (lo, hi, count)
}

#[test]
fn the_shipped_workbench_is_placed_as_a_pinned_reference() {
    let source = open_project(&workbench()).unwrap();
    let content = reference_content(&source).unwrap();
    assert_eq!(content.name, "Welded steel workbench");
    assert!(
        content.revision.starts_with("source "),
        "{}",
        content.revision
    );
    // The same source pins the same content: authoring is deterministic.
    assert_eq!(
        reference_content(&source).unwrap().pin().unwrap(),
        content.pin().unwrap()
    );
    let parts = content
        .template
        .entities()
        .filter(|e| matches!(e.data, EntityData::Part { .. }))
        .count();
    assert_eq!(parts, 20, "every member, foot plate and the worktop");

    let dir = temp("place");
    let host_path = dir.join("Workshop.tcd");
    std::fs::copy(workbench(), dir.join("welded-workbench.tcd")).unwrap();
    let mut session = Session::new(Document::new("Workshop").unwrap()).unwrap();
    let prepared = prepare_reference_file(
        &session,
        session.revision(),
        actor(),
        Some(&host_path),
        &dir.join("welded-workbench.tcd"),
        "Workbench",
        placement(2000., 1000., true),
    )
    .unwrap();
    let instance = prepared.instances[0];
    session.commit(prepared.prepared).unwrap();
    let doc = session.document();
    let sources = symbols::project_sources(doc).unwrap();
    assert_eq!(sources.len(), 1);
    let (record, source_record) = &sources[0];
    assert_eq!(source_record.path, "welded-workbench.tcd");
    assert_eq!(source_record.document, source.id());
    // The pinned content compiles natively at its placement: 20 leaves, the
    // 1500 × 1000 bench turned a quarter, standing on the floor, 900 high.
    let model = compile(&session);
    let (lo, hi, count) = leaf_bounds(&model, instance);
    assert_eq!(count, 20);
    let size = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
    assert!((size[0] - 1000.).abs() < 1., "{size:?}");
    assert!((size[1] - 1500.).abs() < 1., "{size:?}");
    assert!((size[2] - 900.).abs() < 1., "{size:?}");
    assert!(lo[2].abs() < 1e-6, "{lo:?}");

    // A second placement reuses the one embedded definition and record.
    let source_file = dir.join("welded-workbench.tcd");
    let prepared = prepare_reference_file(
        &session,
        session.revision(),
        actor(),
        Some(&host_path),
        &source_file,
        "Workbench 2",
        placement(-2000., 0., false),
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
    assert_eq!(symbols::definitions(session.document()).unwrap().len(), 1);
    assert_eq!(
        symbols::project_sources(session.document()).unwrap().len(),
        1
    );
    assert_eq!(compile(&session).snapshot().leaf_occurrences.len(), 40);

    // Save, reopen: the reference requires its capability and still checks
    // current against the file next to it.
    save(session.document(), &host_path);
    let reopened = open_project(&host_path).unwrap();
    assert_eq!(reopened, *session.document());
    let capabilities = ProjectFile::new(reopened.clone()).unwrap().capabilities();
    assert!(capabilities.contains(symbols::PROJECT_CAPABILITY));
    let checks = check_references(&reopened, Some(&host_path), &[]).unwrap();
    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0].record, *record);
    assert_eq!(
        checks[0].state,
        ProjectSourceState::Current,
        "{}",
        checks[0].summary()
    );
    assert_eq!(checks[0].found.as_deref(), Some(source_file.as_path()));
    assert_eq!(checks[0].instances.len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_copy_elsewhere_resolves_through_bundled_examples_and_a_missing_source_keeps_the_pin() {
    let dir = temp("copy");
    let host = dir.join("project").join("Apartment.tcd");
    std::fs::create_dir_all(host.parent().unwrap()).unwrap();
    let library = dir.join("library");
    std::fs::create_dir_all(&library).unwrap();
    let source_file = library.join("welded-workbench.tcd");
    std::fs::copy(workbench(), &source_file).unwrap();
    let mut session = Session::new(Document::new("Apartment").unwrap()).unwrap();
    let prepared = prepare_reference_file(
        &session,
        session.revision(),
        actor(),
        Some(&host),
        &source_file,
        "Workbench",
        Placement::default(),
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
    let path = symbols::project_sources(session.document()).unwrap()[0]
        .1
        .path
        .clone();
    assert_eq!(path, "../library/welded-workbench.tcd");
    assert!(!path.contains(&*dir.to_string_lossy()), "no absolute path");
    save(session.document(), &host);

    // A user copy in another folder: the relative path no longer leads to
    // the source, the bundled examples folder does.
    let elsewhere = dir.join("elsewhere").join("copies");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let copy = elsewhere.join("My apartment.tcd");
    std::fs::copy(&host, &copy).unwrap();
    let reopened = open_project(&copy).unwrap();
    let check = &check_references(&reopened, Some(&copy), &[examples()]).unwrap()[0];
    assert_eq!(
        check.state,
        ProjectSourceState::Current,
        "{}",
        check.summary()
    );
    assert_eq!(
        check.found.as_deref(),
        Some(examples().join("welded-workbench.tcd").as_path())
    );
    assert_eq!(check.searched.len(), 3, "{:?}", check.searched);

    // Without any source the project still opens and evaluates its pinned
    // content; the check says so explicitly.
    std::fs::remove_file(&source_file).unwrap();
    let reopened = open_project(&host).unwrap();
    let check = &check_references(&reopened, Some(&host), &[]).unwrap()[0];
    assert_eq!(check.state, ProjectSourceState::Missing);
    assert!(
        check
            .summary()
            .contains("not found; the pinned revision stays in use"),
        "{}",
        check.summary()
    );
    let session = Session::new(reopened).unwrap();
    assert_eq!(compile(&session).snapshot().leaf_occurrences.len(), 20);

    // Another project at the same path is reported, not adopted.
    save(&Document::new("Something else").unwrap(), &source_file);
    let check = &check_references(session.document(), Some(&host), &[]).unwrap()[0];
    assert!(
        matches!(check.state, ProjectSourceState::Different { .. }),
        "{:?}",
        check.state
    );
    // Neither can it be adopted by an explicit refresh.
    let record = check.record;
    let refused = prepare_reference_refresh(
        &session,
        session.revision(),
        actor(),
        record,
        Some(&host),
        &source_file,
    );
    assert!(refused.is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_changed_source_is_reported_and_updated_only_explicitly() {
    let dir = temp("update");
    let host = dir.join("Shop.tcd");
    let source_file = dir.join("welded-workbench.tcd");
    std::fs::copy(workbench(), &source_file).unwrap();
    let mut session = Session::new(Document::new("Shop").unwrap()).unwrap();
    let prepared = prepare_reference_file(
        &session,
        session.revision(),
        actor(),
        Some(&host),
        &source_file,
        "Workbench",
        placement(500., 0., false),
    )
    .unwrap();
    let instance = prepared.instances[0];
    session.commit(prepared.prepared).unwrap();
    let before = symbols::definitions(session.document()).unwrap()[0]
        .1
        .clone();

    // Edit the source: a longer top.
    let mut source = Session::new(open_project(&source_file).unwrap()).unwrap();
    let length = source
        .document()
        .entities()
        .find(|e| e.name == "top_length")
        .unwrap()
        .clone();
    let mut longer = length.clone();
    if let EntityData::Parameter { expression, .. } = &mut longer.data {
        *expression = "1800".into();
    }
    source
        .apply(Request {
            expected_revision: source.revision(),
            actor: actor(),
            label: "Longer top".into(),
            edits: vec![Mutation::Replace(longer)],
        })
        .unwrap();
    save(source.document(), &source_file);

    let check = &check_references(session.document(), Some(&host), &[]).unwrap()[0];
    let ProjectSourceState::Updated { revision } = &check.state else {
        panic!("{:?}", check.state)
    };
    assert_ne!(*revision, before.revision);
    // Nothing changed by looking: the pinned bench is still 1500 long.
    let (lo, hi, _) = leaf_bounds(&compile(&session), instance);
    assert!((hi[0] - lo[0] - 1500.).abs() < 1.);

    let prepared = prepare_reference_refresh(
        &session,
        session.revision(),
        actor(),
        check.record,
        Some(&host),
        &source_file,
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
    let (lo, hi, count) = leaf_bounds(&compile(&session), instance);
    assert_eq!(count, 20);
    assert!((hi[0] - lo[0] - 1800.).abs() < 1., "{lo:?} {hi:?}");
    assert!((lo[0] - 500.).abs() < 1., "the placement is kept: {lo:?}");
    let check = &check_references(session.document(), Some(&host), &[]).unwrap()[0];
    assert_eq!(check.state, ProjectSourceState::Current);
    // Undo restores the previous pin and geometry together.
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(
        symbols::definitions(session.document()).unwrap()[0].1,
        before
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_different_revision_of_a_referenced_project_is_not_placed_silently() {
    let source = open_project(&workbench()).unwrap();
    let content = reference_content(&source).unwrap();
    let mut session = Session::new(Document::new("Two benches").unwrap()).unwrap();
    let request = ProjectReferenceRequest::new(
        content.clone(),
        &source,
        "welded-workbench.tcd",
        "Workbench",
        Placement::default(),
    )
    .unwrap();
    session
        .commit(
            symbols::prepare_project_reference(&session, 0, actor(), &request)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let mut changed = content.clone();
    changed.revision = "source changed".into();
    changed.name = "Welded steel workbench (edited)".into();
    let request = ProjectReferenceRequest::new(
        changed,
        &source,
        "welded-workbench.tcd",
        "Workbench 2",
        Placement::default(),
    )
    .unwrap();
    let error = symbols::prepare_project_reference(&session, session.revision(), actor(), &request)
        .err()
        .unwrap();
    assert!(
        error.message.contains("already references another version"),
        "{}",
        error.message
    );
    // A source record cannot claim content from another project.
    let (record, mut value) = symbols::project_sources(session.document()).unwrap()[0].clone();
    value.document = EntityId::new();
    let mut entity = session.document().get(record).unwrap().clone();
    if let EntityData::Extension { definition, .. } = &mut entity.data {
        definition.data = serde_json::to_value(&value).unwrap();
    }
    let refused = session.apply(Request {
        expected_revision: session.revision(),
        actor: actor(),
        label: "Tamper".into(),
        edits: vec![Mutation::Replace(entity)],
    });
    assert!(refused.is_err());
}

#[test]
fn reference_paths_are_portable_and_never_absolute() {
    for bad in [
        "/Users/someone/examples/welded-workbench.tcd",
        "C:/examples/welded-workbench.tcd",
        "examples\\welded-workbench.tcd",
        "examples//welded-workbench.tcd",
        "./welded-workbench.tcd",
        "welded-workbench.step",
        "",
    ] {
        assert!(symbols::validate_path(bad).is_err(), "{bad}");
    }
    for good in [
        "welded-workbench.tcd",
        "../lib/Bench.TCD",
        "a/b/old.multicad",
    ] {
        symbols::validate_path(good).unwrap();
    }
    assert_eq!(
        symbols::relative_path(Path::new("/a/b/c"), Path::new("/a/x/y.tcd")).as_deref(),
        Some("../../x/y.tcd")
    );
    let dirs = installed_example_dirs(Path::new(
        "/Applications/MultiCAD.app/Contents/MacOS/multicad",
    ));
    assert_eq!(
        dirs[0],
        Path::new("/Applications/MultiCAD.app/Contents/MacOS/../Resources/examples")
    );
    assert_eq!(
        dirs[1],
        Path::new("/Applications/MultiCAD.app/Contents/MacOS/resources/examples")
    );
    // Unsaved projects record the file name only.
    assert_eq!(
        stored_path(None, &workbench()).unwrap(),
        "welded-workbench.tcd"
    );
}
