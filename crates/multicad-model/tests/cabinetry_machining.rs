//! Cabinetry machining: grooves, rabbets, pockets, face and edge drilling and
//! hardware patterns with per-material allowances cut into one or several
//! panels, with measured records, edits, refusals, deletion, Undo and
//! save/reopen. A drawer rail's holes reach a panel through the general
//! source-driven transfer engine.
use multicad_core::{
    commands::{Actor, Channel, Prepared, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Mutation},
    materials::*,
    model::*,
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    references::{AnalyticGeometry, GeometryIntent},
};
use multicad_model::{Compiler, Model};
use std::f64::consts::PI;

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
fn checked(s: &mut Session, prepared: Prepared) {
    Compiler::default()
        .compile(prepared.document(), s.revision() + 1)
        .unwrap();
    s.commit(prepared).unwrap();
}
fn volume(m: &Model, part: EntityId) -> f64 {
    let snapshot = m.snapshot();
    let head = snapshot
        .bodies
        .get(&part)
        .or_else(|| snapshot.body_sources.get(&part))
        .unwrap();
    m.solid(*head).unwrap().volume_mm3().unwrap()
}
fn bound(s: &Session, text: &str) -> Dimension {
    Dimension::Expression {
        expression: BoundExpression::bind(s.document(), text, "mm", None).unwrap(),
    }
}

struct Fixture {
    s: Session,
    birch: EntityId,
    mdf: EntityId,
    side: PanelRequest,
    other: PanelRequest,
    door: PanelRequest,
}
fn material(name: &str) -> Entity {
    Entity::new(
        name,
        EntityData::Material {
            density_kg_m3: None,
            color_rgba: [200, 180, 150, 255],
            section_hatch: None,
        },
    )
}
fn stock(material: EntityId, name: &str) -> Entity {
    LibraryDefinition::Stock(Stock {
        material,
        specification: StockKind::Panel {
            thickness_mm: 18.,
            length_mm: 2500.,
            width_mm: 1250.,
            grain: Grain::None,
        },
    })
    .entity(EntityId::new(), name)
    .unwrap()
}
fn panel(stock: EntityId, name: &str, origin: [f64; 3]) -> PanelRequest {
    PanelRequest::new(
        name,
        PanelDefinition {
            output: BodyOutput {
                body: EntityId::new(),
            },
            source: PanelSource::Rectangle {
                stock,
                plane: PanelPlane::Xy,
                origin: origin.map(length),
                length: length(600.),
                width: length(400.),
            },
            grain: GrainAxis::U,
            edges: PanelEdges::default(),
            oversize: None,
        },
    )
}
/// Two 600 × 400 × 18 birch sides and an MDF door, each on XY.
fn fixture() -> Fixture {
    let mut s = Session::new(Document::new("Machining").unwrap()).unwrap();
    let (birch, mdf) = (material("Birch plywood"), material("MDF"));
    let (ply, board) = (stock(birch.id, "Ply 18"), stock(mdf.id, "MDF 18"));
    let ids = (birch.id, mdf.id, ply.id, board.id);
    s.commit(prepare_library(&s, 0, actor(), &[birch, mdf, ply, board]).unwrap())
        .unwrap();
    let mut width = Entity::new(
        "groove_offset",
        EntityData::Parameter {
            expression: "20".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    width.description = "Back groove from the rear edge".into();
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Parameter".into(),
        edits: vec![Mutation::Insert(width)],
    })
    .unwrap();
    let side = panel(ids.2, "Left side", [0., 0., 0.]);
    let other = panel(ids.2, "Right side", [0., 0., 100.]);
    let door = panel(ids.3, "Door", [0., 0., 200.]);
    for p in [&side, &other, &door] {
        let prepared = prepare_panel(&s, s.revision(), actor(), p).unwrap();
        checked(&mut s, prepared.prepared);
    }
    Fixture {
        s,
        birch: ids.0,
        mdf: ids.1,
        side,
        other,
        door,
    }
}
const PANEL: f64 = 600. * 400. * 18.;

fn machine(s: &mut Session, name: &str, def: MachiningDefinition) -> MachiningPrepared {
    let p = prepare_machining(s, s.revision(), actor(), name, &def).unwrap();
    let out = p.clone();
    checked(s, p.model.prepared);
    out
}

#[test]
fn grooves_rabbets_pockets_and_holes_cut_every_target_panel() {
    let mut f = fixture();
    let both = vec![f.side.body, f.other.body];
    let offset = bound(&f.s, "=$groove_offset");
    // A 6 × 8 mm back groove along U, 20 mm (a parameter) from the v start.
    let groove = machine(
        &mut f.s,
        "Back groove",
        MachiningDefinition {
            suppressed: false,
            kind: MachiningKind::Groove {
                along: PanelAxis::U,
                offset,
                width: length(6.),
                depth: length(8.),
            },
            face: MachiningFace::Top,
            targets: both.clone(),
        },
    );
    assert_eq!(groove.model.outputs.len(), 2);
    let m = compile(&f.s);
    near(volume(&m, f.side.body), PANEL - 600. * 6. * 8.);
    near(volume(&m, f.other.body), PANEL - 600. * 6. * 8.);
    // A 10 × 9 rabbet on the bottom face along the u end edge.
    machine(
        &mut f.s,
        "Top rabbet",
        MachiningDefinition {
            suppressed: false,
            kind: MachiningKind::Rabbet {
                edge: PanelEdgeName::UEnd,
                width: length(10.),
                depth: length(9.),
            },
            face: MachiningFace::Bottom,
            targets: vec![f.side.body],
        },
    );
    // A pocket and blind and through holes.
    machine(
        &mut f.s,
        "Hinge cup",
        MachiningDefinition {
            suppressed: false,
            kind: MachiningKind::Pocket {
                center: [length(100.), length(200.)],
                size: [length(40.), length(30.)],
                depth: length(5.),
            },
            face: MachiningFace::Top,
            targets: vec![f.side.body],
        },
    );
    machine(
        &mut f.s,
        "Shelf pins",
        MachiningDefinition {
            suppressed: false,
            kind: MachiningKind::Drill {
                holes: vec![[length(300.), length(100.)], [length(300.), length(300.)]],
                diameter: length(5.),
                depth: Some(length(10.)),
            },
            face: MachiningFace::Top,
            targets: vec![f.side.body],
        },
    );
    let through = machine(
        &mut f.s,
        "Cable hole",
        MachiningDefinition {
            suppressed: false,
            kind: MachiningKind::Drill {
                holes: vec![[length(450.), length(200.)]],
                diameter: length(20.),
                depth: None,
            },
            face: MachiningFace::Top,
            targets: vec![f.side.body],
        },
    );
    // Dowel holes into the v-start edge, centered in the thickness.
    machine(
        &mut f.s,
        "Dowels",
        MachiningDefinition {
            suppressed: false,
            kind: MachiningKind::Drill {
                holes: vec![[length(50.), length(9.)], [length(550.), length(9.)]],
                diameter: length(8.),
                depth: Some(length(15.)),
            },
            face: MachiningFace::Edge {
                edge: PanelEdgeName::VStart,
            },
            targets: vec![f.side.body],
        },
    );
    let m = compile(&f.s);
    let pin = PI * 2.5 * 2.5 * 10.;
    let dowel = PI * 4. * 4. * 15.;
    let cable = PI * 10. * 10. * 18.;
    // The rabbet is on the bottom face, so it does not meet the top groove.
    near(
        volume(&m, f.side.body),
        PANEL - 600. * 6. * 8. - 400. * 10. * 9. - 40. * 30. * 5. - 2. * pin - cable - 2. * dowel,
    );
    // Records: every cut, its holes and recesses, measured removal.
    let records = m.machining(f.s.document()).unwrap();
    assert_eq!(records.len(), 7);
    let cable_rec = records
        .iter()
        .find(|r| r.operation == through.operation)
        .unwrap();
    assert!(cable_rec.holes[0].through);
    near(cable_rec.holes[0].depth_mm, 18.);
    near(cable_rec.removed_mm3, cable);
    assert_eq!(cable_rec.material, Some(f.birch));
    let dowels = records.iter().find(|r| r.name == "Dowels").unwrap();
    assert_eq!(dowels.holes.len(), 2);
    assert_eq!(dowels.holes[0].axis, [0., 1., 0.]);
    near(dowels.holes[0].center[2], 9.);
    near(dowels.removed_mm3, 2. * dowel);
    let groove_rec = records
        .iter()
        .find(|r| r.name == "Back groove" && r.part == f.side.body)
        .unwrap();
    assert_eq!(groove_rec.recesses[0].size_mm, [600., 6.]);
    let list = m.cut_list(f.s.document()).unwrap();
    let side = list.iter().find(|r| r.part == f.side.body).unwrap();
    assert_eq!(side.operations.len(), 6);
    near(side.cut.finished_mm[0], 600.);

    // A parameter moves the groove in both panels; Undo restores it.
    let before = f.s.document().clone();
    let param =
        f.s.document()
            .entities()
            .find(|e| e.name == "groove_offset")
            .unwrap()
            .clone();
    let mut edited = param.clone();
    if let EntityData::Parameter { expression, .. } = &mut edited.data {
        *expression = "394".into();
    }
    let p =
        f.s.prepare(Request {
            expected_revision: f.s.revision(),
            actor: actor(),
            label: "Move groove".into(),
            edits: vec![Mutation::Replace(edited)],
        })
        .unwrap();
    // 394 + 6 = 400 is the far edge: still inside the face.
    checked(&mut f.s, p);
    let m = compile(&f.s);
    let rec = m
        .machining(f.s.document())
        .unwrap()
        .into_iter()
        .find(|r| r.name == "Back groove" && r.part == f.other.body)
        .unwrap();
    near(rec.removed_mm3, 600. * 6. * 8.);
    f.s.undo(f.s.revision(), actor()).unwrap();
    assert_eq!(f.s.document(), &before);

    // Editing an operation rebuilds every target; deleting restores them.
    let mut deeper = read_machining(f.s.document(), groove.operation).unwrap();
    let MachiningKind::Groove { depth, .. } = &mut deeper.kind else {
        unreachable!()
    };
    *depth = length(10.);
    let p =
        prepare_machining_edit(&f.s, f.s.revision(), actor(), groove.operation, &deeper).unwrap();
    checked(&mut f.s, p.model.prepared);
    near(
        volume(&compile(&f.s), f.other.body),
        PANEL - 600. * 6. * 10.,
    );
    let p = prepare_machining_delete(&f.s, f.s.revision(), actor(), groove.operation).unwrap();
    checked(&mut f.s, p);
    let m = compile(&f.s);
    near(volume(&m, f.other.body), PANEL);
    assert_eq!(m.machining(f.s.document()).unwrap().len(), 5);

    // Save and reopen keep the operations and their solids.
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(f.s.document().clone()).unwrap(),
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
    assert!(opened.capabilities().contains(MACHINING_CAPABILITY));
    // Top/bottom/edge grooves, pockets and holes stay readable by builds
    // that predate shelf-pin rows and the inside face.
    assert!(!opened.capabilities().contains(SHELF_PIN_CAPABILITY));
    assert!(!opened.capabilities().contains(INSIDE_FACE_CAPABILITY));
    assert_eq!(&opened.document, f.s.document());
    let reopened = Compiler::default().compile(&opened.document, 0).unwrap();
    near(volume(&reopened, f.side.body), volume(&m, f.side.body));
}

#[test]
fn hardware_patterns_follow_their_source_and_material_allowances() {
    let mut f = fixture();
    // A hinge-plate pattern: two 5 mm holes 32 mm apart, 12 deep; MDF gets
    // 0.5 mm larger and 1 mm deeper holes.
    let pattern = HardwarePatternDefinition {
        holes: vec![
            HardwareHole {
                at: [length(0.), length(-16.)],
                diameter: length(5.),
                depth: Some(length(12.)),
            },
            HardwareHole {
                at: [length(0.), length(16.)],
                diameter: length(5.),
                depth: Some(length(12.)),
            },
        ],
        allowances: vec![MaterialAllowance {
            material: f.mdf,
            diameter: length(0.5),
            depth: length(1.),
        }],
    };
    let (hinge, p) =
        prepare_hardware(&f.s, f.s.revision(), actor(), "Hinge plate", &pattern).unwrap();
    checked(&mut f.s, p);
    machine(
        &mut f.s,
        "Hinge plates",
        MachiningDefinition {
            suppressed: false,
            kind: MachiningKind::Hardware {
                pattern: hinge,
                at: [length(100.), length(37.)],
                quarter_turns: 1,
            },
            face: MachiningFace::Top,
            targets: vec![f.side.body, f.door.body],
        },
    );
    let m = compile(&f.s);
    let birch = 2. * PI * 2.5 * 2.5 * 12.;
    let mdf = 2. * PI * 2.75 * 2.75 * 13.;
    near(volume(&m, f.side.body), PANEL - birch);
    near(volume(&m, f.door.body), PANEL - mdf);
    let records = m.machining(f.s.document()).unwrap();
    let door = records.iter().find(|r| r.part == f.door.body).unwrap();
    assert_eq!(door.material, Some(f.mdf));
    near(door.holes[0].diameter_mm, 5.5);
    // A quarter turn lays the pattern along u: holes at u = 100 ± 16.
    let mut us: Vec<f64> = door.holes.iter().map(|h| h.center[0]).collect();
    us.sort_by(f64::total_cmp);
    near(us[0], 84.);
    near(us[1], 116.);
    near(door.holes[0].center[1], 37.);

    // Editing the pattern rebuilds every placement.
    let mut wider = pattern.clone();
    wider.holes[1].at[1] = length(32.);
    let p = prepare_hardware_edit(&f.s, f.s.revision(), actor(), hinge, &wider).unwrap();
    checked(&mut f.s, p);
    let records = compile(&f.s).machining(f.s.document()).unwrap();
    let side = records.iter().find(|r| r.part == f.side.body).unwrap();
    let mut us: Vec<f64> = side.holes.iter().map(|h| h.center[0]).collect();
    us.sort_by(f64::total_cmp);
    near(us[1], 116.);
    near(us[0], 68.);
}

#[test]
fn unreachable_or_impossible_machining_is_refused() {
    let f = fixture();
    let refuse = |kind: MachiningKind, face: MachiningFace| {
        prepare_machining(
            &f.s,
            f.s.revision(),
            actor(),
            "Bad",
            &MachiningDefinition {
                suppressed: false,
                kind,
                face,
                targets: vec![f.side.body],
            },
        )
        .unwrap_err()
        .message
    };
    let top = MachiningFace::Top;
    let e = refuse(
        MachiningKind::Groove {
            along: PanelAxis::U,
            offset: length(10.),
            width: length(6.),
            depth: length(18.),
        },
        top,
    );
    assert!(e.contains("cut through"), "{e}");
    let e = refuse(
        MachiningKind::Groove {
            along: PanelAxis::V,
            offset: length(598.),
            width: length(6.),
            depth: length(8.),
        },
        top,
    );
    assert!(e.contains("does not fit"), "{e}");
    let e = refuse(
        MachiningKind::Drill {
            holes: vec![[length(2.), length(100.)]],
            diameter: length(8.),
            depth: None,
        },
        top,
    );
    assert!(e.contains("leaves the"), "{e}");
    let e = refuse(
        MachiningKind::Drill {
            holes: vec![[length(50.), length(9.)]],
            diameter: length(20.),
            depth: Some(length(20.)),
        },
        MachiningFace::Edge {
            edge: PanelEdgeName::VStart,
        },
    );
    assert!(e.contains("leaves the"), "{e}");
    let e = refuse(
        MachiningKind::Pocket {
            center: [length(10.), length(10.)],
            size: [length(40.), length(40.)],
            depth: length(5.),
        },
        top,
    );
    assert!(e.contains("leaves the face"), "{e}");
    let e = refuse(
        MachiningKind::Pocket {
            center: [length(100.), length(100.)],
            size: [length(40.), length(40.)],
            depth: length(5.),
        },
        MachiningFace::Edge {
            edge: PanelEdgeName::UStart,
        },
    );
    assert!(e.contains("top or bottom"), "{e}");
    let e = refuse(
        MachiningKind::Rabbet {
            edge: PanelEdgeName::VEnd,
            width: length(400.),
            depth: length(5.),
        },
        top,
    );
    assert!(e.contains("does not leave"), "{e}");
}

/// The drawer-rail example: a rail's holes reach a cabinetry panel through
/// the general transfer engine rather than a cabinetry-specific copy.
#[test]
fn rail_holes_transfer_into_a_cabinet_side_panel() {
    let f = fixture();
    let mut s = f.s;
    let rail = PrimitiveRequest::new(
        "Drawer rail",
        PrimitiveParameters::Box {
            size: [130., 20., 4.].map(length),
            frame: Default::default(),
        },
    );
    let p = prepare_primitive(&s, s.revision(), actor(), &rail).unwrap();
    checked(&mut s, p.prepared);
    let seed = PrimitiveRequest::new(
        "Rail hole",
        PrimitiveParameters::Cylinder {
            radius: length(3.),
            height: length(6.),
            frame: PrimitiveFrame {
                origin: [20., 10., -1.].map(length),
                ..Default::default()
            },
        },
    );
    let p = prepare_primitive(&s, s.revision(), actor(), &seed).unwrap();
    checked(&mut s, p.prepared);
    let cut = BooleanRequest::new(
        Operation::Cut,
        BodyInput {
            body: rail.body,
            producer: rail.feature,
        },
        vec![BodyInput {
            body: seed.body,
            producer: seed.feature,
        }],
    );
    let p = prepare_boolean(&s, s.revision(), actor(), &cut).unwrap();
    checked(&mut s, p.prepared);
    let mut members = vec![];
    for (body, z) in [(rail.body, 18.), (f.side.body, 0.)] {
        let member = ComponentPlacementRequest::new(
            body,
            NumericTransform {
                translation: [length(0.), length(0.), length(z)],
                ..Default::default()
            },
        );
        let p = prepare_place_component(&s, s.revision(), actor(), &member).unwrap();
        s.commit(p.prepared).unwrap();
        members.push(member);
    }
    let assembly = AssemblyRequest::new(
        "Drawer side",
        members.iter().map(|m| m.instance).collect(),
        vec![rail.body, f.side.body],
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
    let top = |producer: EntityId, owner: EntityId, z: f64| {
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
    };
    let source = top(cut.feature, rail.body, 4.);
    let mut request = TransferRequest::new(
        "Rail screws",
        TransferSource {
            component: rail.body,
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
    let face = top(f.side.feature, f.side.feature, 18.);
    request.targets.push(TransferTarget::new(
        BodyInput {
            body: f.side.body,
            producer: f.side.feature,
        },
        OccurrencePath {
            root: outer.instance,
            members: vec![members[1].instance],
        },
        face.intent.id,
        TransferTargetParameters {
            diameter: Some(length(4.)),
            depth: TransferDepth::Blind { depth: length(12.) },
            ..Default::default()
        },
    ));
    request.references.push(face);
    let p = prepare_transfer(&s, s.revision(), actor(), &request).unwrap();
    checked(&mut s, p.prepared);
    let m = compile(&s);
    near(volume(&m, f.side.body), PANEL - PI * 2. * 2. * 12.);
    // The panel is still a cut-list panel with its stock.
    let list = m.cut_list(s.document()).unwrap();
    let side = list.iter().find(|r| r.part == f.side.body).unwrap();
    assert!(side.issues.is_empty(), "{:?}", side.issues);
    near(side.cut.finished_mm[0], 600.);
}
