//! The shipped apartment demo (CAD-185), `examples/apartment.tcd`: it opens
//! through the native project store with its name, description and
//! preview; four named rooms are bounded by editable walls with door and
//! window openings over one floor slab; the workshop holds a pinned
//! reference to the shipped workbench with a welded frame that keeps its source, revision
//! and scale; the drawing book shows a dimensioned plan with room names, a
//! fitted 3D view and a workshop sheet; a copy saved elsewhere still
//! resolves the reference through the installed examples, a missing source
//! is reported without losing the pinned content; walls and the reference
//! stay editable through save and reopen; and the generator still builds
//! the same project.
#[path = "../examples/apartment_demo.rs"]
#[allow(dead_code)]
mod demo;
use demo::{PAGES, ROOMS, actor};
use multicad_core::{
    commands::{Request, Session},
    document::{Document, EntityData, EntityId, Mutation, Placement},
    drawings::{self, AnnotationEvaluation, AnnotationKind, Scale, ViewEvaluation, pdf, render},
    model::{self, OpeningKind},
    project_file::{self, ProjectFile},
    symbols::{self, ProjectSourceState},
};
use multicad_model::{
    Compiler, Model,
    project_references::{
        check_references, installed_example_dirs, open_project, referenced_files,
    },
};
use std::path::{Path, PathBuf};

fn shipped() -> PathBuf {
    demo::repository().join(demo::SHIPPED_PATH)
}
fn examples() -> PathBuf {
    demo::repository().join("examples")
}
fn compile(session: &Session) -> Model {
    Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap()
}
fn write(document: &Document, path: &Path) {
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(document.clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    std::fs::write(path, bytes).unwrap();
}
/// The outer Instance of the reference whose source path is `path`.
fn reference(document: &Document, path: &str) -> EntityId {
    let (_, record) = symbols::project_sources(document)
        .unwrap()
        .into_iter()
        .find(|(_, s)| s.path == path)
        .unwrap_or_else(|| panic!("no reference to {path}"));
    symbols::instances(document, record.definition).unwrap()[0]
        .1
        .instance
}
fn workbench(document: &Document) -> EntityId {
    reference(document, "welded-workbench.tcd")
}
const SOURCES: [&str; 2] = ["cabinet.tcd", "welded-workbench.tcd"];
/// World bounds and leaf count of everything below an outer Instance.
fn bounds(model: &Model, instance: EntityId) -> ([f64; 3], [f64; 3], usize) {
    let mut lo = [f64::MAX; 3];
    let mut hi = [f64::MIN; 3];
    let mut count = 0;
    for (path, leaf) in &model.snapshot().leaf_occurrences {
        if path.root == instance {
            count += 1;
            for i in 0..3 {
                lo[i] = lo[i].min(leaf.bounds.min[i]);
                hi[i] = hi[i].max(leaf.bounds.max[i]);
            }
        }
    }
    (lo, hi, count)
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1., "{a} != {b}");
}
/// The workbench stands in the workshop at its documented place and size.
fn check_workbench(session: &Session) {
    let model = compile(session);
    let (lo, hi, count) = bounds(&model, workbench(session.document()));
    assert_eq!(count, 32, "every bench Part is a referenced leaf");
    let [x, y, _] = demo::WORKBENCH_PLACEMENT.translation_mm;
    near(lo[0], x);
    near(lo[1], y - 205.);
    near(hi[0] - lo[0], 1500.);
    near(hi[1] - lo[1], 1205.);
    near(hi[2] - lo[2], 900.);
    let shop = ROOMS[3].1;
    assert!(lo[0] >= shop[0] && hi[0] <= shop[2] && lo[1] >= shop[1] && hi[1] <= shop[3]);
    // The cabinet: 800 wide, 500 deep plus 18 mm doors, 2000 high, its back
    // on the right wall's inner face, all 64 leaves of the shipped cabinet.
    let (lo, hi, count) = bounds(&model, reference(session.document(), "cabinet.tcd"));
    assert_eq!(count, 64);
    near(hi[0], 9_850.);
    near(lo[0], 9_332.);
    near(lo[1], 5_400.);
    near(hi[1], 6_200.);
    near(hi[2] - lo[2], 2000.);
    assert!(lo[0] >= shop[0] && hi[0] <= shop[2] && lo[1] >= shop[1] && hi[1] <= shop[3]);
}

#[test]
fn shipped_file_opens_with_rooms_walls_openings_and_both_workshop_references() {
    let bytes = std::fs::read(shipped()).unwrap();
    // No developer path is stored anywhere in the file.
    let text = String::from_utf8_lossy(&bytes);
    let root = std::fs::canonicalize(demo::repository()).unwrap();
    assert!(!text.contains(&*root.to_string_lossy()));
    assert!(!text.contains("/Users/") && !text.contains(":\\\\"));
    let document = open_project(&shipped()).unwrap();
    let info = project_file::info::read(&document).unwrap();
    assert_eq!(info.name, demo::DOCUMENT_NAME);
    assert_eq!(info.description, demo::DESCRIPTION);
    assert!(info.preview.is_some());
    let card = project_file::card(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(card.info.name, demo::DOCUMENT_NAME);

    // Four walls, one slab; one entrance and three interior doors, six
    // windows, all on the one level.
    let walls: Vec<_> = model::walls(&document).collect();
    assert_eq!(walls.len(), 4);
    let mut doors = 0;
    let mut windows = 0;
    for wall in &walls {
        let source = model::read_wall(&document, wall.id).unwrap();
        for opening in &source.openings {
            match model::read_opening_type(&document, opening.opening_type)
                .unwrap()
                .kind
            {
                OpeningKind::Door { .. } => doors += 1,
                OpeningKind::Window => windows += 1,
            }
        }
    }
    assert_eq!((doors, windows), (4, 6));
    assert_eq!(model::slabs(&document).count(), 1);

    // The workshop references: one pinned definition per shipped example,
    // recorded against its file, found next to the apartment and current.
    let records = symbols::project_sources(&document).unwrap();
    assert_eq!(records.len(), 2);
    for (path, name) in [
        ("cabinet.tcd", None),
        (
            "welded-workbench.tcd",
            Some("Workbench with a welded frame"),
        ),
    ] {
        let source = open_project(&examples().join(path)).unwrap();
        let (_, record) = records.iter().find(|(_, r)| r.path == path).unwrap();
        assert_eq!(record.document, source.id());
        let definition = symbols::read_definition(&document, record.definition).unwrap();
        assert_eq!(definition.name, name.unwrap_or(source.name()));
        assert!(definition.revision.starts_with("source "));
    }
    let checks = check_references(&document, Some(&shipped()), &[]).unwrap();
    assert_eq!(checks.len(), 2);
    for check in &checks {
        assert_eq!(
            check.state,
            ProjectSourceState::Current,
            "{}",
            check.summary()
        );
        assert!(check.summary().contains(&check.revision));
    }
    let session = Session::new(document).unwrap();
    check_workbench(&session);

    // Drawings: three fitted A3 sheets that evaluate; the plan at 1:50 with
    // its dimensions, room names and areas; all with the title block.
    let model = compile(&session);
    let document = session.document();
    let book = drawings::books(document).next().unwrap();
    let source = drawings::read(document, book).unwrap();
    let numbers: Vec<_> = source.pages.iter().map(|p| p.number.as_str()).collect();
    assert_eq!(numbers, PAGES);
    let plan = &source.pages[0];
    assert_eq!(
        plan.views[0].scale,
        Scale {
            numerator: 1,
            denominator: 50
        }
    );
    let evaluation = model
        .drawings(document, session.revision(), &Default::default())
        .unwrap();
    for page in &source.pages {
        for view in &page.views {
            assert!(
                matches!(
                    evaluation.views.get(&view.id),
                    Some(ViewEvaluation::Ready { .. })
                ),
                "{} {}",
                page.number,
                view.name
            );
        }
        for annotation in &page.annotations {
            if matches!(annotation.kind, AnnotationKind::Dimension { .. }) {
                assert!(matches!(
                    evaluation.annotations.get(&annotation.id),
                    Some(AnnotationEvaluation::Ready { .. })
                ));
            }
        }
    }
    let dimensions = plan
        .annotations
        .iter()
        .filter(|a| matches!(a.kind, AnnotationKind::Dimension { .. }))
        .count();
    assert_eq!(dimensions, 8);
    let renders: Vec<_> = source
        .pages
        .iter()
        .map(|p| {
            render::render_page(document, session.revision(), book, p.id, &evaluation).unwrap()
        })
        .collect();
    let pdf = pdf::render_pdf(&renders, demo::DOCUMENT_NAME);
    let pdf_text = String::from_utf8_lossy(&pdf);
    for (name, _) in ROOMS {
        assert!(pdf_text.contains(&name.to_uppercase()), "{name}");
    }
    for needle in [
        "Floor plan",
        "AP-101",
        "1:50",
        "welded-workbench.tcd",
        "cabinet.tcd",
    ] {
        assert!(pdf_text.contains(needle), "{needle}");
    }
    // Optional visual evidence: MULTICAD_DRAWING_REVIEW=<dir> keeps the PDF.
    if let Some(dir) = std::env::var_os("MULTICAD_DRAWING_REVIEW") {
        std::fs::write(Path::new(&dir).join("apartment.pdf"), &pdf).unwrap();
        if let Some(preview) = &info.preview {
            std::fs::write(Path::new(&dir).join("apartment-preview.png"), preview.png()).unwrap();
        }
    }
}

#[test]
fn the_examples_gallery_copies_the_referenced_examples_next_to_the_apartment() {
    use multicad_ui::examples;
    let catalog = examples::load(&examples()).unwrap();
    let index = catalog
        .entries
        .iter()
        .position(|e| e.file == "apartment.tcd")
        .expect("the apartment is listed in examples.json");
    assert_eq!(
        referenced_files(&shipped()).unwrap(),
        SOURCES.map(PathBuf::from).to_vec()
    );
    let resolver: examples::DependencyResolver =
        std::sync::Arc::new(|path: &Path| referenced_files(path));
    let mut dependencies = catalog.dependencies(index, Some(&resolver)).unwrap();
    dependencies.sort();
    // The cabinet brings its own hardware library along.
    assert_eq!(
        dependencies,
        [
            "cabinet.tcd",
            "libraries/cabinet-hardware-library.tcd",
            "welded-workbench.tcd"
        ]
    );
    let directory = tempfile::tempdir().unwrap();
    let copies = directory.path().join("ToskCAD Examples");
    let copy = examples::instantiate(&catalog, index, &copies, Some(&resolver)).unwrap();
    assert_eq!(copy.copied_dependencies.len(), 3);
    let document = open_project(&copy.path).unwrap();
    for (check, file) in check_references(&document, Some(&copy.path), &[])
        .unwrap()
        .iter()
        .zip(sorted_paths(&document))
    {
        assert_eq!(
            check.state,
            ProjectSourceState::Current,
            "{}",
            check.summary()
        );
        assert_eq!(check.found.as_deref(), Some(copies.join(&file).as_path()));
    }
}
/// Reference paths in the order `check_references` reports them.
fn sorted_paths(document: &Document) -> Vec<String> {
    symbols::project_sources(document)
        .unwrap()
        .into_iter()
        .map(|(_, s)| s.path)
        .collect()
}

#[test]
fn a_copy_elsewhere_resolves_through_installed_examples_and_a_missing_source_is_explicit() {
    let document = open_project(&shipped()).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let copy = directory.path().join("Documents").join("My apartment.tcd");
    std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
    write(&document, &copy);
    let reopened = open_project(&copy).unwrap();
    assert_eq!(reopened, document);

    // Nothing next to the copy: without bundled examples the source is
    // missing, said explicitly, and the pinned bench is still there.
    let checks = check_references(&reopened, Some(&copy), &[]).unwrap();
    assert_eq!(checks.len(), 2);
    for check in &checks {
        assert_eq!(check.state, ProjectSourceState::Missing);
        assert!(check.summary().contains("pinned revision stays in use"));
    }
    check_workbench(&Session::new(reopened.clone()).unwrap());

    // A clean installation: the examples bundled in the app's resources
    // folder, found from the executable's location.
    let install = directory.path().join("ToskCAD");
    let resources = install.join("resources/examples");
    std::fs::create_dir_all(&resources).unwrap();
    for file in SOURCES {
        std::fs::copy(examples().join(file), resources.join(file)).unwrap();
    }
    let bundled: Vec<_> = installed_example_dirs(&install.join("multicad"))
        .into_iter()
        .filter(|d| d.is_dir())
        .collect();
    assert_eq!(bundled.len(), 1);
    let checks = check_references(&reopened, Some(&copy), &bundled).unwrap();
    for (check, file) in checks.iter().zip(sorted_paths(&reopened)) {
        assert_eq!(
            check.state,
            ProjectSourceState::Current,
            "{}",
            check.summary()
        );
        assert_eq!(
            check.found.as_deref(),
            Some(resources.join(&file).as_path())
        );
    }
}

#[test]
fn walls_and_the_reference_stay_editable_through_save_and_reopen() {
    let mut session = Session::new(open_project(&shipped()).unwrap()).unwrap();
    // Move the entrance door 200 mm along the front wall: a native-checked
    // wall edit.
    let exterior = model::walls(session.document())
        .find(|w| model::read_wall(session.document(), w.id).unwrap().closed)
        .map(|w| w.id)
        .unwrap();
    let mut wall = model::read_wall(session.document(), exterior).unwrap();
    let entrance = wall
        .openings
        .iter_mut()
        .find(|o| o.segment == 0 && o.offset == model::length(800.))
        .unwrap();
    entrance.offset = model::length(1000.);
    let prepared =
        model::prepare_wall_edit(&session, session.revision(), actor(), exterior, &wall).unwrap();
    demo::checked(&mut session, prepared.prepared).unwrap();

    // Move the referenced workbench 300 mm along the back wall.
    let bench = workbench(session.document());
    let mut instance = session.document().get(bench).unwrap().clone();
    let EntityData::Instance { placement, .. } = &mut instance.data else {
        panic!("not an Instance")
    };
    *placement = Placement {
        translation_mm: [5_450., 6_800., 0.],
        ..*placement
    };
    let prepared = session
        .prepare(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Move workbench".into(),
            edits: vec![Mutation::Replace(instance)],
        })
        .unwrap();
    demo::checked(&mut session, prepared).unwrap();
    let (lo, _, count) = bounds(&compile(&session), bench);
    assert_eq!(count, 32);
    near(lo[0], 5_450.);

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("apartment-edited.tcd");
    write(session.document(), &path);
    let reopened = open_project(&path).unwrap();
    assert_eq!(&reopened, session.document());
    let wall = model::read_wall(&reopened, exterior).unwrap();
    assert!(
        wall.openings
            .iter()
            .any(|o| o.offset == model::length(1000.))
    );
    // Undo returns to the shipped source.
    session.undo(session.revision(), actor()).unwrap();
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &open_project(&shipped()).unwrap());
}

#[test]
fn generator_rebuilds_the_shipped_project() {
    let mut built = demo::build(&shipped()).unwrap();
    demo::finish(&mut built).unwrap();
    let fresh = built.session.document();
    let shipped = open_project(&shipped()).unwrap();
    // Identities are new on every run, except the reference content, which
    // is derived from the workbench source: names, wall sources, the pinned
    // revision, drawings and file information must match.
    let names = |d: &Document| {
        let mut names: Vec<_> = d
            .entities()
            .filter(|e| !matches!(e.data, EntityData::Parameter { .. }))
            .map(|e| e.name.clone())
            .collect();
        names.sort();
        names
    };
    assert_eq!(names(fresh), names(&shipped));
    let walls = |d: &Document| {
        let mut all: Vec<_> = model::walls(d)
            .map(|w| {
                let mut source = model::read_wall(d, w.id).unwrap();
                source.output.body = EntityId::nil();
                source.wall_type = EntityId::nil();
                source.level = None;
                for o in &mut source.openings {
                    o.id = EntityId::nil();
                    o.opening_type = EntityId::nil();
                }
                (w.name.clone(), format!("{source:?}"))
            })
            .collect();
        all.sort();
        all
    };
    assert_eq!(walls(fresh), walls(&shipped));
    let pins = |d: &Document| {
        symbols::definitions(d)
            .unwrap()
            .into_iter()
            .map(|(_, def)| (def.name.clone(), def.revision.clone(), def.pin().unwrap()))
            .collect::<std::collections::BTreeSet<_>>()
    };
    assert_eq!(pins(fresh), pins(&shipped));
    let paths = |d: &Document| {
        symbols::project_sources(d)
            .unwrap()
            .into_iter()
            .map(|(_, s)| (s.path, s.document, s.source))
            .collect::<std::collections::BTreeSet<_>>()
    };
    assert_eq!(paths(fresh), paths(&shipped));
    let pages = |d: &Document| {
        let book = drawings::books(d).next().unwrap();
        drawings::read(d, book)
            .unwrap()
            .pages
            .into_iter()
            .map(|p| {
                let notes: Vec<_> = p
                    .annotations
                    .iter()
                    .filter_map(|a| match &a.kind {
                        AnnotationKind::Note { text, position_mm } => {
                            Some(format!("{text} @ {position_mm:?}"))
                        }
                        _ => None,
                    })
                    .collect();
                (
                    p.number,
                    p.info.title,
                    p.views
                        .iter()
                        .map(|v| (v.name.clone(), v.scale))
                        .collect::<Vec<_>>(),
                    notes,
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(pages(fresh), pages(&shipped));
    let (a, b) = (
        project_file::info::read(fresh).unwrap(),
        project_file::info::read(&shipped).unwrap(),
    );
    assert_eq!((a.name, a.description), (b.name, b.description));
    let size = |p: Option<project_file::info::PreviewImage>| p.map(|p| (p.width(), p.height()));
    assert_eq!(size(a.preview), size(b.preview));
}
