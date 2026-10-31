//! Loose-tenon (Domino style) joints: a cabinet top whose end meets the
//! inner face of a side gets paired rounded mortises and a tenon Part,
//! all rebuilt from the panels when the cabinet changes. Refusals leave the
//! document unchanged; edits, Undo/Redo, save/reopen and deletion keep the
//! paired references and valid solids.
use multicad_core::{
    commands::{Actor, Channel, Prepared, Session},
    document::{Document, Entity, EntityData, EntityId},
    materials::*,
    model::*,
    project_file::{self, ProjectFile},
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
fn solid(m: &Model, part: EntityId) -> &multicad_kernel::Solid {
    let snapshot = m.snapshot();
    let head = snapshot
        .bodies
        .get(&part)
        .or_else(|| snapshot.body_sources.get(&part))
        .unwrap();
    m.solid(*head).unwrap()
}
fn volume(m: &Model, part: EntityId) -> f64 {
    solid(m, part).volume_mm3().unwrap()
}
/// A rounded slot section: width w (with the round ends) by thickness t.
fn slot(w: f64, t: f64) -> f64 {
    (w - t) * t + PI * t * t / 4.
}

struct Fixture {
    s: Session,
    carcass: EntityId,
    definition: CarcassDefinition,
    roles: Vec<(PanelRole, EntityId)>,
}
impl Fixture {
    fn part(&self, role: PanelRole) -> EntityId {
        self.roles.iter().find(|(r, _)| *r == role).unwrap().1
    }
}
fn fixture() -> Fixture {
    let mut s = Session::new(Document::new("Domino").unwrap()).unwrap();
    let birch = Entity::new(
        "Birch plywood",
        EntityData::Material {
            density_kg_m3: None,
            color_rgba: [214, 186, 140, 255],
            section_hatch: None,
        },
    );
    let ply = LibraryDefinition::Stock(Stock {
        material: birch.id,
        specification: StockKind::Panel {
            thickness_mm: 18.,
            length_mm: 2800.,
            width_mm: 2070.,
            grain: Grain::None,
        },
    })
    .entity(EntityId::new(), "Ply 18")
    .unwrap();
    let stock = ply.id;
    s.commit(prepare_library(&s, 0, actor(), &[birch, ply]).unwrap())
        .unwrap();
    let definition = CarcassDefinition {
        cabinet: None,
        origin: [length(0.), length(0.), length(0.)],
        rotation: angle(0.),
        width: length(600.),
        height: length(720.),
        depth: length(400.),
        stock,
        construction: Construction::SidesFullHeight,
        back: Back::None,
        shelves: Shelves {
            layout: ShelfLayout::Even { count: 0 },
            setback: length(10.),
            clearance: length(1.),
        },
        front_band: None,
        group: None,
    };
    let created = prepare_carcass(
        &s,
        s.revision(),
        actor(),
        &CarcassRequest::new("Cabinet", definition.clone()),
    )
    .unwrap();
    let roles = created.panels.iter().map(|(r, c)| (*r, c.body)).collect();
    let carcass = created.carcass;
    checked(&mut s, created.model.prepared);
    Fixture {
        s,
        carcass,
        definition,
        roles,
    }
}
/// The example: Festool DOMINO 8 × 22 × 40 mm tenons, 15 mm into the
/// 18 mm side face and 28 mm into the top's end, tight in the end and
/// with 6 mm play along the face mortise, two per joint 60 mm from each end.
fn domino() -> LooseTenonSpec {
    LooseTenonSpec {
        thickness: length(8.),
        width: length(22.),
        length: length(40.),
        count: 2,
        inset: length(60.),
        offset: None,
        face_depth: length(15.),
        edge_depth: length(28.),
        face_play: length(6.),
        edge_play: length(0.),
        fit: length(0.),
    }
}
fn joint(f: &Fixture, spec: LooseTenonSpec) -> PanelJointDefinition {
    PanelJointDefinition {
        kind: PanelJointKind::LooseTenon(Box::new(spec)),
        pairs: vec![PanelJointPair {
            a: f.part(PanelRole::RightSide),
            b: f.part(PanelRole::Top),
        }],
    }
}

#[test]
fn a_domino_joint_cuts_paired_mortises_and_builds_tenons() {
    let mut f = fixture();
    let (side, top) = (f.part(PanelRole::RightSide), f.part(PanelRole::Top));
    let side_v = 18. * 400. * 720.;
    let top_v = 18. * 400. * 564.;
    let p = prepare_panel_joint(
        &f.s,
        f.s.revision(),
        actor(),
        "Top joint",
        &joint(&f, domino()),
    )
    .unwrap();
    // Two cuts and the tenon Part.
    assert_eq!(p.model.outputs.len(), 3);
    let id = p.joint;
    checked(&mut f.s, p.model.prepared);
    let tenons = joint_tenons(f.s.document(), id);
    assert_eq!(tenons.len(), 1);
    let tenon = tenons[0].2;
    assert_eq!(f.s.document().get(tenon).unwrap().name, "Top joint tenons");
    let m = compile(&f.s);
    near(volume(&m, side), side_v - 2. * slot(28., 8.) * 15.);
    near(volume(&m, top), top_v - 2. * slot(22., 8.) * 28.);
    near(volume(&m, tenon), 2. * slot(22., 8.) * 40.);
    // The tenons span the joint plane at x = 582: 13.5 mm into the side and
    // 26.5 mm into the top (the 3 mm extra depth is shared).
    let b = solid(&m, tenon).bounds().unwrap();
    near(b.min[0], 582. - 26.5);
    near(b.max[0], 582. + 13.5);
    near(b.min[1], 60. - 11.);
    near(b.max[1], 340. + 11.);
    near(b.min[2], 720. - 9. - 4.);
    near(b.max[2], 720. - 9. + 4.);
    // Records name both panels' mortises and the tenon Part.
    let records: Vec<_> = m
        .machining(f.s.document())
        .unwrap()
        .into_iter()
        .filter(|r| r.operation == id)
        .collect();
    assert_eq!(records.len(), 2);
    let of = |part| records.iter().find(|r| r.part == part).unwrap();
    assert_eq!(of(side).face, MachiningFace::Bottom);
    assert_eq!(
        of(top).face,
        MachiningFace::Edge {
            edge: PanelEdgeName::UEnd
        }
    );
    assert!(records.iter().all(|r| r.tenon == Some(tenon)));
    assert!(records.iter().all(|r| r.kind == "loose-tenon mortise"));
    near(of(side).recesses[0].size_mm[0], 28.);
    near(of(side).recesses[0].depth_mm, 15.);
    near(of(top).recesses[1].size_mm[0], 22.);
    near(of(top).recesses[1].depth_mm, 28.);
    near(of(top).removed_mm3, 2. * slot(22., 8.) * 28.);

    // A wider cabinet moves the side; both mortises and the tenons follow.
    let mut wider = f.definition.clone();
    wider.width = length(700.);
    let p = prepare_carcass_edit(&f.s, f.s.revision(), actor(), f.carcass, &wider).unwrap();
    checked(&mut f.s, p.model.prepared);
    let m = compile(&f.s);
    let b = solid(&m, tenon).bounds().unwrap();
    near(b.max[0], 682. + 13.5);
    near(
        volume(&m, top),
        18. * 400. * 664. - 2. * slot(22., 8.) * 28.,
    );
    near(volume(&m, side), side_v - 2. * slot(28., 8.) * 15.);
}

#[test]
fn invalid_domino_placements_are_refused_with_a_reason() {
    let f = fixture();
    let refuse = |def: PanelJointDefinition, needle: &str| {
        let Err(error) = prepare_panel_joint(&f.s, f.s.revision(), actor(), "Bad", &def) else {
            panic!("accepted a joint that should fail with {needle}");
        };
        let error = error.to_string();
        assert!(error.contains(needle), "{error}");
    };
    // The two sides never meet face to edge.
    let mut apart = joint(&f, domino());
    apart.pairs[0].b = f.part(PanelRole::LeftSide);
    refuse(apart, "do not meet face to edge");
    // A 17 mm face mortise leaves 1 mm of the 18 mm side.
    let mut deep = domino();
    deep.face_depth = length(17.);
    refuse(joint(&f, deep), "at least 2 mm must remain");
    // A 30 mm end mortise is fine, a 600 mm one is longer than the top.
    let mut long = domino();
    long.edge_depth = length(600.);
    refuse(joint(&f, long), "as deep as");
    // 5 mm insets put the 22 mm tenons past the ends.
    let mut tight = domino();
    tight.inset = length(5.);
    refuse(joint(&f, tight), "leaves the");
    // Twelve tenons between 60 mm insets overlap.
    let mut many = domino();
    many.count = 12;
    refuse(joint(&f, many), "overlap");
    // A 50 mm tenon in 15 + 28 mm mortises does not seat.
    let mut longer = domino();
    longer.length = length(50.);
    refuse(joint(&f, longer), "shallower than");
    // A tenon offset past the top's thickness leaves its end.
    let mut off = domino();
    off.offset = Some(length(6.));
    refuse(joint(&f, off), "leaves the");
}

#[test]
fn domino_edit_undo_reopen_and_delete_keep_paired_references() {
    let mut f = fixture();
    let (side, top) = (f.part(PanelRole::RightSide), f.part(PanelRole::Top));
    let p = prepare_panel_joint(
        &f.s,
        f.s.revision(),
        actor(),
        "Top joint",
        &joint(&f, domino()),
    )
    .unwrap();
    let id = p.joint;
    checked(&mut f.s, p.model.prepared);
    let tenon = joint_tenons(f.s.document(), id)[0].2;

    // Three tenons; the tenon Part keeps its identity.
    let mut three = domino();
    three.count = 3;
    let p = prepare_panel_joint_edit(
        &f.s,
        f.s.revision(),
        actor(),
        id,
        &PanelJointKind::LooseTenon(Box::new(three.clone())),
    )
    .unwrap();
    checked(&mut f.s, p.model.prepared);
    let m = compile(&f.s);
    near(volume(&m, tenon), 3. * slot(22., 8.) * 40.);
    assert_eq!(joint_tenons(f.s.document(), id)[0].2, tenon);
    // A loose-tenon joint does not turn into a corner joint.
    assert!(
        prepare_panel_joint_edit(&f.s, f.s.revision(), actor(), id, &PanelJointKind::Miter)
            .is_err()
    );

    f.s.undo(f.s.revision(), actor()).unwrap();
    near(volume(&compile(&f.s), tenon), 2. * slot(22., 8.) * 40.);
    f.s.redo(f.s.revision(), actor()).unwrap();
    near(volume(&compile(&f.s), tenon), 3. * slot(22., 8.) * 40.);

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
    assert!(opened.capabilities().contains(LOOSE_TENON_CAPABILITY));
    assert!(panel_joints_need_loose_tenon(&opened.document));
    let mut older = project_file::supported_capabilities();
    older.remove(LOOSE_TENON_CAPABILITY);
    let Err(refused) = project_file::read(&mut bytes.as_slice(), Default::default(), &older) else {
        panic!("an older build refuses the file")
    };
    assert!(
        refused.to_string().contains(LOOSE_TENON_CAPABILITY),
        "{refused}"
    );
    assert_eq!(&opened.document, f.s.document());
    let j = read_panel_joint(&opened.document, id).unwrap();
    assert_eq!(j.kind, PanelJointKind::LooseTenon(Box::new(three)));
    assert_eq!((j.pairs[0].a, j.pairs[0].b), (side, top));
    let reopened = Compiler::default().compile(&opened.document, 0).unwrap();
    near(volume(&reopened, tenon), 3. * slot(22., 8.) * 40.);

    let p = prepare_panel_joint_delete(&f.s, f.s.revision(), actor(), id).unwrap();
    checked(&mut f.s, p);
    assert!(f.s.document().get(tenon).is_none());
    assert!(joint_tenons(f.s.document(), id).is_empty());
    let m = compile(&f.s);
    near(volume(&m, side), 18. * 400. * 720.);
    near(volume(&m, top), 18. * 400. * 564.);
}
