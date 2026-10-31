//! System 32 shelf-pin rows on the inside faces of both carcass sides: one
//! operation cuts aligned blind holes into each side's inner face, follows
//! carcass changes, lists the shelf support levels both sides share, and a
//! shelf placed at such a level rests on the pins. Edits, suppression,
//! deletion, Undo/Redo and save/reopen keep the pattern and its targets;
//! depths, edge breakouts and overlaps are refused before commit.
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
fn volume(m: &Model, part: EntityId) -> f64 {
    let snapshot = m.snapshot();
    let head = snapshot
        .bodies
        .get(&part)
        .or_else(|| snapshot.body_sources.get(&part))
        .unwrap();
    m.solid(*head).unwrap().volume_mm3().unwrap()
}
fn bounds(m: &Model, part: EntityId) -> multicad_kernel::Bounds {
    m.solid(m.snapshot().bodies[&part])
        .unwrap()
        .bounds()
        .unwrap()
}

struct Fixture {
    s: Session,
    carcass: EntityId,
    definition: CarcassDefinition,
    left: EntityId,
    right: EntityId,
}
/// A 600 × 720 × 400 mm carcass of 18 mm ply without back or shelves.
fn fixture() -> Fixture {
    let mut s = Session::new(Document::new("System 32").unwrap()).unwrap();
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
    let side = |role| {
        created
            .panels
            .iter()
            .find(|(r, _)| *r == role)
            .unwrap()
            .1
            .body
    };
    let (left, right) = (side(PanelRole::LeftSide), side(PanelRole::RightSide));
    let carcass = created.carcass;
    checked(&mut s, created.model.prepared);
    Fixture {
        s,
        carcass,
        definition,
        left,
        right,
    }
}
/// Example rows: Ø5 × 10 mm, 37 mm from front and rear, every 32 mm from
/// 100 mm above the bottom edge to 100 mm below the top edge.
fn pins(targets: Vec<EntityId>) -> MachiningDefinition {
    MachiningDefinition {
        kind: MachiningKind::ShelfPins {
            along: PanelAxis::V,
            rows: vec![
                ShelfPinRow {
                    from: RowEdge::Start,
                    offset: length(37.),
                },
                ShelfPinRow {
                    from: RowEdge::End,
                    offset: length(37.),
                },
            ],
            pitch: length(SYSTEM32_PITCH_MM),
            start: length(100.),
            end: length(100.),
            count: None,
            diameter: length(5.),
            depth: length(10.),
        },
        face: MachiningFace::Inside,
        targets,
        suppressed: false,
    }
}
fn hole(n: usize) -> f64 {
    n as f64 * PI * 2.5 * 2.5 * 10.
}

#[test]
fn rows_cut_aligned_inside_holes_and_shelves_rest_on_shared_levels() {
    let mut f = fixture();
    let side = 18. * 400. * 720.;
    let p = prepare_machining(
        &f.s,
        f.s.revision(),
        actor(),
        "Shelf pins",
        &pins(vec![f.left, f.right]),
    )
    .unwrap();
    let op = p.operation;
    assert_eq!(p.model.outputs.len(), 2);
    checked(&mut f.s, p.model.prepared);
    let m = compile(&f.s);
    // (720 − 200) / 32 → 17 holes per row, two rows per side.
    near(volume(&m, f.left), side - hole(34));
    near(volume(&m, f.right), side - hole(34));
    let records = m.machining(f.s.document()).unwrap();
    assert_eq!(records.len(), 2);
    let of = |part| records.iter().find(|r| r.part == part).unwrap();
    let (l, r) = (of(f.left), of(f.right));
    // Each side is cut on its inner face: left at x = 18, right at x = 582.
    assert_eq!(l.face, MachiningFace::Top);
    assert_eq!(r.face, MachiningFace::Bottom);
    assert_eq!(l.kind, "shelf pins");
    assert_eq!(l.holes.len(), 34);
    for (a, b) in l.holes.iter().zip(&r.holes) {
        near(a.center[0], 18.);
        near(b.center[0], 582.);
        near(a.center[1], b.center[1]);
        near(a.center[2], b.center[2]);
        assert_eq!(a.axis, [-1., 0., 0.]);
        assert_eq!(b.axis, [1., 0., 0.]);
        assert!(!a.through);
        near(a.depth_mm, 10.);
    }
    let first = &l.holes[0];
    near(first.center[1], 37.);
    near(first.center[2], 100.);
    near(l.holes[17].center[1], 400. - 37.);
    near(l.holes[16].center[2], 100. + 16. * 32.);
    near(l.removed_mm3, hole(34));
    // The machining schedule groups the same holes.
    let schedule =
        multicad_model::schedules::machining_schedule(f.s.document(), f.s.revision(), &records)
            .unwrap();
    assert!(
        schedule
            .rows
            .iter()
            .any(|row| row[2] == "holes" && row[3] == "Ø5" && row[5] == "34")
    );

    // Support levels: a shelf underside rests on the 5 mm pin, 2.5 mm above
    // the hole center, measured from the interior bottom at 18 mm.
    let parameters = multicad_core::parameters::evaluate(f.s.document()).unwrap();
    let levels = shelf_support_offsets(f.s.document(), &parameters, f.carcass).unwrap();
    near(levels[0], 100. + 2.5 - 18.);
    near(levels[1] - levels[0], 32.);
    // 17 levels, all with room for an 18 mm shelf below the top.
    assert_eq!(levels.len(), 17);
    let mut shelf = f.definition.clone();
    shelf.shelves.layout = ShelfLayout::Offsets {
        offsets: vec![length(levels[3])],
    };
    let edit = prepare_carcass_edit(&f.s, f.s.revision(), actor(), f.carcass, &shelf).unwrap();
    let shelf_part = edit
        .panels
        .iter()
        .find(|(role, _)| matches!(role, PanelRole::Shelf { .. }))
        .unwrap()
        .1
        .body;
    checked(&mut f.s, edit.model.prepared);
    let m = compile(&f.s);
    let pin_top = l.holes[3].center[2] + 2.5;
    near(bounds(&m, shelf_part).min[2], pin_top);

    // A taller cabinet rebuilds both rows in place: 20 holes per row.
    let mut taller = shelf.clone();
    taller.height = length(820.);
    let edit = prepare_carcass_edit(&f.s, f.s.revision(), actor(), f.carcass, &taller).unwrap();
    checked(&mut f.s, edit.model.prepared);
    let m = compile(&f.s);
    let records = m.machining(f.s.document()).unwrap();
    assert!(
        records
            .iter()
            .all(|r| r.holes.len() == 40 && r.operation == op)
    );
    near(volume(&m, f.right), 18. * 400. * 820. - hole(40));
}

#[test]
fn refused_rows_leave_the_document_unchanged() {
    let mut f = fixture();
    let both = vec![f.left, f.right];
    let refuse = |f: &Fixture, def: MachiningDefinition, needle: &str| {
        let error = prepare_machining(&f.s, f.s.revision(), actor(), "Bad", &def)
            .unwrap_err()
            .to_string();
        assert!(error.contains(needle), "{error}");
    };
    let with = |edit: &dyn Fn(&mut MachiningKind)| {
        let mut def = pins(vec![f.left, f.right]);
        edit(&mut def.kind);
        def
    };
    // As deep as the 18 mm side: it would cut through.
    refuse(
        &f,
        with(&|k| {
            if let MachiningKind::ShelfPins { depth, .. } = k {
                *depth = length(18.);
            }
        }),
        "cut through",
    );
    // A row 1 mm from the front edge breaks out of the face.
    refuse(
        &f,
        with(&|k| {
            if let MachiningKind::ShelfPins { rows, .. } = k {
                rows[0].offset = length(1.);
            }
        }),
        "leaves the",
    );
    // A 4 mm pitch puts 5 mm holes into each other.
    refuse(
        &f,
        with(&|k| {
            if let MachiningKind::ShelfPins { pitch, count, .. } = k {
                *pitch = length(4.);
                *count = Some(3);
            }
        }),
        "overlap",
    );
    // More holes than fit between the limits.
    refuse(
        &f,
        with(&|k| {
            if let MachiningKind::ShelfPins { count, .. } = k {
                *count = Some(18);
            }
        }),
        "17 fit",
    );
    let before = f.s.document().clone();
    let p = prepare_machining(&f.s, f.s.revision(), actor(), "Pins", &pins(both.clone())).unwrap();
    checked(&mut f.s, p.model.prepared);
    // The same rows again overlap the applied holes.
    refuse(&f, pins(both.clone()), "overlaps a hole of Pins");
    assert_ne!(f.s.document(), &before);
}

#[test]
fn rows_edit_suppress_delete_undo_and_reopen_with_their_targets() {
    let mut f = fixture();
    let def = pins(vec![f.left, f.right]);
    let p = prepare_machining(&f.s, f.s.revision(), actor(), "Pins", &def).unwrap();
    let op = p.operation;
    checked(&mut f.s, p.model.prepared);
    let side = 18. * 400. * 720.;

    // A fixed count of five holes per row.
    let mut five = def.clone();
    if let MachiningKind::ShelfPins { count, .. } = &mut five.kind {
        *count = Some(5);
    }
    let p = prepare_machining_edit(&f.s, f.s.revision(), actor(), op, &five).unwrap();
    checked(&mut f.s, p.model.prepared);
    near(volume(&compile(&f.s), f.left), side - hole(10));
    // Targets stay fixed.
    let mut other = five.clone();
    other.targets.pop();
    assert!(prepare_machining_edit(&f.s, f.s.revision(), actor(), op, &other).is_err());

    // Suppressed: the cuts stay in the history but remove nothing.
    let mut off = five.clone();
    off.suppressed = true;
    let p = prepare_machining_edit(&f.s, f.s.revision(), actor(), op, &off).unwrap();
    checked(&mut f.s, p.model.prepared);
    let m = compile(&f.s);
    near(volume(&m, f.left), side);
    let parameters = multicad_core::parameters::evaluate(f.s.document()).unwrap();
    assert!(
        shelf_support_offsets(f.s.document(), &parameters, f.carcass)
            .unwrap()
            .is_empty()
    );
    assert_eq!(read_machining(f.s.document(), op).unwrap(), off);

    // Undo restores the five-hole rows; Redo suppresses again.
    f.s.undo(f.s.revision(), actor()).unwrap();
    near(volume(&compile(&f.s), f.left), side - hole(10));
    f.s.redo(f.s.revision(), actor()).unwrap();
    near(volume(&compile(&f.s), f.left), side);
    f.s.undo(f.s.revision(), actor()).unwrap();

    // Save and reopen keep the pattern, its targets and solids.
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
    assert_eq!(&opened.document, f.s.document());
    assert_eq!(read_machining(&opened.document, op).unwrap(), five);
    let reopened = Compiler::default().compile(&opened.document, 0).unwrap();
    near(volume(&reopened, f.right), side - hole(10));

    // Delete restores both sides.
    let p = prepare_machining_delete(&f.s, f.s.revision(), actor(), op).unwrap();
    checked(&mut f.s, p);
    let m = compile(&f.s);
    near(volume(&m, f.left), side);
    near(volume(&m, f.right), side);
    assert!(m.machining(f.s.document()).unwrap().is_empty());
}
