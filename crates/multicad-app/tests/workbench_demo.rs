//! The shipped workbench with a welded frame (CAD-183), `examples/welded-workbench.tcd`:
//! it opens through the native project store with its name, description and
//! preview; the frame, welds, worktop and its 10 × 10 hole grid come from the
//! canonical model; drawings and schedules agree with it; parameter edits
//! move frame, grid and top together and an edit that pushes the grid off
//! the top is refused; Undo, Redo and save/reopen keep the source; an
//! external MCP client edits it through review; and the generator still
//! builds the same project.
#[path = "../examples/workbench_demo.rs"]
#[allow(dead_code)]
mod demo;
use demo::{GRID, PAGES, actor};
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId},
    drawings::{AnnotationEvaluation, AnnotationKind},
    model::{ConnectionKind, MachiningFace},
    parameters::{self, Unit},
    project_file::{self, ProjectFile},
};
use multicad_kernel::BooleanOperation;
use multicad_mcp::tools::{Call, dispatch};
use multicad_model::{Compiler, DrawingProviders, MachiningRecord, MemberRecord, Model};
use serde_json::{Value, json};
use std::{
    f64::consts::PI,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};

fn shipped() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(demo::SHIPPED_PATH)
}
fn near(a: f64, b: f64, tolerance: f64) {
    assert!((a - b).abs() <= tolerance, "{a} != {b} (±{tolerance})");
}
/// Open a project file with full native validation, as Open does.
fn open(path: &Path) -> Document {
    project_file::read(
        &mut std::fs::File::open(path).unwrap(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document
}
fn compile(session: &Session) -> Model {
    Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap()
}
fn named(document: &Document, name: &str) -> EntityId {
    document
        .entities()
        .find(|e| e.name == name)
        .unwrap_or_else(|| panic!("no entity {name}"))
        .id
}
fn value(document: &Document, name: &str) -> f64 {
    parameters::evaluate(document)
        .unwrap()
        .value(named(document, name), Unit::Millimeter)
        .unwrap()
}
fn member<'a>(list: &'a [MemberRecord], name: &str) -> &'a MemberRecord {
    list.iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("no member {name}"))
}
fn operation<'a>(list: &'a [MachiningRecord], name: &str) -> &'a MachiningRecord {
    list.iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("no operation {name}"))
}
fn book(document: &Document) -> EntityId {
    let books: Vec<_> = multicad_core::drawings::books(document).collect();
    assert_eq!(books.len(), 1);
    books[0]
}

/// The bench in its current parameters: everything checked against values
/// computed from those parameters, not from the generator.
fn check_bench(session: &Session) {
    let doc = session.document();
    let [
        length,
        depth,
        overhang,
        frame,
        stretcher,
        dia,
        pitch,
        ex,
        ey,
    ] = [
        "top_length",
        "top_depth",
        "top_overhang",
        "frame_height",
        "stretcher_height",
        "hole_diameter",
        "hole_pitch",
        "hole_edge_x",
        "hole_edge_y",
    ]
    .map(|n| value(doc, n));
    let m = compile(session);

    // 19 separate member Parts with coherent ends and no review issues.
    let members = m.member_cut_list(doc).unwrap();
    assert_eq!(members.len(), 19);
    for r in &members {
        assert!(r.issues.is_empty(), "{}: {:?}", r.name, r.issues);
    }
    let outside = [length - 2. * overhang, depth - 2. * overhang];
    for (name, want) in [
        ("Top rail 1", outside[0]),
        ("Top rail 2", outside[1]),
        ("Top rail 3", outside[0]),
        ("Top rail 4", outside[1]),
    ] {
        let r = member(&members, name);
        near(r.length_mm, want, 1e-6);
        assert_eq!(r.designation, "RHS 60×40×3");
        for end in &r.ends {
            assert_eq!(end.kind, Some(ConnectionKind::Miter));
            near(end.angle_deg, 45., 1e-9);
        }
    }
    for i in 1..=4 {
        let leg = member(&members, &format!("Leg {i}"));
        // Between the foot plate and the underside of the 60 mm rail.
        near(leg.length_mm, frame - 60. - 8., 1e-6);
        assert!(leg.ends.iter().all(|e| e.angle_deg.abs() < 1e-9));
        near(
            member(&members, &format!("Mounting tab {i}")).length_mm,
            50.,
            1e-6,
        );
        near(
            member(&members, &format!("Foot plate {i}")).length_mm,
            80.,
            1e-6,
        );
    }
    for side in ["L", "R"] {
        near(
            member(&members, &format!("Side stretcher {side}")).length_mm,
            outside[1] - 40. - 40.,
            1e-6,
        );
    }
    near(
        member(&members, "Long stretcher").length_mm,
        // Between the side stretchers, 70 mm in from the frame outside.
        outside[0] - 140. - 40.,
        1e-6,
    );

    // 22 connections, each with one sequenced fillet weld at the joint.
    let welds = m.weld_schedule(doc).unwrap();
    assert_eq!(welds.len(), 22);
    assert_eq!(multicad_core::model::connections(doc).count(), 22);
    for (i, w) in welds.iter().enumerate() {
        assert_eq!(w.step, Some(i + 1), "{}", w.mark);
        assert_eq!(w.size_mm, Some(3.));
    }
    let leg_top = welds
        .iter()
        .find(|w| w.connection_name == "Leg 1 top")
        .unwrap();
    assert_eq!(leg_top.kind, ConnectionKind::Butt);
    near(leg_top.position[0], overhang + 70., 1e-6);
    near(leg_top.position[2], frame, 1e-6);
    let stretcher_weld = welds
        .iter()
        .find(|w| w.connection_name == "Side stretcher L front")
        .unwrap();
    near(stretcher_weld.position[2], stretcher, 1e-6);

    // The worktop: one beech panel resting on the frame top.
    let cuts = m.cut_list(doc).unwrap();
    assert_eq!(cuts.len(), 3, "worktop and both wooden vise jaws");
    let top = cuts.iter().find(|c| c.name == "Worktop").unwrap();
    assert_eq!(top.name, "Worktop");
    near(top.cut.thickness_mm, demo::TOP_MM, 1e-9);
    near(top.cut.finished_mm[0], length, 1e-6);
    near(top.cut.finished_mm[1], depth, 1e-6);
    assert!(top.issues.is_empty(), "{:?}", top.issues);
    let worktop = top.part;
    let fixed = named(doc, "Vise fixed jaw");
    let moving = named(doc, "Vise moving jaw");
    let part_bounds = |id| m.solid(m.snapshot().bodies[&id]).unwrap().bounds().unwrap();
    let fixed_bounds = part_bounds(fixed);
    let moving_bounds = part_bounds(moving);
    near(
        fixed_bounds.min[1] - moving_bounds.max[1],
        value(doc, "vise_opening"),
        1e-6,
    );
    near(fixed_bounds.max[2], frame + demo::TOP_MM, 1e-6);
    near(moving_bounds.max[2], frame + demo::TOP_MM, 1e-6);
    for rod in ["Vise left guide rod", "Vise right guide rod"] {
        let rod = named(doc, rod);
        for jaw in [fixed, moving, named(doc, "Vise guide housing")] {
            let overlap = m
                .solid(m.snapshot().bodies[&rod])
                .unwrap()
                .boolean(
                    m.solid(m.snapshot().bodies[&jaw]).unwrap(),
                    BooleanOperation::Intersect,
                    Default::default(),
                )
                .unwrap();
            assert!(
                overlap.is_none_or(|s| s.volume_mm3().unwrap() < 1e-6),
                "guide rods must pass through the modeled bores"
            );
        }
    }

    let solid = |part: EntityId| m.solid(m.snapshot().bodies[&part]).unwrap();
    // The worktop rests on the frame top: rails and tabs end at its
    // underside, and it covers the frame with the overhang all round.
    let bounds = solid(worktop).bounds().unwrap();
    for (got, want) in bounds.min.iter().zip([0., 0., frame]) {
        near(*got, want, 1e-6);
    }
    for (got, want) in bounds.max.iter().zip([length, depth, frame + demo::TOP_MM]) {
        near(*got, want, 1e-6);
    }
    for i in 0..4 {
        for name in [
            format!("Top rail {}", i + 1),
            format!("Mounting tab {}", i + 1),
        ] {
            let b = solid(member(&members, &name).part).bounds().unwrap();
            near(b.max[2], frame, 1e-6);
            assert!(b.min[0] >= overhang - 1e-6 && b.max[0] <= length - overhang + 1e-6);
            assert!(b.min[1] >= overhang - 1e-6 && b.max[1] <= depth - overhang + 1e-6);
        }
    }
    for name in ["Top rail 1", "Mounting tab 1"] {
        let overlap = solid(worktop)
            .boolean(
                solid(member(&members, name).part),
                BooleanOperation::Intersect,
                Default::default(),
            )
            .unwrap();
        assert!(overlap.is_none_or(|o| o.volume_mm3().unwrap() < 1e-6));
    }

    // Exactly 100 real holes on a 10 × 10 grid from the hole parameters,
    // plus four blind pilot holes centered on the tabs.
    let machining = m.machining(doc).unwrap();
    let grid = operation(&machining, "Hole grid 10 × 10");
    assert_eq!(grid.face, MachiningFace::Top);
    assert_eq!(grid.holes.len(), GRID * GRID);
    let mut expected = vec![];
    for r in 0..GRID {
        for c in 0..GRID {
            expected.push([ex + c as f64 * pitch, ey + r as f64 * pitch]);
        }
    }
    for (hole, want) in grid.holes.iter().zip(&expected) {
        near(hole.center[0], want[0], 1e-6);
        near(hole.center[1], want[1], 1e-6);
        near(hole.diameter_mm, dia, 1e-9);
        assert!(hole.through);
        near(hole.depth_mm, demo::TOP_MM, 1e-9);
        near(hole.axis[2], -1., 1e-9);
    }
    let pilots = operation(&machining, "Tab screw pilots");
    assert_eq!(pilots.face, MachiningFace::Bottom);
    assert_eq!(pilots.holes.len(), 4);
    for (i, hole) in pilots.holes.iter().enumerate() {
        assert!(!hole.through);
        near(hole.depth_mm, demo::PILOT_DEPTH_MM, 1e-9);
        let tab = member(&members, &format!("Mounting tab {}", i + 1));
        let bounds = solid(tab.part).bounds().unwrap();
        near(hole.center[0], (bounds.min[0] + bounds.max[0]) / 2., 1e-6);
        near(hole.center[1], (bounds.min[1] + bounds.max[1]) / 2., 1e-6);
    }
    let hole = |d: f64, h: f64| PI * d * d / 4. * h;
    near(
        solid(worktop).volume_mm3().unwrap(),
        length * depth * demo::TOP_MM
            - 100. * hole(dia, demo::TOP_MM)
            - 4. * hole(demo::PILOT_DIAMETER_MM, demo::PILOT_DEPTH_MM),
        1.,
    );
}

/// The texts of the associative dimensions on the assembly page.
fn assembly_dimensions(session: &Session) -> Vec<String> {
    let doc = session.document();
    let book = book(doc);
    let evaluation = compile(session)
        .drawings(doc, session.revision(), &DrawingProviders::default())
        .unwrap();
    let page = demo::page(session, book, PAGES[0]).unwrap();
    page.annotations
        .iter()
        .filter(|a| matches!(a.kind, AnnotationKind::Dimension { .. }))
        .map(|a| match evaluation.annotations.get(&a.id) {
            Some(AnnotationEvaluation::Ready { graphic }) => graphic.text.clone(),
            other => panic!("dimension not evaluated: {other:?}"),
        })
        .collect()
}

#[test]
fn shipped_file_opens_with_metadata_frame_welds_worktop_and_hole_grid() {
    let path = shipped();
    assert_eq!(path.extension().unwrap(), "tcd");
    // The card is what the home screen, library and examples list read.
    let card = project_file::card(
        &mut std::fs::File::open(&path).unwrap(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(card.info.name, demo::DOCUMENT_NAME);
    assert_eq!(card.info.description, demo::DESCRIPTION);
    assert!(card.info_error.is_none());
    let preview = card.info.preview.as_ref().unwrap();
    assert_eq!((preview.width(), preview.height()), (640, 480));
    let decoded = multicad_ui::library::decode_preview(preview).unwrap();
    // The picture is not blank: the wood and steel colors both appear.
    let distinct: std::collections::BTreeSet<_> =
        decoded.pixels.iter().map(|p| p.to_array()).collect();
    assert!(distinct.len() > 100);

    let document = open(&path);
    assert_eq!(document.name(), demo::DOCUMENT_NAME);
    assert_eq!(card.document, document.id());
    let session = Session::new(document).unwrap();
    for (name, want) in [
        ("top_length", 1500.),
        ("top_depth", 1000.),
        ("top_overhang", 50.),
        ("frame_height", 860.),
        ("stretcher_height", 150.),
        ("hole_diameter", 20.),
        ("hole_pitch", 80.),
        ("hole_edge_x", 390.),
        ("hole_edge_y", 140.),
    ] {
        near(value(session.document(), name), want, 1e-9);
    }
    check_bench(&session);

    // Drawings and schedules agree with the model.
    let book = book(session.document());
    for (number, conflicts) in demo::page_report(&session, book).unwrap() {
        assert!(
            conflicts.iter().all(|c| !c.contains("printable")),
            "{number}: {conflicts:?}"
        );
        // Weld symbols are placed on the isometric view by design.
        if number != PAGES[1] {
            assert!(conflicts.is_empty(), "{number}: {conflicts:?}");
        }
    }
    assert!(
        demo::stale_tables(&session, book).unwrap().is_empty(),
        "placed tables differ from the model"
    );
    assert_eq!(
        assembly_dimensions(&session),
        ["1500", "900", "1000", "860"]
    );
    let t = demo::tables(&session).unwrap();
    let machining: Vec<_> = t.machining.rows.iter().map(|r| r[2..].to_vec()).collect();
    assert!(machining.contains(&vec![
        "holes".into(),
        "Ø20".into(),
        "through".into(),
        "100".into()
    ]));
    assert!(machining.contains(&vec!["holes".into(), "Ø4".into(), "25".into(), "4".into()]));
    let quantity = |length: &str| {
        t.members
            .rows
            .iter()
            .filter(|r| r[3] == length)
            .map(|r| r[1].parse::<usize>().unwrap())
            .sum::<usize>()
    };
    assert_eq!(quantity("1400"), 2);
    assert_eq!(quantity("900"), 2);
    assert_eq!(quantity("792"), 4);
    assert_eq!(quantity("820"), 2);
    assert_eq!(quantity("1220"), 1);
    assert_eq!(quantity("50"), 4);
    assert_eq!(quantity("80"), 4);
    assert_eq!(
        t.welds
            .csv()
            .lines()
            .filter(|l| l.starts_with(char::is_numeric))
            .count(),
        22
    );
    assert!(t.bom.csv().contains("Worktop"));
}

#[test]
fn parameter_edits_move_frame_grid_and_top_with_undo_redo_and_save_reopen() {
    let mut session = Session::new(open(&shipped())).unwrap();
    let original = session.document().clone();
    let book = book(&original);
    let id = |name: &str| named(&original, name);

    // A longer top: rails, stretcher, tabs and the centered grid follow.
    demo::set_parameter(&mut session, id("top_length"), "1800").unwrap();
    near(value(session.document(), "hole_edge_x"), 540., 1e-9);
    check_bench(&session);
    // A finer, smaller grid.
    demo::set_parameter(&mut session, id("hole_pitch"), "90").unwrap();
    demo::set_parameter(&mut session, id("hole_diameter"), "16").unwrap();
    near(value(session.document(), "hole_edge_y"), 95., 1e-9);
    check_bench(&session);
    assert_eq!(assembly_dimensions(&session)[0], "1800");
    // Placed tables are page content: stale until placed again.
    let stale = demo::stale_tables(&session, book).unwrap();
    assert!(stale.iter().any(|e| e.contains("Machining")), "{stale:?}");
    demo::refresh_tables(&mut session, book).unwrap();
    assert!(demo::stale_tables(&session, book).unwrap().is_empty());
    let edited = session.document().clone();

    // A pitch that pushes the grid off the 1000 mm top is refused natively
    // before commit; nothing changes.
    let revision = session.revision();
    let refused = demo::set_parameter(&mut session, id("hole_pitch"), "120").unwrap_err();
    assert!(refused.to_string().contains("leaves"), "{refused}");
    assert_eq!(session.revision(), revision);
    assert_eq!(session.document(), &edited);

    // Undo back to the shipped source, then Redo the first edit.
    while session.document() != &original {
        session.undo(session.revision(), actor()).unwrap();
    }
    near(value(session.document(), "top_length"), 1500., 1e-9);
    session.redo(session.revision(), actor()).unwrap();
    near(value(session.document(), "top_length"), 1800., 1e-9);
    near(value(session.document(), "hole_pitch"), 80., 1e-9);
    check_bench(&session);
    let redone = session.document().clone();

    // Save as a native file and reopen: the same editable source, the same
    // frame connections, grid and top placement.
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("workbench-1800.tcd");
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(redone.clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    std::fs::write(&path, bytes).unwrap();
    let reopened = Session::new(open(&path)).unwrap();
    assert_eq!(reopened.document(), &redone);
    check_bench(&reopened);
    let (a, b) = (compile(&session), compile(&reopened));
    assert_eq!(
        a.member_cut_list(session.document()).unwrap(),
        b.member_cut_list(reopened.document()).unwrap()
    );
    assert_eq!(
        a.weld_schedule(session.document()).unwrap(),
        b.weld_schedule(reopened.document()).unwrap()
    );
}

#[test]
fn generator_rebuilds_the_shipped_project() {
    let mut built = demo::build().unwrap();
    demo::finish(&mut built).unwrap();
    let fresh = &built.session;
    let shipped = Session::new(open(&shipped())).unwrap();
    // Identities are new on every run; names, geometry records, schedules
    // and file information must match.
    let (a, b) = (compile(fresh), compile(&shipped));
    let members = |m: &Model, d: &Document| {
        m.member_cut_list(d)
            .unwrap()
            .into_iter()
            .map(|r| (r.name, (r.length_mm * 1e6).round() as i64))
            .collect::<std::collections::BTreeSet<_>>()
    };
    assert_eq!(
        members(&a, fresh.document()),
        members(&b, shipped.document())
    );
    let holes = |m: &Model, d: &Document| {
        m.machining(d)
            .unwrap()
            .into_iter()
            .map(|r| (r.name, format!("{:?}", r.holes)))
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    assert_eq!(holes(&a, fresh.document()), holes(&b, shipped.document()));
    let welds = |m: &Model, d: &Document| {
        m.weld_schedule(d)
            .unwrap()
            .into_iter()
            .map(|w| (w.mark, w.connection_name, w.position, w.step_note))
            .collect::<Vec<_>>()
    };
    assert_eq!(welds(&a, fresh.document()), welds(&b, shipped.document()));
    let rows = |s: &Session| {
        let t = demo::tables(s).unwrap();
        // Member marks and the order of names in a group follow entity
        // identities, which are new on every build.
        let members = t.members.rows.into_iter().map(|mut r| {
            r[0].clear();
            let mut names: Vec<_> = r[7].split("; ").map(String::from).collect();
            names.sort();
            r[7] = names.join("; ");
            r
        });
        let mut all = [
            members.collect(),
            t.machining.rows,
            t.panels.rows,
            t.welds.rows,
        ];
        for rows in &mut all {
            rows.sort();
        }
        all
    };
    assert_eq!(rows(fresh), rows(&shipped));
    let (fi, si) = (
        project_file::info::read(fresh.document()).unwrap(),
        project_file::info::read(shipped.document()).unwrap(),
    );
    assert_eq!((fi.name, fi.description), (si.name, si.description));
    assert!(si.capture_on_save);
    assert!(demo::stale_tables(fresh, built.book).unwrap().is_empty());
    // The archive the generator would write reads back to the same source.
    let bytes = demo::archive(fresh).unwrap();
    let read = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(&read.project.document, fresh.document());
}

/// An owner that reviews every client, like the desktop window by default.
struct Reviewed {
    session: Session,
    ledger: multicad_core::control::ControlLedger,
    resources: multicad_core::project_file::ProjectResources,
}
impl multicad_mcp::Target for Reviewed {
    fn session(&self) -> &Session {
        &self.session
    }
    fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }
    fn resources(&self) -> &multicad_core::project_file::ProjectResources {
        &self.resources
    }
    fn project_path(&self) -> Option<&Path> {
        None
    }
    fn save(&mut self) -> Result<std::path::PathBuf, String> {
        Err("unused".into())
    }
    fn review_policy(&self, _: &Actor) -> multicad_core::control::ReviewPolicy {
        multicad_core::control::ReviewPolicy::Reviewed
    }
    fn control(&mut self) -> (&mut Session, &mut multicad_core::control::ControlLedger) {
        (&mut self.session, &mut self.ledger)
    }
    fn ledger(&self) -> &multicad_core::control::ControlLedger {
        &self.ledger
    }
}
struct Window(std::sync::Mutex<Reviewed>);
impl multicad_mcp::Host for Window {
    fn scope(&self) -> multicad_mcp::Scope {
        multicad_mcp::Scope {
            mode: "application",
            transport: "local-socket",
            project: None,
            export_directory: None,
            read_only: false,
        }
    }
    fn submit(&self, job: multicad_mcp::Job) -> Result<(), String> {
        job(Ok(&mut *self.0.lock().unwrap()));
        Ok(())
    }
}

#[test]
fn external_client_changes_the_grid_pitch_through_review() {
    let session = Session::new(open(&shipped())).unwrap();
    let base = session.revision();
    let host = Window(std::sync::Mutex::new(Reviewed {
        session,
        ledger: Default::default(),
        resources: Default::default(),
    }));
    let call = Call {
        actor: Actor {
            id: "mcp:workbench".into(),
            channel: Channel::Mcp,
        },
        cancel: Arc::new(AtomicBool::new(false)),
    };
    let run = |name: &str, arguments: Value| dispatch(&host, &call, name, arguments).unwrap();
    let queued = run(
        "set_parameter",
        json!({ "expected_revision": base, "name": "hole_pitch",
            "expression": "70", "unit": "mm" }),
    );
    assert_eq!(queued["status"], "pending_review");
    let review_id = queued["review"].as_u64().unwrap();
    {
        let owner = &mut *host.0.lock().unwrap();
        assert_eq!(owner.session.revision(), base);
        assert_eq!(
            owner.ledger.accept(review_id, &mut owner.session).unwrap(),
            base + 1
        );
        check_bench(&owner.session);
        near(value(owner.session.document(), "hole_edge_y"), 185., 1e-9);
    }
    let welds = run("weld_schedule", json!({}));
    assert_eq!(welds["welds"].as_array().unwrap().len(), 22);
    let machining = run("schedule", json!({ "kind": "machining" }));
    assert!(
        machining["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r[3] == "Ø20" && r[5] == "100")
    );
    let history = run("history", json!({ "limit": 1 }));
    assert_eq!(history["entries"][0]["actor"], "mcp:workbench");
}
