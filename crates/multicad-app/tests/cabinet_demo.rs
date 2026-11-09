//! The shipped 2 m cabinet demo (CAD-182): the committed `.tcd` files open
//! with their structure, joinery, drilling, library hardware, movement and
//! drawings; a parameter edit propagates and survives Undo/Redo and
//! save/reopen with stable identities and resolved references; library
//! hardware changes only through an explicit symbol update; and the saved
//! poses are free of overlaps.
#[path = "../examples/cabinet_demo.rs"]
#[allow(dead_code)]
mod demo;
use multicad_core::{
    assembly_inspection::{
        InspectionStamp, OccurrencePair, PairQuery, PairRelation, RangeCoverage, SampleUnit,
        read_saved_motion_study,
    },
    commands::{Request, Session},
    document::{Document, EntityData, EntityId, Mutation},
    drawings::{self, AnnotationEvaluation},
    model::{self, JointCoordinate, OccurrencePath, PanelJointKind},
    project_file,
    symbols::{self, SymbolUpdateRequest},
};
use multicad_model::{
    Compiler, DrawingProviders, Model,
    assembly_inspection::{inspect_assembly, sample_assembly_range},
};
use std::path::PathBuf;

fn shipped(file: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(file)
}
fn open(file: &str) -> Document {
    let bytes = std::fs::read(shipped(file)).unwrap();
    project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document
}
fn compile(session: &Session) -> Model {
    let model = Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    assert!(
        model
            .snapshot()
            .references
            .for_model(session.document())
            .unwrap()
            .all_resolved()
    );
    model
}
fn named(doc: &Document, name: &str) -> EntityId {
    let found: Vec<_> = doc.entities().filter(|e| e.name == name).collect();
    assert_eq!(found.len(), 1, "exactly one entity named {name}");
    found[0].id
}
fn part(doc: &Document, name: &str) -> EntityId {
    let found: Vec<_> = doc
        .entities()
        .filter(|e| e.name == name && matches!(e.data, EntityData::Part { .. }))
        .collect();
    assert_eq!(found.len(), 1, "exactly one Part named {name}");
    found[0].id
}
fn instance(doc: &Document, name: &str) -> EntityId {
    let found: Vec<_> = doc
        .entities()
        .filter(|e| e.name == name && matches!(e.data, EntityData::Instance { .. }))
        .collect();
    assert_eq!(found.len(), 1, "exactly one Instance named {name}");
    found[0].id
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}
/// World bounds of the single leaf of `part`, or of the standalone Part.
fn bounds(model: &Model, part: EntityId) -> ([f64; 3], [f64; 3]) {
    let leaves: Vec<_> = model
        .snapshot()
        .leaf_occurrences
        .values()
        .filter(|l| l.component == part)
        .collect();
    assert_eq!(leaves.len(), 1, "one physical occurrence");
    (leaves[0].bounds.min, leaves[0].bounds.max)
}
fn leaf_path(model: &Model, part: EntityId) -> OccurrencePath {
    model
        .snapshot()
        .leaf_occurrences
        .values()
        .find(|l| l.component == part)
        .unwrap()
        .path
        .clone()
}
fn set_parameter(session: &mut Session, name: &str, expression: &str) {
    let id = named(session.document(), name);
    let mut entity = session.document().get(id).unwrap().clone();
    let EntityData::Parameter { expression: e, .. } = &mut entity.data else {
        panic!("not a parameter")
    };
    *e = expression.into();
    let prepared = session
        .prepare(Request {
            expected_revision: session.revision(),
            actor: demo::actor(),
            label: "Edit parameter".into(),
            edits: vec![Mutation::Replace(entity)],
        })
        .unwrap();
    session.commit(prepared).unwrap();
}
fn apply_pose(session: &mut Session, name: &str) {
    let pose = named(session.document(), name);
    let prepared =
        model::prepare_apply_pose(session, session.revision(), demo::actor(), pose).unwrap();
    session.commit(prepared.prepared).unwrap();
}
fn drawing_texts(session: &Session) -> Vec<String> {
    let model = compile(session);
    let evaluation = model
        .drawings(
            session.document(),
            session.revision(),
            &DrawingProviders::default(),
        )
        .unwrap();
    let book = drawings::books(session.document()).next().unwrap();
    let pages = drawings::read(session.document(), book).unwrap().pages;
    for page in &pages {
        evaluation
            .checked_page(session.document(), session.revision(), book, page.id)
            .unwrap();
    }
    pages[0]
        .annotations
        .iter()
        .filter_map(|a| match evaluation.annotations.get(&a.id) {
            Some(AnnotationEvaluation::Ready { graphic }) if !graphic.text.is_empty() => {
                Some(graphic.text.clone())
            }
            _ => None,
        })
        .collect()
}

#[test]
fn shipped_cabinet_opens_with_its_structure_hardware_and_drawings() {
    let bytes = std::fs::read(shipped(demo::CABINET_FILE)).unwrap();
    let card = project_file::card(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(card.info.name, demo::CABINET_NAME);
    assert_eq!(card.info.description, demo::CABINET_DESCRIPTION);
    let preview = card.info.preview.expect("a stored preview");
    assert!(preview.width() >= 200 && preview.height() >= 200);
    assert_eq!(card.symbol_definitions, 4);
    let project = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    for capability in [
        model::LOOSE_TENON_CAPABILITY,
        model::SHELF_PIN_CAPABILITY,
        model::INSIDE_FACE_CAPABILITY,
        model::NAMED_POSE_CAPABILITY,
        model::JOINT_CAPABILITY,
        symbols::CAPABILITY,
    ] {
        assert!(project.capabilities().contains(capability), "{capability}");
    }
    let session = Session::new(project.document).unwrap();
    let doc = session.document();
    let model = compile(&session);

    // 2000 mm tall; doors below the drawers below the open shelving.
    let left = part(doc, "Cabinet left side");
    near(bounds(&model, left).1[2], 2000.);
    let door_l = bounds(&model, part(doc, "Left door"));
    let door_r = bounds(&model, part(doc, "Right door"));
    let drawer_1 = bounds(&model, part(doc, "Drawer 1 front"));
    let drawer_2 = bounds(&model, part(doc, "Drawer 2 front"));
    let shelf = bounds(&model, part(doc, "Adjustable shelf, open shelving 1"));
    assert!(door_l.1[0] < door_r.0[0]);
    assert!(door_l.1[2] < drawer_1.0[2] && drawer_1.1[2] < drawer_2.0[2]);
    assert!(drawer_2.1[2] < shelf.0[2]);
    // A shelf behind the doors.
    let lower = bounds(&model, part(doc, "Adjustable shelf, door compartment"));
    assert!(lower.1[2] < door_l.1[2] && lower.0[1] > 0.);

    // Domino carcass joints: 8 face-to-edge pairs, 3 tenons each, with a
    // tenon Part per pair and paired mortise records.
    let dominos = named(doc, "Carcass Dominos");
    let joint = model::read_panel_joint(doc, dominos).unwrap();
    assert!(matches!(&joint.kind, PanelJointKind::LooseTenon(s) if s.count == 3));
    assert_eq!(joint.pairs.len(), 8);
    assert_eq!(model::joint_tenons(doc, dominos).len(), 8);
    let records = model.machining(doc).unwrap();
    let mortises = records
        .iter()
        .filter(|r| r.kind == "loose-tenon mortise")
        .count();
    assert_eq!(mortises, 16);
    // Finger-jointed drawer boxes.
    for n in 1..=2 {
        let fingers =
            model::read_panel_joint(doc, named(doc, &format!("Drawer {n} box fingers"))).unwrap();
        assert!(matches!(
            fingers.kind,
            PanelJointKind::Finger { count: 5, .. }
        ));
        assert_eq!(fingers.pairs.len(), 4);
    }
    // System 32 rows: two rows per side face, aligned on both sides.
    let holes = |op: &str, part: EntityId| -> Vec<[f64; 3]> {
        let op = named(doc, op);
        records
            .iter()
            .filter(|r| r.operation == op && r.part == part)
            .flat_map(|r| r.holes.iter().map(|h| h.center))
            .collect()
    };
    let right = part(doc, "Cabinet right side");
    for (op, count) in [
        ("Shelf pins, door compartment", 24),
        ("Shelf pins, open shelving", 30),
    ] {
        let (l, r) = (holes(op, left), holes(op, right));
        assert_eq!((l.len(), r.len()), (count, count));
        for (a, b) in l.iter().zip(&r) {
            near(a[1], b[1]);
            near(a[2], b[2]);
        }
    }
    // Hinge borings on each door and plate holes on each side.
    for door in ["Left door", "Right door"] {
        let part = part(doc, door);
        let cups = records
            .iter()
            .filter(|r| r.part == part)
            .flat_map(|r| r.holes.iter())
            .filter(|h| (h.diameter_mm - 35.).abs() < 1e-9)
            .count();
        assert_eq!(cups, 2, "{door}");
    }
    assert_eq!(holes("Hinge plate holes, lower", left).len(), 2);
    assert_eq!(
        holes("Drawer 1 runner screws, carcass sides", right).len(),
        4
    );

    // Library hardware: four pinned definitions, identities and pins equal
    // to the shipped library file's.
    let library = open(demo::LIBRARY_FILE);
    let shipped_defs = symbols::definitions(&library).unwrap();
    let defs = symbols::definitions(doc).unwrap();
    assert_eq!(defs.len(), 4);
    for (_, def) in &defs {
        let source = shipped_defs
            .iter()
            .find(|(_, d)| d.symbol == def.symbol)
            .expect("same library symbol identity");
        assert_eq!(source.1.pin().unwrap(), def.pin().unwrap());
        assert!(demo::library_symbol_ids().contains(&def.symbol));
        assert_eq!(
            symbols::instances(
                doc,
                symbols::definitions(doc)
                    .unwrap()
                    .iter()
                    .find(|(_, d)| d.symbol == def.symbol)
                    .unwrap()
                    .0
            )
            .unwrap()
            .len(),
            4
        );
    }

    // Movement: two revolute doors, two slider drawers and four poses.
    let joints: Vec<_> = doc
        .entities()
        .filter_map(|e| model::read_joint(doc, e.id).ok())
        .map(|(j, _)| j.parameters.kind)
        .collect();
    assert_eq!(
        joints
            .iter()
            .filter(|k| **k == model::JointKind::Revolute)
            .count(),
        2
    );
    assert_eq!(
        joints
            .iter()
            .filter(|k| **k == model::JointKind::Slider)
            .count(),
        2
    );
    for pose in [
        demo::POSE_CLOSED,
        demo::POSE_DOORS,
        demo::POSE_DRAWERS,
        demo::POSE_ALL,
    ] {
        model::read_named_pose(doc, named(doc, pose)).unwrap();
    }

    // Drawings: every page evaluates; the assembly dimensions read the
    // cabinet size; schedules include purchased hardware.
    let texts = drawing_texts(&session);
    for t in ["800", "2000", "500"] {
        assert!(texts.iter().any(|x| x == t), "{t} in {texts:?}");
    }
    let bom = model.bom(doc, &Default::default()).unwrap();
    assert!(
        bom.rows
            .iter()
            .any(|r| r.name == "Hinge cup"
                && r.supply == multicad_model::schedules::Supply::Purchased)
    );
}

#[test]
fn shipped_drawer_motion_study_runs_the_existing_slider_at_its_saved_samples() {
    let mut session = Session::new(open(demo::CABINET_FILE)).unwrap();
    let study_id = named(session.document(), demo::DRAWER_STUDY);
    let study = read_saved_motion_study(session.document(), study_id).unwrap();
    let query = study.query(&session).unwrap();
    let drawer = instance(session.document(), "Drawer 1");
    assert_eq!(
        model::read_joint(session.document(), query.joint)
            .unwrap()
            .0
            .owner,
        drawer
    );
    assert_eq!(query.coordinate, JointCoordinate::Z);
    assert_eq!(query.sample_values().unwrap(), [0., 150., 300., 450.]);
    assert_eq!(study.required_clearance_mm, 1.);
    assert_eq!(query.pairs.len(), 1);
    assert_eq!(query.pairs[0].a.root, drawer);
    assert_eq!(
        query.pairs[0].b,
        OccurrencePath {
            root: part(session.document(), "Cabinet left side"),
            members: vec![],
        }
    );

    let original = session.document().clone();
    let report =
        sample_assembly_range(&Compiler::default(), session.document(), &query, None).unwrap();
    assert_eq!(report.coverage, RangeCoverage::SamplesOnly);
    assert_eq!(report.unit, SampleUnit::Millimeters);
    assert_ne!(
        report.samples.first().unwrap().candidate_stamp,
        report.samples.last().unwrap().candidate_stamp
    );
    for sample in &report.samples {
        let values = sample.result.as_ref().unwrap().pairs[0]
            .result
            .as_ref()
            .unwrap();
        assert_eq!(values.relation, PairRelation::Separated);
        assert!(values.minimum_clearance_mm >= study.required_clearance_mm);
    }
    assert_eq!(session.document(), &original);
    assert_eq!(session.revision(), 0);

    // The study's bound end follows the same parameter as the slider limit.
    set_parameter(&mut session, "drawer_travel", "360");
    let changed = read_saved_motion_study(session.document(), study_id).unwrap();
    assert_eq!(
        changed.query(&session).unwrap().sample_values().unwrap(),
        [0., 150., 300., 360.]
    );
}
#[test]
fn width_edit_undo_redo_and_reopen_keep_identities_and_references() {
    let mut session = Session::new(open(demo::CABINET_FILE)).unwrap();
    let ids: Vec<EntityId> = session.document().entities().map(|e| e.id).collect();
    let right = part(session.document(), "Cabinet right side");
    let right_door = part(session.document(), "Right door");
    let runner = |s: &Session, m: &Model| {
        // The right runner cabinet member of drawer 1, a library symbol.
        let instance = instance(s.document(), "Drawer 1 runner, right cabinet member");
        let path = m
            .snapshot()
            .leaf_occurrences
            .values()
            .find(|l| l.path.root == instance)
            .unwrap();
        path.bounds.max[0]
    };
    let m = compile(&session);
    near(bounds(&m, right).1[0], 800.);
    near(runner(&session, &m), 782.);

    set_parameter(&mut session, "cabinet_width", "900");
    let m = compile(&session);
    near(bounds(&m, right).1[0], 900.);
    near(bounds(&m, right_door).1[0], 898.);
    near(runner(&session, &m), 882.);
    assert!(drawing_texts(&session).iter().any(|t| t == "900"));
    let edited = session.document().clone();

    session.undo(session.revision(), demo::actor()).unwrap();
    near(bounds(&compile(&session), right).1[0], 800.);
    session.redo(session.revision(), demo::actor()).unwrap();
    assert_eq!(session.document(), &edited);

    // Open the drawers through the saved pose, then save and reopen.
    apply_pose(&mut session, demo::POSE_DRAWERS);
    let m = compile(&session);
    near(
        bounds(&m, part(session.document(), "Drawer 1 front")).0[1],
        -468.,
    );
    let bytes = demo::archive(&session).unwrap();
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document;
    assert_eq!(&reopened, session.document());
    for id in &ids {
        assert!(reopened.get(*id).is_some(), "identity {id} kept");
    }
    let reopened = Session::new(reopened).unwrap();
    let m = compile(&reopened);
    near(bounds(&m, right).1[0], 900.);
    assert!(
        model::read_panel_joint(
            reopened.document(),
            named(reopened.document(), "Carcass Dominos")
        )
        .is_ok()
    );
}

#[test]
fn library_hardware_changes_only_through_an_explicit_update() {
    let mut session = Session::new(open(demo::CABINET_FILE)).unwrap();
    let before = session.document().clone();
    let revised = demo::build_library(500., "2").unwrap();
    let (definition, _) = symbols::definitions(session.document())
        .unwrap()
        .into_iter()
        .find(|(_, d)| d.symbol == revised.hardware.runner_cabinet.symbol)
        .unwrap();
    // Comparing reports the change and touches nothing.
    let report = symbols::compare_update(
        session.document(),
        definition,
        &revised.hardware.runner_cabinet,
    )
    .unwrap();
    assert_eq!(report.affected_instances.len(), 4);
    assert_ne!(report.old_pin, report.new_pin);
    assert_eq!(session.document(), &before);
    // Applying is one undoable transaction that keeps the symbol identity.
    let request = SymbolUpdateRequest::new(
        session.document(),
        definition,
        revised.hardware.runner_cabinet.clone(),
    )
    .unwrap();
    let prepared =
        symbols::prepare_update(&session, session.revision(), demo::actor(), &request).unwrap();
    session.commit(prepared.prepared).unwrap();
    let def = symbols::read_definition(session.document(), definition).unwrap();
    assert_eq!(def.revision, "2");
    let m = compile(&session);
    let instance = instance(session.document(), "Drawer 1 runner, left cabinet member");
    let leaf = m
        .snapshot()
        .leaf_occurrences
        .values()
        .find(|l| l.path.root == instance)
        .unwrap();
    near(leaf.bounds.max[1] - leaf.bounds.min[1], 500.);
    session.undo(session.revision(), demo::actor()).unwrap();
    assert_eq!(session.document(), &before);
}

#[test]
fn saved_poses_have_no_overlaps_between_moving_and_fixed_parts() {
    let mut session = Session::new(open(demo::CABINET_FILE)).unwrap();
    let doc = session.document().clone();
    let pairs = [
        ("Left door", "Cabinet left side"),
        ("Right door", "Cabinet right side"),
        ("Left door", "Right door"),
        ("Left door", "Drawer 1 front"),
        ("Drawer 1 front", "Drawer 2 front"),
        ("Drawer 1 box left side", "Cabinet left side"),
        ("Drawer 1 box bottom", "Cabinet shelf 1"),
        ("Drawer 2 box back", "Cabinet shelf 2"),
    ];
    for pose in [demo::POSE_CLOSED, demo::POSE_ALL] {
        apply_pose(&mut session, pose);
        let m = compile(&session);
        let query = PairQuery {
            expected: InspectionStamp::capture(session.document(), session.revision()).unwrap(),
            pairs: pairs
                .iter()
                .map(|(a, b)| OccurrencePair {
                    a: leaf_path(&m, part(&doc, a)),
                    b: leaf_path(&m, part(&doc, b)),
                })
                .collect(),
            contact_tolerance_mm: 1e-3,
        };
        let report = inspect_assembly(&m, &query).unwrap();
        for (evaluation, (a, b)) in report.pairs.iter().zip(&pairs) {
            let values = evaluation.result.as_ref().unwrap();
            assert_ne!(
                values.relation,
                PairRelation::Overlapping,
                "{a} overlaps {b} in {pose}"
            );
        }
    }
}
