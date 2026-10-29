//! Schedules from real placed models: a deterministic BOM keyed by Part with
//! quantities from physical occurrences, supply classification and explicit
//! inclusion rules for aids, purchased Parts and stack layers; CSV and
//! drawing tables built from the same records, with stale detection; and
//! panel cut list and machining schedules from cabinetry records.
use multicad_core::{
    commands::{Actor, Channel, Prepared, Request, Session},
    document::{Document, Entity, EntityData, EntityId, Mutation},
    materials::*,
    model::*,
};
use multicad_model::{
    Compiler, Model,
    schedules::{BomOptions, Supply, bom_schedule, cut_list_schedule, machining_schedule},
};

fn actor() -> Actor {
    Actor::local(Channel::Ui)
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
fn apply(s: &mut Session, edits: Vec<Mutation>) {
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Fixture".into(),
        edits,
    })
    .unwrap();
}

struct Fixture {
    s: Session,
    ply: EntityId,
    carcass: EntityId,
    handle: PrimitiveRequest,
    caster: PrimitiveRequest,
    width: EntityId,
}
/// A carcass, a handle placed twice as a component, a purchased caster and
/// a two-layer laminated top.
fn fixture() -> Fixture {
    let mut s = Session::new(Document::new("Schedules").unwrap()).unwrap();
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
            length_mm: 2500.,
            width_mm: 1250.,
            grain: Grain::AlongLength,
        },
    })
    .entity(EntityId::new(), "Ply 18")
    .unwrap();
    let ply_id = ply.id;
    s.commit(prepare_library(&s, 0, actor(), &[birch, ply]).unwrap())
        .unwrap();
    let width = Entity::new(
        "w",
        EntityData::Parameter {
            expression: "600".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let width_id = width.id;
    apply(&mut s, vec![Mutation::Insert(width)]);
    let carcass = prepare_carcass(
        &s,
        s.revision(),
        actor(),
        &CarcassRequest::new(
            "Case",
            CarcassDefinition {
                origin: [length(0.), length(0.), length(0.)],
                rotation: angle(0.),
                width: multicad_core::constraint_sketch::Dimension::Expression {
                    expression: multicad_core::parameters::BoundExpression::bind(
                        s.document(),
                        "=$w",
                        "mm",
                        None,
                    )
                    .unwrap(),
                },
                height: length(400.),
                depth: length(400.),
                stock: ply_id,
                construction: Construction::SidesFullHeight,
                back: Back::None,
                shelves: Shelves {
                    layout: ShelfLayout::Even { count: 1 },
                    setback: length(0.),
                    clearance: length(0.),
                },
                front_band: None,
                group: None,
            },
        ),
    )
    .unwrap();
    let carcass_id = carcass.carcass;
    checked(&mut s, carcass.model.prepared);
    let handle = PrimitiveRequest::new(
        "Handle",
        PrimitiveParameters::Box {
            size: [120., 20., 20.].map(length),
            frame: PrimitiveFrame {
                origin: [0., -100., 0.].map(length),
                ..Default::default()
            },
        },
    );
    let p = prepare_primitive(&s, s.revision(), actor(), &handle).unwrap();
    checked(&mut s, p.prepared);
    for x in [100., 400.] {
        let place = ComponentPlacementRequest::new(
            handle.body,
            NumericTransform {
                translation: [length(x), length(-30.), length(300.)],
                ..Default::default()
            },
        );
        let p = prepare_place_component(&s, s.revision(), actor(), &place).unwrap();
        checked(&mut s, p.prepared);
    }
    let caster = PrimitiveRequest::new(
        "Caster",
        PrimitiveParameters::Cylinder {
            radius: length(25.),
            height: length(40.),
            frame: PrimitiveFrame {
                origin: [50., 50., -40.].map(length),
                ..Default::default()
            },
        },
    );
    let p = prepare_primitive(&s, s.revision(), actor(), &caster).unwrap();
    checked(&mut s, p.prepared);
    let p = prepare_tags(
        &s,
        s.revision(),
        actor(),
        &[caster.body],
        &TagEdit::Add(["purchased".to_string()].into()),
    )
    .unwrap();
    checked(&mut s, p);
    Fixture {
        s,
        ply: ply_id,
        carcass: carcass_id,
        handle,
        caster,
        width: width_id,
    }
}

#[test]
fn bom_counts_occurrences_classifies_supply_and_applies_inclusion_rules() {
    let mut f = fixture();
    // A laminated top: two rectangle panels stacked face to face.
    let layer = |z: f64, name: &str| {
        PanelRequest::new(
            name,
            PanelDefinition {
                output: BodyOutput {
                    body: EntityId::new(),
                },
                source: PanelSource::Rectangle {
                    stock: f.ply,
                    plane: PanelPlane::Xy,
                    origin: [length(0.), length(0.), length(z)],
                    length: length(600.),
                    width: length(400.),
                },
                grain: GrainAxis::U,
                edges: PanelEdges::default(),
                oversize: None,
            },
        )
    };
    let (a, b) = (layer(400., "Top core"), layer(418., "Top face"));
    for p in [&a, &b] {
        let prepared = prepare_panel(&f.s, f.s.revision(), actor(), p).unwrap();
        checked(&mut f.s, prepared.prepared);
    }
    let (_, p) = prepare_stack(
        &f.s,
        f.s.revision(),
        actor(),
        "Top",
        &[a.body, b.body],
        None,
        false,
    )
    .unwrap();
    checked(&mut f.s, p);
    // A breakaway tab on the handle is an FDM aid.
    let tab = AidRequest::new(
        "Handle tab",
        AidDefinition {
            output: BodyOutput {
                body: EntityId::new(),
            },
            target: f.handle.body,
            shape: AidShape::Tab {
                origin: [length(120.), length(-90.), length(0.)],
                angle: angle(0.),
                length: length(8.),
                width: length(6.),
                thickness: length(0.6),
                overlap: length(2.),
            },
            process: None,
        },
    );
    let p = prepare_aid(&f.s, f.s.revision(), actor(), &tab).unwrap();
    checked(&mut f.s, p.prepared);

    let m = compile(&f.s);
    let bom = m.bom(f.s.document(), &BomOptions::default()).unwrap();
    let row = |name: &str| bom.rows.iter().find(|r| r.name == name);
    // The handle body and its two placed copies are three physical
    // occurrences of one Part: one row with quantity 3.
    let handle = row("Handle").unwrap();
    assert_eq!(handle.quantity, 3);
    assert_eq!(handle.occurrences.len(), handle.quantity);
    // Carcass panels: one row each, with their material and stock.
    let side = row("Case left side").unwrap();
    assert_eq!(side.quantity, 1);
    assert_eq!(side.stock.as_ref().unwrap().1, "Ply 18");
    assert_eq!(side.material.as_ref().unwrap().1, "Birch plywood");
    assert_eq!(side.supply, Supply::Manufactured);
    assert_eq!(row("Caster").unwrap().supply, Supply::Purchased);
    // Stack layers are listed individually with their stack.
    assert_eq!(row("Top core").unwrap().stack.as_ref().unwrap().1, "Top");
    assert_eq!(row("Top face").unwrap().stack.as_ref().unwrap().1, "Top");
    // The aid is excluded by default and listed with the reason.
    assert!(row("Handle tab").is_none());
    assert!(bom.excluded.iter().any(|(_, why)| why == "FDM aid"));
    let with_aids = m
        .bom(
            f.s.document(),
            &BomOptions {
                include_aids: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(
        with_aids
            .rows
            .iter()
            .find(|r| r.name == "Handle tab")
            .unwrap()
            .supply,
        Supply::Aid
    );
    let made = m
        .bom(
            f.s.document(),
            &BomOptions {
                include_purchased: false,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(made.rows.iter().all(|r| r.supply == Supply::Manufactured));
    // Purchased Parts sort after manufactured ones; the order is stable.
    assert_eq!(bom.rows.last().unwrap().name, "Caster");
    let again = compile(&f.s)
        .bom(f.s.document(), &BomOptions::default())
        .unwrap();
    assert_eq!(again, bom);

    // CSV and the drawing table come from the same schedule.
    let schedule = bom_schedule(&bom);
    let csv = schedule.csv();
    assert_eq!(csv, bom_schedule(&again).csv());
    assert!(csv.starts_with("# MultiCAD bom schedule\n# revision "));
    let lines: Vec<_> = csv.lines().filter(|l| !l.starts_with('#')).collect();
    assert_eq!(lines[0], "Item,Part,Qty,Material,Stock,Supply,Stack");
    assert_eq!(lines.len(), bom.rows.len() + 1);
    let table = schedule.table([20., 20.]);
    schedule.check_table(&table).unwrap();

    // A source change makes the placed table stale.
    let mut w = f.s.document().get(f.width).unwrap().clone();
    if let EntityData::Parameter { expression, .. } = &mut w.data {
        *expression = "700".into();
    }
    apply(&mut f.s, vec![Mutation::Replace(w)]);
    let m = compile(&f.s);
    let records = m.cut_list(f.s.document()).unwrap();
    let cuts = cut_list_schedule(f.s.document(), f.s.revision(), &records).unwrap();
    let old_cuts = cut_list_schedule(
        f.s.document(),
        f.s.revision(),
        &compile(&{
            let mut s = Session::new(f.s.document().clone()).unwrap();
            let mut w = s.document().get(f.width).unwrap().clone();
            if let EntityData::Parameter { expression, .. } = &mut w.data {
                *expression = "600".into();
            }
            apply(&mut s, vec![Mutation::Replace(w)]);
            s
        })
        .cut_list(f.s.document())
        .unwrap(),
    )
    .unwrap();
    let stale = cuts.check_table(&old_cuts.table([0., 0.])).unwrap_err();
    assert!(stale.contains("stale"), "{stale}");
    let top = cuts.rows.iter().find(|r| r[0] == "Case top").unwrap();
    assert_eq!(top[4], "664");
    assert!(
        cuts.csv()
            .contains("Case top,top,Ply 18,18,664,400,664,400,")
    );
    let _ = (f.carcass, f.caster.body);
}

#[test]
fn csv_quotes_separators_and_formula_starts() {
    let mut f = fixture();
    let p = prepare_tags(
        &f.s,
        f.s.revision(),
        actor(),
        &[f.handle.body],
        &TagEdit::Add(["purchased".to_string()].into()),
    )
    .unwrap();
    checked(&mut f.s, p);
    let mut caster = f.s.document().get(f.caster.body).unwrap().clone();
    caster.name = "=HYPERLINK(\"x\"), \"big\"".into();
    apply(&mut f.s, vec![Mutation::Replace(caster)]);
    let bom = compile(&f.s)
        .bom(f.s.document(), &BomOptions::default())
        .unwrap();
    let csv = bom_schedule(&bom).csv();
    assert!(
        csv.contains("\"=HYPERLINK(\"\"x\"\"), \"\"big\"\"\""),
        "{csv}"
    );
}

#[test]
fn machining_schedule_groups_holes_by_size_and_lists_recesses() {
    let mut f = fixture();
    let side = carcass_panels(f.s.document(), f.carcass)
        .unwrap()
        .into_iter()
        .find(|(role, ..)| *role == PanelRole::LeftSide)
        .unwrap()
        .2;
    for (name, kind) in [
        (
            "Pins",
            MachiningKind::Drill {
                holes: vec![
                    [length(100.), length(100.)],
                    [length(100.), length(132.)],
                    [length(100.), length(164.)],
                ],
                diameter: length(5.),
                depth: Some(length(10.)),
            },
        ),
        (
            "Back groove",
            MachiningKind::Groove {
                along: PanelAxis::V,
                offset: length(370.),
                width: length(6.),
                depth: length(8.),
            },
        ),
    ] {
        let p = prepare_machining(
            &f.s,
            f.s.revision(),
            actor(),
            name,
            &MachiningDefinition {
                kind,
                face: MachiningFace::Top,
                targets: vec![side],
            },
        )
        .unwrap();
        checked(&mut f.s, p.model.prepared);
    }
    let m = compile(&f.s);
    let records = m.machining(f.s.document()).unwrap();
    let schedule = machining_schedule(f.s.document(), f.s.revision(), &records).unwrap();
    assert!(schedule.rows.contains(&vec![
        "Case left side".to_string(),
        "Pins".into(),
        "holes".into(),
        "Ø5".into(),
        "10".into(),
        "3".into(),
    ]));
    assert!(schedule.rows.contains(&vec![
        "Case left side".to_string(),
        "Back groove".into(),
        "groove".into(),
        "400 × 6".into(),
        "8".into(),
        String::new(),
    ]));
}
