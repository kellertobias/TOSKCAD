//! The shipped C-channel front panel example (CAD-184) end to end: the
//! committed `examples/front-panel.tcd` opens with its name, description and
//! preview; the folded sheet, its developed blank and the bend values agree;
//! six buttons and the display sit in their openings and the printed end
//! parts fit inside the C without touching the sheet, with screws through
//! both flanges into their insert holes; parameter edits move openings,
//! components and end parts together and survive Undo/Redo and
//! save/reopen; and the generator still rebuilds the same project, whose
//! drawings, sheet STEP, developed DXF and per-end-part print sets are read
//! back independently.
#[path = "../examples/front_panel_demo.rs"]
#[allow(dead_code)]
mod demo;
use ::dxf::{Drawing, entities::EntityType};
use multicad_core::{
    commands::Session,
    document::{Document, EntityData, EntityId},
    drawings::ViewEvaluation,
    project_file,
};
use multicad_kernel::{BooleanOperation, GeometryPolicy, Solid};
use multicad_model::{Compiler, DrawingProviders, Model, SheetRecord};
use std::{
    collections::BTreeMap,
    f64::consts::PI,
    path::{Path, PathBuf},
};

fn near(a: f64, b: f64, tolerance: f64) {
    assert!((a - b).abs() <= tolerance, "{a} != {b} (±{tolerance})");
}
fn shipped_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(demo::SHIPPED)
}
fn open(bytes: &[u8]) -> Document {
    project_file::read(
        &mut &bytes[..],
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
/// Parts and parameters of an opened project, by name.
struct Ids {
    parts: BTreeMap<String, EntityId>,
    parameters: BTreeMap<String, EntityId>,
}
impl Ids {
    fn of(document: &Document) -> Self {
        let mut parts = BTreeMap::new();
        let mut parameters = BTreeMap::new();
        for e in document.entities() {
            match e.data {
                EntityData::Part { .. } => {
                    parts.insert(e.name.clone(), e.id);
                }
                EntityData::Parameter { .. } => {
                    parameters.insert(e.name.clone(), e.id);
                }
                _ => {}
            }
        }
        Self { parts, parameters }
    }
    fn part(&self, name: &str) -> EntityId {
        self.parts[name]
    }
}
/// World solids of every physical leaf of `part`.
fn leaves(model: &Model, part: EntityId) -> Vec<&Solid> {
    model
        .snapshot()
        .leaf_occurrences
        .values()
        .filter(|l| l.component == part)
        .map(|l| model.leaf_solid(&l.path).unwrap())
        .collect()
}
fn one(model: &Model, part: EntityId) -> &Solid {
    let all = leaves(model, part);
    assert_eq!(all.len(), 1);
    all[0]
}
fn common(a: &Solid, b: &Solid) -> f64 {
    a.boolean(b, BooleanOperation::Intersect, GeometryPolicy::default())
        .unwrap()
        .map_or(0., |s| s.volume_mm3().unwrap())
}
fn sheet(model: &Model, document: &Document) -> SheetRecord {
    let mut records = model.sheet_metal(document).unwrap();
    assert_eq!(records.len(), 1);
    records.remove(0)
}
fn allowance(r: f64, k: f64, t: f64) -> f64 {
    PI / 2. * (r + k * t)
}
/// Developed area: the flat front and two flanges with one allowance each,
/// minus the display window, six button holes and eight M3 clearance holes.
fn flat_area(width: f64, height: f64, leg: f64, r: f64, hole: f64) -> f64 {
    let circle = |d: f64| PI * d * d / 4.;
    width * (height + 2. * leg + 2. * allowance(r, 0.44, 1.5))
        - demo::DISPLAY_WINDOW[0] * demo::DISPLAY_WINDOW[1]
        - 6. * circle(hole)
        - 8. * circle(3.4)
}

/// Geometry checks shared by the shipped and edited states: the C section,
/// components in their openings, end parts fitted and fastened.
fn check_assembly(document: &Document, model: &Model, width: f64, height: f64, leg: f64, r: f64) {
    let ids = Ids::of(document);
    let t = 1.5;
    let panel = one(model, ids.part("Front panel sheet"));
    let b = panel.bounds().unwrap();
    // Folded C: flat front at y ∈ [−t, 0], flanges back to r + leg, outer
    // faces r + t beyond the flat height.
    near(b.min[0], 0., 1e-6);
    near(b.max[0], width, 1e-6);
    near(b.min[1], -t, 1e-6);
    near(b.max[1], r + leg, 1e-6);
    near(b.min[2], -(r + t), 1e-6);
    near(b.max[2], height + r + t, 1e-6);
    assert_eq!(panel.solid_count().unwrap(), 1);

    // Exactly six buttons and one display, each through its opening without
    // touching the sheet: the bushing crosses the sheet plane.
    let buttons = leaves(model, ids.part("Push button"));
    assert_eq!(buttons.len(), demo::BUTTONS);
    for button in &buttons {
        let bb = button.bounds().unwrap();
        assert!(bb.min[1] < -t && bb.max[1] > 0.);
        assert!(bb.min[0] > 0. && bb.max[0] < width);
        near(common(button, panel), 0., 1e-6);
    }
    for (i, a) in buttons.iter().enumerate() {
        for b in &buttons[i + 1..] {
            near(common(a, b), 0., 1e-6);
        }
    }
    let display = one(model, ids.part("Display module"));
    near(common(display, panel), 0., 1e-6);
    let hardware = model.sheet_hardware(document).unwrap();
    assert_eq!(hardware.len(), 3);
    let standoffs = hardware
        .iter()
        .find(|h| h.designation.starts_with("Hex standoff"))
        .unwrap();
    assert_eq!(standoffs.quantity, 4);
    near(common(display, one(model, standoffs.part)), 0., 1e-6);
    near(common(one(model, standoffs.part), panel), 0., 1e-6);

    // Printed end parts: separate Parts outside the panel ends whose plugs
    // fill the C with clearance; M3 screws pass the flanges' clearance holes
    // and end inside the insert pilot holes without touching either part.
    let ends = [ids.part("Left end part"), ids.part("Right end part")];
    let screws: Vec<_> = hardware
        .iter()
        .filter(|h| h.designation.starts_with("ISO 4762"))
        .collect();
    assert_eq!(screws.len(), 2);
    assert!(screws.iter().all(|h| h.quantity == 2 && h.hole_mm == 3.4));
    for (side, end) in ends.iter().enumerate() {
        let solid = one(model, *end);
        assert_eq!(solid.solid_count().unwrap(), 1);
        let eb = solid.bounds().unwrap();
        near(common(solid, panel), 0., 1e-6);
        near(common(solid, display), 0., 1e-6);
        for button in &buttons {
            near(common(solid, button), 0., 1e-6);
        }
        // Plate flush with the outer C profile, plug 16 mm into the C.
        near(eb.min[1], -t, 1e-6);
        near(eb.max[1], r + leg, 1e-6);
        near(eb.min[2], -(r + t), 1e-6);
        near(eb.max[2], height + r + t, 1e-6);
        if side == 0 {
            near(eb.min[0], -4., 1e-6);
            near(eb.max[0], 16., 1e-6);
        } else {
            near(eb.min[0], width - 16., 1e-6);
            near(eb.max[0], width + 4., 1e-6);
        }
        for h in &screws {
            near(common(solid, one(model, h.part)), 0., 1e-6);
        }
    }
    for h in &screws {
        let s = one(model, h.part);
        near(common(s, panel), 0., 1e-6);
        let sb = s.bounds().unwrap();
        // Each flange's screws sit 8 mm in from both panel ends, mid-leg.
        near(sb.min[0], 8. - 2.75, 1e-6);
        near(sb.max[0], width - 8. + 2.75, 1e-6);
        // Heads outside the flange; tips inside the plugs' Ø4 × 8 insert
        // pilot holes (the plug faces sit 0.3 mm inside the flanges).
        let plug = [-r + 0.3, height + r - 0.3];
        if sb.min[2] < 0. {
            near(sb.min[2], -(r + t) - 3., 1e-6);
            assert!(sb.max[2] > plug[0] && sb.max[2] < plug[0] + demo::INSERT_DEPTH);
        } else {
            near(sb.max[2], height + r + t + 3., 1e-6);
            assert!(sb.min[2] < plug[1] && sb.min[2] > plug[1] - demo::INSERT_DEPTH);
        }
    }
}

#[test]
fn shipped_example_opens_with_metadata_bends_openings_and_fitted_end_parts() {
    let bytes = std::fs::read(shipped_path()).expect("examples/front-panel.tcd is committed");
    let card = project_file::card(
        &mut &bytes[..],
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(card.info.name, demo::PROJECT_NAME);
    assert_eq!(card.info.description, demo::DESCRIPTION);
    let preview = card.info.preview.expect("a stored preview");
    assert_eq!((preview.width(), preview.height()), (640, 400));
    assert!(card.info_error.is_none());

    let document = open(&bytes);
    let session = Session::new(document).unwrap();
    let ids = Ids::of(session.document());
    for (name, expression, ..) in demo::PARAMETERS {
        let EntityData::Parameter { expression: e, .. } =
            &session.document().get(ids.parameters[name]).unwrap().data
        else {
            panic!()
        };
        assert_eq!(e, expression, "{name}");
    }
    let model = compile(&session);
    assert!(
        model
            .snapshot()
            .references
            .for_model(session.document())
            .unwrap()
            .all_resolved()
    );
    check_assembly(session.document(), &model, 260., 80., 40., 1.5);

    // Folded and flat agree: two 90° bends with the part's R and K, and a
    // developed width of front + legs + allowances.
    let record = sheet(&model, session.document());
    near(record.thickness_mm, 1.5, 1e-12);
    near(record.inside_radius_mm, 1.5, 1e-12);
    near(record.k_factor, 0.44, 1e-12);
    assert_eq!(record.bends.len(), 2);
    near(
        record.flat_size_mm[1],
        80. + 80. + 2. * allowance(1.5, 0.44, 1.5),
        1e-6,
    );
    near(record.flat_size_mm[0], 260., 1e-6);
    near(
        record.flat_area_mm2,
        flat_area(260., 80., 40., 1.5, 12.5),
        1e-6,
    );
    let pattern = model
        .unfold(session.document(), ids.part("Front panel sheet"))
        .unwrap();
    assert_eq!(pattern.drills.len(), 6 + 4 + 4);
    assert_eq!(pattern.bends.len(), 2);

    // Every drawing view evaluates, including the exploded arrangement and
    // the print-orientation views of the end parts.
    let drawings = model
        .drawings(
            session.document(),
            session.revision(),
            &DrawingProviders::default(),
        )
        .unwrap();
    assert_eq!(drawings.views.len(), 3 + 6 + 3 + 4);
    assert!(
        drawings
            .views
            .values()
            .all(|v| matches!(v, ViewEvaluation::Ready { .. }))
    );
}

#[test]
fn edits_move_openings_components_and_end_parts_through_undo_redo_and_reopen() {
    let bytes = std::fs::read(shipped_path()).unwrap();
    let mut session = Session::new(open(&bytes)).unwrap();
    let ids = Ids::of(session.document());
    let p = |name: &str| ids.parameters[name];

    demo::set_parameter(&mut session, p("panel_width"), "300").unwrap();
    demo::set_parameter(&mut session, p("flange_depth"), "50").unwrap();
    demo::set_parameter(&mut session, p("bend_radius"), "2").unwrap();
    demo::set_parameter(&mut session, p("button_pitch"), "40").unwrap();
    let model = compile(&session);
    check_assembly(session.document(), &model, 300., 80., 50., 2.);
    let record = sheet(&model, session.document());
    near(record.inside_radius_mm, 2., 1e-12);
    near(
        record.flat_area_mm2,
        flat_area(300., 80., 50., 2., 12.5),
        1e-6,
    );
    // The last button column moved with the pitch: 150 + 2 × 40.
    let xs: Vec<f64> = leaves(&model, ids.part("Push button"))
        .iter()
        .map(|b| {
            let bb = b.bounds().unwrap();
            (bb.min[0] + bb.max[0]) / 2.
        })
        .collect();
    assert!(xs.iter().any(|x| (x - 230.).abs() < 1e-6), "{xs:?}");

    // Undo restores the shipped width and pitch; Redo reapplies them.
    let revision = session.revision();
    for _ in 0..4 {
        session.undo(session.revision(), demo::actor()).unwrap();
    }
    check_assembly(session.document(), &compile(&session), 260., 80., 40., 1.5);
    for _ in 0..4 {
        session.redo(session.revision(), demo::actor()).unwrap();
    }
    assert!(session.revision() > revision);
    check_assembly(session.document(), &compile(&session), 300., 80., 50., 2.);

    // Save and reopen keep the edit, the bends, openings, mounting patterns,
    // the assembly and the file metadata.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("edited.tcd");
    std::fs::write(&path, demo::archive(&session).unwrap()).unwrap();
    let reopened = Session::new(open(&std::fs::read(&path).unwrap())).unwrap();
    let model = compile(&reopened);
    check_assembly(reopened.document(), &model, 300., 80., 50., 2.);
    let ids = Ids::of(reopened.document());
    let definition = multicad_core::model::read_sheet(
        reopened.document(),
        multicad_core::model::sheet_feature(reopened.document(), ids.part("Front panel sheet"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(definition.flanges.len(), 2);
    assert_eq!(definition.cutouts.len(), 1 + demo::BUTTONS);
    assert_eq!(definition.fasteners.len(), 3);
    let info = project_file::info::read(reopened.document()).unwrap();
    assert_eq!(info.description, demo::DESCRIPTION);
    assert!(info.preview.is_some());
}

/// Independent ZIP reading of stored entries.
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

#[test]
fn generator_rebuilds_the_shipped_project_and_its_deliverables() {
    let s = demo::build().unwrap();
    let shipped = open(&std::fs::read(shipped_path()).unwrap());
    // Same structure as the committed file: entity names by kind and every
    // parameter expression. Regenerate examples/front-panel.tcd when this
    // fails after an intended change.
    let signature = |d: &Document| {
        let mut v: Vec<_> = d
            .entities()
            .map(|e| {
                let kind = format!("{:?}", e.data);
                let kind = kind.split([' ', '(', '{']).next().unwrap().to_owned();
                (kind, e.name.clone())
            })
            .collect();
        v.sort();
        v
    };
    assert_eq!(signature(s.session.document()), signature(&shipped));
    assert_eq!(s.session.document().name(), shipped.name());

    let dir = tempfile::tempdir().unwrap();
    let manifest = demo::deliverables(&s, &dir.path().join("out")).unwrap();
    let files: Vec<PathBuf> = manifest["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| PathBuf::from(f["path"].as_str().unwrap()))
        .collect();
    for f in manifest["files"].as_array().unwrap() {
        let path = Path::new(f["path"].as_str().unwrap());
        assert_eq!(
            f["sha256"].as_str().unwrap(),
            project_file::ContentHash::of(&std::fs::read(path).unwrap()).to_string()
        );
    }
    let with = |ext: &str| -> Vec<&PathBuf> {
        files
            .iter()
            .filter(|f| f.extension().is_some_and(|e| e == ext))
            .collect()
    };
    // Four drawing pages in one PDF and one DXF per page.
    let pdf = std::fs::read(with("pdf")[0]).unwrap();
    assert!(pdf.starts_with(b"%PDF-"));
    let pages = pdf.windows(10).filter(|w| w == b"/Type /Pag").count();
    assert!(pages >= 4, "{pages}");
    // The developed DXF and the per-page drawing DXFs.
    let developed = files
        .iter()
        .find(|f| f.ends_with("front-panel-developed.dxf"))
        .unwrap();
    let drawing = Drawing::load_file(developed.to_str().unwrap()).unwrap();
    let count = |layer: &str, kind: fn(&EntityType) -> bool| {
        drawing
            .entities()
            .filter(|e| e.common.layer == layer && kind(&e.specific))
            .count()
    };
    // 14 drill points (six buttons, four standoffs, four end screws), their
    // 14 inner cut contours plus the display window, and the two bend lines.
    assert_eq!(
        count("DRILL", |e| matches!(e, EntityType::ModelPoint(_))),
        14
    );
    assert_eq!(
        count("CUT_INNER", |e| matches!(e, EntityType::LwPolyline(_))),
        15
    );
    assert_eq!(count("BEND_DOWN", |e| matches!(e, EntityType::Line(_))), 2);
    assert_eq!(count("CUT", |e| matches!(e, EntityType::LwPolyline(_))), 1);
    assert_eq!(with("dxf").len(), 4 + 1);
    // STEP: the sheet and one per end part; 3MF: exactly one per end part,
    // each holding a single mesh object in millimetres.
    assert_eq!(with("step").len(), 3);
    let three: Vec<_> = with("3mf");
    assert_eq!(three.len(), 2);
    for (path, name) in three.iter().zip(["Left_end_part", "Right_end_part"]) {
        assert!(
            path.file_name().unwrap().to_str().unwrap().contains(name),
            "{path:?}"
        );
        let xml =
            String::from_utf8(unzip(&std::fs::read(path).unwrap(), "3D/3dmodel.model")).unwrap();
        assert_eq!(xml.matches("<object ").count(), 1, "{path:?}");
        assert!(xml.contains("unit=\"millimeter\""));
    }
    assert_eq!(manifest["sheet"]["bends"], 2);
    assert_eq!(manifest["verification"]["physical_print_or_fit"], false);
    assert_eq!(manifest["verification"]["interactive_macos"], false);
}
