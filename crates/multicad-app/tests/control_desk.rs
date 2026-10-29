//! The PCB-driven control desk end to end on one native project: a KiCad
//! board with generated and missing STEP models drives the panel's standoff
//! pattern and openings; buttons pass through the openings without touching
//! the bent aluminium; a reimport that moves, removes and adds components is
//! reported until the panel is synchronized, while the printed guide follows
//! through the transfer engine; hardware keeps its Parts through sheet edits;
//! folded, developed and scheduled values agree; the deliverables are read
//! back by independent readers; and an external MCP client previews,
//! applies and undoes a panel change with attributed activity.
#[path = "../examples/control_desk_review.rs"]
#[allow(dead_code)]
mod review;
use ::dxf::{Drawing, entities::EntityType, enums::Units};
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::EntityId,
    drawings::AnnotationEvaluation,
    model::HardwareItem,
    project_file,
};
use multicad_kernel::{BooleanOperation, GeometryPolicy, RigidTransform, Solid};
use multicad_mcp::tools::{Call, dispatch};
use multicad_model::{Compiler, DevelopedPattern, DrawingProviders, Model, SheetRecord};
use review::{Finding, Scenario, actor};
use serde_json::{Value, json};
use std::{
    f64::consts::PI,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};

const T: f64 = 1.5;
const R: f64 = 1.5;
const K: f64 = 0.44;
fn allowance() -> f64 {
    PI / 2. * (R + K * T)
}
fn near(a: f64, b: f64, tolerance: f64) {
    assert!((a - b).abs() <= tolerance, "{a} != {b} (±{tolerance})");
}
fn compile(session: &Session) -> Model {
    Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap()
}
/// The Part's evaluated solid (its own definition, unplaced).
fn part(model: &Model, part: EntityId) -> &Solid {
    let s = model.snapshot();
    let head = s.bodies.get(&part).or_else(|| s.body_sources.get(&part));
    model.solid(*head.expect("evaluated Part")).unwrap()
}
fn common(a: &Solid, b: &Solid) -> f64 {
    a.boolean(b, BooleanOperation::Intersect, GeometryPolicy::default())
        .unwrap()
        .map_or(0., |s| s.volume_mm3().unwrap())
}
fn sheet(model: &Model, s: &Scenario) -> SheetRecord {
    model
        .sheet_metal(s.session.document())
        .unwrap()
        .into_iter()
        .find(|r| r.part == s.sheet.body)
        .unwrap()
}
fn kinds(findings: &[Finding]) -> Vec<(&'static str, String)> {
    let mut out: Vec<_> = findings
        .iter()
        .map(|f| (f.kind, f.subject.clone()))
        .collect();
    out.sort();
    out
}
fn pair(kind: &'static str, subject: &str) -> (&'static str, String) {
    (kind, subject.into())
}
/// Assert a drill point's center in the developed (base-frame) pattern.
fn drill(pattern: &DevelopedPattern, id: &str, at: [f64; 2]) {
    let center = pattern
        .drills
        .iter()
        .find(|d| d.id == id)
        .unwrap_or_else(|| panic!("no drill {id}"))
        .center;
    near(center[0], at[0], 1e-9);
    near(center[1], at[1], 1e-9);
}
/// Area of every opening on the panel at the given desk width: standoff,
/// screw and insert holes, the display window and the component holes.
fn openings_area(sw3: bool, d1: bool) -> f64 {
    let circle = |d: f64| PI * d * d / 4.;
    4. * circle(3.4)
        + 4. * circle(4.5)
        + 2. * circle(4.2)
        + 30. * 16.
        + (2. + f64::from(u8::from(sw3))) * circle(12.5)
        + circle(7.5)
        + if d1 { circle(5.2) } else { 0. }
}
/// Developed area: base, two flanges of 212 mm (desk width minus insets)
/// with their legs and one allowance each, minus four relief notches.
fn flat_area(width: f64, sw3: bool, d1: bool) -> f64 {
    let along = width - 28.;
    width * 160. + along * (50. + allowance()) + along * (30. + allowance())
        - 4. * 2. * 4.
        - openings_area(sw3, d1)
}
/// Folded volume: the developed strip of each bend replaced by the exact
/// bend sector θ/2 · ((r + t)² − r²) · w.
fn folded_volume(width: f64, sw3: bool, d1: bool) -> f64 {
    let along = width - 28.;
    let bend = PI / 4. * ((R + T).powi(2) - R * R) * along;
    T * (flat_area(width, sw3, d1) - 2. * allowance() * along) + 2. * bend
}

#[test]
fn board_drives_panel_and_guide_through_reimport_and_repair() {
    let dir = tempfile::tempdir().unwrap();
    let sources = dir.path().join("sources");
    let mut s = review::build(&sources).unwrap();

    // Import report: ENC1's model is missing; vias and oval drills are
    // reported, not dropped.
    assert_eq!(
        s.missing_models,
        [(
            "ENC1".to_string(),
            "${KIPRJMOD}/models/rotary_encoder_ec11.step".to_string()
        )]
    );
    assert!(s.unsupported.iter().any(|u| u.contains("2 vias")));
    assert!(s.unsupported.iter().any(|u| u.contains("oval drill")));
    let m = compile(&s.session);
    // The board: a 160 × 100 outline with R3 corners and four M3 holes.
    let board = 16000. - (4. - PI) * 9. - 4. * PI * 1.6 * 1.6;
    near(
        part(&m, s.board.body).volume_mm3().unwrap(),
        board * 1.6,
        1e-3,
    );
    // Linked STEP models, and ENC1's 14 × 12 courtyard envelope, 5 mm high.
    for (reference, volume) in [
        ("DS1", 40. * 26. * 4.),
        ("SW1", 12. * 12. * 3.5 + 8. * 8. * 9.5),
        ("SW2", 12. * 12. * 3.5 + 8. * 8. * 9.5),
        ("D1", 5. * 5. * 8.6),
        ("ENC1", 14. * 12. * 5.),
    ] {
        near(
            part(&m, s.component(reference).body.body)
                .volume_mm3()
                .unwrap(),
            volume,
            1e-3,
        );
    }
    assert_eq!(
        kinds(&s.findings().unwrap()),
        [pair("model_missing", "ENC1")]
    );

    // The panel agrees with its stock inputs and the developed blank.
    let record = sheet(&m, &s);
    assert_eq!(
        (
            record.thickness_mm,
            record.inside_radius_mm,
            record.k_factor
        ),
        (T, R, K)
    );
    assert_eq!(
        (
            format!("{:?}", record.radius_source),
            format!("{:?}", record.k_factor_source)
        ),
        ("Stock".into(), "Stock".into())
    );
    near(record.measured_thickness_mm, T, 1e-9);
    near(record.flat_area_mm2, flat_area(240., false, true), 1e-6);
    near(record.volume_mm3, folded_volume(240., false, true), 1e-6);
    near(record.flat_size_mm[1], 160. + 80. + 2. * allowance(), 1e-9);
    assert_eq!(record.bends.len(), 2);
    for b in &record.bends {
        assert_eq!((b.direction, b.angle_deg), ("down", 90.));
        near(b.allowance_mm, allowance(), 1e-12);
        near(b.length_mm, 212., 1e-9);
    }
    let pattern = m.unfold(s.session.document(), s.sheet.body).unwrap();
    // Only the tooling diagnostic: no hole near a bend, short leg or
    // missing relief at these sizes.
    assert_eq!(
        pattern
            .diagnostics
            .iter()
            .map(|d| d.kind)
            .collect::<Vec<_>>(),
        ["tooling"]
    );
    // Standoff points follow the board holes: board origin at (40, 130).
    drill(&pattern, "fastener.0.0", [45., 125.]);
    drill(&pattern, "fastener.0.3", [195., 35.]);

    // Hardware: four standoffs below the base face between panel and board,
    // the board top touching their ends; screws from above into the cheeks.
    let standoffs = part(
        &m,
        s.hardware_part(review::STANDOFFS, HardwareItem::Standoff),
    );
    assert_eq!(standoffs.solid_count().unwrap(), 4);
    let b = standoffs.bounds().unwrap();
    near(b.min[2], -10., 1e-6);
    near(b.max[2], 0., 1e-6);
    near((b.min[0] + b.max[0]) / 2., 120., 1e-6);
    near((b.min[1] + b.max[1]) / 2., 80., 1e-6);
    let board_top = m.solid(s.board_member).unwrap().bounds().unwrap().max[2];
    near(board_top, -10., 1e-9);
    let records = m.sheet_hardware(s.session.document()).unwrap();
    assert_eq!(records.len(), 4);
    for r in &records {
        assert!(r.findings.is_empty(), "{r:?}");
    }

    // Every component clears the panel: the buttons pass through their
    // openings above the panel top, the others stay below it.
    let panel = part(&m, s.sheet.body);
    for c in &s.components {
        let solid = m.solid(c.member).unwrap();
        assert!(
            common(panel, solid) < 1e-6,
            "{} hits the panel",
            c.reference
        );
    }
    let sw1 = m
        .solid(s.component("SW1").member)
        .unwrap()
        .bounds()
        .unwrap();
    near(sw1.max[2], 3., 1e-6);
    assert!(sw1.min[2] < 0. && sw1.max[2] > T);
    // The guide frame: 160 × 100 × 10 with a 140 × 80 window and four
    // transferred standoff clearance holes of 6.8 mm.
    let guide = m.solid(s.guide_member).unwrap();
    near(
        guide.volume_mm3().unwrap(),
        (16000. - 11200. - 4. * PI * 3.4 * 3.4) * 10.,
        1e-3,
    );
    assert!(common(guide, standoffs) < 1e-6);

    // Revision B: SW1 and H4 move, D1 is gone and SW3 is new.
    let plan = review::reimport(&mut s, &sources).unwrap();
    assert_eq!(plan.moved, ["SW1"]);
    assert_eq!(plan.added, ["SW3"]);
    assert_eq!(plan.missing, ["D1"]);
    assert!(plan.holes_changed && !plan.outline_changed);
    let findings = s.findings().unwrap();
    assert_eq!(
        kinds(&findings),
        [
            pair("component_missing", "D1"),
            pair("model_missing", "ENC1"),
            pair("model_missing", "SW3"),
            pair("panel_stale", "cutout.1"),
            pair("panel_stale", "cutout.5"),
            pair("panel_stale", "fastener.0.3"),
        ]
    );
    let stale = |id: &str| findings.iter().find(|f| f.subject == id).unwrap();
    assert!(stale("fastener.0.3").message.contains("(H4)"));
    assert!(stale("fastener.0.3").message.contains("5 mm away"));
    assert!(stale("cutout.1").message.contains("6 mm away"));
    assert!(stale("cutout.5").message.contains("not on the panel"));
    let m = compile(&s.session);
    // The moved button now collides with the unsynchronized panel.
    let panel = part(&m, s.sheet.body);
    assert!(common(panel, m.solid(s.component("SW1").member).unwrap()) > 1.);
    // The guide frame's holes followed the board through the transfer: the
    // H4 clearance hole is now centered at (190, 35).
    let guide = m.solid(s.guide_member).unwrap();
    let probe = |x: f64| {
        Solid::box_solid([2., 2., 30.], GeometryPolicy::default())
            .unwrap()
            .transformed(RigidTransform {
                translation_mm: [x - 1., 34., -20.],
                ..Default::default()
            })
            .unwrap()
    };
    assert!(common(guide, &probe(190.)) < 1e-6);
    assert!(common(guide, &probe(195.)) > 1.);
    let hardware: Vec<_> = s.hardware.iter().map(|(.., p)| *p).collect();

    // Repair: link SW3's model and synchronize the panel. The hardware
    // keeps its Parts and the standoff follows H4.
    let before = s.session.document().clone();
    review::link_model(&mut s, &sources, "SW3").unwrap();
    let prepared = review::prepare_sync(&s).unwrap();
    review::checked(&mut s.session, prepared).unwrap();
    assert_eq!(
        kinds(&s.findings().unwrap()),
        [
            pair("component_missing", "D1"),
            pair("model_missing", "ENC1")
        ]
    );
    let m = compile(&s.session);
    let pattern = m.unfold(s.session.document(), s.sheet.body).unwrap();
    drill(&pattern, "fastener.0.3", [190., 35.]);
    drill(&pattern, "cutout.1", [156., 95.]);
    drill(&pattern, "cutout.5", [144., 68.]);
    let ids: Vec<_> = m
        .sheet_hardware(s.session.document())
        .unwrap()
        .iter()
        .map(|r| r.part)
        .collect();
    for id in &hardware {
        assert!(ids.contains(id));
    }
    let panel = part(&m, s.sheet.body);
    for c in &s.components {
        let solid = m.solid(c.member).unwrap();
        assert!(
            common(panel, solid) < 1e-6,
            "{} hits the panel",
            c.reference
        );
        assert!(common(m.solid(s.guide_member).unwrap(), solid) < 1e-6);
    }
    near(
        part(&m, s.component("SW3").body.body).volume_mm3().unwrap(),
        12. * 12. * 3.5 + 8. * 8. * 9.5,
        1e-3,
    );
    // D1's opening is retained for review, so the area still includes it.
    near(
        sheet(&m, &s).flat_area_mm2,
        flat_area(240., true, true),
        1e-6,
    );
    // Undo returns to the reimported state, findings included.
    while s.session.document() != &before {
        s.session.undo(s.session.revision(), actor()).unwrap();
    }
    assert_eq!(kinds(&s.findings().unwrap()), kinds(&findings));
}

#[test]
fn sheet_edits_keep_hardware_parts_and_source_driven_holes() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = review::build(&dir.path().join("sources")).unwrap();
    let before = s.session.document().clone();
    let width = |s: &Scenario| {
        let evaluation = compile(&s.session)
            .drawings(
                s.session.document(),
                s.session.revision(),
                &DrawingProviders::default(),
            )
            .unwrap();
        for page in s.pages {
            evaluation
                .checked_page(s.session.document(), s.session.revision(), s.book, page)
                .unwrap();
        }
        match evaluation.annotations.get(&s.width_dimension) {
            Some(AnnotationEvaluation::Ready { graphic }) => graphic.text.clone(),
            other => panic!("width not evaluated: {other:?}"),
        }
    };
    assert_eq!(width(&s), "240");
    let hardware: Vec<_> = s.hardware.iter().map(|(.., p)| *p).collect();
    // The captive inserts sit on the back flange leg, 15 mm beyond its bend:
    // in the blank that is past the base edge by one allowance. The leg runs
    // from the flange start at u = 240 − 14 toward −u.
    let leg = 160. + allowance() + 15.;
    let pattern = compile(&s.session)
        .unfold(s.session.document(), s.sheet.body)
        .unwrap();
    drill(&pattern, "fastener.2.0", [196., leg]);
    drill(&pattern, "fastener.2.1", [44., leg]);

    // A wider desk: the panel, cheeks and board placement follow, the
    // standoffs and cheek screws move with their sources and keep their
    // Parts, and the bends lengthen.
    review::set_parameter(&mut s.session, s.desk_width, "280").unwrap();
    let m = compile(&s.session);
    let record = sheet(&m, &s);
    near(record.flat_area_mm2, flat_area(280., false, true), 1e-6);
    near(record.volume_mm3, folded_volume(280., false, true), 1e-6);
    for b in &record.bends {
        near(b.length_mm, 252., 1e-9);
    }
    let pattern = m.unfold(s.session.document(), s.sheet.body).unwrap();
    drill(&pattern, "fastener.0.0", [65., 125.]);
    drill(&pattern, "fastener.1.2", [274., 40.]);
    drill(&pattern, "fastener.2.0", [236., leg]);
    drill(&pattern, "fastener.2.1", [44., leg]);
    let records = m.sheet_hardware(s.session.document()).unwrap();
    assert_eq!(
        records.iter().map(|r| r.part).collect::<Vec<_>>().len(),
        hardware.len()
    );
    for r in &records {
        assert!(hardware.contains(&r.part));
        assert!(r.findings.is_empty(), "{r:?}");
    }
    let standoffs = part(
        &m,
        s.hardware_part(review::STANDOFFS, HardwareItem::Standoff),
    );
    let b = standoffs.bounds().unwrap();
    near((b.min[0] + b.max[0]) / 2., 140., 1e-6);
    near(
        part(&m, s.cheeks[1].body).bounds().unwrap().max[0],
        280.,
        1e-9,
    );
    // The board moved with $board_x: still clear of the panel.
    for c in &s.components {
        assert!(common(part(&m, s.sheet.body), m.solid(c.member).unwrap()) < 1e-6);
    }
    assert_eq!(
        kinds(&s.findings().unwrap()),
        [pair("model_missing", "ENC1")]
    );
    assert_eq!(width(&s), "280");
    // Placed tables are page content: the bend table is now stale.
    let fresh = review::schedules(&s, true).unwrap();
    let tables = review::placed_tables(&s).unwrap();
    let bend = &fresh.iter().find(|(n, _)| *n == "bend-schedule").unwrap().1;
    assert!(bend.check_table(&tables["bend-schedule"]).is_err());
    s.session.undo(s.session.revision(), actor()).unwrap();
    assert_eq!(s.session.document(), &before);
    assert_eq!(width(&s), "240");
    let fresh = review::schedules(&s, true).unwrap();
    for (name, schedule) in &fresh {
        schedule.check_table(&tables[name]).unwrap();
    }

    // Moving the cheek screws inward keeps every hardware Part.
    let mut patterns = multicad_core::model::read_sheet(s.session.document(), s.sheet.feature)
        .unwrap()
        .fasteners;
    for p in &mut patterns[review::CHEEK_SCREWS].points {
        if p[0] == multicad_core::model::length(6.) {
            p[0] = multicad_core::model::length(7.);
        }
    }
    let prepared = multicad_core::model::prepare_sheet_fasteners(
        &s.session,
        s.session.revision(),
        actor(),
        s.sheet.feature,
        &patterns,
    )
    .unwrap();
    let kept: Vec<_> = prepared.hardware.iter().map(|(.., o)| o.body).collect();
    review::checked(&mut s.session, prepared.model.prepared).unwrap();
    assert_eq!(kept.len(), hardware.len());
    assert!(kept.iter().all(|k| hardware.contains(k)));
    let pattern = compile(&s.session)
        .unfold(s.session.document(), s.sheet.body)
        .unwrap();
    drill(&pattern, "fastener.1.0", [7., 40.]);
}

/// Independent ZIP reading of stored entries (local headers through the
/// central directory).
fn unzip(bytes: &[u8], wanted: &str) -> Vec<u8> {
    let u16le = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]) as usize;
    let u32le = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
    let eocd = (0..bytes.len() - 21)
        .rev()
        .find(|&i| u32le(i) == 0x0605_4b50)
        .unwrap();
    let (count, mut at) = (u16le(eocd + 10), u32le(eocd + 16));
    for _ in 0..count {
        assert_eq!(u32le(at), 0x0201_4b50);
        let (method, size, name_len) = (u16le(at + 10), u32le(at + 20), u16le(at + 28));
        let extra = u16le(at + 30) + u16le(at + 32);
        let name = String::from_utf8(bytes[at + 46..at + 46 + name_len].to_vec()).unwrap();
        let local = u32le(at + 42);
        if name == wanted {
            assert_eq!(method, 0, "stored entry");
            let data = local + 30 + u16le(local + 26) + u16le(local + 28);
            return bytes[data..data + size].to_vec();
        }
        at += 46 + name_len + extra;
    }
    panic!("{wanted} is not in the archive")
}
/// The volume enclosed by a 3MF mesh, from its vertices and triangles.
fn mesh_volume(xml: &str) -> (String, f64) {
    let attr = |tag: &str, name: &str| -> f64 {
        let key = format!("{name}=\"");
        let start = tag.find(&key).unwrap() + key.len();
        tag[start..start + tag[start..].find('"').unwrap()]
            .parse()
            .unwrap()
    };
    let unit = {
        let at = xml.find("unit=\"").unwrap() + 6;
        xml[at..at + xml[at..].find('"').unwrap()].to_owned()
    };
    let mut vertices = vec![];
    let mut volume = 0.;
    for tag in xml.split('<') {
        if tag.starts_with("vertex ") {
            vertices.push([attr(tag, "x"), attr(tag, "y"), attr(tag, "z")]);
        } else if tag.starts_with("triangle ") {
            let [a, b, c] = ["v1", "v2", "v3"].map(|k| vertices[attr(tag, k) as usize]);
            volume += (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                + a[2] * (b[0] * c[1] - b[1] * c[0]))
                / 6.;
        }
    }
    (unit, volume)
}
/// Area of a closed LWPOLYLINE with bulges.
fn polyline_area(poly: &::dxf::entities::LwPolyline) -> f64 {
    poly.vertices
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let b = &poly.vertices[(i + 1) % poly.vertices.len()];
            let cross = (a.x * b.y - a.y * b.x) * 0.5;
            if a.bulge.abs() < 1e-12 {
                cross
            } else {
                let theta = 4. * a.bulge.atan();
                let chord = (a.x - b.x).hypot(a.y - b.y);
                let radius = chord / (2. * (theta.abs() * 0.5).sin());
                cross + radius * radius * (theta - theta.sin()) * 0.5
            }
        })
        .sum()
}
fn tool(name: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(name).args(args).output().ok()?;
    assert!(out.status.success(), "{name} rejected {args:?}");
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[test]
fn generated_deliverables_read_back_independently() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("control-desk");
    let manifest = review::generate(&out).unwrap();
    let files: Vec<PathBuf> = manifest["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| PathBuf::from(f["path"].as_str().unwrap()))
        .collect();
    // Hashes in the manifest match the files.
    for f in manifest["files"].as_array().unwrap() {
        let path = Path::new(f["path"].as_str().unwrap());
        assert_eq!(
            project_file::ContentHash::of(&std::fs::read(path).unwrap()).to_string(),
            f["sha256"].as_str().unwrap()
        );
    }
    let with = |ext: &str| -> Vec<&PathBuf> {
        files
            .iter()
            .filter(|f| f.extension().is_some_and(|e| e == ext))
            .collect()
    };
    let verification = &manifest["verification"];
    for flag in [
        "interactive_macos",
        "human_acceptance",
        "press_brake_tooling_or_springback",
        "physical_fit_or_manufacture",
        "real_kicad_project",
    ] {
        assert_eq!(verification[flag], false, "{flag}");
    }
    let after = manifest["findings"]["after_repair"].as_array().unwrap();
    assert_eq!(after.len(), 2);

    // The saved project reopens to the same document and compiles.
    let bytes = std::fs::read(with("multicad")[0]).unwrap();
    let opened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    let reopened = Compiler::default().compile(&opened.document, 0).unwrap();
    let record = reopened
        .sheet_metal(&opened.document)
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    near(record.flat_area_mm2, flat_area(240., true, true), 1e-6);
    near(record.volume_mm3, folded_volume(240., true, true), 1e-6);

    // Developed DXF: 1:1 millimeters, one cut contour, every opening, both
    // bend lines down, a drill point per round hole and labelled ids.
    let developed = files
        .iter()
        .find(|f| f.ends_with("control-panel-developed.dxf"))
        .unwrap();
    let drawing = Drawing::load_file(developed.to_str().unwrap()).unwrap();
    assert_eq!(drawing.header.default_drawing_units, Units::Millimeters);
    let (mut outer, mut inner, mut down, mut up, mut drills, mut labels) =
        (vec![], vec![], vec![], 0, 0, vec![]);
    for e in drawing.entities() {
        match (&e.specific, e.common.layer.as_str()) {
            (EntityType::LwPolyline(p), "CUT") => outer.push(polyline_area(p).abs()),
            (EntityType::LwPolyline(p), "CUT_INNER") => inner.push(polyline_area(p).abs()),
            (EntityType::Circle(c), "CUT_INNER") => inner.push(PI * c.radius * c.radius),
            (EntityType::Line(l), "BEND_DOWN") => {
                down.push(l.p1.x.max(l.p2.x) - l.p1.x.min(l.p2.x))
            }
            (EntityType::Line(_), "BEND_UP") => up += 1,
            (EntityType::ModelPoint(_), "DRILL") => drills += 1,
            (EntityType::Text(t), "LABEL") => labels.push(t.value.clone()),
            _ => {}
        }
    }
    assert_eq!(outer.len(), 1);
    assert_eq!(inner.len(), 4 + 4 + 2 + 1 + 3 + 1 + 1);
    near(
        outer[0] - inner.iter().sum::<f64>(),
        flat_area(240., true, true),
        1e-6,
    );
    near(inner.iter().sum::<f64>(), openings_area(true, true), 1e-6);
    assert_eq!(down, [212., 212.]);
    assert_eq!(up, 0);
    assert_eq!(drills, 15);
    for label in [
        "flange.0.bend.0 DOWN 90.000 R1.500 #1",
        "flange.1.bend.0 DOWN 90.000 R1.500 #2",
        "fastener.0.3 D3.400",
        "fastener.2.1 D4.200",
        "cutout.5 D12.500",
    ] {
        assert!(labels.iter().any(|l| l == label), "{label}: {labels:?}");
    }

    // CSV schedules from the same records.
    let csv = |name: &str| {
        std::fs::read_to_string(files.iter().find(|f| f.ends_with(name)).unwrap()).unwrap()
    };
    let bends = csv("bend-schedule.csv");
    assert!(bends.contains("Control panel,flange.0.bend.0,1,DOWN,90,1.5,1.5,0.44,3.393,212"));
    assert!(bends.contains("Control panel,flange.1.bend.0,2,DOWN,90,1.5,1.5,0.44,3.393,212"));
    let bom = csv("bom.csv");
    for row in [
        "Control panel,1,EN AW-5754 H22,AlMg3 1.5 sheet,manufactured,",
        "Left cheek,1,Birch plywood,,manufactured,",
        "Printed guide frame,1,PETG,,manufactured,",
        "\"Control panel Hex standoff M3 × 10, AF 5.5\",1,,,purchased,",
        "Control panel ISO 4762 M4 × 20,1,,,purchased,",
        "control-panel ENC1,1,,,imported,",
    ] {
        assert!(bom.contains(row), "{row}\n{bom}");
    }
    assert!(!bom.contains("solid 1"), "no loose STEP body: {bom}");
    let hardware = csv("hardware.csv");
    for row in [
        "\"Control panel Hex standoff M3 × 10, AF 5.5\",\"Hex standoff M3 × 10, AF 5.5\",4,3.4,",
        "Control panel ISO 4762 M4 × 20,ISO 4762 M4 × 20,4,4.5,ISO 4762 nominal dimensions,",
        "Control panel ISO 7089 M4,ISO 7089 M4,4,4.5,",
        "Self-clinching nut M3 (example entry),2,4.2,User-entered catalogue values,",
    ] {
        assert!(hardware.contains(row), "{row}\n{hardware}");
    }
    let drive = csv("board-drive.csv");
    assert!(drive.contains("cutout.4,D1,\"hole D5.2, source missing\",component_missing: D1"));
    assert!(
        drive.contains("cutout.3,ENC1,hole D7.5,\"model_missing: ENC1 has no linked STEP model")
    );

    // The four-page PDF and one drawing DXF per page.
    let pdf = with("pdf");
    assert_eq!(pdf.len(), 1);
    let bytes = std::fs::read(pdf[0]).unwrap();
    assert!(bytes.starts_with(b"%PDF-"));
    let pages = bytes
        .windows(b"/Type /Page".len() + 1)
        .filter(|w| w.starts_with(b"/Type /Page") && w[w.len() - 1] != b's')
        .count();
    assert_eq!(pages, 4);
    if let Some(info) = tool("pdfinfo", &[pdf[0].to_str().unwrap()]) {
        assert!(info.contains("Pages:           4"), "{info}");
    }
    if let Some(text) = tool("pdftotext", &["-layout", pdf[0].to_str().unwrap(), "-"]) {
        let pages: Vec<_> = text.split('\u{c}').collect();
        assert!(pages[0].contains("240") && pages[0].contains("CD-101"));
        assert!(pages[0].contains("no press-brake"));
        assert!(pages[1].contains("flange.0.bend.0") && pages[1].contains("3.393"));
        assert!(pages[2].contains("Hex standoff M3") && pages[2].contains("Bill of materials"));
        assert!(pages[3].contains("component_missing") && pages[3].contains("model_missing"));
    }
    let page_dxfs: Vec<_> = with("dxf")
        .into_iter()
        .filter(|f| !f.ends_with("control-panel-developed.dxf"))
        .collect();
    assert_eq!(page_dxfs.len(), 4);
    for f in &page_dxfs {
        let d = Drawing::load_file(f.to_str().unwrap()).unwrap();
        assert!(d.entities().count() > 10, "{}", f.display());
    }

    // STEP: only the fabricated Parts (panel and cheeks).
    let steps = with("step");
    assert_eq!(steps.len(), 3);
    let mut volumes: Vec<f64> = steps
        .iter()
        .map(|p| {
            Solid::read_step(p, GeometryPolicy::default())
                .unwrap()
                .volume_mm3()
                .unwrap()
        })
        .collect();
    volumes.sort_by(f64::total_cmp);
    let cheek = 12. * 150. * 90.;
    near(volumes[0], folded_volume(240., true, true), 1e-3);
    near(volumes[1], cheek, 1e-3);
    near(volumes[2], cheek, 1e-3);

    // 3MF: only the printed guide frame, in millimeters.
    let three = with("3mf");
    assert_eq!(three.len(), 1);
    if let Some(ok) = std::process::Command::new("unzip")
        .arg("-tq")
        .arg(three[0])
        .output()
        .ok()
        .map(|o| o.status.success())
    {
        assert!(ok, "system unzip rejects the 3MF");
    }
    let xml =
        String::from_utf8(unzip(&std::fs::read(three[0]).unwrap(), "3D/3dmodel.model")).unwrap();
    let (unit, volume) = mesh_volume(&xml);
    assert_eq!(unit, "millimeter");
    let frame = (16000. - 11200. - 4. * PI * 3.4 * 3.4) * 10.;
    near(volume, frame, frame * 2e-3);
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
    fn save(&mut self) -> Result<PathBuf, String> {
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
fn external_client_previews_applies_and_undoes_a_wider_desk() {
    let dir = tempfile::tempdir().unwrap();
    let s = review::build(&dir.path().join("sources")).unwrap();
    let base = s.session.revision();
    let original = s.session.document().clone();
    let sheet_part = s.sheet.body;
    let standoffs = s.hardware_part(review::STANDOFFS, HardwareItem::Standoff);
    let host = Window(std::sync::Mutex::new(Reviewed {
        session: s.session,
        ledger: Default::default(),
        resources: Default::default(),
    }));
    let call = Call {
        actor: Actor {
            id: "mcp:control-desk".into(),
            channel: Channel::Mcp,
        },
        cancel: Arc::new(AtomicBool::new(false)),
    };
    let run = |name: &str, arguments: Value| dispatch(&host, &call, name, arguments).unwrap();
    let queued = run(
        "set_parameter",
        json!({ "expected_revision": base, "name": "desk_width",
            "expression": "280", "unit": "mm" }),
    );
    assert_eq!(queued["status"], "pending_review");
    let review_id = queued["review"].as_u64().unwrap();

    // Preview: the owner evaluates the candidate natively while the
    // document stays at the base revision.
    {
        let owner = host.0.lock().unwrap();
        assert_eq!(owner.session.revision(), base);
        let candidate = owner.ledger.pending()[0].candidate().unwrap();
        let preview = Compiler::default()
            .compile(candidate.document(), base + 1)
            .unwrap();
        let pattern = preview.unfold(candidate.document(), sheet_part).unwrap();
        drill(&pattern, "fastener.0.0", [65., 125.]);
        assert!(pattern.bends.iter().all(|b| b.record.length_mm == 252.));
        assert_eq!(owner.session.document(), &original);
    }
    {
        let owner = &mut *host.0.lock().unwrap();
        assert_eq!(
            owner.ledger.accept(review_id, &mut owner.session).unwrap(),
            base + 1
        );
    }
    // Read tools see the applied change: the developed blank and the
    // hardware keep their identities.
    let unfolded = run("unfold_sheet", json!({ "part": sheet_part.to_string() }));
    assert_eq!(unfolded["bends"][0]["length_mm"], 252.);
    let listed = run("sheet_hardware", json!({}));
    assert!(listed.to_string().contains(&standoffs.to_string()));
    let history = run("history", json!({ "limit": 1 }));
    assert_eq!(history["entries"][0]["actor"], "mcp:control-desk");

    // Undo is reviewed too; accepting it restores the original desk.
    let undo = run("undo", json!({ "expected_revision": base + 1 }));
    assert_eq!(undo["status"], "pending_review");
    {
        let owner = &mut *host.0.lock().unwrap();
        owner
            .ledger
            .accept(undo["review"].as_u64().unwrap(), &mut owner.session)
            .unwrap();
        assert_eq!(owner.session.document(), &original);
    }
    let activity = run("activity", json!({ "limit": 50 }));
    let entries = activity["entries"].as_array().unwrap();
    let applied = entries.iter().find(|e| e["outcome"] == "applied").unwrap();
    assert_eq!(applied["tool"], "set_parameter");
    assert_eq!(applied["arguments"]["name"], "desk_width");
    assert_eq!(applied["actor"]["id"], "mcp:control-desk");
    assert_eq!(applied["base_revision"], base);
    assert!(entries.iter().any(|e| e["outcome"] == "undone"));
    let owner = host.0.lock().unwrap();
    let m = compile(&owner.session);
    let b = part(&m, standoffs).bounds().unwrap();
    near((b.min[0] + b.max[0]) / 2., 120., 1e-6);
}
