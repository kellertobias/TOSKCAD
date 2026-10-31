//! Cabinetry corner joints: butt with either panel through, miter and finger
//! joints own the matching cuts in both panels, rebuild both sides on stock
//! changes, refuse impossible corners, and survive Undo/Redo, deletion and
//! save/reopen. Volumes and sizes are measured from the native solids.
use multicad_core::{
    commands::{Actor, Channel, Prepared, Session},
    document::{Document, Entity, EntityData, EntityId},
    materials::*,
    model::{
        self, BodyOutput, GrainAxis, PanelDefinition, PanelEdges, PanelJointDefinition,
        PanelJointKind, PanelJointPair, PanelPlane, PanelRequest, PanelSource, length,
    },
    project_file::{self, ProjectFile},
};
use multicad_model::{Compiler, Model};

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
    m.solid(m.snapshot().bodies[&part])
        .unwrap()
        .volume_mm3()
        .unwrap()
}

struct Box3 {
    s: Session,
    stock: EntityId,
    side: PanelRequest,
    other: PanelRequest,
    bottom: PanelRequest,
}
fn panel(
    stock: EntityId,
    name: &str,
    plane: PanelPlane,
    origin: [f64; 3],
    l: f64,
    w: f64,
) -> PanelRequest {
    PanelRequest::new(
        name,
        PanelDefinition {
            output: BodyOutput {
                body: EntityId::new(),
            },
            source: PanelSource::Rectangle {
                stock,
                plane,
                origin: origin.map(length),
                length: length(l),
                width: length(w),
            },
            grain: GrainAxis::U,
            edges: PanelEdges::default(),
            oversize: None,
        },
    )
}
/// A 600 × 400 bottom with an 18 mm side standing on each end; all three
/// reach the outside corners, so each corner block is 18 × 18 × 400.
fn fixture() -> Box3 {
    let mut s = Session::new(Document::new("Joints").unwrap()).unwrap();
    let birch = Entity::new(
        "Birch plywood",
        EntityData::Material {
            density_kg_m3: None,
            color_rgba: [214, 186, 140, 255],
            section_hatch: None,
        },
    );
    let stock = LibraryDefinition::Stock(Stock {
        material: birch.id,
        specification: StockKind::Panel {
            thickness_mm: 18.,
            length_mm: 2500.,
            width_mm: 1250.,
            grain: Grain::AlongLength,
        },
    })
    .entity(EntityId::new(), "Birch ply 18")
    .unwrap();
    let id = stock.id;
    s.commit(prepare_library(&s, 0, actor(), &[birch, stock]).unwrap())
        .unwrap();
    let side = panel(id, "Left side", PanelPlane::Yz, [0., 0., 0.], 400., 300.);
    let other = panel(id, "Right side", PanelPlane::Yz, [582., 0., 0.], 400., 300.);
    let bottom = panel(id, "Bottom", PanelPlane::Xy, [0., 0., 0.], 600., 400.);
    for p in [&side, &other, &bottom] {
        let prepared = model::prepare_panel(&s, s.revision(), actor(), p).unwrap();
        checked(&mut s, prepared.prepared);
    }
    Box3 {
        s,
        stock: id,
        side,
        other,
        bottom,
    }
}
const SIDE: f64 = 18. * 400. * 300.;
const BOTTOM: f64 = 600. * 400. * 18.;
const BLOCK: f64 = 18. * 18. * 400.;

fn joint(
    f: &mut Box3,
    name: &str,
    kind: PanelJointKind,
    pairs: &[(EntityId, EntityId)],
) -> EntityId {
    let def = PanelJointDefinition {
        kind,
        pairs: pairs
            .iter()
            .map(|(a, b)| PanelJointPair { a: *a, b: *b })
            .collect(),
    };
    let p = model::prepare_panel_joint(&f.s, f.s.revision(), actor(), name, &def).unwrap();
    assert_eq!(p.model.outputs.len(), 2 * pairs.len());
    checked(&mut f.s, p.model.prepared);
    p.joint
}

#[test]
fn each_joint_kind_gives_both_panels_their_share_of_the_corner() {
    let cases = [
        (PanelJointKind::ButtAThrough, SIDE, BOTTOM - BLOCK),
        (PanelJointKind::ButtBThrough, SIDE - BLOCK, BOTTOM),
        (
            PanelJointKind::Miter,
            SIDE - BLOCK / 2.,
            BOTTOM - BLOCK / 2.,
        ),
        (
            PanelJointKind::Finger {
                count: 5,
                fit: length(0.2),
            },
            // A keeps fingers 1, 3, 5 and loses 2 and 4, each 80 mm + 0.2 fit.
            SIDE - 2. * 80.2 * 18. * 18.,
            // B loses the end fingers (80.1) and the middle one (80.2).
            BOTTOM - (80.1 + 80.2 + 80.1) * 18. * 18.,
        ),
    ];
    for (kind, side, bottom) in cases {
        let mut f = fixture();
        let (a, b) = (f.side.body, f.bottom.body);
        let label = format!("{kind:?}");
        joint(&mut f, "Corner", kind, &[(a, b)]);
        let m = compile(&f.s);
        near(volume(&m, a), side);
        near(volume(&m, b), bottom);
        // Panels keep their Parts; the cut list follows the shortened panel.
        let list = m.cut_list(f.s.document()).unwrap();
        let rec = |part| list.iter().find(|r| r.part == part).unwrap();
        match label.as_str() {
            "ButtAThrough" => near(rec(b).cut.finished_mm[0], 582.),
            "ButtBThrough" => near(rec(a).cut.finished_mm[1], 282.),
            _ => {
                near(rec(a).cut.finished_mm[0], 400.);
                near(rec(b).cut.finished_mm[0], 600.);
            }
        }
        assert_eq!(rec(a).joints.len(), 1);
    }
}

#[test]
fn one_apply_joins_several_pairs_and_stock_changes_rebuild_both_sides() {
    let mut f = fixture();
    let (l, r, b) = (f.side.body, f.other.body, f.bottom.body);
    let j = joint(
        &mut f,
        "Box corners",
        PanelJointKind::Finger {
            count: 5,
            fit: length(0.),
        },
        &[(l, b), (r, b)],
    );
    let m = compile(&f.s);
    near(volume(&m, l), SIDE - 2. * 80. * 324.);
    near(volume(&m, r), SIDE - 2. * 80. * 324.);
    near(volume(&m, b), BOTTOM - 2. * 3. * 80. * 324.);
    // The bottom carries two chained joint cuts.
    let EntityData::Part { features, .. } = &f.s.document().get(b).unwrap().data else {
        unreachable!()
    };
    assert_eq!(features.len(), 3);

    // Undo and Redo restore the exact documents.
    let after = f.s.document().clone();
    f.s.undo(f.s.revision(), actor()).unwrap();
    assert!(model::panel_joints(f.s.document()).next().is_none());
    f.s.redo(f.s.revision(), actor()).unwrap();
    assert_eq!(f.s.document(), &after);

    // Changing the joint kind rebuilds both sides of both pairs.
    let p =
        model::prepare_panel_joint_edit(&f.s, f.s.revision(), actor(), j, &PanelJointKind::Miter)
            .unwrap();
    checked(&mut f.s, p.model.prepared);
    let m = compile(&f.s);
    near(volume(&m, l), SIDE - BLOCK / 2.);
    near(volume(&m, b), BOTTOM - BLOCK);

    // Thicker stock moves both mating faces of a corner that stays flush
    // (the left corner: both panels start at the outside faces).
    let mut g = fixture();
    let (gl, gb) = (g.side.body, g.bottom.body);
    joint(&mut g, "Left corner", PanelJointKind::Miter, &[(gl, gb)]);
    let stock = g.s.document().get(g.stock).unwrap().clone();
    let Some(LibraryDefinition::Stock(mut spec)) = read(&stock).unwrap() else {
        unreachable!()
    };
    let StockKind::Panel { thickness_mm, .. } = &mut spec.specification else {
        unreachable!()
    };
    *thickness_mm = 19.;
    let replaced = LibraryDefinition::Stock(spec)
        .entity(g.stock, stock.name.clone())
        .unwrap();
    let p = prepare_library(&g.s, g.s.revision(), actor(), &[replaced]).unwrap();
    checked(&mut g.s, p);
    let gm = compile(&g.s);
    let block = 19. * 19. * 400.;
    near(volume(&gm, gl), 19. * 400. * 300. - block / 2.);
    near(volume(&gm, gb), 600. * 400. * 19. - block / 2.);
    // On the full box the right side no longer meets the bottom flush, so
    // the same stock change is refused natively instead of mis-cutting.
    let stock = f.s.document().get(f.stock).unwrap().clone();
    let Some(LibraryDefinition::Stock(mut spec)) = read(&stock).unwrap() else {
        unreachable!()
    };
    let StockKind::Panel { thickness_mm, .. } = &mut spec.specification else {
        unreachable!()
    };
    *thickness_mm = 19.;
    let replaced = LibraryDefinition::Stock(spec)
        .entity(f.stock, stock.name.clone())
        .unwrap();
    let p = prepare_library(&f.s, f.s.revision(), actor(), &[replaced]).unwrap();
    let Err(e) = Compiler::default().compile(p.document(), 0) else {
        panic!("a misaligned corner must not compile")
    };
    assert!(e.to_string().contains("full thickness"), "{e}");
    let m = compile(&f.s);

    // Save and reopen keep the joint and its geometry.
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
    assert!(
        opened
            .capabilities()
            .contains(model::PANEL_JOINT_CAPABILITY)
    );
    assert!(
        opened
            .capabilities()
            .contains(model::PANEL_JOINT_CUT_CAPABILITY)
    );
    assert!(
        !opened
            .capabilities()
            .contains(model::LOOSE_TENON_CAPABILITY)
    );
    assert_eq!(&opened.document, f.s.document());
    let reopened = Compiler::default().compile(&opened.document, 0).unwrap();
    near(volume(&reopened, b), volume(&m, b));
    near(volume(&reopened, r), SIDE - BLOCK / 2.);
}

#[test]
fn deleting_a_joint_reattaches_later_cuts_and_restores_its_panels() {
    let mut f = fixture();
    let (l, r, b) = (f.side.body, f.other.body, f.bottom.body);
    let first = joint(&mut f, "Left", PanelJointKind::ButtAThrough, &[(l, b)]);
    joint(&mut f, "Right", PanelJointKind::ButtAThrough, &[(r, b)]);
    near(volume(&compile(&f.s), b), BOTTOM - 2. * BLOCK);
    let p = model::prepare_panel_joint_delete(&f.s, f.s.revision(), actor(), first).unwrap();
    checked(&mut f.s, p);
    let m = compile(&f.s);
    near(volume(&m, b), BOTTOM - BLOCK);
    near(volume(&m, l), SIDE);
    assert_eq!(model::panel_joints(f.s.document()).count(), 1);
    let EntityData::Part { features, .. } = &f.s.document().get(b).unwrap().data else {
        unreachable!()
    };
    assert_eq!(features.len(), 2);
}

#[test]
fn impossible_corners_and_finger_counts_are_refused() {
    let f = fixture();
    let (a, b) = (f.side.body, f.bottom.body);
    let try_joint = |s: &Session, kind: PanelJointKind, a: EntityId, b: EntityId| {
        model::prepare_panel_joint(
            s,
            s.revision(),
            actor(),
            "Bad",
            &PanelJointDefinition {
                kind,
                pairs: vec![PanelJointPair { a, b }],
            },
        )
        .unwrap_err()
        .message
    };
    let e = try_joint(
        &f.s,
        PanelJointKind::Finger {
            count: 100,
            fit: length(4.),
        },
        a,
        b,
    );
    assert!(e.contains("no wider"), "{e}");
    let e = try_joint(
        &f.s,
        PanelJointKind::Finger {
            count: 1,
            fit: length(0.),
        },
        a,
        b,
    );
    assert!(e.contains("2–128"), "{e}");
    // The two sides are parallel.
    let e = try_joint(&f.s, PanelJointKind::Miter, a, f.other.body);
    assert!(
        e.contains("do not overlap") || e.contains("parallel"),
        "{e}"
    );
    // A bottom that starts half way into the side does not cover its thickness.
    let mut f = f;
    let shifted = panel(f.stock, "Shifted", PanelPlane::Xy, [9., 0., 0.], 600., 400.);
    let p = model::prepare_panel(&f.s, f.s.revision(), actor(), &shifted).unwrap();
    checked(&mut f.s, p.prepared);
    let e = try_joint(&f.s, PanelJointKind::Miter, a, shifted.body);
    assert!(e.contains("full thickness"), "{e}");
    // Panels apart from each other.
    let apart = panel(
        f.stock,
        "Apart",
        PanelPlane::Xy,
        [100., 0., 500.],
        600.,
        400.,
    );
    let p = model::prepare_panel(&f.s, f.s.revision(), actor(), &apart).unwrap();
    checked(&mut f.s, p.prepared);
    let e = try_joint(&f.s, PanelJointKind::Miter, a, apart.body);
    assert!(e.contains("do not overlap"), "{e}");
    // Nothing was staged by any refusal.
    assert!(model::panel_joints(f.s.document()).next().is_none());
}
