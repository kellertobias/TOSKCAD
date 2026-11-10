//! Cabinetry panels: a parametric carcass generating named panel Parts with
//! finished and core sizes, parameter edits and Undo, shelf and back
//! changes, explicit banding and grain on rectangle panels, sketch-region
//! panels, face-to-face stacks, and save/reopen. Every size is measured from
//! the native solids.
use multicad_core::{
    commands::{Actor, Channel, Prepared, Request, Session},
    constraint_sketch::{Curve, CurveKind, Dimension, source::Source},
    document::{Document, Entity, EntityData, EntityId, Mutation},
    manufacturing::PlanarEdge,
    materials::*,
    model::{
        self, Back, Band, BodyOutput, CarcassDefinition, CarcassRequest, Construction, GrainAxis,
        MachiningKind, PanelDefinition, PanelEdges, PanelJointKind, PanelPlane, PanelRequest,
        PanelRole, PanelSource, ProfileSource, ShelfLayout, Shelves, angle, length,
    },
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    references::{self, AnalyticGeometry, Attachment, GeometryIntent, Resolution},
    sketch::Datum,
    sketch_regions,
};
use multicad_model::{Compiler, CutRecord, GeometryBackend, Model};

fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() <= 1e-6, "{a} != {b}");
}
fn same_cut_record(actual: &CutRecord, expected: &CutRecord) {
    // Native integration can differ in its last bits after recompilation.
    near(actual.volume_mm3, expected.volume_mm3);
    let metadata = CutRecord {
        volume_mm3: expected.volume_mm3,
        ..actual.clone()
    };
    assert_eq!(&metadata, expected);
}
fn same_cut_records(actual: &[CutRecord], expected: &[CutRecord]) {
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.iter().zip(expected) {
        same_cut_record(actual, expected);
    }
}
fn pair(a: [f64; 2], b: [f64; 2]) {
    near(a[0], b[0]);
    near(a[1], b[1]);
}
fn compile(s: &Session) -> Model {
    Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap()
}
/// Evaluate the candidate natively, then commit it.
fn checked(s: &mut Session, prepared: Prepared) {
    Compiler::default()
        .compile(prepared.document(), s.revision() + 1)
        .unwrap();
    s.commit(prepared).unwrap();
}
fn insert(s: &mut Session, edits: Vec<Mutation>) {
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Fixture".into(),
        edits,
    })
    .unwrap();
}
fn record<'a>(list: &'a [CutRecord], name: &str) -> &'a CutRecord {
    list.iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("no cut record {name}"))
}

struct Fixture {
    s: Session,
    ply: EntityId,
    ply_material: EntityId,
    hdf: EntityId,
    width: EntityId,
}
fn material(name: &str, color: [u8; 4]) -> Entity {
    Entity::new(
        name,
        EntityData::Material {
            density_kg_m3: Some(650.),
            color_rgba: color,
            section_hatch: None,
        },
    )
}
fn stock(material: EntityId, thickness: f64, grain: Grain, name: &str) -> Entity {
    LibraryDefinition::Stock(Stock {
        material,
        specification: StockKind::Panel {
            thickness_mm: thickness,
            length_mm: 2500.,
            width_mm: 1250.,
            grain,
        },
    })
    .entity(EntityId::new(), name)
    .unwrap()
}
fn fixture() -> Fixture {
    let mut s = Session::new(Document::new("Cabinetry").unwrap()).unwrap();
    let birch = material("Birch plywood", [214, 186, 140, 255]);
    let fibre = material("HDF", [120, 96, 70, 255]);
    let ply = stock(birch.id, 18., Grain::AlongLength, "Birch ply 18");
    let hdf = stock(fibre.id, 6., Grain::None, "HDF 6");
    let mut width = Entity::new(
        "cab_width",
        EntityData::Parameter {
            expression: "600".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    width.description = "Outside cabinet width".into();
    let ids = (ply.id, birch.id, hdf.id, width.id);
    s.commit(prepare_library(&s, 0, actor(), &[birch, fibre]).unwrap())
        .unwrap();
    s.commit(prepare_library(&s, s.revision(), actor(), &[ply, hdf]).unwrap())
        .unwrap();
    insert(&mut s, vec![Mutation::Insert(width)]);
    Fixture {
        s,
        ply: ids.0,
        ply_material: ids.1,
        hdf: ids.2,
        width: ids.3,
    }
}
fn set_parameter(s: &mut Session, id: EntityId, expression: &str) {
    let mut entity = s.document().get(id).unwrap().clone();
    let EntityData::Parameter { expression: e, .. } = &mut entity.data else {
        unreachable!()
    };
    *e = expression.into();
    let prepared = s
        .prepare(Request {
            expected_revision: s.revision(),
            actor: actor(),
            label: "Edit parameter".into(),
            edits: vec![Mutation::Replace(entity)],
        })
        .unwrap();
    checked(s, prepared);
}
fn band(thickness: f64, allowance: f64) -> Band {
    Band {
        thickness: length(thickness),
        allowance: Some(length(allowance)),
    }
}
fn cabinet(f: &Fixture) -> CarcassDefinition {
    CarcassDefinition {
        cabinet: None,
        origin: [length(0.), length(0.), length(0.)],
        rotation: angle(0.),
        width: Dimension::Expression {
            expression: BoundExpression::bind(f.s.document(), "=$cab_width", "mm", None).unwrap(),
        },
        height: length(720.),
        depth: length(560.),
        stock: f.ply,
        construction: Construction::SidesFullHeight,
        back: Back::Inset {
            stock: f.hdf,
            recess: length(10.),
        },
        shelves: Shelves {
            layout: ShelfLayout::Even { count: 2 },
            setback: length(20.),
            clearance: length(1.),
        },
        front_band: Some(band(2., 0.5)),
        group: None,
    }
}
fn bounds(m: &Model, s: &Session, part: EntityId) -> multicad_kernel::Bounds {
    let head = m.snapshot().bodies[&part];
    let _ = s;
    m.solid(head).unwrap().bounds().unwrap()
}

fn explicit_cabinet(f: &Fixture) -> CarcassDefinition {
    let mut definition = cabinet(f);
    definition.width = length(800.);
    definition.height = length(2000.);
    definition.depth = length(600.);
    definition.cabinet = Some(model::CabinetOptions {
        toe_clearance: length(100.),
        shelf_stock: f.ply,
        shelves: vec![
            model::CabinetShelf {
                key: 0,
                underside: length(500.),
                depth: length(420.),
                stock: None,
                support: model::ShelfSupport::Fixed,
            },
            model::CabinetShelf {
                key: 3,
                underside: length(1053.5),
                depth: length(350.),
                stock: Some(f.hdf),
                support: model::ShelfSupport::System32 { level: 28 },
            },
        ],
        joinery: Default::default(),
        pins: Default::default(),
    });
    definition
}

#[test]
fn explicit_cabinet_floor_datum_independent_shelves_and_stable_rows_are_native() {
    let mut f = fixture();
    let definition = explicit_cabinet(&f);
    let created = model::prepare_carcass(
        &f.s,
        f.s.revision(),
        actor(),
        &CarcassRequest::new("Tall cabinet", definition.clone()),
    )
    .unwrap();
    let carcass = created.carcass;
    checked(&mut f.s, created.model.prepared);
    let native = compile(&f.s);
    let cuts = native.cut_list(f.s.document()).unwrap();
    let bottom = record(&cuts, "Tall cabinet bottom");
    near(bounds(&native, &f.s, bottom.part).min[2], 100.);
    let side = record(&cuts, "Tall cabinet left side");
    pair(side.cut.finished_mm, [2000., 600.]);
    near(bounds(&native, &f.s, side.part).min[2], 0.);
    let fixed = record(&cuts, "Tall cabinet shelf 1");
    pair(fixed.cut.finished_mm, [764., 420.]);
    near(fixed.cut.thickness_mm, 18.);
    let adjustable = record(&cuts, "Tall cabinet shelf 4");
    pair(adjustable.cut.finished_mm, [762., 350.]);
    near(adjustable.cut.thickness_mm, 6.);
    assert_eq!(adjustable.stock, f.hdf);
    near(bounds(&native, &f.s, adjustable.part).min[2], 1053.5);

    // Remove the first row and move the cabinet; the retained fourth shelf
    // still owns its original Part, stock, depth and local floor dimension.
    let mut edited = definition;
    edited.cabinet.as_mut().unwrap().shelves.remove(0);
    edited.origin = [length(200.), length(100.), length(50.)];
    edited.rotation = angle(90.);
    edited.back = Back::None;
    let p = model::prepare_carcass_edit(&f.s, f.s.revision(), actor(), carcass, &edited).unwrap();
    checked(&mut f.s, p.model.prepared);
    let result = compile(&f.s);
    let changed = result.cut_list(f.s.document()).unwrap();
    let retained = record(&changed, "Tall cabinet shelf 4");
    assert_eq!(retained.part, adjustable.part);
    assert_eq!(retained.stock, f.hdf);
    near(bounds(&result, &f.s, retained.part).min[2], 1103.5);
    near(bounds(&result, &f.s, bottom.part).min[2], 150.);
    assert_eq!(changed.len(), 5);
    f.s.undo(f.s.revision(), actor()).unwrap();
    same_cut_records(&compile(&f.s).cut_list(f.s.document()).unwrap(), &cuts);
    f.s.redo(f.s.revision(), actor()).unwrap();
    same_cut_records(&compile(&f.s).cut_list(f.s.document()).unwrap(), &changed);

    let project = ProjectFile::new(f.s.document().clone()).unwrap();
    assert!(project.capabilities().contains(model::CABINET_CAPABILITY));
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let mut old_reader = project_file::supported_capabilities();
    old_reader.remove(model::CABINET_CAPABILITY);
    assert!(project_file::read(&mut bytes.as_slice(), Default::default(), &old_reader).is_err());
    let opened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    assert_eq!(&opened.document, f.s.document());
    same_cut_records(
        &Compiler::default()
            .compile(&opened.document, 0)
            .unwrap()
            .cut_list(&opened.document)
            .unwrap(),
        &changed,
    );
}

#[test]
fn explicit_cabinet_bad_sizes_collisions_keys_and_support_levels_are_source_neutral() {
    let f = fixture();
    let original = f.s.document().clone();
    let definition = explicit_cabinet(&f);
    let mut invalid = vec![];
    let mut d = definition.clone();
    d.cabinet.as_mut().unwrap().toe_clearance = length(-1.);
    invalid.push((d, "Toe clearance"));
    let mut d = definition.clone();
    d.cabinet.as_mut().unwrap().shelves[0].depth = length(600.);
    invalid.push((d, "requested depth"));
    let mut d = definition.clone();
    d.cabinet.as_mut().unwrap().shelves[1].underside = length(501.);
    d.cabinet.as_mut().unwrap().shelves[1].support = model::ShelfSupport::Fixed;
    invalid.push((d, "overlaps"));
    let mut d = definition.clone();
    d.cabinet.as_mut().unwrap().shelves[1].support = model::ShelfSupport::System32 { level: 10000 };
    invalid.push((d, "does not fit"));
    let mut d = definition.clone();
    d.cabinet.as_mut().unwrap().shelves[1].key = 0;
    invalid.push((d, "distinct stable slots"));
    let mut d = definition;
    d.cabinet.as_mut().unwrap().shelves[0].underside = length(105.);
    invalid.push((d, "does not fit"));
    for (d, message) in invalid {
        let error = model::prepare_carcass(
            &f.s,
            f.s.revision(),
            actor(),
            &CarcassRequest::new("Invalid", d),
        )
        .expect_err("invalid cabinet refused");
        assert!(error.to_string().contains(message), "{error}");
        assert_eq!(f.s.document(), &original);
    }
}

#[test]
fn explicit_cabinet_default_stock_updates_inheriting_shelves_only() {
    let mut f = fixture();
    let definition = explicit_cabinet(&f);
    let created = model::prepare_carcass(
        &f.s,
        f.s.revision(),
        actor(),
        &CarcassRequest::new("Stock cabinet", definition.clone()),
    )
    .unwrap();
    let carcass = created.carcass;
    checked(&mut f.s, created.model.prepared);
    let before = compile(&f.s).cut_list(f.s.document()).unwrap();
    let thicker = stock(f.ply_material, 24., Grain::AlongLength, "Birch ply 24");
    let thicker_id = thicker.id;
    let p = prepare_library(&f.s, f.s.revision(), actor(), &[thicker]).unwrap();
    f.s.commit(p).unwrap();
    let original = f.s.document().clone();
    let mut changed = definition;
    changed.cabinet.as_mut().unwrap().shelf_stock = thicker_id;
    let p = model::prepare_carcass_edit(&f.s, f.s.revision(), actor(), carcass, &changed).unwrap();
    // Native preview is a candidate, not a source mutation.
    Compiler::default()
        .compile(p.model.prepared.document(), f.s.revision() + 1)
        .unwrap();
    assert_eq!(f.s.document(), &original);
    checked(&mut f.s, p.model.prepared);
    let after = compile(&f.s).cut_list(f.s.document()).unwrap();
    for name in [
        "Stock cabinet shelf 1",
        "Stock cabinet shelf 4",
        "Stock cabinet left side",
    ] {
        assert_eq!(record(&before, name).part, record(&after, name).part);
    }
    assert_eq!(record(&after, "Stock cabinet shelf 1").stock, thicker_id);
    near(
        record(&after, "Stock cabinet shelf 1").cut.thickness_mm,
        24.,
    );
    for name in ["Stock cabinet shelf 4", "Stock cabinet left side"] {
        same_cut_record(record(&after, name), record(&before, name));
    }
}

#[test]
fn cabinet_connections_follow_parameters_levels_joinery_and_user_owned_dependencies() {
    let mut f = fixture();
    let mut definition = explicit_cabinet(&f);
    definition.width = length(300.);
    definition.height = length(400.);
    definition.depth = length(220.);
    definition.back = Back::None;
    let depth = Dimension::Expression {
        expression: BoundExpression::bind(f.s.document(), "=$cab_width / 4", "mm", None).unwrap(),
    };
    let c = definition.cabinet.as_mut().unwrap();
    c.toe_clearance = length(20.);
    c.shelves[0].underside = length(200.);
    c.shelves[0].depth = depth.clone();
    c.shelves[1].depth = depth;
    c.shelves[1].support = model::ShelfSupport::System32 { level: 3 };
    let created = model::prepare_carcass(
        &f.s,
        f.s.revision(),
        actor(),
        &CarcassRequest::new("Connected", definition.clone()),
    )
    .unwrap();
    let id = created.carcass;
    checked(&mut f.s, created.model.prepared);
    let joints = |s: &Session| {
        model::panel_joints(s.document())
            .filter(|e| {
                model::read_panel_joint(s.document(), e.id)
                    .unwrap()
                    .generated_by
                    == Some(id)
            })
            .map(|e| e.id)
            .collect::<std::collections::BTreeSet<_>>()
    };
    let operations = |s: &Session| {
        model::machinings(s.document())
            .filter(|e| {
                model::read_machining(s.document(), e.id)
                    .unwrap()
                    .generated_by
                    == Some(id)
            })
            .map(|e| e.id)
            .collect::<Vec<_>>()
    };
    let original_joints = joints(&f.s);
    assert_eq!(original_joints.len(), 6);
    let pins = operations(&f.s);
    assert_eq!(pins.len(), 1);
    let list = compile(&f.s).cut_list(f.s.document()).unwrap();
    let adjustable = record(&list, "Connected shelf 4").part;
    let fixed = record(&list, "Connected shelf 1").part;
    let side = record(&list, "Connected left side").part;
    near(bounds(&compile(&f.s), &f.s, adjustable).min[2], 173.5);
    let holes = |s: &Session| {
        compile(s)
            .machining(s.document())
            .unwrap()
            .into_iter()
            .find(|r| r.operation == pins[0] && r.part == side)
            .unwrap()
            .holes
    };
    assert!(
        holes(&f.s)
            .iter()
            .any(|h| (h.center[1] - 133.).abs() < 1e-6)
    );
    set_parameter(&mut f.s, f.width, "640");
    assert_eq!(joints(&f.s), original_joints);
    assert_eq!(operations(&f.s), pins);
    assert!(
        holes(&f.s)
            .iter()
            .any(|h| (h.center[1] - 143.).abs() < 1e-6)
    );
    let current = compile(&f.s).cut_list(f.s.document()).unwrap();
    pair(
        record(&current, "Connected shelf 4").cut.finished_mm,
        [262., 160.],
    );
    // Moving the bottom datum keeps the selected adjustable level and moves
    // its pin support together, while the fixed shelf retains 200 mm.
    definition.cabinet.as_mut().unwrap().toe_clearance = length(30.);
    let before = f.s.document().clone();
    let p = model::prepare_carcass_edit(&f.s, f.s.revision(), actor(), id, &definition).unwrap();
    Compiler::default()
        .compile(p.model.prepared.document(), f.s.revision() + 1)
        .unwrap();
    assert_eq!(f.s.document(), &before);
    checked(&mut f.s, p.model.prepared);
    let m = compile(&f.s);
    near(bounds(&m, &f.s, adjustable).min[2], 183.5);
    near(bounds(&m, &f.s, fixed).min[2], 200.);
    assert_eq!(joints(&f.s), original_joints);
    assert_eq!(operations(&f.s), pins);
    let snapshot = f.s.document().clone();
    assert!(
        model::prepare_panel_joint_delete(
            &f.s,
            f.s.revision(),
            actor(),
            *original_joints.first().unwrap()
        )
        .is_err()
    );
    assert!(model::prepare_machining_delete(&f.s, f.s.revision(), actor(), pins[0]).is_err());
    assert_eq!(f.s.document(), &snapshot);
    // Switching connection family replaces only owned connection geometry.
    definition.cabinet.as_mut().unwrap().joinery =
        model::CabinetJoinery::Domino(Box::new(model::LooseTenonSpec {
            thickness: length(6.),
            width: length(20.),
            length: length(40.),
            count: 2,
            inset: length(60.),
            offset: None,
            face_depth: length(12.),
            edge_depth: length(30.),
            face_play: length(0.),
            edge_play: length(0.),
            fit: length(0.1),
        }));
    let p = model::prepare_carcass_edit(&f.s, f.s.revision(), actor(), id, &definition).unwrap();
    checked(&mut f.s, p.model.prepared);
    assert_eq!(joints(&f.s).len(), 6);
    assert!(joints(&f.s).is_disjoint(&original_joints));
    let m = compile(&f.s);
    let records = m.machining(f.s.document()).unwrap();
    assert_eq!(
        records
            .iter()
            .filter(|r| r.kind == "loose-tenon mortise")
            .count(),
        12
    );
    assert_eq!(records.iter().filter(|r| r.kind == "shelf pins").count(), 2);
    assert!(
        records
            .iter()
            .filter(|r| r.kind == "loose-tenon mortise")
            .all(|r| r.tenon.is_some())
    );
    // Making the fixed shelf adjustable removes its two fixed joints only.
    definition.cabinet.as_mut().unwrap().shelves[0].support =
        model::ShelfSupport::System32 { level: 0 };
    let p = model::prepare_carcass_edit(&f.s, f.s.revision(), actor(), id, &definition).unwrap();
    checked(&mut f.s, p.model.prepared);
    assert_eq!(joints(&f.s).len(), 4);
    assert_eq!(operations(&f.s), pins);
    assert!(
        !compile(&f.s)
            .machining(f.s.document())
            .unwrap()
            .iter()
            .any(|r| r.part == fixed && r.kind == "loose-tenon mortise")
    );
    let user = model::prepare_machining(
        &f.s,
        f.s.revision(),
        actor(),
        "User cable hole",
        &model::MachiningDefinition {
            generated_by: None,
            kind: model::MachiningKind::Drill {
                holes: vec![[length(20.), length(20.)]],
                diameter: length(3.),
                depth: None,
            },
            face: model::MachiningFace::Top,
            targets: vec![adjustable],
            suppressed: false,
        },
    )
    .unwrap();
    let user_id = user.operation;
    checked(&mut f.s, user.model.prepared);
    let mut remove = definition.clone();
    remove.cabinet.as_mut().unwrap().shelves.remove(1);
    let snapshot = f.s.document().clone();
    let error = model::prepare_carcass_edit(&f.s, f.s.revision(), actor(), id, &remove)
        .expect_err("user machining blocks panel removal");
    assert!(error.to_string().contains("User cable hole"), "{error}");
    assert_eq!(f.s.document(), &snapshot);
    let deleted = model::prepare_machining_delete(&f.s, f.s.revision(), actor(), user_id).unwrap();
    checked(&mut f.s, deleted);
    let p = model::prepare_carcass_edit(&f.s, f.s.revision(), actor(), id, &remove).unwrap();
    checked(&mut f.s, p.model.prepared);
    assert_eq!(joints(&f.s).len(), 4);
    assert_eq!(operations(&f.s), pins);
    assert!(f.s.document().get(adjustable).is_none());
    assert!(f.s.document().get(fixed).is_some());
}

#[test]
fn cabinet_receiving_cut_collisions_are_refused_before_apply_and_after_parameter_edits() {
    let mut f = fixture();
    let mut definition = explicit_cabinet(&f);
    definition.width = length(300.);
    definition.height = length(400.);
    definition.depth = length(220.);
    definition.back = Back::None;
    let c = definition.cabinet.as_mut().unwrap();
    c.toe_clearance = length(20.);
    c.pins.start = length(43.);
    c.shelves[0].underside = length(200.);
    c.shelves[0].depth = length(150.);
    c.shelves[1].depth = length(150.);
    c.shelves[1].support = model::ShelfSupport::System32 { level: 3 };
    let model::CabinetJoinery::Dowels(spec) = &mut c.joinery else {
        unreachable!()
    };
    spec.inset = Dimension::Expression {
        expression: BoundExpression::bind(f.s.document(), "=$cab_width / 10", "mm", None).unwrap(),
    };
    let created = model::prepare_carcass(
        &f.s,
        f.s.revision(),
        actor(),
        &CarcassRequest::new("Collision cabinet", definition.clone()),
    )
    .unwrap();
    let id = created.carcass;
    let upstream = created
        .panels
        .iter()
        .find(|(r, _)| *r == PanelRole::LeftSide)
        .unwrap()
        .1
        .feature;
    checked(&mut f.s, created.model.prepared);
    let original = f.s.document().clone();
    let mut invalid = definition.clone();
    let model::CabinetJoinery::Dowels(spec) = &mut invalid.cabinet.as_mut().unwrap().joinery else {
        unreachable!()
    };
    spec.inset = length(37.);
    let error = model::prepare_carcass_edit(&f.s, f.s.revision(), actor(), id, &invalid)
        .expect_err("dowel and pin collision refused");
    assert!(error.to_string().contains("receiving cuts"), "{error}");
    assert_eq!(f.s.document(), &original);
    invalid.width = length(70.);
    invalid.cabinet.as_mut().unwrap().shelves.clear();
    let error = model::prepare_carcass_edit(&f.s, f.s.revision(), actor(), id, &invalid)
        .expect_err("opposed blind depths refused");
    assert!(error.to_string().contains("overlap"), "{error}");
    let mut parameter = f.s.document().get(f.width).unwrap().clone();
    let EntityData::Parameter { expression, .. } = &mut parameter.data else {
        unreachable!()
    };
    *expression = "370".into();
    let candidate =
        f.s.prepare(Request {
            expected_revision: f.s.revision(),
            actor: actor(),
            label: "Colliding parameter edit".into(),
            edits: vec![Mutation::Replace(parameter)],
        })
        .unwrap();
    let error = Compiler::default()
        .compile(candidate.document(), f.s.revision() + 1)
        .err()
        .expect("parameter-only collision refused");
    assert!(error.to_string().contains("receiving cuts"), "{error}");
    assert_eq!(f.s.document(), &original);
    // A downstream failure still allows upstream face/sketch repair queries.
    Compiler::default()
        .compile_sources(candidate.document(), f.s.revision() + 1, &[upstream])
        .unwrap();
    let mut suppressed = vec![];
    for entity in model::machining_cuts(candidate.document()) {
        let mut entity = entity.clone();
        if let EntityData::Feature { suppressed, .. } = &mut entity.data {
            *suppressed = true;
        }
        suppressed.push(Mutation::Replace(entity));
    }
    let muted = candidate.document().edited(&suppressed).unwrap();
    Compiler::default()
        .compile(&muted, f.s.revision() + 1)
        .unwrap();
    assert!(
        model::check_cabinet_connections(f.s.document(), f.s.parameters(), id, Some(&|| true))
            .is_err()
    );
}

#[test]
fn carcass_generates_named_panels_with_finished_and_core_sizes() {
    let mut f = fixture();
    let created = model::prepare_carcass(
        &f.s,
        f.s.revision(),
        actor(),
        &CarcassRequest::new("Base cabinet", cabinet(&f)),
    )
    .unwrap();
    let carcass = created.carcass;
    assert_eq!(created.panels.len(), 7);
    checked(&mut f.s, created.model.prepared);
    let m = compile(&f.s);
    let list = m.cut_list(f.s.document()).unwrap();
    assert_eq!(list.len(), 7);
    for r in &list {
        assert!(r.issues.is_empty(), "{}: {:?}", r.name, r.issues);
        assert!(r.fits_stock, "{}", r.name);
        assert_eq!(r.carcass, Some(carcass));
    }

    // Sides: full height, grain up, front edge banded 2 mm with 0.5 allowance.
    let side = record(&list, "Base cabinet left side");
    assert_eq!(side.role, Some(PanelRole::LeftSide));
    assert_eq!(side.material, Some(f.ply_material));
    assert_eq!(side.stock, f.ply);
    pair(side.cut.finished_mm, [720., 560.]);
    pair(side.cut.core_mm, [720., 558.5]);
    near(side.cut.thickness_mm, 18.);
    near(side.volume_mm3, 18. * 560. * 720.);
    assert_eq!(side.bands.len(), 1);
    assert_eq!(side.bands[0].0, "u_start");
    // Top and bottom fit between the sides, grain along the width.
    let top = record(&list, "Base cabinet top");
    pair(top.cut.finished_mm, [564., 560.]);
    pair(top.cut.core_mm, [564., 558.5]);
    // The inset back sits 10 mm in from the rear, captured on all four sides.
    let back = record(&list, "Base cabinet back");
    assert_eq!(back.stock, f.hdf);
    pair(back.cut.finished_mm, [684., 564.]);
    near(back.cut.thickness_mm, 6.);
    assert!(back.bands.is_empty());
    let b = bounds(&m, &f.s, back.part);
    near(b.min[1], 544.);
    near(b.max[1], 550.);
    // Two evenly spaced shelves, 1 mm clear of each side, from the setback to the back.
    let shelf = record(&list, "Base cabinet shelf 1");
    pair(shelf.cut.finished_mm, [562., 524.]);
    pair(shelf.cut.core_mm, [562., 522.5]);
    near(bounds(&m, &f.s, shelf.part).min[2], 18. + 216.);
    let shelf2 = record(&list, "Base cabinet shelf 2");
    near(bounds(&m, &f.s, shelf2.part).min[2], 18. + 2. * 216. + 18.);
    // The panels are held by a group named after the carcass.
    let group = model::read_carcass(f.s.document(), carcass)
        .unwrap()
        .group
        .unwrap();
    let members = &f.s.document().hierarchy().members[&group];
    assert!(
        created
            .panels
            .iter()
            .all(|(_, o)| members.contains(&o.body))
    );

    // A parameter edit resizes the panels in place; Undo restores them.
    let before = f.s.document().clone();
    set_parameter(&mut f.s, f.width, "800");
    let wide = compile(&f.s).cut_list(f.s.document()).unwrap();
    pair(
        record(&wide, "Base cabinet top").cut.finished_mm,
        [764., 560.],
    );
    pair(
        record(&wide, "Base cabinet shelf 2").cut.core_mm,
        [762., 522.5],
    );
    pair(
        record(&wide, "Base cabinet back").cut.finished_mm,
        [684., 764.],
    );
    assert_eq!(record(&wide, "Base cabinet top").part, top.part);
    f.s.undo(f.s.revision(), actor()).unwrap();
    assert_eq!(f.s.document(), &before);

    // Explicit shelf offsets add a third shelf; existing shelves keep their Parts.
    let mut edited = cabinet(&f);
    edited.shelves.layout = ShelfLayout::Offsets {
        offsets: vec![length(100.), length(300.), length(500.)],
    };
    let p = model::prepare_carcass_edit(&f.s, f.s.revision(), actor(), carcass, &edited).unwrap();
    checked(&mut f.s, p.model.prepared);
    let m = compile(&f.s);
    let list = m.cut_list(f.s.document()).unwrap();
    assert_eq!(list.len(), 8);
    assert_eq!(record(&list, "Base cabinet shelf 1").part, shelf.part);
    let third = record(&list, "Base cabinet shelf 3");
    near(bounds(&m, &f.s, third.part).min[2], 518.);
    assert!(f.s.document().hierarchy().members[&group].contains(&third.part));
    // One shelf removes the others with their properties; Undo brings them back.
    let mut one = cabinet(&f);
    one.shelves.layout = ShelfLayout::Even { count: 1 };
    let with_three = f.s.document().clone();
    let p = model::prepare_carcass_edit(&f.s, f.s.revision(), actor(), carcass, &one).unwrap();
    checked(&mut f.s, p.model.prepared);
    assert!(f.s.document().get(third.part).is_none());
    assert!(
        part_properties(f.s.document(), third.part)
            .unwrap()
            .is_none()
    );
    let list = compile(&f.s).cut_list(f.s.document()).unwrap();
    assert_eq!(list.len(), 6);
    near(
        bounds(&compile(&f.s), &f.s, shelf.part).min[2],
        18. + (684. - 18.) / 2.,
    );
    f.s.undo(f.s.revision(), actor()).unwrap();
    assert_eq!(f.s.document(), &with_three);

    // An overlay back covers the whole rear; sides, top and shelves get shallower.
    let mut overlay = cabinet(&f);
    overlay.back = Back::Overlay { stock: f.hdf };
    overlay.construction = Construction::TopBottomFullWidth;
    let p = model::prepare_carcass_edit(&f.s, f.s.revision(), actor(), carcass, &overlay).unwrap();
    checked(&mut f.s, p.model.prepared);
    let list = compile(&f.s).cut_list(f.s.document()).unwrap();
    pair(
        record(&list, "Base cabinet back").cut.finished_mm,
        [720., 600.],
    );
    pair(
        record(&list, "Base cabinet left side").cut.finished_mm,
        [684., 554.],
    );
    pair(
        record(&list, "Base cabinet top").cut.finished_mm,
        [600., 554.],
    );
    pair(
        record(&list, "Base cabinet shelf 1").cut.finished_mm,
        [562., 534.],
    );

    // Save and reopen keep the editable carcass and identical cut records.
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
    assert!(opened.capabilities().contains(model::CARCASS_CAPABILITY));
    assert!(opened.capabilities().contains(model::PANEL_CAPABILITY));
    assert_eq!(&opened.document, f.s.document());
    let reopened = Compiler::default()
        .compile(&opened.document, 0)
        .unwrap()
        .cut_list(&opened.document)
        .unwrap();
    same_cut_records(&reopened, &list);

    // Sizes that cannot be built are refused before anything is staged.
    let mut crowded = cabinet(&f);
    crowded.shelves.layout = ShelfLayout::Even { count: 40 };
    let e =
        model::prepare_carcass_edit(&f.s, f.s.revision(), actor(), carcass, &crowded).unwrap_err();
    assert!(e.message.contains("do not fit"), "{}", e.message);
    let mut narrow = cabinet(&f);
    narrow.width = length(30.);
    assert!(model::prepare_carcass_edit(&f.s, f.s.revision(), actor(), carcass, &narrow).is_err());
    let mut overlapping = cabinet(&f);
    overlapping.shelves.layout = ShelfLayout::Offsets {
        offsets: vec![length(100.), length(110.)],
    };
    let e = model::prepare_carcass_edit(&f.s, f.s.revision(), actor(), carcass, &overlapping)
        .unwrap_err();
    assert!(e.message.contains("overlaps"), "{}", e.message);
}

#[test]
fn rotated_carcass_places_panels_about_its_origin() {
    let mut f = fixture();
    let mut def = cabinet(&f);
    def.origin = [length(1000.), length(500.), length(100.)];
    def.rotation = angle(90.);
    def.shelves.layout = ShelfLayout::Even { count: 0 };
    let created = model::prepare_carcass(
        &f.s,
        f.s.revision(),
        actor(),
        &CarcassRequest {
            group: false,
            ..CarcassRequest::new("Tall", def)
        },
    )
    .unwrap();
    assert_eq!(created.panels.len(), 5);
    checked(&mut f.s, created.model.prepared);
    let m = compile(&f.s);
    let left = created.panels[0].1.body;
    // Width now runs along +Y, depth along −X.
    let b = bounds(&m, &f.s, left);
    near(b.min[0], 1000. - 560.);
    near(b.max[0], 1000.);
    near(b.min[1], 500.);
    near(b.max[1], 518.);
    near(b.min[2], 100.);
    near(b.max[2], 820.);
    let list = m.cut_list(f.s.document()).unwrap();
    pair(
        record(&list, "Tall left side").cut.finished_mm,
        [720., 560.],
    );
}

#[test]
fn rectangle_panels_band_edges_follow_grain_and_stack_face_to_face() {
    let mut f = fixture();
    let rect = |stock: EntityId, z: f64, l: f64, w: f64, grain: GrainAxis| PanelDefinition {
        output: BodyOutput {
            body: EntityId::new(),
        },
        source: PanelSource::Rectangle {
            stock,
            plane: PanelPlane::Xy,
            origin: [length(0.), length(0.), length(z)],
            length: length(l),
            width: length(w),
        },
        grain,
        edges: PanelEdges::default(),
        oversize: None,
    };
    let mut top = rect(f.ply, 0., 800., 400., GrainAxis::U);
    top.edges = PanelEdges {
        u_start: Some(band(1., 0.)),
        u_end: Some(band(1., 0.)),
        v_start: Some(band(2., 1.)),
        v_end: None,
    };
    top.oversize = Some(length(2.));
    let a = PanelRequest::new("Desk top", top);
    let p = model::prepare_panel(&f.s, f.s.revision(), actor(), &a).unwrap();
    checked(&mut f.s, p.prepared);
    let b = PanelRequest::new("Desk lining", rect(f.hdf, 18., 800., 400., GrainAxis::Free));
    let p = model::prepare_panel(&f.s, f.s.revision(), actor(), &b).unwrap();
    checked(&mut f.s, p.prepared);
    let list = compile(&f.s).cut_list(f.s.document()).unwrap();
    let desk = record(&list, "Desk top");
    pair(desk.cut.finished_mm, [800., 400.]);
    // u loses two 1 mm bands; v loses 2 mm and keeps 1 mm allowance; both get 2 mm oversize.
    pair(desk.cut.core_mm, [800., 401.]);
    assert_eq!(desk.bands.len(), 3);
    assert_eq!(desk.carcass, None);

    // Grain decides whether a long panel fits the 2500 × 1250 stock with grain along its length.
    let along = PanelRequest::new("Rail", rect(f.ply, 100., 1300., 300., GrainAxis::U));
    let p = model::prepare_panel(&f.s, f.s.revision(), actor(), &along).unwrap();
    checked(&mut f.s, p.prepared);
    let mut across = rect(f.ply, 100., 1300., 300., GrainAxis::V);
    across.output.body = along.body;
    let p =
        model::prepare_panel_edit(&f.s, f.s.revision(), actor(), along.feature, &across).unwrap();
    let list = Compiler::default()
        .compile(p.prepared.document(), 0)
        .unwrap()
        .cut_list(p.prepared.document())
        .unwrap();
    assert!(!record(&list, "Rail").fits_stock);
    pair(record(&list, "Rail").cut.core_mm, [300., 1300.]);
    assert!(record(&compile(&f.s).cut_list(f.s.document()).unwrap(), "Rail").fits_stock);

    // A face-to-face stack keeps each layer's own Part, material and stock.
    let (stack, p) = model::prepare_stack(
        &f.s,
        f.s.revision(),
        actor(),
        "Desk laminate",
        &[a.body, b.body],
        Some("PVA".into()),
        true,
    )
    .unwrap();
    checked(&mut f.s, p);
    let m = compile(&f.s);
    let stacks = m.stacks(f.s.document()).unwrap();
    assert_eq!(stacks.len(), 1);
    let s0 = &stacks[0];
    assert!(s0.face_to_face, "{:?}", s0.issues);
    near(s0.total_thickness_mm, 24.);
    assert_eq!(
        s0.layers.iter().map(|l| l.stock).collect::<Vec<_>>(),
        [f.ply, f.hdf]
    );
    assert_ne!(s0.layers[0].material, s0.layers[1].material);
    assert_eq!(s0.adhesive.as_deref(), Some("PVA"));
    let list = m.cut_list(f.s.document()).unwrap();
    assert_eq!(record(&list, "Desk top").stack, Some(stack));
    assert_eq!(record(&list, "Desk lining").stack, Some(stack));
    let group = model::read_stack(f.s.document(), stack)
        .unwrap()
        .group
        .unwrap();
    assert_eq!(
        f.s.document().hierarchy().members[&group],
        vec![a.body, b.body]
    );

    // Moving a layer away is reported; the stack is not silently regrouped.
    let mut lifted = rect(f.hdf, 20., 800., 400., GrainAxis::Free);
    lifted.output.body = b.body;
    let p = model::prepare_panel_edit(&f.s, f.s.revision(), actor(), b.feature, &lifted).unwrap();
    checked(&mut f.s, p.prepared);
    let stacks = compile(&f.s).stacks(f.s.document()).unwrap();
    assert!(!stacks[0].face_to_face);
    assert!(
        stacks[0].issues[0].contains("2.000 mm apart"),
        "{:?}",
        stacks[0].issues
    );

    // Only panel Parts stack, and a layer belongs to one stack.
    let e = model::prepare_stack(
        &f.s,
        f.s.revision(),
        actor(),
        "Again",
        &[a.body, along.body],
        None,
        false,
    )
    .unwrap_err();
    assert!(e.message.contains("already a layer"), "{}", e.message);
    let e = model::prepare_stack(
        &f.s,
        f.s.revision(),
        actor(),
        "Odd",
        &[along.body, f.ply],
        None,
        false,
    )
    .unwrap_err();
    assert!(e.message.contains("not a panel"), "{}", e.message);
    // Carcass panels cannot be edited directly.
    let carcass_panel = PanelDefinition {
        output: BodyOutput { body: a.body },
        source: PanelSource::Carcass {
            carcass: EntityId::new(),
            role: PanelRole::Top,
        },
        grain: GrainAxis::U,
        edges: PanelEdges::default(),
        oversize: None,
    };
    assert!(
        model::prepare_panel_edit(&f.s, f.s.revision(), actor(), a.feature, &carcass_panel)
            .is_err()
    );
}

#[test]
fn sketch_region_panel_takes_stock_thickness_and_follows_stock_edits() {
    let mut f = fixture();
    let (sketch, edits) = Source::create(Datum::Xz, "Side profile").unwrap();
    insert(&mut f.s, edits);
    let mut source = Source::read(f.s.document(), sketch).unwrap().0;
    source.add_rectangle([0., 0.], [450., 700.]).unwrap();
    let regions = sketch_regions::extract(&source);
    assert_eq!(regions.regions.len(), 1);
    let region = regions.regions[0].intent();
    let edits = source.replace(f.s.document(), sketch).unwrap();
    insert(&mut f.s, edits);
    let request = PanelRequest::new(
        "Shaped side",
        PanelDefinition {
            output: BodyOutput {
                body: EntityId::new(),
            },
            source: PanelSource::Profile {
                stock: f.ply,
                profile: ProfileSource { sketch, region },
            },
            grain: GrainAxis::V,
            edges: PanelEdges::default(),
            oversize: None,
        },
    );
    let p = model::prepare_panel(&f.s, f.s.revision(), actor(), &request).unwrap();
    checked(&mut f.s, p.prepared);
    let list = compile(&f.s).cut_list(f.s.document()).unwrap();
    let side = record(&list, "Shaped side");
    pair(side.cut.finished_mm, [700., 450.]);
    near(side.cut.thickness_mm, 18.);
    near(side.volume_mm3, 450. * 700. * 18.);

    // Banding is refused on sketch-region panels.
    let mut banded = model::read_panel(f.s.document(), request.feature).unwrap();
    banded.edges.u_start = Some(band(2., 0.));
    assert!(
        model::prepare_panel_edit(&f.s, f.s.revision(), actor(), request.feature, &banded).is_err()
    );

    // Editing the stock thickness rebuilds the panel; the assignment shows review.
    let stock = f.s.document().get(f.ply).unwrap().clone();
    let LibraryDefinition::Stock(mut spec) = read(&stock).unwrap().unwrap() else {
        unreachable!()
    };
    let StockKind::Panel { thickness_mm, .. } = &mut spec.specification else {
        unreachable!()
    };
    *thickness_mm = 19.;
    let replaced = LibraryDefinition::Stock(spec)
        .entity(f.ply, stock.name.clone())
        .unwrap();
    let p = prepare_library(&f.s, f.s.revision(), actor(), &[replaced]).unwrap();
    checked(&mut f.s, p);
    let list = compile(&f.s).cut_list(f.s.document()).unwrap();
    near(record(&list, "Shaped side").cut.thickness_mm, 19.);
    near(record(&list, "Shaped side").volume_mm3, 450. * 700. * 19.);
}

fn read_bundled_project(mut bytes: &[u8]) -> ProjectFile {
    project_file::read(
        &mut bytes,
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
}

fn compile_stock_sources(
    document: &Document,
    roots: &[EntityId],
    backend: GeometryBackend,
) -> Model {
    Compiler::default()
        .compile_sources_with_backend(document, 0, roots, backend, None)
        .unwrap_or_else(|error| {
            let name = error
                .entity
                .and_then(|id| document.get(id))
                .map(|e| &e.name);
            panic!("{backend:?} failed for {name:?}: {error}")
        })
}

fn compare_stock_source(name: &str, id: EntityId, rust: &Model, native: &Model) {
    assert!(rust.solid(id).unwrap().uses_rust_geometry(), "{name}");
    let a = &rust.snapshot().shapes[&id];
    let b = &native.snapshot().shapes[&id];
    assert_eq!(a.solid_count, b.solid_count, "{name}");
    assert_eq!(a.solid_count, 1, "{name}");
    assert!(
        (a.volume_mm3 - b.volume_mm3).abs() <= 1e-7 + b.volume_mm3 * 1e-10,
        "{name}: {} != {}",
        a.volume_mm3,
        b.volume_mm3
    );
    for (a, b) in a
        .bounds
        .min
        .into_iter()
        .chain(a.bounds.max)
        .zip(b.bounds.min.into_iter().chain(b.bounds.max))
    {
        near(a, b);
    }
}

fn compare_stock_cut_lists(document: &Document, rust: &Model, native: &Model) -> usize {
    let rust_cuts = rust.cut_list(document).unwrap();
    let native_cuts = native.cut_list(document).unwrap();
    assert_eq!(rust_cuts.len(), native_cuts.len());
    for (a, b) in rust_cuts.iter().zip(native_cuts) {
        assert_eq!((a.part, a.stock, a.grain), (b.part, b.stock, b.grain));
        pair(a.cut.finished_mm, b.cut.finished_mm);
        near(a.cut.thickness_mm, b.cut.thickness_mm);
    }
    rust_cuts.len()
}

#[test]
fn rust_rebuilds_the_bundled_cabinet_panels_finger_joints_and_drawer_grooves() {
    let project = read_bundled_project(include_bytes!("../../../examples/cabinet.tcd"));
    let document = &project.document;
    let panels: Vec<_> = model::panels(document).map(|entity| entity.id).collect();
    let (finger_cuts, rounded_slots): (Vec<_>, Vec<_>) = model::panel_joint_cuts(document)
        .partition(|entity| {
            let cut = model::read_panel_joint_cut(document, entity.id).unwrap();
            let joint = model::read_panel_joint(document, cut.joint).unwrap();
            matches!(joint.kind, PanelJointKind::Finger { .. })
        });
    let grooves: Vec<_> = model::machining_cuts(document)
        .filter(|entity| {
            let cut = model::read_machining_cut(document, entity.id).unwrap();
            let source = model::read_machining(document, cut.operation).unwrap();
            matches!(source.kind, MachiningKind::Groove { .. })
        })
        .map(|entity| entity.id)
        .collect();
    assert_eq!(
        (panels.len(), finger_cuts.len(), grooves.len()),
        (24, 16, 8)
    );
    let roots: Vec<_> = panels
        .iter()
        .copied()
        .chain(finger_cuts.iter().map(|entity| entity.id))
        .chain(grooves)
        .collect();
    let rust = compile_stock_sources(document, &roots, GeometryBackend::Rust);
    let native = compile_stock_sources(document, &roots, GeometryBackend::OpenCascade);
    for id in roots {
        let name = &document.get(id).unwrap().name;
        compare_stock_source(name, id, &rust, &native);
        let a = &rust.snapshot().shapes[&id];
        let b = &native.snapshot().shapes[&id];
        if panels.contains(&id) {
            assert_eq!(
                rust.snapshot().panel_frames[&id],
                native.snapshot().panel_frames[&id]
            );
            for face in b.topology.elements.iter().filter(|e| !e.frames.is_empty()) {
                for anchor in &face.frames {
                    let mut intent = GeometryIntent::capture(&b.topology, face.key).unwrap();
                    intent.attachment = Some(Attachment {
                        anchor: anchor.origin.clone(),
                        offset_mm: [0.; 3],
                        angle_rad: 0.,
                    });
                    let Resolution::Resolved {
                        frame: Some(frame), ..
                    } = references::resolve(&intent, &a.topology, 0)
                    else {
                        panic!("{name} lost authored face attachment {:?}", anchor.origin);
                    };
                    for (a, b) in frame
                        .origin_mm
                        .into_iter()
                        .chain(frame.u)
                        .chain(frame.v)
                        .zip(
                            anchor
                                .frame
                                .origin_mm
                                .into_iter()
                                .chain(anchor.frame.u)
                                .chain(anchor.frame.v),
                        )
                    {
                        near(a, b);
                    }
                }
            }
        }
    }
    assert!(compare_stock_cut_lists(document, &rust, &native) >= 8);

    // Select the first rounded-slot cut on a panel without including earlier cuts.
    let unsupported = rounded_slots
        .into_iter()
        .find(|entity| {
            let source = model::read_panel_joint_cut(document, entity.id).unwrap();
            panels.contains(&source.input.producer)
        })
        .unwrap();
    let Err(error) = Compiler::default().compile_sources_with_backend(
        document,
        0,
        &[unsupported.id],
        GeometryBackend::Rust,
        None,
    ) else {
        panic!("rounded slot unexpectedly built");
    };
    assert_eq!(error.entity, Some(unsupported.id));
    assert!(error.message.contains("slot"), "{error}");
}

#[test]
fn rust_rebuilds_both_hinge_boring_groups_on_each_bundled_cabinet_door() {
    let project = read_bundled_project(include_bytes!("../../../examples/cabinet.tcd"));
    let document = &project.document;
    let parameters = multicad_core::parameters::evaluate(document).unwrap();
    let operations: Vec<_> = model::machining_cuts(document)
        .filter_map(|entity| {
            let cut = model::read_machining_cut(document, entity.id).unwrap();
            document
                .get(cut.operation)
                .unwrap()
                .name
                .contains("door hinge boring")
                .then_some((entity, cut))
        })
        .collect();
    assert_eq!(operations.len(), 4);
    let roots: Vec<_> = operations.iter().map(|(entity, _)| entity.id).collect();
    let rust = compile_stock_sources(document, &roots, GeometryBackend::Rust);
    let native = compile_stock_sources(document, &roots, GeometryBackend::OpenCascade);
    let mut surviving_floors = 0;
    for (entity, cut) in operations {
        let id = entity.id;
        compare_stock_source(&entity.name, id, &rust, &native);
        let definition = model::read_machining(document, cut.operation).unwrap();
        let resolved =
            model::resolve_machining(document, &parameters, &definition, cut.output.body).unwrap();
        assert_eq!(resolved.holes.len(), 3);
        let removed: f64 = resolved
            .holes
            .iter()
            .map(|hole| {
                assert!(!hole.through);
                std::f64::consts::PI * (hole.diameter_mm / 2.).powi(2) * hole.depth_mm
            })
            .sum();
        let solid = rust.solid(id).unwrap();
        near(
            rust.solid(cut.input.producer)
                .unwrap()
                .volume_mm3()
                .unwrap()
                - solid.volume_mm3().unwrap(),
            removed,
        );
        let snapshot = &rust.snapshot().shapes[&id].topology;
        let first_hole = resolved.holes[0];
        let expected: [f64; 3] = std::array::from_fn(|i| {
            first_hole.center[i] + first_hole.axis[i] * first_hole.depth_mm
        });
        let floor = solid
            .faces()
            .unwrap()
            .into_iter()
            .find(|face| {
                face.plane.is_some()
                    && face
                        .center_mm
                        .into_iter()
                        .zip(expected)
                        .all(|(a, b)| (a - b).abs() < 1e-6)
            })
            .expect("hinge cup has a planar floor at its resolved center");
        near(
            floor.area_mm2,
            std::f64::consts::PI * (first_hole.diameter_mm / 2.).powi(2),
        );
        near(
            floor
                .plane
                .unwrap()
                .1
                .into_iter()
                .zip(first_hole.axis)
                .map(|(a, b)| a * b)
                .sum(),
            -1.,
        );
        let entry = snapshot
            .elements
            .iter()
            .find(|item| {
                matches!(item.geometry, Some(AnalyticGeometry::Plane { .. }))
                    && item.key.ordinal == floor.index
            })
            .unwrap();
        let anchor = entry
            .frames
            .iter()
            .find(|frame| frame.origin.source == id)
            .unwrap();
        let mut intent = GeometryIntent::capture(snapshot, entry.key).unwrap();
        intent.attachment = Some(Attachment {
            anchor: anchor.origin.clone(),
            offset_mm: [0.; 3],
            angle_rad: 0.,
        });
        if let Some(next) = roots.iter().copied().find(|next| {
            model::read_machining_cut(document, *next)
                .unwrap()
                .input
                .producer
                == id
        }) {
            let terminal = &rust.snapshot().shapes[&next].topology;
            let Resolution::Resolved {
                frame: Some(frame), ..
            } = references::resolve(&intent, terminal, 0)
            else {
                panic!("the second hinge group lost the first floor attachment");
            };
            assert_eq!(frame, anchor.frame);
            surviving_floors += 1;
        }
    }
    assert_eq!(surviving_floors, 2);
}

#[test]
fn rust_rebuilds_the_bundled_toolcase_stock_panels_with_exact_hole_outlines() {
    let project = read_bundled_project(include_bytes!("../../../examples/toolcase.tcd"));
    let document = &project.document;
    let panels: Vec<_> = model::panels(document).collect();
    assert_eq!(panels.len(), 7);
    let roots: Vec<_> = panels.iter().map(|panel| panel.id).collect();
    let rust = compile_stock_sources(document, &roots, GeometryBackend::Rust);
    let native = compile_stock_sources(document, &roots, GeometryBackend::OpenCascade);
    for panel in panels {
        let id = panel.id;
        compare_stock_source(&panel.name, id, &rust, &native);
        let contours = |model: &Model| {
            let snap = model.snapshot();
            let start = snap.shapes[&id]
                .topology
                .elements
                .iter()
                .find(|element| element.frames.iter().any(|f| f.origin.role == "cap.start"))
                .expect("panel start cap has an authored attachment");
            model
                .solid(id)
                .unwrap()
                .planar_face_contours(start.key.ordinal, snap.panel_frames[&id])
                .unwrap()
        };
        let actual = contours(&rust);
        let expected = contours(&native);
        near(actual.area_mm2(), expected.area_mm2());
        assert_eq!(actual.loops.len(), expected.loops.len(), "{}", panel.name);
        let circles = actual
            .loops
            .iter()
            .filter(|wire| {
                !wire.outer
                    && matches!(wire.edges.as_slice(), [PlanarEdge::Arc { sweep_rad, .. }]
                        if (sweep_rad.abs() - std::f64::consts::TAU).abs() < 1e-7)
            })
            .count();
        let expected_circles = match panel.name.as_str() {
            "Left side feature" | "Right side feature" => 10,
            _ => 0,
        };
        assert_eq!(circles, expected_circles, "{}", panel.name);
    }
    // The two side panels have later edge treatments outside these source roots.
    assert_eq!(compare_stock_cut_lists(document, &rust, &native), 5);
}

#[test]
fn rust_stock_backed_profile_panel_keeps_its_round_through_hole_and_cut_dimensions() {
    let mut f = fixture();
    let (sketch, edits) = Source::create(Datum::Xz, "Drilled panel profile").unwrap();
    insert(&mut f.s, edits);
    let mut source = Source::read(f.s.document(), sketch).unwrap().0;
    source.add_rectangle([0., 0.], [120., 70.]).unwrap();
    let center = source.add_point([27., 33.]);
    source.sketch.curves.push(Curve {
        id: EntityId::new(),
        construction: false,
        kind: CurveKind::Circle {
            center,
            radius_mm: 6.,
        },
    });
    let region = sketch_regions::extract(&source)
        .regions
        .into_iter()
        .find(|region| region.holes.len() == 1)
        .unwrap()
        .intent();
    let edits = source.replace(f.s.document(), sketch).unwrap();
    insert(&mut f.s, edits);
    let panel = PanelRequest::new(
        "Drilled panel",
        PanelDefinition {
            output: BodyOutput {
                body: EntityId::new(),
            },
            source: PanelSource::Profile {
                stock: f.ply,
                profile: ProfileSource { sketch, region },
            },
            grain: GrainAxis::V,
            edges: PanelEdges::default(),
            oversize: None,
        },
    );
    let prepared = model::prepare_panel(&f.s, f.s.revision(), actor(), &panel).unwrap();
    checked(&mut f.s, prepared.prepared);
    let rust = Compiler::default()
        .compile_with_backend(f.s.document(), f.s.revision(), GeometryBackend::Rust, None)
        .unwrap();
    assert!(rust.solid(panel.feature).unwrap().uses_rust_geometry());
    let list = rust.cut_list(f.s.document()).unwrap();
    let cut = record(&list, "Drilled panel");
    assert_eq!(cut.stock, f.ply);
    pair(cut.cut.finished_mm, [70., 120.]);
    near(cut.cut.thickness_mm, 18.);
    near(
        cut.volume_mm3,
        (120. * 70. - std::f64::consts::PI * 6. * 6.) * 18.,
    );
    same_cut_records(&list, &compile(&f.s).cut_list(f.s.document()).unwrap());
    let topology = &rust.snapshot().shapes[&panel.feature].topology;
    let round_edges = topology.elements.iter().filter(|e| {
        matches!(e.geometry, Some(AnalyticGeometry::CircularArc { radius_mm, .. }) if radius_mm == 6.)
    }).count();
    assert_eq!(round_edges, 2);
}
