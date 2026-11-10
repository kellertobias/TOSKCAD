//! Schedules as drawing tables and CSV files: a placed table prints in PDF
//! and drawing DXF and blocks them once stale, an export-set CSV recipe and
//! the direct CSV export write exactly `Schedule::csv`, and a changed source
//! writes nothing.
use ::dxf::{Drawing, entities::EntityType};
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId},
    drawings::*,
    exchange::*,
    model::{self, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, length},
    schedules::{ScheduleKind, ScheduleSpec},
};
use multicad_model::{
    Compiler,
    export::{Exporter, ScheduleTarget},
    schedules,
};
use std::path::Path;

fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn add_box(s: &mut Session, name: &str, x: f64) {
    let request = PrimitiveRequest::new(
        name,
        PrimitiveParameters::Box {
            size: [length(20.), length(10.), length(5.)],
            frame: PrimitiveFrame {
                origin: [length(x), length(0.), length(0.)],
                ..Default::default()
            },
        },
    );
    let p = model::prepare_primitive(s, s.revision(), actor(), &request).unwrap();
    s.commit(p.prepared).unwrap();
}
fn bom() -> ScheduleSpec {
    ScheduleSpec::new(ScheduleKind::Bom)
}

#[test]
fn rust_worker_publishes_schedule_csv_in_an_export_set_and_refuses_a_stale_direct_write() {
    use multicad_core::project_file::ContentHash;
    use multicad_model::{GeometryBackend, export::ExportWorker};
    use std::time::{Duration, Instant};

    let mut s = Session::new(Document::new("Rust schedule outputs").unwrap()).unwrap();
    add_box(&mut s, "Leg", 0.);
    add_box(&mut s, "Top", 40.);
    let set = EntityId::new();
    let definition = SetDefinition {
        recipes: vec![Recipe {
            id: EntityId::new(),
            name: "BOM".into(),
            output: RecipeOutput::ScheduleCsv { schedule: bom() },
        }],
        ..Default::default()
    };
    s.commit(prepare_set(&s, s.revision(), actor(), set, "Shop", &definition).unwrap())
        .unwrap();
    let worker =
        ExportWorker::with_geometry_backend(Exporter::default(), GeometryBackend::Rust).unwrap();
    let wait = |id| {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(reply) = worker.poll() {
                assert_eq!(reply.id, id);
                break reply.result;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
    };
    let ExportResult::Schedule(schedule) = wait(
        worker
            .schedule(s.document(), s.revision(), bom(), None, actor())
            .unwrap(),
    ) else {
        panic!("expected the direct schedule preview");
    };
    let schedule = schedule.unwrap();
    let ExportResult::Preview(preview) = wait(
        worker
            .preview(s.document(), s.revision(), set, actor())
            .unwrap(),
    ) else {
        panic!("expected the export set preview");
    };
    let preview = preview.unwrap();
    assert!(preview.can_run());
    let [artifact] = preview.artifacts.as_slice() else {
        panic!("expected one schedule file");
    };
    assert_eq!(
        artifact.schedule.as_ref().unwrap().csv_hash,
        schedule.csv_hash
    );
    let root = tempfile::tempdir().unwrap();
    let ExportResult::Run(receipt) = wait(
        worker
            .run(
                s.document(),
                s.revision(),
                &preview.key,
                root.path(),
                actor(),
            )
            .unwrap(),
    ) else {
        panic!("expected the export set publication");
    };
    let receipt = receipt.unwrap();
    let path = Path::new(&receipt.directory).join(&artifact.output.filename);
    let csv = std::fs::read_to_string(path).unwrap();
    assert_eq!(csv, schedule.csv);
    assert_eq!(ContentHash::of(csv.as_bytes()), schedule.csv_hash);
    assert!(csv.contains("\n1,Leg,1,") && csv.contains("\n2,Top,1,"));

    let direct = root.path().join("existing.csv");
    std::fs::write(&direct, b"keep me").unwrap();
    add_box(&mut s, "Shelf", 80.);
    let ExportResult::Schedule(stale) = wait(
        worker
            .schedule(
                s.document(),
                s.revision(),
                bom(),
                Some((&direct, schedule.csv_hash)),
                actor(),
            )
            .unwrap(),
    ) else {
        panic!("expected the direct write refusal");
    };
    assert_eq!(stale.unwrap_err().kind, ErrorKind::Stale);
    assert_eq!(std::fs::read(direct).unwrap(), b"keep me");
}
fn fresh(s: &Session, spec: ScheduleSpec) -> schedules::Schedule {
    let m = Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap();
    schedules::build(&m, s.document(), s.revision(), spec).unwrap()
}

struct Fixture {
    s: Session,
    book: EntityId,
    table: EntityId,
    set: EntityId,
}
impl Fixture {
    fn new() -> Self {
        let mut s = Session::new(Document::new("Schedule outputs").unwrap()).unwrap();
        add_box(&mut s, "Leg", 0.);
        add_box(&mut s, "Top", 40.);
        let mut page = Page::new("S-1");
        let table = EntityId::new();
        page.annotations.push(Annotation {
            id: table,
            kind: fresh(&s, bom()).placed_table(bom(), [20., 30.]),
        });
        let (book, p) = prepare_create(&s, s.revision(), actor(), "Sheets", Some(page)).unwrap();
        s.commit(p).unwrap();
        let set = EntityId::new();
        let recipe = |name: &str, output| Recipe {
            id: EntityId::new(),
            name: name.into(),
            output,
        };
        let definition = SetDefinition {
            scope: Scope::default(),
            pages: Pages::Project,
            recipes: vec![
                recipe("BOM", RecipeOutput::ScheduleCsv { schedule: bom() }),
                recipe(
                    "Machining",
                    RecipeOutput::ScheduleCsv {
                        schedule: ScheduleSpec::new(ScheduleKind::Machining),
                    },
                ),
                recipe("PDF", RecipeOutput::Pdf),
                recipe(
                    "DXF",
                    RecipeOutput::DrawingDxf {
                        units: LengthUnit::Millimeter,
                        layers: Layers::default(),
                    },
                ),
            ],
            ..Default::default()
        };
        s.commit(prepare_set(&s, s.revision(), actor(), set, "Shop", &definition).unwrap())
            .unwrap();
        Self {
            s,
            book,
            table,
            set,
        }
    }
    fn preview(&self) -> ExportPreview {
        Exporter::default()
            .preview(
                self.s.document(),
                self.s.revision(),
                self.set,
                &actor(),
                None,
            )
            .unwrap()
    }
    fn run(&self, preview: &ExportPreview, root: &Path) -> Result<RunReceipt, ExportError> {
        Exporter::default().run(
            self.s.document(),
            self.s.revision(),
            &preview.key,
            root,
            &actor(),
            None,
        )
    }
}
fn reasons(preview: &ExportPreview) -> Vec<String> {
    preview
        .recipes
        .iter()
        .filter_map(|r| match &r.status {
            RecipeStatus::Unsupported(reason) => Some(reason.clone()),
            _ => None,
        })
        .collect()
}
fn texts(dxf: &Path) -> Vec<String> {
    Drawing::load_file(dxf.to_str().unwrap())
        .unwrap()
        .entities()
        .filter_map(|e| match &e.specific {
            EntityType::Text(t) => Some(t.value.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn csv_recipes_write_exactly_the_schedule_csv_and_reuse_the_verified_bundle() {
    let f = Fixture::new();
    let preview = f.preview();
    assert!(preview.can_run(), "{:?}", preview.recipes);
    let csv: Vec<_> = preview
        .artifacts
        .iter()
        .filter(|a| a.schedule.is_some())
        .collect();
    assert_eq!(csv.len(), 2);
    assert!(csv[0].output.filename.ends_with(".csv"));
    assert!(
        csv[0].output.filename.contains("bom"),
        "{}",
        csv[0].output.filename
    );
    let evidence = csv[0].schedule.as_ref().unwrap();
    assert_eq!((evidence.rows, evidence.columns), (2, 7));
    assert_eq!(evidence.revision, f.s.revision());
    let root = tempfile::tempdir().unwrap();
    let receipt = f.run(&preview, root.path()).unwrap();
    assert!(!receipt.reused);
    let written =
        std::fs::read_to_string(Path::new(&receipt.directory).join(&csv[0].output.filename))
            .unwrap();
    let expected = fresh(&f.s, bom());
    assert_eq!(written, expected.csv());
    let (_, records) = schedules::read_csv(&written).unwrap();
    assert_eq!(records[1][1], "Leg");
    assert_eq!(records[2][1], "Top");
    let machining =
        std::fs::read_to_string(Path::new(&receipt.directory).join(&csv[1].output.filename))
            .unwrap();
    assert_eq!(
        machining,
        fresh(&f.s, ScheduleSpec::new(ScheduleKind::Machining)).csv()
    );
    // The same source reuses the bundle after reading every file back.
    assert!(f.run(&preview, root.path()).unwrap().reused);
}

#[test]
fn placed_schedule_tables_print_in_pdf_and_dxf_and_block_both_once_stale() {
    let mut f = Fixture::new();
    let preview = f.preview();
    let root = tempfile::tempdir().unwrap();
    let receipt = f.run(&preview, root.path()).unwrap();
    let file = |ext: &str| {
        receipt
            .files
            .iter()
            .find(|w| w.artifact.output.filename.ends_with(ext))
            .map(|w| Path::new(&receipt.directory).join(&w.artifact.output.filename))
            .unwrap()
    };
    let pdf = String::from_utf8_lossy(&std::fs::read(file(".pdf")).unwrap()).into_owned();
    for cell in ["(Leg)", "(Top)", "(Material)", "(manufactured)"] {
        assert!(pdf.contains(cell), "PDF lacks {cell}");
    }
    let dxf = texts(&file(".dxf"));
    for cell in ["Leg", "Top", "Qty", "Supply"] {
        assert!(dxf.iter().any(|t| t == cell), "DXF lacks {cell}: {dxf:?}");
    }
    assert!(
        dxf.iter().any(|t| t.starts_with("Bill of materials (rev ")),
        "{dxf:?}"
    );

    // A new Part changes the BOM: the placed table is stale and both drawing
    // formats refuse it; the CSV recipe exports the new rows.
    add_box(&mut f.s, "Shelf", 80.);
    let blocked = f.preview();
    assert!(!blocked.can_run());
    let why = reasons(&blocked);
    assert_eq!(why.len(), 2, "{why:?}");
    assert!(
        why.iter().all(|r| r.contains("Page S-1 cannot export")
            && r.contains("stale")
            && r.contains("refresh")),
        "{why:?}"
    );
    let csv = blocked
        .artifacts
        .iter()
        .find_map(|a| a.schedule.as_ref().filter(|e| e.spec == bom()))
        .unwrap();
    assert_eq!(csv.rows, 3);

    // Refreshing the table's cells in place makes the page current again.
    let mut book = multicad_core::drawings::read(f.s.document(), f.book).unwrap();
    let annotation = book.pages[0]
        .annotations
        .iter_mut()
        .find(|a| a.id == f.table)
        .unwrap();
    annotation.kind = fresh(&f.s, bom()).placed_table(bom(), [20., 30.]);
    f.s.commit(
        prepare(
            &f.s,
            f.s.revision(),
            actor(),
            f.book,
            PageAction::Replace(book.pages[0].clone()),
        )
        .unwrap(),
    )
    .unwrap();
    let refreshed = f.preview();
    assert!(refreshed.can_run(), "{:?}", refreshed.recipes);
}

#[test]
fn direct_csv_export_writes_only_the_previewed_schedule() {
    let mut f = Fixture::new();
    let exporter = Exporter::default();
    let (document, revision) = (f.s.document().clone(), f.s.revision());
    let preview = exporter
        .schedule(&document, revision, bom(), None, None)
        .unwrap();
    assert_eq!(preview.csv, fresh(&f.s, bom()).csv());
    assert_eq!(preview.source.revision, revision);
    assert_eq!(preview.rows.len(), 2);
    assert!(preview.written.is_none());

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bom.csv");
    let receipt = exporter
        .schedule(
            &document,
            revision,
            bom(),
            Some(ScheduleTarget {
                path: &path,
                expected: preview.csv_hash,
            }),
            None,
        )
        .unwrap();
    assert_eq!(receipt.written.as_deref(), path.to_str());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), preview.csv);

    // After an edit the accepted CSV no longer matches: nothing is written.
    add_box(&mut f.s, "Shelf", 80.);
    let other = dir.path().join("stale.csv");
    let stale = exporter
        .schedule(
            f.s.document(),
            f.s.revision(),
            bom(),
            Some(ScheduleTarget {
                path: &other,
                expected: preview.csv_hash,
            }),
            None,
        )
        .unwrap_err();
    assert_eq!(stale.kind, ErrorKind::Stale);
    assert!(stale.message.contains("nothing was written"), "{stale}");
    assert!(!other.exists());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), preview.csv);

    // Invalid targets are refused before any work.
    for bad in [
        Path::new("relative.csv").to_path_buf(),
        dir.path().join("bom.txt"),
        dir.path().join("missing").join("bom.csv"),
    ] {
        let error = exporter
            .schedule(
                &document,
                revision,
                bom(),
                Some(ScheduleTarget {
                    path: &bad,
                    expected: preview.csv_hash,
                }),
                None,
            )
            .unwrap_err();
        assert!(
            matches!(error.kind, ErrorKind::Invalid | ErrorKind::Io),
            "{error}"
        );
    }
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}
