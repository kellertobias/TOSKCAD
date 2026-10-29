//! Unfolding a selected sheet Part: the developed pattern keeps the folded
//! Part, links cut loops, bend lines and drill points to the same Part and
//! feature, follows thickness, angle, radius and K-factor edits
//! deterministically, and reports scoped process diagnostics.
use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId},
    materials::*,
    model::{
        self, BendDirection, BodyOutput, Cutout, CutoutFace, CutoutShape, Flange, FlangeKind,
        PanelPlane, PrimitiveParameters, PrimitiveRequest, Relief, SheetBase, SheetDefinition,
        SheetRequest, angle, length,
    },
    project_file::{self, ProjectFile},
};
use multicad_model::{Compiler, DevelopedPattern, DiagnosticScope, Model};
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
fn flange(edge: u32, leg: f64) -> Flange {
    Flange {
        edge,
        kind: FlangeKind::Bend { angle: angle(90.) },
        direction: BendDirection::Up,
        length: length(leg),
        inside_radius: None,
        start_inset: None,
        end_inset: None,
        relief: None,
    }
}
fn circle(face: CutoutFace, x: f64, y: f64, d: f64) -> Cutout {
    Cutout {
        face,
        shape: CutoutShape::Circle {
            center: [length(x), length(y)],
            diameter: length(d),
        },
    }
}
fn kinds(p: &DevelopedPattern) -> Vec<&'static str> {
    p.diagnostics.iter().map(|d| d.kind).collect()
}
fn commit(s: &mut Session, prepared: multicad_core::commands::Prepared) {
    Compiler::default()
        .compile(prepared.document(), s.revision() + 1)
        .unwrap();
    s.commit(prepared).unwrap();
}

#[test]
fn unfolding_links_bends_and_drills_and_reports_scoped_diagnostics() {
    let mut s = Session::new(Document::new("Unfold").unwrap()).unwrap();
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
            k_factor: 0.4,
        },
    })
    .entity(EntityId::new(), "DC01 2.0")
    .unwrap();
    let stock_id = stock.id;
    s.commit(prepare_library(&s, 0, actor(), &[steel, stock]).unwrap())
        .unwrap();
    let (t, r, k) = (2., 2., 0.4);
    let mut short_a = flange(1, 15.);
    short_a.start_inset = Some(length(3.));
    short_a.end_inset = Some(length(3.));
    let mut short_b = flange(3, 15.);
    short_b.start_inset = Some(length(3.));
    short_b.end_inset = Some(length(3.));
    short_b.inside_radius = Some(length(1.));
    short_b.relief = Some(Relief {
        width: length(3.),
        depth: length(2.),
    });
    let request = SheetRequest::new(
        "Bracket",
        SheetDefinition {
            output: BodyOutput {
                body: EntityId::new(),
            },
            stock: stock_id,
            base: SheetBase::Rectangle {
                plane: PanelPlane::Xy,
                origin: [length(0.), length(0.), length(0.)],
                length: length(100.),
                width: length(60.),
            },
            reverse: false,
            inside_radius: None,
            k_factor: None,
            flanges: vec![flange(0, 20.), short_a, flange(2, 20.), short_b],
            cutouts: vec![
                circle(CutoutFace::Base, 50., 4., 2.),
                circle(CutoutFace::Base, 50., 30., 8.),
                circle(CutoutFace::Flange { index: 0 }, 50., 12., 5.),
            ],
            fasteners: vec![],
        },
    );
    let p = model::prepare_sheet(&s, s.revision(), actor(), &request).unwrap();
    commit(&mut s, p.prepared);
    let before = s.document().clone();
    let m = compile(&s);
    let folded = m.snapshot().shapes[&request.feature].volume_mm3;
    let pattern = m.unfold(s.document(), request.body).unwrap();
    // Unfolding reads the model; the folded Part and the document stay.
    assert_eq!(s.document(), &before);
    assert_eq!(m.snapshot().bodies[&request.body], request.feature);
    near(
        compile(&s).snapshot().shapes[&request.feature].volume_mm3,
        folded,
    );
    assert_eq!(
        (pattern.part, pattern.feature),
        (request.body, request.feature)
    );
    assert_eq!(pattern.units, "mm");
    near(pattern.thickness_mm, t);
    near(pattern.k_factor, k);

    // Bend lines: one per flange, in feature order, linked by id.
    assert_eq!(
        pattern
            .bends
            .iter()
            .map(|b| b.id.as_str())
            .collect::<Vec<_>>(),
        [
            "flange.0.bend.0",
            "flange.1.bend.0",
            "flange.2.bend.0",
            "flange.3.bend.0"
        ]
    );
    assert_eq!(
        pattern.bends.iter().map(|b| b.order).collect::<Vec<_>>(),
        [0, 1, 2, 3]
    );
    let allowance = PI / 2. * (r + k * t);
    near(pattern.bends[0].record.allowance_mm, allowance);
    near(pattern.bends[3].record.inside_radius_mm, 1.);
    near(pattern.bends[3].record.allowance_mm, PI / 2. * (1. + k * t));
    assert!(pattern.bends.iter().all(|b| b.record.direction == "up"));

    // Drill points: the three circular cutouts at their flat positions.
    assert_eq!(pattern.drills.len(), 3);
    assert_eq!(pattern.drills[2].id, "cutout.2");
    near(pattern.drills[2].center[0], 50.);
    near(pattern.drills[2].center[1], -(allowance + 12.));
    near(pattern.drills[1].diameter_mm, 8.);
    // The cut loops: one outer boundary and three openings.
    assert_eq!(pattern.cut.loops.len(), 4);
    assert_eq!(pattern.cut.loops.iter().filter(|l| l.outer).count(), 1);
    let quarter = |rb: f64| PI / 2. * (rb + k * t);
    near(
        pattern.area_mm2,
        100. * 60.
            + 2. * 100. * (20. + allowance)
            + 54. * (15. + allowance)
            + 54. * (15. + quarter(1.))
            - 2. * 3. * 2.
            - PI * (1. + 16. + 6.25),
    );
    near(pattern.size_mm[1], 60. + 2. * (20. + allowance));

    // Scoped diagnostics.
    let k0 = kinds(&pattern);
    assert!(k0.contains(&"hole_near_bend"), "{k0:?}");
    assert!(k0.contains(&"missing_relief"), "{k0:?}");
    assert!(k0.contains(&"tight_radius"), "{k0:?}");
    assert!(k0.contains(&"shallow_relief"), "{k0:?}");
    assert!(!k0.contains(&"exceeds_stock"), "{k0:?}");
    let near_hole = pattern
        .diagnostics
        .iter()
        .find(|d| d.kind == "hole_near_bend")
        .unwrap();
    assert_eq!(near_hole.subject.as_deref(), Some("cutout.0"));
    assert_eq!(near_hole.scope, DiagnosticScope::RuleOfThumb);
    assert!(
        near_hole.message.contains("flange.0.bend.0"),
        "{}",
        near_hole.message
    );
    let tooling = pattern.diagnostics.last().unwrap();
    assert_eq!(
        (tooling.kind, tooling.scope),
        ("tooling", DiagnosticScope::NeedsData)
    );
    // Only the hole 3 mm from the bend zone is flagged.
    assert_eq!(
        pattern
            .diagnostics
            .iter()
            .filter(|d| d.kind == "hole_near_bend")
            .count(),
        1
    );

    // Deterministic: the same source unfolds to the same pattern.
    assert_eq!(
        compile(&s).unfold(s.document(), request.body).unwrap(),
        pattern
    );

    // A K-factor override and an angle edit change the developed sizes.
    let mut sheet = model::read_sheet(s.document(), request.feature).unwrap();
    sheet.k_factor = Some(Dimension::Literal {
        value: 0.5,
        unit: "ratio".into(),
    });
    sheet.flanges[2].kind = FlangeKind::Bend { angle: angle(60.) };
    let p = model::prepare_sheet_edit(&s, s.revision(), actor(), request.feature, &sheet).unwrap();
    commit(&mut s, p.prepared);
    let edited = compile(&s).unfold(s.document(), request.body).unwrap();
    near(edited.k_factor, 0.5);
    let a90 = PI / 2. * (r + 0.5 * t);
    let a60 = PI / 3. * (r + 0.5 * t);
    near(edited.size_mm[1], 60. + 20. + a90 + 20. + a60);
    near(edited.bends[2].record.angle_deg, 60.);
    near(edited.bends[2].record.allowance_mm, a60);
    near(edited.drills[2].center[1], -(a90 + 12.));

    // A stock sheet smaller than the blank is reported against the model.
    let entity = s.document().get(stock_id).unwrap().clone();
    let LibraryDefinition::Stock(mut spec) = read(&entity).unwrap().unwrap() else {
        unreachable!()
    };
    let StockKind::Sheet {
        length_mm,
        width_mm,
        ..
    } = &mut spec.specification
    else {
        unreachable!()
    };
    *length_mm = 150.;
    *width_mm = 100.;
    let p = prepare_library(
        &s,
        s.revision(),
        actor(),
        &[LibraryDefinition::Stock(spec)
            .entity(stock_id, entity.name.clone())
            .unwrap()],
    )
    .unwrap();
    commit(&mut s, p);
    let small = compile(&s).unfold(s.document(), request.body).unwrap();
    let exceeds = small
        .diagnostics
        .iter()
        .find(|d| d.kind == "exceeds_stock")
        .expect("exceeds_stock");
    assert_eq!(exceeds.scope, DiagnosticScope::Model);

    // Save and reopen unfold to the same pattern.
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
    let reopened = Compiler::default()
        .compile(&opened.document, 0)
        .unwrap()
        .unfold(&opened.document, request.body)
        .unwrap();
    assert_eq!(reopened, small);

    // Only sheet parts unfold.
    let block = PrimitiveRequest::new(
        "Block",
        PrimitiveParameters::Box {
            size: [length(10.), length(10.), length(10.)],
            frame: Default::default(),
        },
    );
    let p = model::prepare_primitive(&s, s.revision(), actor(), &block).unwrap();
    commit(&mut s, p.prepared);
    let Err(e) = compile(&s).unfold(s.document(), block.body) else {
        panic!("a box unfolded");
    };
    assert!(e.message.contains("not a sheet part"), "{}", e.message);
}
