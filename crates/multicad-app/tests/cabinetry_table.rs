//! The mixed-material fabrication slice end to end on one native project: a
//! welded steel table frame with mitered and butted SHS members and an
//! authored weld sequence, a laminated worktop stack, a birch plywood
//! cabinet and finger-jointed upstands. Table and panel thickness edits
//! propagate into members, connections, joints, cut records and drawings;
//! placed schedule tables are reported stale until refreshed; an impossible
//! depth is refused; Undo and save/reopen keep the source; and the export
//! sets write PDF, drawing DXF, panel DXF and STEP files that are read back
//! and compared with the editable project. An external MCP client makes a
//! reviewed edit.
#[path = "../examples/cabinetry_table_review.rs"]
#[allow(dead_code)]
mod review;
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId},
    drawings::AnnotationEvaluation,
    model::PanelRole,
    project_file,
};
use multicad_kernel::{BooleanOperation, GeometryPolicy, Solid};
use multicad_mcp::tools::{Call, dispatch};
use multicad_model::{Compiler, CutRecord, DrawingProviders, MemberRecord, Model};
use review::{Scenario, actor};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};

const SHS_AREA: f64 = 40. * 40. - 36. * 36.;

fn near(a: f64, b: f64, tolerance: f64) {
    assert!((a - b).abs() <= tolerance, "{a} != {b} (±{tolerance})");
}
fn sizes(got: [f64; 2], want: [f64; 2]) {
    near(got[0], want[0], 1e-6);
    near(got[1], want[1], 1e-6);
}
fn compile(session: &Session) -> Model {
    Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap()
}
fn member<'a>(list: &'a [MemberRecord], name: &str) -> &'a MemberRecord {
    list.iter().find(|r| r.name == name).unwrap()
}
fn panel<'a>(list: &'a [CutRecord], name: &str) -> &'a CutRecord {
    list.iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("no panel {name}"))
}
fn solid(m: &Model, part: EntityId) -> &Solid {
    m.solid(m.snapshot().bodies[&part]).unwrap()
}
fn volume(m: &Model, part: EntityId) -> f64 {
    solid(m, part).volume_mm3().unwrap()
}
/// The three dimension texts on the assembly page after checking every page.
fn dimensions(s: &Scenario) -> [String; 3] {
    let session = &s.session;
    let evaluation = compile(session)
        .drawings(
            session.document(),
            session.revision(),
            &DrawingProviders::default(),
        )
        .unwrap();
    for page in multicad_core::drawings::read(session.document(), s.book)
        .unwrap()
        .pages
    {
        evaluation
            .checked_page(session.document(), session.revision(), s.book, page.id)
            .unwrap();
    }
    [s.width_dimension, s.height_dimension, s.depth_dimension].map(|id| {
        match evaluation.annotations.get(&id) {
            Some(AnnotationEvaluation::Ready { graphic }) => graphic.text.clone(),
            other => panic!("dimension not evaluated: {other:?}"),
        }
    })
}
/// The finger joint leaves the two upstands touching without overlap, and
/// their volumes add up to both panels minus one corner block and the fit.
fn check_finger_joint(m: &Model, s: &Scenario, width: f64, t: f64) {
    let (back, left) = (solid(m, s.back_upstand), solid(m, s.left_upstand));
    let overlap = back
        .boolean(left, BooleanOperation::Intersect, Default::default())
        .unwrap();
    assert!(overlap.is_none_or(|o| o.volume_mm3().unwrap() < 1e-6));
    let sum = back.volume_mm3().unwrap() + left.volume_mm3().unwrap();
    let full = (width + 40.) * 150. * t + 300. * 150. * t;
    // Four finger faces with a 0.2 mm total gap each.
    near(sum, full - 150. * t * t - 4. * 0.2 * t * t, 1.);
}

#[test]
fn table_and_thickness_edits_propagate_to_members_joints_cut_records_and_drawings() {
    let mut s = review::build().unwrap();
    let m = compile(&s.session);
    let doc = s.session.document();

    // The frame: mitered rectangles, legs butted between them, a cross rail.
    let members = m.member_cut_list(doc).unwrap();
    assert_eq!(members.len(), 13);
    for name in ["Top rail 1", "Base rail 1"] {
        let r = member(&members, name);
        near(r.length_mm, 1640., 1e-6);
        assert!(r.issues.is_empty(), "{:?}", r.issues);
        for end in &r.ends {
            assert_eq!(end.kind, Some(multicad_core::model::ConnectionKind::Miter));
            near(end.angle_deg, 45., 1e-9);
            // The long point is on the outside of the frame, toward −Y.
            let long = end.long_point.unwrap();
            near(long.direction[1], -1., 1e-9);
        }
        near(r.volume_mm3, SHS_AREA * 1600., 1e-3);
    }
    near(member(&members, "Top rail 2").length_mm, 840., 1e-6);
    let leg = member(&members, "Leg 1");
    near(leg.length_mm, 820., 1e-6);
    near(leg.span_mm[0], 40., 1e-6);
    assert!(leg.ends.iter().all(|e| e.angle_deg.abs() < 1e-9));
    near(member(&members, "Cross rail").length_mm, 760., 1e-6);
    let groups = multicad_model::group_cuts(&members);
    let mut quantities: Vec<_> = groups
        .iter()
        .map(|g| (g.length_mm.round() as i64, g.quantity, g.long_points))
        .collect();
    quantities.sort();
    assert_eq!(
        quantities,
        [
            (760, 1, None),
            (820, 4, None),
            (840, 4, Some("same face")),
            (1640, 4, Some("same face")),
        ]
    );

    // Authored weld order with joints resolved from the members.
    let welds = m.weld_schedule(doc).unwrap();
    assert_eq!(welds.len(), 18);
    for (i, w) in welds.iter().enumerate() {
        assert_eq!(w.mark, format!("W{}", i + 1));
        assert_eq!(w.step, Some(i + 1));
    }
    assert_eq!(welds[4].position, [800., 0., 880.]);
    assert_eq!(welds[14].position, [60., 0., 0.]);
    assert_eq!(welds[0].step_note, "Tack corners, check diagonals");

    // Panels: finished and core sizes, grain, bands and the laminated stack.
    let cuts = m.cut_list(doc).unwrap();
    assert_eq!(cuts.len(), 9);
    let face = panel(&cuts, "Worktop face");
    sizes(face.cut.finished_mm, [1640., 840.]);
    sizes(face.cut.core_mm, [1636., 839.]);
    assert!(face.cut.grain_fixed);
    let core = panel(&cuts, "Worktop core");
    sizes(core.cut.core_mm, [1643., 843.]);
    assert!(!core.cut.grain_fixed);
    let stack = &m.stacks(doc).unwrap()[0];
    assert!(stack.face_to_face, "{:?}", stack.issues);
    near(stack.total_thickness_mm, 31., 1e-9);
    let top = panel(&cuts, "Cabinet top");
    near(top.cut.thickness_mm, 18., 1e-9);
    sizes(top.cut.finished_mm, [464., 760.]);
    near(top.cut.core_mm[1], 759.5, 1e-9);
    assert!(cuts.iter().all(|c| c.fits_stock && c.issues.is_empty()));
    check_finger_joint(&m, &s, 1600., 18.);
    // Shelf pins are drilled from the cabinet's inside face.
    let pins = m.machining(doc).unwrap();
    assert_eq!(pins[0].holes.len(), 4);
    assert!(
        pins[0]
            .holes
            .iter()
            .all(|h| (h.center[0] - 118.).abs() < 1e-6 && h.axis == [-1., 0., 0.] && !h.through)
    );
    assert_eq!(
        dimensions(&s),
        ["1640".to_string(), "931".into(), "840".into()]
    );
    let pages = review::page_report(&s.session, s.book).unwrap();
    for (number, conflicts) in &pages {
        assert!(
            conflicts.iter().all(|c| !c.contains("printable")),
            "{number}: {conflicts:?}"
        );
        if number != "T-102" {
            assert!(conflicts.is_empty(), "{number}: {conflicts:?}");
        }
    }
    assert!(review::stale_tables(&s).unwrap().is_empty());

    // A wider table rebuilds the frame, the worktop, the upstand joint, the
    // cut records, the weld positions and the width dimension.
    let before = s.session.document().clone();
    review::set_parameter(&mut s.session, s.table_width, "1800").unwrap();
    let m = compile(&s.session);
    let doc = s.session.document();
    let members = m.member_cut_list(doc).unwrap();
    near(member(&members, "Top rail 1").length_mm, 1840., 1e-6);
    near(member(&members, "Base rail 3").length_mm, 1840., 1e-6);
    near(member(&members, "Top rail 2").length_mm, 840., 1e-6);
    let welds = m.weld_schedule(doc).unwrap();
    assert_eq!(welds[4].position, [900., 0., 880.]);
    assert_eq!(welds[15].position, [1740., 0., 0.]);
    let cuts = m.cut_list(doc).unwrap();
    sizes(panel(&cuts, "Worktop face").cut.finished_mm, [1840., 840.]);
    sizes(panel(&cuts, "Back upstand").cut.finished_mm, [1840., 150.]);
    check_finger_joint(&m, &s, 1800., 18.);
    assert_eq!(dimensions(&s)[0], "1840");
    // Placed tables are page content: the cut list, member list and weld
    // plan are reported stale until they are placed again.
    let stale = review::stale_tables(&s).unwrap();
    for title in ["Fabrication cut list", "Member cut list", "weld plan"] {
        assert!(stale.iter().any(|e| e.contains(title)), "{stale:?}");
    }
    review::refresh_tables(&mut s).unwrap();
    assert!(review::stale_tables(&s).unwrap().is_empty());
    let t = review::tables(&s.session).unwrap();
    assert!(t.members.rows.iter().any(|r| r[3] == "1840" && r[1] == "4"));
    while s.session.document() != &before {
        s.session.undo(s.session.revision(), actor()).unwrap();
    }
    assert_eq!(dimensions(&s)[0], "1640");

    // Thinner plywood: the cabinet and upstands follow, the joint stays a
    // full block of each panel, and the steel frame is untouched.
    review::set_stock_thickness(&mut s.session, s.ply, 15.).unwrap();
    let m = compile(&s.session);
    let doc = s.session.document();
    let cuts = m.cut_list(doc).unwrap();
    let top = panel(&cuts, "Cabinet top");
    near(top.cut.thickness_mm, 15., 1e-9);
    sizes(top.cut.finished_mm, [470., 760.]);
    near(panel(&cuts, "Left upstand").cut.thickness_mm, 15., 1e-9);
    near(panel(&cuts, "Worktop face").cut.thickness_mm, 12., 1e-9);
    near(
        volume(&m, s.cabinet_panel(PanelRole::LeftSide)),
        400. * 760. * 15. - 4. * std::f64::consts::PI * 2.5 * 2.5 * 10.,
        1.,
    );
    check_finger_joint(&m, &s, 1600., 15.);
    near(
        member(&m.member_cut_list(doc).unwrap(), "Leg 1").length_mm,
        820.,
        1e-6,
    );
    assert!(
        review::stale_tables(&s)
            .unwrap()
            .iter()
            .any(|e| e.contains("Fabrication cut list"))
    );
    s.session.undo(s.session.revision(), actor()).unwrap();
    assert_eq!(s.session.document(), &before);

    // A table too shallow for its cabinet is refused before commit.
    let revision = s.session.revision();
    assert!(review::set_parameter(&mut s.session, s.table_depth, "30").is_err());
    assert_eq!(s.session.revision(), revision);
    assert_eq!(s.session.document(), &before);

    // Save and reopen keep the exact editable source.
    let bytes = review::archive(&s.session).unwrap();
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(&reopened.project.document, s.session.document());
    let m = Compiler::default()
        .compile(&reopened.project.document, 0)
        .unwrap();
    assert_eq!(
        m.member_cut_list(&reopened.project.document).unwrap(),
        compile(&s.session)
            .member_cut_list(s.session.document())
            .unwrap()
    );
}

/// Closed polylines of a DXF as (layer, vertices).
fn polylines(text: &str) -> Vec<(String, Vec<[f64; 2]>)> {
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let mut out: Vec<(String, Vec<[f64; 2]>)> = vec![];
    let mut inside = false;
    for pair in lines.chunks(2) {
        let [code, value] = pair else { break };
        match *code {
            "0" => {
                inside = *value == "LWPOLYLINE";
                if inside {
                    out.push((String::new(), vec![]));
                }
            }
            "8" if inside => out.last_mut().unwrap().0 = value.to_string(),
            "10" if inside => out.last_mut().unwrap().1.push([value.parse().unwrap(), 0.]),
            "20" if inside => {
                out.last_mut().unwrap().1.last_mut().unwrap()[1] = value.parse().unwrap()
            }
            _ => {}
        }
    }
    out
}
fn extent(points: &[[f64; 2]]) -> [f64; 2] {
    let span = |i: usize| {
        let v = points.iter().map(|p| p[i]);
        v.clone().fold(f64::MIN, f64::max) - v.fold(f64::MAX, f64::min)
    };
    let mut e = [span(0), span(1)];
    e.sort_by(f64::total_cmp);
    e
}
fn written(receipt: &multicad_core::exchange::RunReceipt) -> Vec<PathBuf> {
    let directory = Path::new(&receipt.directory);
    receipt
        .files
        .iter()
        .map(|f| directory.join(&f.artifact.output.filename))
        .collect()
}

#[test]
fn export_sets_and_schedules_match_the_saved_project() {
    let built = review::build().unwrap();
    // Everything below comes from the reopened project file.
    let bytes = review::archive(&built.session).unwrap();
    let document: Document = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document;
    let session = Session::new(document).unwrap();
    let m = compile(&session);
    let root = tempfile::tempdir().unwrap();

    // Frame set: a four-page PDF, four drawing DXFs and 13 member STEPs.
    let files = written(&review::export(&session, built.frame_set, root.path()).unwrap());
    let with = |files: &[PathBuf], ext: &str| -> Vec<PathBuf> {
        files
            .iter()
            .filter(|f| f.extension().is_some_and(|e| e == ext))
            .cloned()
            .collect()
    };
    let pdf = std::fs::read(&with(&files, "pdf")[0]).unwrap();
    assert!(pdf.starts_with(b"%PDF-"));
    let pages = pdf
        .windows(b"/Type /Page".len() + 1)
        .filter(|w| w.starts_with(b"/Type /Page") && w[w.len() - 1] != b's')
        .count();
    assert_eq!(pages, 4);
    let drawings: Vec<String> = with(&files, "dxf")
        .iter()
        .map(|p| std::fs::read_to_string(p).unwrap())
        .collect();
    assert_eq!(drawings.len(), 4);
    let find = |needle: &str| {
        drawings
            .iter()
            .find(|t| t.contains(needle))
            .unwrap_or_else(|| panic!("no drawing DXF contains {needle}"))
    };
    let assembly = find("931");
    assert!(assembly.contains("1640") && assembly.contains("$INSUNITS"));
    assert!(find("W18 135 MAG").contains("Weld sequence"));
    assert!(find("Fabrication cut list").contains("Leveling foot M10"));
    assert!(find("Member cut list").contains("long point -x"));
    let mut step: Vec<f64> = with(&files, "step")
        .iter()
        .map(|p| {
            Solid::read_step(p, GeometryPolicy::default())
                .unwrap()
                .volume_mm3()
                .unwrap()
        })
        .collect();
    let mut expected: Vec<f64> = m
        .member_cut_list(session.document())
        .unwrap()
        .iter()
        .map(|r| r.volume_mm3)
        .collect();
    step.sort_by(f64::total_cmp);
    expected.sort_by(f64::total_cmp);
    assert_eq!(step.len(), 13);
    for (a, b) in step.iter().zip(&expected) {
        near(*a, *b, 1e-3);
    }

    // Panel set: one 1:1 DXF and one STEP per panel, matching the cut list.
    let files = written(&review::export(&session, built.panel_set, root.path()).unwrap());
    let cuts = m.cut_list(session.document()).unwrap();
    let dxf = with(&files, "dxf");
    assert_eq!(dxf.len(), 9);
    for r in &cuts {
        let path = dxf
            .iter()
            .find(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(&r.name.replace(' ', "_"))
            })
            .unwrap_or_else(|| panic!("no DXF for {}", r.name));
        let text = std::fs::read_to_string(path).unwrap();
        assert!(text.contains("$INSUNITS") && text.contains(&r.name));
        let loops = polylines(&text);
        let outer: Vec<_> = loops.iter().filter(|(l, _)| l == "CUT").collect();
        assert_eq!(outer.len(), 1, "{}", r.name);
        let mut finished = r.cut.finished_mm;
        finished.sort_by(f64::total_cmp);
        let got = extent(&outer[0].1);
        near(got[0], finished[0], 1e-6);
        near(got[1], finished[1], 1e-6);
        let inner = loops.iter().filter(|(l, _)| l == "CUT_INNER").count();
        match r.name.as_str() {
            // The drilled face with its four shelf pin holes.
            "Cabinet left side" => assert_eq!(inner, 4),
            // Fingers 1, 3 and 5 stay on the back upstand: two notches.
            "Back upstand" => {
                assert_eq!(inner, 0);
                assert_eq!(outer[0].1.len(), 12);
            }
            _ => assert_eq!(inner, 0, "{}", r.name),
        }
    }
    let mut step: Vec<f64> = with(&files, "step")
        .iter()
        .map(|p| {
            Solid::read_step(p, GeometryPolicy::default())
                .unwrap()
                .volume_mm3()
                .unwrap()
        })
        .collect();
    let mut expected: Vec<f64> = cuts.iter().map(|r| r.volume_mm3).collect();
    step.sort_by(f64::total_cmp);
    expected.sort_by(f64::total_cmp);
    assert_eq!(step.len(), 9);
    for (a, b) in step.iter().zip(&expected) {
        near(*a, *b, 1e-3);
    }

    // CSV schedules from the shared builders: panels and layers, members
    // and purchased hardware in one cut list, with quantities.
    let t = review::tables(&session).unwrap();
    let csv = t.fabrication.csv();
    let rows: Vec<&str> = csv.lines().filter(|l| !l.starts_with('#')).collect();
    assert_eq!(
        rows[0],
        "Section,Item,Qty,Stock,T,Finished,Core,Cut / grain,Notes"
    );
    let count = |prefix: &str| rows.iter().filter(|r| r.starts_with(prefix)).count();
    assert_eq!(count("panel,"), 7);
    assert_eq!(count("layer of Worktop,"), 2);
    assert_eq!(count("member,"), 4);
    assert!(rows.contains(&"purchased,Leveling foot M10,4,,,,,,\"buy, do not make\""));
    assert!(rows.iter().any(|r| r.starts_with(
        "layer of Worktop,Worktop face,1,Beech multiplex 12,12,1640 × 840,1636 × 839,grain along L,"
    )));
    assert!(
        rows.iter().any(
            |r| r.contains(",4,SHS 40×40×2,,1640,,\"miter 45° -x / miter 45° -x, same face\",")
        )
    );
    let members = t.members.csv();
    assert!(members.contains(
        ",4,SHS 40×40×2,1640,miter 45° long point -x,miter 45° long point -x,same face,"
    ));
    let bom = t.bom.csv();
    assert!(bom.contains(",Leveling foot M10,4,,,purchased,"));
    assert!(bom.contains(",Worktop core,1,MDF,MDF 19,manufactured,Worktop"));
    assert_eq!(
        t.welds
            .csv()
            .lines()
            .filter(|l| l.starts_with(char::is_numeric))
            .count(),
        18
    );
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
fn external_client_widens_the_table_through_review_with_attributed_history() {
    let s = review::build().unwrap();
    let base = s.session.revision();
    let host = Window(std::sync::Mutex::new(Reviewed {
        session: s.session,
        ledger: Default::default(),
        resources: Default::default(),
    }));
    let call = Call {
        actor: Actor {
            id: "mcp:workshop".into(),
            channel: Channel::Mcp,
        },
        cancel: Arc::new(AtomicBool::new(false)),
    };
    let run = |name: &str, arguments: Value| dispatch(&host, &call, name, arguments).unwrap();
    let queued = run(
        "set_parameter",
        json!({ "expected_revision": base, "name": "table_width",
            "expression": "1800", "unit": "mm" }),
    );
    assert_eq!(queued["status"], "pending_review");
    let review_id = queued["review"].as_u64().unwrap();
    assert_eq!(host.0.lock().unwrap().session.revision(), base);
    {
        let owner = &mut *host.0.lock().unwrap();
        assert_eq!(
            owner.ledger.accept(review_id, &mut owner.session).unwrap(),
            base + 1
        );
    }
    let members = run("schedule", json!({ "kind": "member_cut_list" }));
    let long = members["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r[3] == "1840")
        .unwrap();
    assert_eq!(long[1], "4");
    assert_eq!(long[6], "same face");
    let welds = run("weld_schedule", json!({}));
    assert_eq!(welds["welds"].as_array().unwrap().len(), 18);
    assert_eq!(welds["welds"][4]["position_mm"], json!([900., 0., 880.]));
    let cuts = run("schedule", json!({ "kind": "panel_cut_list" }));
    assert!(
        cuts["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r[0] == "Back upstand" && r[4] == "1840")
    );
    let history = run("history", json!({ "limit": 1 }));
    assert_eq!(history["entries"][0]["actor"], "mcp:workshop");
}
