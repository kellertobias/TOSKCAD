//! Sheet-metal forming: flanges, hems and jogs on base edges rebuild their
//! folded geometry from thickness, radius, angle and length; corner insets
//! and reliefs, cutouts in the base and in flange legs, the developed flat
//! pattern and bend records of the same feature; refused forming leaves the
//! last valid part. Every value is measured from native solids.
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Mutation},
    materials::*,
    model::{
        self, BendDirection, BodyOutput, Cutout, CutoutFace, CutoutShape, Flange, FlangeKind,
        PanelPlane, Relief, SheetBase, SheetDefinition, SheetRequest, angle, length,
    },
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
};
use multicad_model::{Compiler, Model, SheetRecord};
use std::f64::consts::PI;

fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() <= 1e-6 * b.abs().max(1.), "{a} != {b}");
}
fn compile(s: &Session) -> Model {
    Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap()
}
fn record(s: &Session) -> SheetRecord {
    compile(s).sheet_metal(s.document()).unwrap().remove(0)
}
fn bounds(s: &Session, part: EntityId) -> multicad_kernel::Bounds {
    let m = compile(s);
    m.solid(m.snapshot().bodies[&part])
        .unwrap()
        .bounds()
        .unwrap()
}

struct Fixture {
    s: Session,
    stock: EntityId,
    angle: EntityId,
}
fn fixture(k: f64) -> Fixture {
    let mut s = Session::new(Document::new("Forming").unwrap()).unwrap();
    let steel = Entity::new(
        "DC01",
        EntityData::Material {
            density_kg_m3: Some(7850.),
            color_rgba: [180, 184, 190, 255],
            section_hatch: None,
        },
    );
    let stock = LibraryDefinition::Stock(Stock {
        material: steel.id,
        specification: StockKind::Sheet {
            thickness_mm: 2.,
            length_mm: 2000.,
            width_mm: 1000.,
            inside_bend_radius_mm: 2.,
            k_factor: k,
        },
    })
    .entity(EntityId::new(), "DC01 2.0")
    .unwrap();
    let mut flange_angle = Entity::new(
        "flange_angle",
        EntityData::Parameter {
            expression: "90".into(),
            unit: "deg".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    flange_angle.description = "Side flange angle".into();
    let ids = (stock.id, flange_angle.id);
    s.commit(prepare_library(&s, 0, actor(), &[steel, stock]).unwrap())
        .unwrap();
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Parameter".into(),
        edits: vec![Mutation::Insert(flange_angle)],
    })
    .unwrap();
    Fixture {
        s,
        stock: ids.0,
        angle: ids.1,
    }
}
fn bend(edge: u32, angle: Dimension, leg: f64) -> Flange {
    Flange {
        edge,
        kind: FlangeKind::Bend { angle },
        direction: BendDirection::Up,
        length: length(leg),
        inside_radius: None,
        start_inset: None,
        end_inset: None,
        relief: None,
    }
}
fn tray(f: &Fixture) -> SheetDefinition {
    let side = Dimension::Expression {
        expression: BoundExpression::bind(f.s.document(), "=$flange_angle", "deg", None).unwrap(),
    };
    SheetDefinition {
        output: BodyOutput {
            body: EntityId::new(),
        },
        stock: f.stock,
        base: SheetBase::Rectangle {
            plane: PanelPlane::Xy,
            origin: [length(0.), length(0.), length(0.)],
            length: length(100.),
            width: length(60.),
        },
        reverse: false,
        inside_radius: None,
        k_factor: None,
        flanges: vec![bend(0, side.clone(), 20.), bend(2, side, 20.)],
        cutouts: vec![],
    }
}
/// Volume of one bend zone: angle/2 · ((r + t)² − r²) · width.
fn bend_volume(angle: f64, r: f64, t: f64, width: f64) -> f64 {
    angle / 2. * ((r + t).powi(2) - r * r) * width
}
fn apply(f: &mut Fixture, feature: EntityId, sheet: &SheetDefinition) {
    let p = model::prepare_sheet_edit(&f.s, f.s.revision(), actor(), feature, sheet).unwrap();
    Compiler::default()
        .compile(p.prepared.document(), f.s.revision() + 1)
        .unwrap();
    f.s.commit(p.prepared).unwrap();
}
/// Native evaluation of an edit that must be refused; nothing is committed.
fn refused(f: &Fixture, feature: EntityId, sheet: &SheetDefinition) -> String {
    let p = model::prepare_sheet_edit(&f.s, f.s.revision(), actor(), feature, sheet).unwrap();
    let Err(e) = Compiler::default().compile(p.prepared.document(), f.s.revision() + 1) else {
        panic!("the refused edit evaluated");
    };
    assert_eq!(e.entity, Some(feature));
    e.message
}

#[test]
fn flanges_rebuild_from_thickness_radius_angle_and_length() {
    let mut f = fixture(0.5);
    let request = SheetRequest::new("Tray", tray(&f));
    let p = model::prepare_sheet(&f.s, f.s.revision(), actor(), &request).unwrap();
    Compiler::default()
        .compile(p.prepared.document(), f.s.revision() + 1)
        .unwrap();
    f.s.commit(p.prepared).unwrap();
    let (t, r) = (2., 2.);
    let r0 = record(&f.s);
    assert!(r0.issues.is_empty(), "{:?}", r0.issues);
    let folded = 100. * 60. * t + 2. * (bend_volume(PI / 2., r, t, 100.) + 100. * 20. * t);
    near(r0.volume_mm3, folded);
    near(r0.measured_thickness_mm, t);
    assert_eq!(r0.solid_count, 1);
    // An up flange on v = 0 rises outside the edge by r + t.
    let b = bounds(&f.s, request.body);
    near(b.min[1], -(r + t));
    near(b.max[1], 60. + r + t);
    near(b.max[2], t + r + 20.);
    // Developed: base plus two legs of 20 mm and a bend allowance each.
    let allowance = PI / 2. * (r + 0.5 * t);
    near(r0.flat_size_mm[0], 100.);
    near(r0.flat_size_mm[1], 60. + 2. * (20. + allowance));
    // With K = 0.5 the neutral layer is mid-thickness: flat and folded volumes agree.
    near(r0.flat_area_mm2 * t, folded);
    assert_eq!(r0.bends.len(), 2);
    let b0 = &r0.bends[0];
    assert_eq!((b0.flange, b0.edge, b0.direction), (0, 0, "up"));
    near(b0.angle_deg, 90.);
    near(b0.allowance_mm, allowance);
    near(b0.line[0][1], -allowance / 2.);
    near(b0.line[1][0], 100.);

    // The angle parameter drives both flanges; Undo restores the document.
    let before = f.s.document().clone();
    let mut entity = f.s.document().get(f.angle).unwrap().clone();
    let EntityData::Parameter { expression, .. } = &mut entity.data else {
        unreachable!()
    };
    *expression = "120".into();
    let p =
        f.s.prepare(Request {
            expected_revision: f.s.revision(),
            actor: actor(),
            label: "Angle".into(),
            edits: vec![Mutation::Replace(entity)],
        })
        .unwrap();
    f.s.commit(p).unwrap();
    let a = 120f64.to_radians();
    let r1 = record(&f.s);
    near(
        r1.volume_mm3,
        100. * 60. * t + 2. * (bend_volume(a, r, t, 100.) + 100. * 20. * t),
    );
    near(r1.bends[0].angle_deg, 120.);
    near(r1.bends[1].allowance_mm, a * (r + 0.5 * t));
    f.s.undo(f.s.revision(), actor()).unwrap();
    assert_eq!(f.s.document(), &before);

    // Radius and leg length per flange; thickness from a stock edit.
    let mut sheet = model::read_sheet(f.s.document(), request.feature).unwrap();
    sheet.flanges[0].inside_radius = Some(length(5.));
    sheet.flanges[1].length = length(35.);
    apply(&mut f, request.feature, &sheet);
    let r2 = record(&f.s);
    near(
        r2.volume_mm3,
        100. * 60. * t
            + bend_volume(PI / 2., 5., t, 100.)
            + 100. * 20. * t
            + bend_volume(PI / 2., r, t, 100.)
            + 100. * 35. * t,
    );
    near(r2.bends[0].inside_radius_mm, 5.);
    near(r2.flat_area_mm2 * t, r2.volume_mm3);
    let stock = f.s.document().get(f.stock).unwrap().clone();
    let LibraryDefinition::Stock(mut spec) = read(&stock).unwrap().unwrap() else {
        unreachable!()
    };
    let StockKind::Sheet { thickness_mm, .. } = &mut spec.specification else {
        unreachable!()
    };
    *thickness_mm = 3.;
    let p = prepare_library(
        &f.s,
        f.s.revision(),
        actor(),
        &[LibraryDefinition::Stock(spec)
            .entity(f.stock, stock.name.clone())
            .unwrap()],
    )
    .unwrap();
    f.s.commit(p).unwrap();
    let r3 = record(&f.s);
    let t = 3.;
    near(
        r3.volume_mm3,
        100. * 60. * t
            + bend_volume(PI / 2., 5., t, 100.)
            + 100. * 20. * t
            + bend_volume(PI / 2., r, t, 100.)
            + 100. * 35. * t,
    );
    near(r3.measured_thickness_mm, 3.);
    near(r3.flat_area_mm2 * t, r3.volume_mm3);

    // Bend faces keep sheet provenance, distinct from ordinary fillets.
    let m = compile(&f.s);
    let topology = &m.snapshot().shapes[&request.feature].topology;
    for role in [
        "sheet.flange.0.bend.0.a",
        "sheet.flange.0.bend.0.b",
        "sheet.flange.1.leg.a",
        "sheet.flange.1.end",
    ] {
        assert!(
            topology
                .elements
                .iter()
                .any(|e| e.origins.iter().any(|o| o.role == role)),
            "no face with role {role}"
        );
    }
}

#[test]
fn hems_jogs_corners_reliefs_and_cutouts_keep_their_intent() {
    let mut f = fixture(0.4);
    let mut base = tray(&f);
    base.flanges.clear();
    let request = SheetRequest::new("Cover", base);
    let p = model::prepare_sheet(&f.s, f.s.revision(), actor(), &request).unwrap();
    f.s.commit(p.prepared).unwrap();
    let feature = request.feature;
    let (t, r, k) = (2., 2., 0.4);

    // Four 90° flanges meeting at the corners touch: refused, nothing changes.
    let mut sheet = model::read_sheet(f.s.document(), feature).unwrap();
    sheet.flanges = (0..4).map(|e| bend(e, angle(90.), 15.)).collect();
    let before = f.s.document().clone();
    let message = refused(&f, feature, &sheet);
    assert!(message.contains("touch at a corner"), "{message}");
    assert_eq!(f.s.document(), &before);
    // Insets on the short-edge flanges open the corners; reliefs cut the base.
    for i in [1, 3] {
        sheet.flanges[i].start_inset = Some(length(3.));
        sheet.flanges[i].end_inset = Some(length(3.));
        sheet.flanges[i].relief = Some(Relief {
            width: length(3.),
            depth: length(4.),
        });
    }
    apply(&mut f, feature, &sheet);
    let rec = record(&f.s);
    let quarter = bend_volume(PI / 2., r, t, 1.);
    let expected =
        100. * 60. * t + 2. * 100. * (quarter + 15. * t) + 2. * 54. * (quarter + 15. * t)
            - 4. * 3. * 4. * t;
    near(rec.volume_mm3, expected);
    assert_eq!(rec.bends.len(), 4);
    let allowance = PI / 2. * (r + k * t);
    near(
        rec.flat_area_mm2,
        100. * 60. + 2. * 100. * (15. + allowance) + 2. * 54. * (15. + allowance) - 4. * 12.,
    );

    // A hem and a jog replace the short-edge flanges and keep their values.
    sheet.flanges[1].kind = FlangeKind::Hem { gap: length(1.) };
    sheet.flanges[1].direction = BendDirection::Down;
    sheet.flanges[3].kind = FlangeKind::Jog {
        angle: angle(90.),
        offset: length(12.),
    };
    apply(&mut f, feature, &sheet);
    let reopened = model::read_sheet(f.s.document(), feature).unwrap();
    assert_eq!(
        reopened.flanges[1].kind,
        FlangeKind::Hem { gap: length(1.) }
    );
    let rec = record(&f.s);
    let hem = bend_volume(PI, 0.5, t, 54.) + 54. * 15. * t;
    // Jog: 12 = t + 2r + straight for 90° bends.
    let straight = 12. - t - 2. * r;
    let jog = 2. * bend_volume(PI / 2., r, t, 54.) + 54. * (straight + 15.) * t;
    let expected = 100. * 60. * t + 2. * 100. * (quarter + 15. * t) + hem + jog - 4. * 3. * 4. * t;
    near(rec.volume_mm3, expected);
    assert_eq!(rec.bends.len(), 5);
    let hem_bend = rec.bends.iter().find(|b| b.flange == 1).unwrap();
    assert_eq!(hem_bend.direction, "down");
    near(hem_bend.angle_deg, 180.);
    near(hem_bend.inside_radius_mm, 0.5);
    // The hem folds under the base: its leg lies 1 mm below the base face.
    let b = bounds(&f.s, request.body);
    near(b.min[2], -(1. + t));
    // The jog leg ends 12 mm above the base face (plus thickness).
    let jog_bends: Vec<_> = rec.bends.iter().filter(|b| b.flange == 3).collect();
    assert_eq!(jog_bends.len(), 2);
    assert_eq!(
        (jog_bends[0].direction, jog_bends[1].direction),
        ("up", "down")
    );
    near(jog_bends[1].zone_mm[0], allowance + straight);

    // Cutouts in the base and in a flange leg follow folded coordinates.
    sheet.cutouts = vec![
        Cutout {
            face: CutoutFace::Base,
            shape: CutoutShape::Circle {
                center: [length(50.), length(30.)],
                diameter: length(10.),
            },
        },
        Cutout {
            face: CutoutFace::Flange { index: 0 },
            shape: CutoutShape::Rectangle {
                center: [length(50.), length(8.)],
                size: [length(20.), length(6.)],
            },
        },
    ];
    apply(&mut f, feature, &sheet);
    let cut = record(&f.s);
    near(cut.volume_mm3, expected - PI * 25. * t - 120. * t);
    assert_eq!(cut.flat.loops.len(), 3);
    // The leg cutout sits beyond flange 0's bend allowance in the flat.
    let hole = cut
        .flat
        .loops
        .iter()
        .filter(|l| !l.outer)
        .map(|l| {
            let pts: Vec<_> = l.edges.iter().map(|e| e.ends().0).collect();
            let ys: Vec<_> = pts.iter().map(|p| p[1]).collect();
            ys.iter().copied().fold(f64::INFINITY, f64::min)
        })
        .fold(f64::INFINITY, f64::min);
    near(hole, -(allowance + 8. + 3.));
    // Moving the flange leg moves its cutout with it: a longer radius keeps
    // the cutout on the leg, and the folded volume stays consistent.
    sheet.flanges[0].inside_radius = Some(length(4.));
    apply(&mut f, feature, &sheet);
    near(
        record(&f.s).volume_mm3,
        expected - PI * 25. * t - 120. * t - 100. * quarter + bend_volume(PI / 2., 4., t, 100.),
    );

    // A cutout outside its face and an edge that does not exist are refused.
    let mut wrong = sheet.clone();
    wrong.cutouts[0].shape = CutoutShape::Circle {
        center: [length(2.), length(30.)],
        diameter: length(10.),
    };
    let message = refused(&f, feature, &wrong);
    assert!(message.contains("not fully inside"), "{message}");
    let mut wrong = sheet.clone();
    wrong.flanges[0].edge = 7;
    let message = refused(&f, feature, &wrong);
    assert!(message.contains("no edge 7"), "{message}");
    let mut wrong = sheet.clone();
    wrong.flanges[3].kind = FlangeKind::Jog {
        angle: angle(90.),
        offset: length(3.),
    };
    let e = model::prepare_sheet_edit(&f.s, f.s.revision(), actor(), feature, &wrong).unwrap_err();
    assert!(e.message.contains("too small"), "{}", e.message);
    // The last valid part is still the committed one.
    near(
        record(&f.s).volume_mm3,
        expected - PI * 25. * t - 120. * t - 100. * quarter + bend_volume(PI / 2., 4., t, 100.),
    );

    // Save and reopen keep every flange, hem, jog and cutout value.
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
    assert_eq!(model::read_sheet(&opened.document, feature).unwrap(), sheet);
    let reopened = Compiler::default().compile(&opened.document, 0).unwrap();
    assert_eq!(
        reopened.sheet_metal(&opened.document).unwrap(),
        vec![record(&f.s)]
    );
}
