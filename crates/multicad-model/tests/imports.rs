//! STEP and KiCad imports: located exact solids with source identities,
//! reimport matching that keeps Parts, a board with outline, cutout and
//! mounting holes, components from linked STEP models or courtyard
//! envelopes with missing-model diagnostics, board reimport, mounting holes
//! driving a panel cutout through the transfer engine, and save/reopen.
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId},
    model::{imports::*, *},
    project_file::{self, ProjectFile},
    references::{AnalyticGeometry, GeometryIntent},
};
use multicad_kernel::{BooleanOperation, GeometryPolicy, RigidTransform, Solid};
use multicad_model::{
    Compiler, Model,
    imports::{KicadOptions, read_kicad_file, read_step_file},
};
use std::{f64::consts::PI, path::Path};

fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() <= 1e-3, "{a} != {b}");
}
fn compile(s: &Session) -> Model {
    Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap()
}
fn commit(s: &mut Session, p: ModelPrepared) {
    Compiler::default()
        .compile(p.prepared.document(), s.revision() + 1)
        .unwrap();
    s.commit(p.prepared).unwrap();
}
fn solid(m: &Model, part: EntityId) -> &Solid {
    let s = m.snapshot();
    let head = s
        .bodies
        .get(&part)
        .or_else(|| s.body_sources.get(&part))
        .unwrap();
    m.solid(*head).unwrap()
}
fn top(m: &Model, producer: EntityId, owner: EntityId, z: f64) -> AlignmentInput {
    let mut snapshot = m.snapshot().shapes[&producer].topology.clone();
    snapshot.owner = owner;
    let selected = snapshot
        .elements
        .iter()
        .find(|e| matches!(e.geometry, Some(AnalyticGeometry::Plane { frame }) if frame.normal()[2] > 0.999 && (frame.origin_mm[2] - z).abs() < 1e-7))
        .unwrap();
    AlignmentInput {
        source: owner,
        intent: GeometryIntent::capture(&snapshot, selected.key).unwrap(),
    }
}
/// The Part's current head producer.
fn transfer_head(m: &Model, part: EntityId) -> EntityId {
    let s = m.snapshot();
    *s.bodies
        .get(&part)
        .or_else(|| s.body_sources.get(&part))
        .unwrap()
}
fn boxes(path: &Path, parts: &[([f64; 3], [f64; 3])]) {
    let policy = GeometryPolicy::default();
    let mut all: Option<Solid> = None;
    for (size, at) in parts {
        let b = Solid::box_solid(*size, policy)
            .unwrap()
            .transformed(RigidTransform {
                translation_mm: *at,
                ..Default::default()
            })
            .unwrap();
        all = Some(match all {
            None => b,
            Some(a) => a
                .boolean(&b, BooleanOperation::Union, policy)
                .unwrap()
                .unwrap(),
        });
    }
    all.unwrap().write_step(path).unwrap();
}

#[test]
fn step_solids_import_as_parts_and_reimport_keeps_matching_parts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bracket.step");
    boxes(
        &path,
        &[
            ([10., 20., 30.], [0., 0., 0.]),
            ([5., 5., 5.], [100., 0., 0.]),
        ],
    );
    let mut s = Session::new(Document::new("STEP").unwrap()).unwrap();
    let file = read_step_file(&path).unwrap();
    assert_eq!(file.solids.len(), 2);
    let p = prepare_step_import(&s, 0, actor(), &file).unwrap();
    let source = p.source;
    let parts: Vec<_> = p.model.outputs.iter().map(|o| o.body).collect();
    commit(&mut s, p.model);
    let m = compile(&s);
    let mut volumes: Vec<f64> = parts
        .iter()
        .map(|p| solid(&m, *p).volume_mm3().unwrap())
        .collect();
    volumes.sort_by(f64::total_cmp);
    near(volumes[0], 125.);
    near(volumes[1], 6000.);
    // The far solid keeps its file placement.
    let far = parts
        .iter()
        .find(|p| solid(&m, **p).volume_mm3().unwrap() < 200.)
        .unwrap();
    near(solid(&m, *far).bounds().unwrap().min[0], 100.);
    let near_part = *parts.iter().find(|p| *p != far).unwrap();

    // Reimport: the big box is unchanged, the small one moved and a third added.
    boxes(
        &path,
        &[
            ([10., 20., 30.], [0., 0., 0.]),
            ([5., 5., 5.], [200., 0., 0.]),
            ([1., 1., 1.], [-50., 0., 0.]),
        ],
    );
    let file = read_step_file(&path).unwrap();
    let (plan, p) = prepare_step_update(&s, s.revision(), actor(), source, &file).unwrap();
    assert_eq!(plan.matched.len(), 1);
    assert!(!plan.matched[0].2);
    // Unnamed solids cannot be matched once their geometry changes.
    assert_eq!(plan.missing.len(), 1);
    assert_eq!(plan.added.len(), 2);
    commit(&mut s, p.model);
    let m = compile(&s);
    near(solid(&m, near_part).volume_mm3().unwrap(), 6000.);
    // The missing solid's Part keeps its last geometry and stays linked.
    near(solid(&m, *far).bounds().unwrap().min[0], 100.);
    let stored = read_step_source(s.document(), source).unwrap();
    assert_eq!(stored.solids.iter().filter(|s| s.missing).count(), 1);
    assert_eq!(step_bodies(s.document()).count(), 4);
}

#[test]
fn named_solids_match_by_name_when_their_geometry_changes() {
    let sig = |v: f64| SolidSignature {
        volume_mm3: v,
        min: [0.; 3],
        max: [1.; 3],
    };
    let source = StepSource {
        file_name: "a.step".into(),
        path: None,
        content: multicad_core::project_file::ContentHash::of(b"a"),
        solids: vec![ImportedSolid {
            id: "solid-1".into(),
            name: "Lid".into(),
            signature: sig(10.),
            brep: String::new(),
            missing: false,
        }],
    };
    let file = StepFile {
        file_name: "a.step".into(),
        path: None,
        content: multicad_core::project_file::ContentHash::of(b"b"),
        solids: vec![("Lid".into(), String::new(), sig(12.))],
    };
    let plan = plan_step_update(&source, &file);
    assert_eq!(plan.matched, vec![("solid-1".to_string(), 0, true)]);
    assert!(plan.added.is_empty() && plan.missing.is_empty());
}

const BOARD: &str = r#"(kicad_pcb (version 20240108) (generator "pcbnew")
  (general (thickness 1.6))
  (gr_line (start 0 0) (end 95 0) (layer "Edge.Cuts") (stroke (width 0.1)))
  (gr_arc (start 95 0) (mid 98.5355339 1.4644661) (end 100 5) (layer "Edge.Cuts"))
  (gr_line (start 100 5) (end 100 60) (layer "Edge.Cuts"))
  (gr_line (start 100 60) (end 0 60) (layer "Edge.Cuts"))
  (gr_line (start 0 60) (end 0 0) (layer "Edge.Cuts"))
  (gr_circle (center 50 30) (end 55 30) (layer "Edge.Cuts"))
  (gr_text "rev A" (at 10 10) (layer "F.SilkS"))
  (via (at 10 10) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu"))
  (footprint "MountingHole:MountingHole_3.2mm_M3" (layer "F.Cu") (at 5 5)
    (property "Reference" "H1" (at 0 0) (layer "F.SilkS"))
    (property "Value" "M3" (at 0 0) (layer "F.Fab"))
    (pad "" np_thru_hole circle (at 0 0) (size 3.2 3.2) (drill 3.2) (layers "*.Cu")))
  (footprint "MountingHole:MountingHole_3.2mm_M3" (layer "F.Cu") (at 5 55)
    (property "Reference" "H2" (at 0 0) (layer "F.SilkS"))
    (pad "" np_thru_hole circle (at 0 0) (size 3.2 3.2) (drill 3.2) (layers "*.Cu")))
  (footprint "Package:SOIC" (layer "F.Cu") (at 30 20 90)
    (property "Reference" "U1" (at 0 0) (layer "F.SilkS"))
    (property "Value" "MCU" (at 0 0) (layer "F.Fab"))
    (fp_rect (start -5 -4) (end 5 4) (layer "F.CrtYd"))
    (pad "1" thru_hole circle (at -3 0) (size 1.5 1.5) (drill 0.8) (layers "*.Cu"))
    (model "${KIPRJMOD}/u1.step" (offset (xyz 0 0 0)) (scale (xyz 1 1 1)) (rotate (xyz 0 0 0))))
  (footprint "Connector:J" (layer "F.Cu") (at 70 40)
    (property "Reference" "J1" (at 0 0) (layer "F.SilkS"))
    (fp_line (start -3 -2) (end 3 -2) (layer "F.CrtYd"))
    (fp_line (start 3 -2) (end 3 2) (layer "F.CrtYd"))
    (fp_line (start 3 2) (end -3 2) (layer "F.CrtYd"))
    (fp_line (start -3 2) (end -3 -2) (layer "F.CrtYd"))
    (model "${MULTICAD_TEST_UNSET_DIR}/Connector.3dshapes/J.wrl"))
  (footprint "Resistor:R" (layer "B.Cu") (at 60 10)
    (property "Reference" "R1" (at 0 0) (layer "B.SilkS"))
    (fp_rect (start -1 -0.5) (end 1 0.5) (layer "B.CrtYd")))
)"#;
fn board_area() -> f64 {
    6000. - (25. - PI * 25. / 4.) - PI * 25. - 2. * PI * 1.6 * 1.6
}

#[test]
fn kicad_board_components_and_reimport() {
    let dir = tempfile::tempdir().unwrap();
    boxes(
        &dir.path().join("u1.step"),
        &[([10., 8., 3.], [-5., -4., 0.])],
    );
    let board_path = dir.path().join("control.kicad_pcb");
    std::fs::write(&board_path, BOARD).unwrap();
    let board_path = board_path.canonicalize().unwrap();
    let import = read_kicad_file(&board_path, &KicadOptions::default()).unwrap();
    assert_eq!(import.board.holes.len(), 2, "only the M3 mounting holes");
    assert_eq!(import.models.len(), 1);
    assert!(import.board.unsupported.iter().any(|u| u.contains("vias")));
    let mut s = Session::new(Document::new("KiCad").unwrap()).unwrap();
    let p = prepare_kicad_import(&s, 0, actor(), &import).unwrap();
    assert_eq!(p.missing_models.len(), 1);
    assert_eq!(p.missing_models[0].0, "J1");
    let board = p.board.clone();
    let parts: std::collections::BTreeMap<_, _> = p
        .components
        .iter()
        .map(|(r, o, m)| (r.clone(), (o.clone(), *m)))
        .collect();
    assert!(parts["U1"].1.is_some() && parts["J1"].1.is_none());
    commit(&mut s, p.model);
    let m = compile(&s);
    near(
        solid(&m, board.body).volume_mm3().unwrap(),
        board_area() * 1.6,
    );
    // U1 is its STEP model, turned 90° and standing on the board top.
    let u1 = solid(&m, parts["U1"].0.body);
    near(u1.volume_mm3().unwrap(), 240.);
    let b = u1.bounds().unwrap();
    for (v, e) in b
        .min
        .iter()
        .zip([26., -25., 1.6])
        .chain(b.max.iter().zip([34., -15., 4.6]))
    {
        near(*v, e);
    }
    // J1's model is missing: a 6 × 4 courtyard envelope 5 mm high.
    let j1 = solid(&m, parts["J1"].0.body);
    near(j1.volume_mm3().unwrap(), 120.);
    near(j1.bounds().unwrap().min[1], -42.);
    // R1 is on the back, below the board.
    let r1 = solid(&m, parts["R1"].0.body).bounds().unwrap();
    near(r1.min[2], -5.);
    near(r1.max[2], 0.);

    // Reimport with U1 moved and J1 removed: Parts are kept.
    let moved = BOARD
        .replace("(at 30 20 90)", "(at 40 20 90)")
        .replace("\"Connector:J\"", "\"Connector:Gone\"")
        .replace("\"Reference\" \"J1\"", "\"Reference\" \"J9\"");
    std::fs::write(&board_path, moved).unwrap();
    let mut again = read_kicad_file(&board_path, &KicadOptions::default()).unwrap();
    again.components = Some(vec!["U1".into(), "H1".into(), "H2".into(), "R1".into()]);
    let (plan, p) = prepare_kicad_update(&s, s.revision(), actor(), board.feature, &again).unwrap();
    assert_eq!(plan.moved, vec!["U1".to_string()]);
    assert_eq!(plan.missing, vec!["J1".to_string()]);
    assert!(plan.added.is_empty());
    assert!(!plan.outline_changed && !plan.holes_changed);
    commit(&mut s, p);
    let m = compile(&s);
    near(solid(&m, parts["U1"].0.body).bounds().unwrap().min[0], 36.);
    near(solid(&m, parts["J1"].0.body).volume_mm3().unwrap(), 120.);

    // A replacement model for the missing connector.
    let file = read_step_file(&dir.path().join("u1.step")).unwrap();
    let p = prepare_step_import(&s, s.revision(), actor(), &file).unwrap();
    let replacement = p.source;
    commit(&mut s, p.model);
    let j1 = components(s.document())
        .find(|e| {
            read_component(s.document(), e.id)
                .unwrap()
                .footprint
                .reference
                == "J1"
        })
        .unwrap()
        .id;
    let p = prepare_component_model(&s, s.revision(), actor(), j1, Some(replacement)).unwrap();
    // The replacement takes the missing model's place at J1's position.
    commit(&mut s, p);
    let m = compile(&s);
    let j1 = solid(&m, parts["J1"].0.body);
    near(j1.volume_mm3().unwrap(), 240.);
    near(j1.bounds().unwrap().min[0], 65.);
    near(j1.bounds().unwrap().min[2], 1.6);

    // Save and reopen keep the imports.
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(s.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let opened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    assert!(opened.capabilities().contains(BOARD_CAPABILITY));
    assert!(opened.capabilities().contains(STEP_SOURCE_CAPABILITY));
    assert_eq!(&opened.document, s.document());
    let reopened = Compiler::default().compile(&opened.document, 0).unwrap();
    near(
        solid(&reopened, board.body).volume_mm3().unwrap(),
        board_area() * 1.6,
    );
}

#[test]
fn malformed_boards_are_refused_with_reasons() {
    assert!(
        kicad::read_board("(not_a_board)", 2.)
            .unwrap_err()
            .contains("Not a KiCad board")
    );
    let open = r#"(kicad_pcb (gr_line (start 0 0) (end 10 0) (layer "Edge.Cuts")))"#;
    assert!(
        kicad::read_board(open, 2.)
            .unwrap_err()
            .contains("not closed")
    );
    let none = r#"(kicad_pcb (general (thickness 1.6)))"#;
    assert!(
        kicad::read_board(none, 2.)
            .unwrap_err()
            .contains("no closed")
    );
}

/// The board's mounting holes drive a panel cutout through the transfer
/// engine.
#[test]
fn board_mounting_holes_transfer_into_a_mounting_panel() {
    let dir = tempfile::tempdir().unwrap();
    let board_path = dir.path().join("control.kicad_pcb");
    std::fs::write(&board_path, BOARD).unwrap();
    let import = read_kicad_file(
        &board_path,
        &KicadOptions {
            components: Some(vec!["J1".into()]),
            ..Default::default()
        },
    )
    .unwrap();
    let mut s = Session::new(Document::new("Mounting").unwrap()).unwrap();
    let p = prepare_kicad_import(&s, 0, actor(), &import).unwrap();
    let board = p.board.clone();
    let j1 = p.components[0].1.clone();
    commit(&mut s, p.model);
    let plate = PrimitiveRequest::new(
        "Mounting plate",
        PrimitiveParameters::Box {
            size: [120., 80., 3.].map(length),
            frame: PrimitiveFrame {
                origin: [-10., -70., 0.].map(length),
                ..Default::default()
            },
        },
    );
    let p = prepare_primitive(&s, s.revision(), actor(), &plate).unwrap();
    commit(&mut s, p);
    let mut members = vec![];
    for (body, z) in [(board.body, 10.), (plate.body, 0.), (j1.body, 10.)] {
        let member = ComponentPlacementRequest::new(
            body,
            NumericTransform {
                translation: [length(0.), length(0.), length(z)],
                ..Default::default()
            },
        );
        s.commit(
            prepare_place_component(&s, s.revision(), actor(), &member)
                .unwrap()
                .prepared,
        )
        .unwrap();
        members.push(member);
    }
    let assembly = AssemblyRequest::new(
        "Board on plate",
        members.iter().map(|m| m.instance).collect(),
        vec![board.body, plate.body, j1.body],
    );
    s.commit(
        prepare_assembly(&s, s.revision(), actor(), &assembly)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let outer = ComponentPlacementRequest::new(assembly.assembly, NumericTransform::default());
    s.commit(
        prepare_place_component(&s, s.revision(), actor(), &outer)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let m = compile(&s);
    let source = top(&m, board.feature, board.body, 1.6);
    let mut request = TransferRequest::new(
        "Board standoffs",
        TransferSource {
            component: board.body,
            path: OccurrencePath {
                root: outer.instance,
                members: vec![members[0].instance],
            },
            selection: TransferSelection::FaceHoles {
                face: source.intent.id,
            },
        },
    );
    request.references.push(source);
    let face = top(&m, plate.feature, plate.feature, 3.);
    request.targets.push(TransferTarget::new(
        BodyInput {
            body: plate.body,
            producer: plate.feature,
        },
        OccurrencePath {
            root: outer.instance,
            members: vec![members[1].instance],
        },
        face.intent.id,
        TransferTargetParameters::default(),
    ));
    request.references.push(face);
    let p = prepare_transfer(&s, s.revision(), actor(), &request).unwrap();
    commit(&mut s, p);
    let m = compile(&s);
    // The board's 50 mm-diameter cutout is also a hole in its top face, so it
    // transfers with the two M3 holes.
    let removed = (PI * 1.6 * 1.6 * 2. + PI * 25.) * 3.;
    near(
        solid(&m, plate.body).volume_mm3().unwrap(),
        120. * 80. * 3. - removed,
    );

    // The connector's envelope outline cuts a 6 × 4 mm opening for it.
    let source = top(&m, j1.feature, j1.body, 6.6);
    let mut request = TransferRequest::new(
        "Connector opening",
        TransferSource {
            component: j1.body,
            path: OccurrencePath {
                root: outer.instance,
                members: vec![members[2].instance],
            },
            selection: TransferSelection::FaceBoundary {
                face: source.intent.id,
            },
        },
    );
    request.references.push(source);
    let head = transfer_head(&m, plate.body);
    let face = top(&m, head, head, 3.);
    request.targets.push(TransferTarget::new(
        BodyInput {
            body: plate.body,
            producer: transfer_head(&m, plate.body),
        },
        OccurrencePath {
            root: outer.instance,
            members: vec![members[1].instance],
        },
        face.intent.id,
        TransferTargetParameters::default(),
    ));
    request.references.push(face);
    let p = prepare_transfer(&s, s.revision(), actor(), &request).unwrap();
    commit(&mut s, p);
    let m = compile(&s);
    near(
        solid(&m, plate.body).volume_mm3().unwrap(),
        120. * 80. * 3. - removed - 6. * 4. * 3.,
    );
}
