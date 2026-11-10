//! The rolling tool case end to end on one native project: the purchased
//! slides' hole pattern drills both carcass sides and both printed drawer
//! walls; rail pitch and hole-count edits move every hole and Undo restores
//! them; replacing a slide with other hardware is flagged until the
//! transfer is repaired; drawer poses, limits, interference and sampled
//! travel use the actual assembly; FDM aids are inspected; the saved
//! project, panel drawing PDF/DXF, BOM and cut-list CSV and both 3MF print
//! packages are read back by readers written here, independent of the
//! writers; and an external MCP client changes the rail pitch through
//! review with Undo and attributed activity.
#[path = "../examples/tool_case_review.rs"]
#[allow(dead_code)]
mod review;
use multicad_core::{
    assembly_inspection::{InspectionFailureKind, PairRelation, RangeCoverage},
    commands::{Actor, Channel, Request, Session},
    document::{EntityData, EntityId, Mutation},
    drawings::AnnotationEvaluation,
    model::{
        self, AlignmentInput, BodyInput, PrimitiveFrame, PrimitiveParameters, TransferRequest,
    },
    project_file,
};
use multicad_kernel::SolidPairRelation;
use multicad_mcp::tools::{Call, dispatch};
use multicad_model::{Compiler, DrawingProviders, Model, schedules::Supply};
use review::{
    CASE, DRAWER_D, DRAWER_W, DRAWER_X, FLOOR, HOLE_Z, PILOT_D, PLY, RAIL_HOLE_R, SIDE_DEPTH,
    SLIDE, Scenario, TRAVEL, WALL, WALL_DEPTH, actor,
};
use serde_json::{Value, json};
use std::{
    f64::consts::PI,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, atomic::AtomicBool},
};

fn near(a: f64, b: f64, tolerance: f64) {
    assert!((a - b).abs() <= tolerance, "{a} != {b} (±{tolerance})");
}
fn compile(session: &Session) -> Model {
    Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap()
}
fn volume(model: &Model, part: EntityId) -> f64 {
    let snapshot = model.snapshot();
    let head = snapshot
        .bodies
        .get(&part)
        .or_else(|| snapshot.body_sources.get(&part))
        .unwrap();
    model.solid(*head).unwrap().volume_mm3().unwrap()
}
fn pilot_area() -> f64 {
    PI * (PILOT_D / 2.).powi(2)
}
fn side_volume(holes: usize) -> f64 {
    PLY * CASE[1] * CASE[2] - holes as f64 * pilot_area() * SIDE_DEPTH
}
fn drawer_volume(height: f64, holes_per_wall: usize) -> f64 {
    DRAWER_W * DRAWER_D * height
        - (DRAWER_W - 2. * WALL) * (DRAWER_D - 2. * WALL) * (height - FLOOR)
        - 2. * holes_per_wall as f64 * pilot_area() * WALL_DEPTH
}
const INSERT_VOLUME: f64 = 120. * 60. * 40. - 80. * 60. * 20.;
fn rail_volume(holes: usize) -> f64 {
    SLIDE[0] * SLIDE[1] * SLIDE[2] - holes as f64 * PI * RAIL_HOLE_R * RAIL_HOLE_R * SLIDE[0]
}
/// Pilot hole centers on each drilled entry face, as hole y positions.
fn drilled(s: &Scenario, m: &Model) -> [Vec<f64>; 4] {
    let r = PILOT_D / 2.;
    let entries = [
        (s.transfers[0].targets[0].feature, PLY),
        (s.transfers[1].targets[0].feature, CASE[0] - PLY),
        (s.transfers[2].targets[0].feature, DRAWER_X),
        (s.transfers[3].targets[0].feature, DRAWER_X + DRAWER_W),
    ];
    entries.map(|(feature, x)| {
        review::hole_centers(m, feature, x, r)
            .into_iter()
            .map(|[y, z]| {
                near(z, HOLE_Z, 1e-6);
                y
            })
            .collect()
    })
}
fn assert_holes(s: &Scenario, m: &Model, want: &[f64]) {
    for (face, got) in drilled(s, m).iter().enumerate() {
        assert_eq!(got.len(), want.len(), "entry face {face}: {got:?}");
        for (a, b) in got.iter().zip(want) {
            near(*a, *b, 1e-6);
        }
    }
    for t in &s.transfers {
        assert_eq!(
            m.snapshot().transfer_counts.get(&t.targets[0].feature),
            Some(&want.len())
        );
    }
    m.snapshot().transfer_evaluation().validate().unwrap();
}
/// Texts of the three panel drawing dimensions; the page must check.
fn dimensions(s: &Scenario) -> [String; 3] {
    let session = &s.session;
    let evaluation = compile(session)
        .drawings(
            session.document(),
            session.revision(),
            &DrawingProviders::default(),
        )
        .unwrap();
    evaluation
        .checked_page(session.document(), session.revision(), s.book, s.panel_page)
        .unwrap();
    [s.length_dimension, s.height_dimension, s.hole_dimension].map(|id| {
        match evaluation.annotations.get(&id) {
            Some(AnnotationEvaluation::Ready { graphic }) => graphic.text.clone(),
            other => panic!("dimension not evaluated: {other:?}"),
        }
    })
}
fn leaf_min(m: &Model, path: &model::OccurrencePath) -> [f64; 3] {
    m.leaf_snapshot(path).unwrap().bounds.min
}

#[test]
fn rail_pattern_drives_carcass_and_drawer_drilling_and_pitch_edits_update_it() {
    let mut s = review::build().unwrap();
    let m = compile(&s.session);
    // Source-driven pilots: 3 holes at 29 + n·96 on all four entry faces.
    assert_holes(&s, &m, &[29., 125., 221.]);
    near(volume(&m, s.left_side.body), side_volume(3), 1e-3);
    near(volume(&m, s.right_side.body), side_volume(3), 1e-3);
    near(volume(&m, s.drawer.part), drawer_volume(80., 3), 1e-3);
    near(volume(&m, s.insert.part), INSERT_VOLUME, 1e-3);
    // The hardware keeps its own Ø4.5 through holes; transfers never edit it.
    near(volume(&m, s.cabinet_member), rail_volume(3), 1e-3);
    near(volume(&m, s.drawer_member), rail_volume(3), 1e-3);
    // The drilled sides are still cut-list panels with their stock.
    let cut = m.cut_list(s.session.document()).unwrap();
    assert_eq!(cut.len(), 4);
    for r in &cut {
        assert!(r.issues.is_empty(), "{}: {:?}", r.name, r.issues);
        assert!(r.fits_stock);
    }
    assert_eq!(dimensions(&s), ["300", "200", "Ø3"].map(String::from));

    // A new pitch moves every hole on both sides and both drawer walls.
    let before = s.session.document().clone();
    review::set_parameter(&mut s.session, s.rail_pitch, "80").unwrap();
    let m = compile(&s.session);
    assert_holes(&s, &m, &[29., 109., 189.]);
    near(volume(&m, s.left_side.body), side_volume(3), 1e-3);
    // The drawing dimension stays attached to the moved hole.
    assert_eq!(dimensions(&s)[2], "Ø3");
    s.session.undo(s.session.revision(), actor()).unwrap();
    assert_eq!(s.session.document(), &before);
    assert_holes(&s, &compile(&s.session), &[29., 125., 221.]);

    // Four holes at 64 mm: every receiver gets one more pilot.
    review::set_parameter(&mut s.session, s.rail_pitch, "64").unwrap();
    review::set_parameter(&mut s.session, s.rail_holes, "4").unwrap();
    let m = compile(&s.session);
    assert_holes(&s, &m, &[29., 93., 157., 221.]);
    near(volume(&m, s.right_side.body), side_volume(4), 1e-3);
    near(volume(&m, s.drawer.part), drawer_volume(80., 4), 1e-3);
    s.session.undo(s.session.revision(), actor()).unwrap();
    s.session.undo(s.session.revision(), actor()).unwrap();
    assert_eq!(s.session.document(), &before);
}

#[test]
fn replacing_slide_hardware_is_flagged_until_the_drilling_is_repaired() {
    let mut s = review::build().unwrap();
    let before = s.session.document().clone();
    let start = s.session.revision();
    // Other purchased hardware: a heavy-duty member with two Ø5.5 holes at
    // 50 and 200 mm.
    let bar = review::block(
        &mut s.session,
        "Slide 250 heavy cabinet member",
        review::xyz(SLIDE),
        [PLY, 0., review::SLIDE_Z],
    )
    .unwrap();
    let mut head = bar.feature;
    for y in [50., 200.] {
        let hole = review::primitive(
            &mut s.session,
            "Heavy slide hole",
            PrimitiveParameters::Cylinder {
                radius: model::length(2.75),
                height: model::length(SLIDE[0] + 2.),
                frame: PrimitiveFrame {
                    origin: review::xyz([PLY - 1., y, HOLE_Z]),
                    u: [0., 1., 0.],
                    v: [0., 0., 1.],
                },
            },
        )
        .unwrap();
        head = review::cut(
            &mut s.session,
            BodyInput {
                body: bar.body,
                producer: head,
            },
            review::input(&hole),
        )
        .unwrap();
    }
    // Swap the left slide occurrence to the new hardware and make it an
    // assembly definition Part, without a native preflight.
    let mut slide = s
        .session
        .document()
        .get(s.slide_left.instance)
        .unwrap()
        .clone();
    let EntityData::Instance { component, .. } = &mut slide.data else {
        panic!()
    };
    *component = bar.body;
    s.session
        .apply(Request {
            expected_revision: s.session.revision(),
            actor: actor(),
            label: "Replace left slide".into(),
            edits: vec![Mutation::Replace(slide)],
        })
        .unwrap();
    let mut gear = model::AssemblyRequest::existing(s.session.document(), s.assembly).unwrap();
    gear.definition_parts.push(bar.body);
    let prepared =
        model::prepare_assembly_edit(&s.session, s.session.revision(), actor(), &gear).unwrap();
    s.session.commit(prepared.prepared).unwrap();
    // The affected drilling is flagged; nothing silently keeps old holes.
    let error = Compiler::default()
        .compile(s.session.document(), s.session.revision())
        .err()
        .expect("replaced hardware must not evaluate silently");
    assert!(
        error
            .message
            .contains("Transfer occurrence binding needs repair")
            && error.message.contains("different source"),
        "{}",
        error.message
    );
    // Repair: re-pick the new hardware's mounting face from a source-only
    // inspection and edit the same transfer group.
    let source = Compiler::default()
        .compile_sources(s.session.document(), s.session.revision(), &[head])
        .unwrap();
    let face = review::plane(&source, head, bar.body, [-1., 0., 0.], -PLY).unwrap();
    let mut repair = TransferRequest::existing(s.session.document(), s.transfers[0].group).unwrap();
    let old = repair.source.selection.captures()[0];
    repair.references.retain(|r| r.intent.id != old);
    repair.source.component = bar.body;
    repair.source.selection = model::TransferSelection::FaceHoles {
        face: face.intent.id,
    };
    repair.references.insert(
        0,
        AlignmentInput {
            source: bar.body,
            intent: face.intent,
        },
    );
    let prepared =
        model::prepare_transfer_edit(&s.session, s.session.revision(), actor(), &repair).unwrap();
    review::checked(&mut s.session, prepared.prepared).unwrap();
    let m = compile(&s.session);
    let r = PILOT_D / 2.;
    let left: Vec<f64> = review::hole_centers(&m, s.transfers[0].targets[0].feature, PLY, r)
        .into_iter()
        .map(|c| c[0])
        .collect();
    assert_eq!(left, [50., 200.]);
    near(volume(&m, s.left_side.body), side_volume(2), 1e-3);
    // The right side still follows the original slide.
    assert_eq!(drilled(&s, &m)[1], [29., 125., 221.]);
    // Undo every step: the hardware, the swap and the repair.
    for _ in start..s.session.revision() {
        s.session.undo(s.session.revision(), actor()).unwrap();
    }
    assert_eq!(s.session.document(), &before);
    assert_holes(&s, &compile(&s.session), &[29., 125., 221.]);
}

#[test]
fn drawer_poses_limits_interference_and_sampled_travel_use_the_actual_assembly() {
    let mut s = review::build().unwrap();
    let slide = s.path(&s.slide_left);
    let moving = [
        s.path(&s.drawer),
        s.path(&s.runner_left),
        s.path(&s.runner_right),
        s.path(&s.insert),
    ];
    let m = compile(&s.session);
    let closed: Vec<_> = moving.iter().map(|p| leaf_min(&m, p)).collect();
    near(closed[0][1], 0., 1e-9);
    near(closed[2][0], DRAWER_X + DRAWER_W, 1e-9);
    let slide_closed = leaf_min(&m, &slide);

    // The open pose pulls the drawer, both runners and the insert out
    // together along −Y; the slides and panels stay. Undo closes it.
    let before = s.session.document().clone();
    review::apply_pose(&mut s.session, s.open).unwrap();
    let m = compile(&s.session);
    for (path, start) in moving.iter().zip(&closed) {
        let now = leaf_min(&m, path);
        near(now[0], start[0], 1e-9);
        near(now[1], start[1] - TRAVEL, 1e-9);
        near(now[2], start[2], 1e-9);
    }
    assert_eq!(leaf_min(&m, &slide), slide_closed);
    // The transfers follow relative poses, so the open drawer keeps its holes.
    assert_holes(&s, &m, &[29., 125., 221.]);
    s.session.undo(s.session.revision(), actor()).unwrap();
    assert_eq!(s.session.document(), &before);

    // Travel beyond the 250 mm limit is refused, not clamped.
    let mut beyond = s.slider.clone();
    beyond.parameters.values.translation[2] = model::length(TRAVEL + 1.);
    let refused = model::prepare_joint_edit(&s.session, s.session.revision(), actor(), &beyond)
        .map(|p| Compiler::default().compile(p.prepared.document(), s.session.revision()));
    assert!(
        !matches!(refused, Ok(Ok(_))),
        "travel beyond the limit evaluated"
    );

    // Closed interference: real clearances to the carcass, contact on the
    // slide and under the insert.
    let report = review::interference(&s, &compile(&s.session)).unwrap();
    let values: Vec<_> = report
        .pairs
        .iter()
        .map(|p| p.result.as_ref().unwrap().clone())
        .collect();
    for (v, clearance) in values.iter().zip([13., 13., 2., 82.]) {
        assert_eq!(v.relation, PairRelation::Separated);
        near(v.minimum_clearance_mm, clearance, 1e-6);
        near(v.overlap_volume_mm3, 0., 1e-9);
    }
    for v in &values[4..] {
        assert_eq!(v.relation, PairRelation::ContactWithinTolerance);
        near(v.overlap_volume_mm3, 0., 1e-6);
    }

    // Sampled travel against the actual assembly, stating its coverage.
    let revision = s.session.revision();
    let travel = review::travel(&s, 50.).unwrap();
    assert_eq!(
        travel.sample_values,
        [0., 50., 100., 150., 200., 250., 275.]
    );
    assert_eq!(travel.actual_intervals, [50., 50., 50., 50., 50., 25.]);
    assert_eq!(travel.coverage, RangeCoverage::SamplesOnly);
    for sample in &travel.samples[..6] {
        let pairs = &sample.result.as_ref().unwrap().pairs;
        assert_eq!(pairs.len(), 6);
        for (p, clearance) in pairs.iter().zip([13., 13., 2., 82., 0., 0.]) {
            let v = p.result.as_ref().unwrap();
            near(v.minimum_clearance_mm, clearance, 1e-6);
            near(v.overlap_volume_mm3, 0., 1e-6);
        }
    }
    let past = travel.samples[6].result.as_ref().unwrap_err();
    assert_eq!(past.kind, InspectionFailureKind::Invalid);
    assert!(past.message.contains("limits"), "{}", past.message);
    let summary = review::travel_summary(&s, &travel);
    assert!(
        summary["statement"]
            .as_str()
            .unwrap()
            .contains("not checked")
    );
    assert_eq!(s.session.document(), &before);
    assert_eq!(s.session.revision(), revision);

    // A drawer too tall for the case is a real overlap with the top panel:
    // its 4 mm walls reach 8 mm into the top.
    review::set_parameter(&mut s.session, s.drawer_height, "170").unwrap();
    let report = review::interference(&s, &compile(&s.session)).unwrap();
    let top = report.pairs[3].result.as_ref().unwrap();
    assert_eq!(top.relation, PairRelation::Overlapping);
    let walls = DRAWER_W * DRAWER_D - (DRAWER_W - 2. * WALL) * (DRAWER_D - 2. * WALL);
    near(top.overlap_volume_mm3, walls * 8., 1e-3);
    s.session.undo(s.session.revision(), actor()).unwrap();
    assert_eq!(s.session.document(), &before);
}

#[test]
fn printed_parts_carry_inspected_aids_that_never_change_them() {
    let s = review::build().unwrap();
    let m = compile(&s.session);
    let aids = m.fdm_aids(s.session.document()).unwrap();
    assert_eq!(aids.len(), 5);
    for aid in &aids {
        assert!(aid.issues.is_empty(), "{}: {:?}", aid.name, aid.issues);
    }
    for tab in aids.iter().filter(|a| a.name.starts_with("Drawer tab")) {
        assert_eq!(tab.target.part, s.drawer.part);
        assert_ne!(tab.target.relation, SolidPairRelation::Separated);
        near(tab.bounds_mm[0][2], review::DRAWER_Z, 1e-6);
        near(tab.bounds_mm[1][2] - tab.bounds_mm[0][2], 0.6, 1e-6);
    }
    let support = aids
        .iter()
        .find(|a| a.name == "Bit holder tunnel support")
        .unwrap();
    assert_eq!(support.target.part, s.insert.part);
    // Derived top: 0.2 mm under the 20 mm tunnel ceiling.
    near(support.target.distance_mm, 0.2, 1e-6);
    near(
        support.bounds_mm[1][2],
        review::DRAWER_Z + FLOOR + 20. - 0.2,
        1e-6,
    );
    // The printed Parts keep their exact volumes with their aids.
    near(volume(&m, s.drawer.part), drawer_volume(80., 3), 1e-3);
    near(volume(&m, s.insert.part), INSERT_VOLUME, 1e-3);
}

// ---------------------------------------------------------------------------
// Independent readers: nothing below calls the writers' code.

fn tool(name: &str, args: &[&str]) -> Option<String> {
    Command::new(name)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
}
/// RFC 4180 rows of a CSV file, skipping `#` comment lines.
fn csv_rows(text: &str) -> Vec<Vec<String>> {
    let mut rows = vec![];
    let mut row = vec![];
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = text
        .lines()
        .filter(|l| !l.starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    chars.push('\n');
    let mut it = chars.chars().peekable();
    while let Some(c) = it.next() {
        match (quoted, c) {
            (true, '"') if it.peek() == Some(&'"') => {
                it.next();
                cell.push('"');
            }
            (true, '"') => quoted = false,
            (true, c) => cell.push(c),
            (false, '"') => quoted = true,
            (false, ',') => row.push(std::mem::take(&mut cell)),
            (false, '\n') => {
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
            }
            (false, c) => cell.push(c),
        }
    }
    rows
}
/// DXF (group code, value) pairs.
fn dxf_pairs(text: &str) -> Vec<(i32, String)> {
    let lines: Vec<_> = text.lines().map(str::trim).collect();
    lines
        .chunks(2)
        .filter(|c| c.len() == 2)
        .map(|c| (c[0].parse().unwrap(), c[1].to_string()))
        .collect()
}
/// Stored entries of a ZIP archive by name.
fn zip_entries(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let u16le = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]) as usize;
    let u32le = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
    let eocd = (0..bytes.len() - 21)
        .rev()
        .find(|&i| u32le(i) == 0x0605_4b50)
        .unwrap();
    let (count, mut at) = (u16le(eocd + 10), u32le(eocd + 16));
    let mut out = vec![];
    for _ in 0..count {
        assert_eq!(u32le(at), 0x0201_4b50);
        let (method, size, name_len) = (u16le(at + 10), u32le(at + 20), u16le(at + 28));
        assert_eq!(method, 0, "stored entry");
        let skip = u16le(at + 30) + u16le(at + 32);
        let name = String::from_utf8(bytes[at + 46..at + 46 + name_len].to_vec()).unwrap();
        let local = u32le(at + 42);
        let data = local + 30 + u16le(local + 26) + u16le(local + 28);
        out.push((name, bytes[data..data + size].to_vec()));
        at += 46 + name_len + skip;
    }
    out
}
fn attribute(tag: &str, name: &str) -> Option<String> {
    let key = format!(" {name}=\"");
    let start = tag.find(&key)? + key.len();
    Some(tag[start..start + tag[start..].find('"')?].to_string())
}
/// One mesh object of a 3MF model: name, vertices and triangles.
struct MeshObject {
    id: String,
    name: String,
    vertices: Vec<[f64; 3]>,
    triangles: Vec<[usize; 3]>,
}
impl MeshObject {
    fn volume(&self) -> f64 {
        self.triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| self.vertices[i]);
                (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                    + a[2] * (b[0] * c[1] - b[1] * c[0]))
                    / 6.
            })
            .sum()
    }
    fn min_z(&self) -> f64 {
        self.vertices.iter().map(|v| v[2]).fold(f64::MAX, f64::min)
    }
}
/// The unit, mesh objects and build item object ids of a 3MF package.
fn three_mf(bytes: &[u8]) -> (String, Vec<MeshObject>, Vec<String>) {
    let entries = zip_entries(bytes);
    let names: Vec<_> = entries.iter().map(|(n, _)| n.as_str()).collect();
    assert!(names.contains(&"[Content_Types].xml") && names.contains(&"_rels/.rels"));
    let xml = entries
        .iter()
        .find(|(n, _)| n.ends_with(".model"))
        .map(|(_, b)| String::from_utf8(b.clone()).unwrap())
        .unwrap();
    let model_tag = &xml[xml.find("<model").unwrap()..];
    let unit = attribute(&model_tag[..model_tag.find('>').unwrap()], "unit").unwrap();
    let mut objects = vec![];
    for chunk in xml.split("<object ").skip(1) {
        let head = &chunk[..chunk.find('>').unwrap()];
        let body = &chunk[..chunk.find("</object>").unwrap()];
        let mut object = MeshObject {
            id: attribute(&format!(" {head}"), "id").unwrap(),
            name: attribute(&format!(" {head}"), "name").unwrap_or_default(),
            vertices: vec![],
            triangles: vec![],
        };
        for v in body.split("<vertex ").skip(1) {
            let tag = format!(" {}", &v[..v.find("/>").unwrap()]);
            object
                .vertices
                .push(["x", "y", "z"].map(|k| attribute(&tag, k).unwrap().parse().unwrap()));
        }
        for t in body.split("<triangle ").skip(1) {
            let tag = format!(" {}", &t[..t.find("/>").unwrap()]);
            object
                .triangles
                .push(["v1", "v2", "v3"].map(|k| attribute(&tag, k).unwrap().parse().unwrap()));
        }
        objects.push(object);
    }
    let build = &xml[xml.find("<build").unwrap()..];
    let items = build
        .split("<item ")
        .skip(1)
        .map(|i| attribute(&format!(" {i}"), "objectid").unwrap())
        .collect();
    (unit, objects, items)
}
fn written(receipt: &multicad_core::exchange::RunReceipt) -> Vec<PathBuf> {
    receipt
        .files
        .iter()
        .map(|f| Path::new(&receipt.directory).join(&f.artifact.output.filename))
        .collect()
}

#[test]
fn deliverables_are_validated_by_independent_readers() {
    let s = review::build().unwrap();
    let m = compile(&s.session);

    // Saved project: reopens equal and evaluates to the same drilling.
    let bytes = review::archive(&s.session).unwrap();
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(&reopened.project.document, s.session.document());
    let again = Compiler::default()
        .compile(&reopened.project.document, 0)
        .unwrap();
    assert_eq!(
        again.snapshot().transfer_counts,
        m.snapshot().transfer_counts
    );

    let root = tempfile::tempdir().unwrap();
    // Panel drawing: one A3 PDF page and one millimeter DXF.
    let drawing = written(&review::export(&s.session, s.drawing_set, root.path()).unwrap());
    let pdf_path = drawing
        .iter()
        .find(|p| p.extension().is_some_and(|e| e == "pdf"))
        .unwrap();
    let pdf = std::fs::read(pdf_path).unwrap();
    assert!(pdf.starts_with(b"%PDF-"));
    let pages = pdf
        .windows(b"/Type /Page".len() + 1)
        .filter(|w| w.starts_with(b"/Type /Page") && w[w.len() - 1] != b's')
        .count();
    assert_eq!(pages, 1);
    if let Some(info) = tool("pdfinfo", &[pdf_path.to_str().unwrap()]) {
        assert!(info.contains("Pages:           1"), "{info}");
        assert!(info.contains("(A3)"), "{info}");
    }
    if let Some(text) = tool(
        "pdftotext",
        &["-enc", "UTF-8", pdf_path.to_str().unwrap(), "-"],
    ) {
        for needle in [
            "Panel cut list",
            "Tool case left side",
            "TC-101",
            "Ø3",
            "300",
        ] {
            assert!(text.contains(needle), "PDF text lacks {needle}: {text:?}");
        }
    }
    let dxf_path = drawing
        .iter()
        .find(|p| p.extension().is_some_and(|e| e == "dxf"))
        .unwrap();
    let pairs = dxf_pairs(&std::fs::read_to_string(dxf_path).unwrap());
    let units = pairs
        .iter()
        .position(|(c, v)| *c == 9 && v == "$INSUNITS")
        .map(|i| pairs[i + 1].1.clone())
        .unwrap();
    assert_eq!(units, "4", "millimeters");
    let texts: Vec<_> = pairs
        .iter()
        .filter(|(c, _)| *c == 1)
        .map(|(_, v)| v.as_str())
        .collect();
    for needle in ["300", "200", "%%c3", "Tool case right side"] {
        assert!(
            texts.iter().any(|t| t.contains(needle)),
            "DXF lacks {needle}"
        );
    }
    // Six pilot holes: the writer tessellates each 3 mm circle (0.75 mm
    // radius at 1:2) into short visible LINEs. Cluster them and compare the
    // centers with the hole positions projected by hand: views at x = 40
    // (looking −X, +Y right) and x = 380 (looking +X, −Y right), model
    // z = 0 at 260 mm below the top of the 297 mm sheet.
    let mut clusters: Vec<Vec<[f64; 2]>> = vec![];
    for (i, _) in pairs
        .iter()
        .enumerate()
        .filter(|(_, (c, v))| *c == 0 && v == "LINE")
    {
        let fields: Vec<_> = pairs[i + 1..].iter().take_while(|(c, _)| *c != 0).collect();
        let get =
            |code: i32| -> String { fields.iter().find(|(c, _)| *c == code).unwrap().1.clone() };
        if get(8) != "VIEW_VISIBLE" {
            continue;
        }
        let [x1, y1, x2, y2] = [10, 20, 11, 21].map(|c| get(c).parse::<f64>().unwrap());
        if (x2 - x1).hypot(y2 - y1) >= 1. {
            continue;
        }
        let mid = [(x1 + x2) / 2., (y1 + y2) / 2.];
        match clusters
            .iter_mut()
            .find(|c| (c[0][0] - mid[0]).hypot(c[0][1] - mid[1]) < 2.)
        {
            Some(c) => c.extend([[x1, y1], [x2, y2]]),
            None => clusters.push(vec![mid, [x1, y1], [x2, y2]]),
        }
    }
    let mut centers: Vec<[f64; 2]> = clusters
        .iter()
        .map(|c| {
            let points = &c[1..];
            let n = points.len() as f64;
            let center = [
                points.iter().map(|p| p[0]).sum::<f64>() / n,
                points.iter().map(|p| p[1]).sum::<f64>() / n,
            ];
            for p in points {
                near((p[0] - center[0]).hypot(p[1] - center[1]), 0.75, 1e-3);
            }
            center
        })
        .collect();
    centers.sort_by(|a, b| a[0].total_cmp(&b[0]));
    let paper_z = 297. - 260. + HOLE_Z / 2.;
    let mut expected: Vec<f64> = [29., 125., 221.]
        .iter()
        .flat_map(|y| [40. + y / 2., 380. - y / 2.])
        .collect();
    expected.sort_by(f64::total_cmp);
    assert_eq!(centers.len(), 6, "{centers:?}");
    for (c, x) in centers.iter().zip(expected) {
        near(c[0], x, 1e-3);
        near(c[1], paper_z, 1e-3);
    }

    // BOM CSV: purchased hardware counted per occurrence, aids left out.
    let bom = csv_rows(&review::bom(&s.session, &m).unwrap().csv());
    assert_eq!(bom[0][..3], ["Item", "Part", "Qty"]);
    let row = |name: &str| bom.iter().find(|r| r[1] == name).unwrap().clone();
    for (name, qty, supply) in [
        ("Caster 60", "4", "purchased"),
        ("Slide 250 cabinet member", "2", "purchased"),
        ("Slide 250 drawer member", "2", "purchased"),
        ("Printed drawer", "1", "manufactured"),
        ("Printed bit holder", "1", "manufactured"),
        ("Tool case left side", "1", "manufactured"),
    ] {
        let r = row(name);
        assert_eq!((r[2].as_str(), r[5].as_str()), (qty, supply), "{name}");
    }
    assert_eq!(bom.len(), 1 + 9);
    assert!(
        !bom.iter()
            .any(|r| r[1].contains("tab") || r[1].contains("support"))
    );
    let model_bom = m
        .bom(
            s.session.document(),
            &multicad_model::schedules::BomOptions {
                include_aids: true,
                include_purchased: true,
                roots: None,
            },
        )
        .unwrap();
    assert_eq!(
        model_bom
            .rows
            .iter()
            .filter(|r| r.supply == Supply::Aid)
            .count(),
        5
    );
    // Cut list CSV: four 18 mm panels; the sides are 200 × 300.
    let cut = csv_rows(&review::cut_list(&s.session, &m).unwrap().csv());
    assert_eq!(cut.len(), 1 + 4);
    let side = cut.iter().find(|r| r[0] == "Tool case left side").unwrap();
    assert_eq!(side[3..6], ["18", "200", "300"]);

    // 3MF print packages, with and without the removable aids.
    let print = written(&review::export(&s.session, s.print_set, root.path()).unwrap());
    let package = |needle: &str| {
        let path = print
            .iter()
            .find(|p| p.to_string_lossy().contains(needle))
            .unwrap();
        three_mf(&std::fs::read(path).unwrap())
    };
    let (unit, with, items) = package("3MF_with_aids");
    assert_eq!(unit, "millimeter");
    assert_eq!(with.len(), 7);
    assert_eq!(items.len(), 7);
    let (_, without, items) = package("3MF_without_aids");
    assert_eq!(without.len(), 2);
    assert_eq!(items.len(), 2);
    for object in &without {
        assert!(items.contains(&object.id));
    }
    let by_name = |list: &[MeshObject], name: &str| -> f64 {
        list.iter()
            .find(|o| o.name.contains(name))
            .unwrap_or_else(|| panic!("no 3MF object {name}"))
            .volume()
    };
    for list in [&with, &without] {
        near(by_name(list, "drawer"), drawer_volume(80., 3), 5.);
        near(by_name(list, "bit holder"), INSERT_VOLUME, 1.);
    }
    let tabs: Vec<_> = with.iter().filter(|o| o.name.contains("tab")).collect();
    assert_eq!(tabs.len(), 4);
    for tab in tabs {
        near(tab.volume(), 10. * 10. * 0.6, 1e-3);
        near(tab.min_z(), review::DRAWER_Z, 1e-6);
    }
    near(by_name(&with, "support"), 76. * 56. * (20. - 0.2), 1e-3);
    assert!(!without.iter().any(|o| o.name.contains("tab")));
}

// ---------------------------------------------------------------------------
// An external MCP client changes the rail pitch through review.

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
fn external_client_changes_rail_pitch_through_review_with_undo_and_attribution() {
    let s = review::build().unwrap();
    let base = s.session.revision();
    let original = s.session.document().clone();
    let (left, right) = (
        s.transfers[0].targets[0].feature,
        s.transfers[3].targets[0].feature,
    );
    let host = Window(std::sync::Mutex::new(Reviewed {
        session: s.session,
        ledger: Default::default(),
        resources: Default::default(),
    }));
    let call = Call {
        actor: Actor {
            id: "mcp:tool-case".into(),
            channel: Channel::Mcp,
        },
        cancel: Arc::new(AtomicBool::new(false)),
    };
    let run = |name: &str, arguments: Value| dispatch(&host, &call, name, arguments).unwrap();
    let holes = |m: &Model| {
        [(left, PLY), (right, DRAWER_X + DRAWER_W)].map(|(f, x)| {
            review::hole_centers(m, f, x, PILOT_D / 2.)
                .into_iter()
                .map(|c| c[0])
                .collect::<Vec<_>>()
        })
    };

    let queued = run(
        "set_parameter",
        json!({ "expected_revision": base, "name": "rail_pitch",
            "expression": "80", "unit": "mm" }),
    );
    assert_eq!(queued["status"], "pending_review");
    let review_id = queued["review"].as_u64().unwrap();
    assert_eq!(
        run("review_status", json!({ "review": review_id }))["state"]["state"],
        "pending"
    );
    {
        // Preview in the owner: the candidate evaluates natively with the
        // moved holes while the document stays at its base.
        let owner = host.0.lock().unwrap();
        assert_eq!(owner.session.revision(), base);
        assert_eq!(owner.session.document(), &original);
        let pending = owner
            .ledger
            .pending()
            .iter()
            .find(|p| p.id == review_id)
            .unwrap();
        assert!(pending.affected.contains(&s.rail_pitch));
        let candidate = pending.candidate().unwrap();
        let preview = Compiler::default()
            .compile(candidate.document(), base)
            .unwrap();
        assert_eq!(
            holes(&preview),
            [vec![29., 109., 189.], vec![29., 109., 189.]]
        );
    }
    {
        let owner = &mut *host.0.lock().unwrap();
        assert_eq!(
            owner.ledger.accept(review_id, &mut owner.session).unwrap(),
            base + 1
        );
        assert_eq!(
            holes(&compile(&owner.session)),
            [vec![29., 109., 189.], vec![29., 109., 189.]]
        );
    }
    let history = run("history", json!({ "limit": 1 }));
    assert_eq!(history["entries"][0]["actor"], "mcp:tool-case");

    // Undo through the same review path restores the exact model.
    let queued = run("undo", json!({ "expected_revision": base + 1 }));
    assert_eq!(queued["status"], "pending_review");
    let undo_review = queued["review"].as_u64().unwrap();
    {
        let owner = &mut *host.0.lock().unwrap();
        owner
            .ledger
            .accept(undo_review, &mut owner.session)
            .unwrap();
        assert_eq!(owner.session.document(), &original);
        assert_eq!(
            holes(&compile(&owner.session)),
            [vec![29., 125., 221.], vec![29., 125., 221.]]
        );
    }
    // The audit keeps the preview, the application and the Undo, each
    // attributed to the client.
    let activity = run("activity", json!({ "limit": 20 }));
    let entries = activity["entries"].as_array().unwrap();
    let find = |outcome: &str, tool: &str| {
        entries
            .iter()
            .find(|e| e["outcome"] == outcome && e["tool"] == tool)
            .unwrap_or_else(|| panic!("no {outcome} {tool} in {entries:?}"))
    };
    for outcome in ["previewed", "applied"] {
        let e = find(outcome, "set_parameter");
        assert_eq!(e["arguments"]["name"], "rail_pitch");
        assert_eq!(e["actor"]["id"], "mcp:tool-case");
    }
    let undone = entries
        .iter()
        .find(|e| e["outcome"] == "undone")
        .unwrap_or_else(|| panic!("no undone entry in {entries:?}"));
    assert_eq!(undone["actor"]["id"], "mcp:tool-case");
}
