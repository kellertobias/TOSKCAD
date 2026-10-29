//! Sheet metal: base sheets from sheet stock with separate radius and
//! K-factor inputs, parameter and stock edits with Undo, sketch-region bases
//! with openings, the developed flat pattern of the same feature,
//! conversion of constant-thickness solids with actionable refusals, and
//! save/reopen. Every size is measured from the native solids.
use multicad_core::{
    commands::{Actor, Channel, Prepared, Request, Session},
    constraint_sketch::{Dimension, source::Source},
    document::{Document, Entity, EntityData, EntityId, Mutation},
    materials::*,
    model::{
        self, BodyOutput, PanelPlane, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest,
        ProfileSource, RevolveParameters, RevolveRequest, SheetBase, SheetDefinition, SheetRequest,
        ValueSource, angle, length,
    },
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    sketch::Datum,
    sketch_regions,
};
use multicad_model::{Compiler, Model, SheetRecord, sheet_metal};

fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() <= 1e-6, "{a} != {b}");
}
fn close(a: f64, b: f64, tolerance: f64) {
    assert!((a - b).abs() <= tolerance, "{a} != {b} ± {tolerance}");
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
fn record<'a>(list: &'a [SheetRecord], name: &str) -> &'a SheetRecord {
    list.iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("no sheet record {name}"))
}

struct Fixture {
    s: Session,
    steel: EntityId,
    steel_material: EntityId,
    alu: EntityId,
    alu_material: EntityId,
    length: EntityId,
}
fn sheet_stock(material: EntityId, thickness: f64, radius: f64, k: f64, name: &str) -> Entity {
    LibraryDefinition::Stock(Stock {
        material,
        specification: StockKind::Sheet {
            thickness_mm: thickness,
            length_mm: 2500.,
            width_mm: 1250.,
            inside_bend_radius_mm: radius,
            k_factor: k,
        },
    })
    .entity(EntityId::new(), name)
    .unwrap()
}
fn fixture() -> Fixture {
    let mut s = Session::new(Document::new("Sheet metal").unwrap()).unwrap();
    let material = |name: &str, density: f64| {
        Entity::new(
            name,
            EntityData::Material {
                density_kg_m3: Some(density),
                color_rgba: [180, 184, 190, 255],
                section_hatch: None,
            },
        )
    };
    let dc01 = material("DC01 steel", 7850.);
    let al = material("AlMg3", 2660.);
    let steel = sheet_stock(dc01.id, 1.5, 1.5, 0.42, "DC01 1.5");
    let alu = sheet_stock(al.id, 2., 2., 0.38, "AlMg3 2.0");
    let mut len = Entity::new(
        "tray_length",
        EntityData::Parameter {
            expression: "200".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    len.description = "Base sheet length".into();
    let ids = (steel.id, dc01.id, alu.id, al.id, len.id);
    s.commit(prepare_library(&s, 0, actor(), &[dc01, al]).unwrap())
        .unwrap();
    s.commit(prepare_library(&s, s.revision(), actor(), &[steel, alu]).unwrap())
        .unwrap();
    insert(&mut s, vec![Mutation::Insert(len)]);
    Fixture {
        s,
        steel: ids.0,
        steel_material: ids.1,
        alu: ids.2,
        alu_material: ids.3,
        length: ids.4,
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
fn rectangle(f: &Fixture, stock: EntityId) -> SheetDefinition {
    SheetDefinition {
        output: BodyOutput {
            body: EntityId::new(),
        },
        stock,
        base: SheetBase::Rectangle {
            plane: PanelPlane::Xy,
            origin: [length(10.), length(20.), length(5.)],
            length: Dimension::Expression {
                expression: BoundExpression::bind(f.s.document(), "=$tray_length", "mm", None)
                    .unwrap(),
            },
            width: length(120.),
        },
        reverse: false,
        inside_radius: None,
        k_factor: None,
    }
}
fn bounds(m: &Model, part: EntityId) -> multicad_kernel::Bounds {
    m.solid(m.snapshot().bodies[&part])
        .unwrap()
        .bounds()
        .unwrap()
}

#[test]
fn rectangle_base_sheet_follows_parameters_stock_and_part_values() {
    let mut f = fixture();
    let request = SheetRequest::new("Tray base", rectangle(&f, f.steel));
    let p = model::prepare_sheet(&f.s, f.s.revision(), actor(), &request).unwrap();
    checked(&mut f.s, p.prepared);
    let m = compile(&f.s);
    let list = m.sheet_metal(f.s.document()).unwrap();
    assert_eq!(list.len(), 1);
    let r = record(&list, "Tray base");
    assert!(r.issues.is_empty(), "{:?}", r.issues);
    assert_eq!((r.part, r.feature), (request.body, request.feature));
    assert_eq!(r.material, Some(f.steel_material));
    assert_eq!(r.stock, f.steel);
    assert_eq!(r.base, "rectangle");
    near(r.thickness_mm, 1.5);
    near(r.measured_thickness_mm, 1.5);
    near(r.volume_mm3, 200. * 120. * 1.5);
    assert_eq!(r.solid_count, 1);
    // Bend inputs come from the stock unless the part overrides them.
    near(r.inside_radius_mm, 1.5);
    near(r.k_factor, 0.42);
    assert_eq!(
        (r.radius_source, r.k_factor_source),
        (ValueSource::Stock, ValueSource::Stock)
    );
    // The flat pattern of the same feature is the base outline, in base frame.
    assert_eq!(r.flat.loops.len(), 1);
    assert!(r.flat.loops[0].outer);
    near(r.flat_size_mm[0], 200.);
    near(r.flat_size_mm[1], 120.);
    near(r.flat_area_mm2, 24_000.);
    let b = bounds(&m, request.body);
    near(b.min[0], 10.);
    near(b.max[0], 210.);
    near(b.min[2], 5.);
    near(b.max[2], 6.5);
    let properties = part_properties(f.s.document(), request.body)
        .unwrap()
        .unwrap()
        .1;
    assert_eq!(properties.stock.unwrap().id, f.steel);

    // A parameter edit resizes the sheet in place; Undo restores it.
    let before = f.s.document().clone();
    set_parameter(&mut f.s, f.length, "320");
    let r = compile(&f.s).sheet_metal(f.s.document()).unwrap();
    near(r[0].flat_size_mm[0], 320.);
    near(r[0].volume_mm3, 320. * 120. * 1.5);
    assert_eq!(r[0].part, request.body);
    f.s.undo(f.s.revision(), actor()).unwrap();
    assert_eq!(f.s.document(), &before);

    // Part overrides stay separate from the stock values.
    let mut edited = model::read_sheet(f.s.document(), request.feature).unwrap();
    edited.inside_radius = Some(length(3.));
    edited.k_factor = Some(Dimension::Literal {
        value: 0.5,
        unit: "ratio".into(),
    });
    edited.reverse = true;
    let p =
        model::prepare_sheet_edit(&f.s, f.s.revision(), actor(), request.feature, &edited).unwrap();
    checked(&mut f.s, p.prepared);
    let m = compile(&f.s);
    let r = &m.sheet_metal(f.s.document()).unwrap()[0];
    near(r.inside_radius_mm, 3.);
    near(r.k_factor, 0.5);
    assert_eq!(
        (r.radius_source, r.k_factor_source),
        (ValueSource::Part, ValueSource::Part)
    );
    let StockKind::Sheet {
        inside_bend_radius_mm,
        k_factor,
        ..
    } = read_sheet_stock_spec(&f.s, f.steel)
    else {
        unreachable!()
    };
    near(inside_bend_radius_mm, 1.5);
    near(k_factor, 0.42);
    // Reversed material lies below the base face.
    let b = bounds(&m, request.body);
    near(b.min[2], 3.5);
    near(b.max[2], 5.);
    near(r.measured_thickness_mm, 1.5);

    // A stock change follows in thickness, material and stock assignment.
    let mut restocked = edited.clone();
    restocked.stock = f.alu;
    let p = model::prepare_sheet_edit(&f.s, f.s.revision(), actor(), request.feature, &restocked)
        .unwrap();
    checked(&mut f.s, p.prepared);
    let r = &compile(&f.s).sheet_metal(f.s.document()).unwrap()[0];
    assert!(r.issues.is_empty(), "{:?}", r.issues);
    assert_eq!(r.material, Some(f.alu_material));
    near(r.thickness_mm, 2.);
    near(r.volume_mm3, 200. * 120. * 2.);

    // Editing the stock thickness rebuilds the sheet; the acknowledged stock
    // assignment keeps its review state in manufacturing properties.
    set_stock_thickness(&mut f.s, f.alu, 3.);
    let r = &compile(&f.s).sheet_metal(f.s.document()).unwrap()[0];
    near(r.thickness_mm, 3.);
    near(r.measured_thickness_mm, 3.);
    near(r.volume_mm3, 200. * 120. * 3.);

    // Panel stock and invalid values are refused before any geometry exists.
    let mut wrong = model::read_sheet(f.s.document(), request.feature).unwrap();
    wrong.k_factor = Some(Dimension::Literal {
        value: 1.5,
        unit: "ratio".into(),
    });
    let e = model::prepare_sheet_edit(&f.s, f.s.revision(), actor(), request.feature, &wrong)
        .unwrap_err();
    assert!(e.message.contains("K-factor"), "{}", e.message);
    let mut wrong = model::read_sheet(f.s.document(), request.feature).unwrap();
    wrong.inside_radius = Some(length(-1.));
    assert!(
        model::prepare_sheet_edit(&f.s, f.s.revision(), actor(), request.feature, &wrong).is_err()
    );
    let mut wrong = model::read_sheet(f.s.document(), request.feature).unwrap();
    wrong.stock = f.steel_material;
    let e = model::prepare_sheet_edit(&f.s, f.s.revision(), actor(), request.feature, &wrong)
        .unwrap_err();
    assert!(e.message.contains("not sheet stock"), "{}", e.message);
}

fn read_sheet_stock_spec(s: &Session, id: EntityId) -> StockKind {
    model::read_sheet_stock(s.document(), id)
        .unwrap()
        .specification
}
fn set_stock_thickness(s: &mut Session, id: EntityId, thickness: f64) {
    let stock = s.document().get(id).unwrap().clone();
    let LibraryDefinition::Stock(mut spec) = read(&stock).unwrap().unwrap() else {
        unreachable!()
    };
    let StockKind::Sheet { thickness_mm, .. } = &mut spec.specification else {
        unreachable!()
    };
    *thickness_mm = thickness;
    let replaced = LibraryDefinition::Stock(spec)
        .entity(id, stock.name.clone())
        .unwrap();
    let p = prepare_library(s, s.revision(), actor(), &[replaced]).unwrap();
    checked(s, p);
}

#[test]
fn sketch_region_sheet_keeps_openings_in_its_flat_pattern_and_reopens() {
    let mut f = fixture();
    let (sketch, edits) = Source::create(Datum::Xz, "Cover").unwrap();
    insert(&mut f.s, edits);
    let mut source = Source::read(f.s.document(), sketch).unwrap().0;
    source.add_rectangle([0., 0.], [150., 90.]).unwrap();
    source.add_rectangle([30., 20.], [70., 50.]).unwrap();
    let regions = sketch_regions::extract(&source);
    let region = regions
        .regions
        .iter()
        .map(|r| r.intent())
        .find(|r| !r.holes.is_empty())
        .unwrap();
    let edits = source.replace(f.s.document(), sketch).unwrap();
    insert(&mut f.s, edits);
    let request = SheetRequest::new(
        "Cover plate",
        SheetDefinition {
            output: BodyOutput {
                body: EntityId::new(),
            },
            stock: f.steel,
            base: SheetBase::Profile {
                profile: ProfileSource { sketch, region },
            },
            reverse: false,
            inside_radius: None,
            k_factor: None,
        },
    );
    let p = model::prepare_sheet(&f.s, f.s.revision(), actor(), &request).unwrap();
    checked(&mut f.s, p.prepared);
    let m = compile(&f.s);
    let r = record(&m.sheet_metal(f.s.document()).unwrap(), "Cover plate").clone();
    assert!(r.issues.is_empty(), "{:?}", r.issues);
    assert_eq!(r.base, "profile");
    let area = 150. * 90. - 40. * 30.;
    near(r.flat_area_mm2, area);
    near(r.volume_mm3, area * 1.5);
    near(r.measured_thickness_mm, 1.5);
    assert_eq!(r.flat.loops.len(), 2);
    assert_eq!(r.flat.loops.iter().filter(|l| l.outer).count(), 1);
    near(r.flat_size_mm[0], 150.);
    near(r.flat_size_mm[1], 90.);

    // Save and reopen keep the editable sheet and identical records.
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
    assert!(opened.capabilities().contains(model::SHEET_CAPABILITY));
    assert_eq!(&opened.document, f.s.document());
    let reopened = Compiler::default().compile(&opened.document, 1).unwrap();
    assert_eq!(reopened.sheet_metal(&opened.document).unwrap(), vec![r]);
}

fn primitive(s: &mut Session, name: &str, parameters: PrimitiveParameters) -> EntityId {
    let request = PrimitiveRequest::new(name, parameters);
    let p = model::prepare_primitive(s, s.revision(), actor(), &request).unwrap();
    checked(s, p.prepared);
    request.body
}
fn plate(size: [f64; 3], origin: [f64; 3]) -> PrimitiveParameters {
    PrimitiveParameters::Box {
        size: size.map(length),
        frame: PrimitiveFrame {
            origin: origin.map(length),
            u: [1., 0., 0.],
            v: [0., 1., 0.],
        },
    }
}

#[test]
fn constant_thickness_solids_convert_and_others_are_refused_with_reasons() {
    let mut f = fixture();
    // A 1.5 mm plate converts: its bottom face is stationary, and the
    // captured contour rebuilds the same material.
    let source = primitive(&mut f.s, "Plate", plate([80., 40., 1.5], [5., 5., 2.]));
    let m = compile(&f.s);
    let analysis = m.analyze_sheet_conversion(source, Some(1.5)).unwrap();
    assert!(analysis.convertible(), "{:?}", analysis.issues);
    near(analysis.thickness_mm.unwrap(), 1.5);
    let (frame, loops) = analysis.stationary.clone().unwrap();
    near(frame.origin[2], 2.);
    near(frame.normal()[2], 1.);
    assert_eq!(loops.len(), 1);
    let (p, _) = sheet_metal::prepare_conversion(
        &m,
        &f.s,
        f.s.revision(),
        actor(),
        "Converted plate",
        source,
        f.steel,
    )
    .unwrap();
    let converted = p.outputs[0].clone();
    checked(&mut f.s, p.prepared);
    let m = compile(&f.s);
    let r = record(&m.sheet_metal(f.s.document()).unwrap(), "Converted plate").clone();
    assert!(r.issues.is_empty(), "{:?}", r.issues);
    assert_eq!(r.base, "contour");
    assert_eq!(r.converted_from, Some(source));
    assert_eq!(r.material, Some(f.steel_material));
    near(r.volume_mm3, 80. * 40. * 1.5);
    near(r.flat_area_mm2, 3200.);
    let (a, b) = (bounds(&m, source), bounds(&m, converted.body));
    for i in 0..3 {
        near(a.min[i], b.min[i]);
        near(a.max[i], b.max[i]);
    }
    // The converted sheet takes its thickness from the stock, not the source.
    set_stock_thickness(&mut f.s, f.steel, 2.);
    let r = record(
        &compile(&f.s).sheet_metal(f.s.document()).unwrap(),
        "Converted plate",
    )
    .clone();
    near(r.volume_mm3, 80. * 40. * 2.);
    set_stock_thickness(&mut f.s, f.steel, 1.5);

    // A disc keeps its curved rim as a through-thickness side wall.
    let disc = primitive(
        &mut f.s,
        "Disc",
        PrimitiveParameters::Cylinder {
            radius: length(20.),
            height: length(1.5),
            frame: PrimitiveFrame::default(),
        },
    );
    let m = compile(&f.s);
    let analysis = m.analyze_sheet_conversion(disc, Some(1.5)).unwrap();
    assert!(analysis.convertible(), "{:?}", analysis.issues);
    let (p, _) = sheet_metal::prepare_conversion(
        &m,
        &f.s,
        f.s.revision(),
        actor(),
        "Converted disc",
        disc,
        f.steel,
    )
    .unwrap();
    checked(&mut f.s, p.prepared);
    let r = record(
        &compile(&f.s).sheet_metal(f.s.document()).unwrap(),
        "Converted disc",
    )
    .clone();
    close(r.volume_mm3, std::f64::consts::PI * 400. * 1.5, 1e-6);
    close(r.flat_area_mm2, std::f64::consts::PI * 400., 1e-6);

    // Thickness that differs from the stock is refused with the measurement.
    let block = primitive(&mut f.s, "Block", plate([60., 30., 10.], [200., 0., 0.]));
    let m = compile(&f.s);
    let e =
        sheet_metal::prepare_conversion(&m, &f.s, f.s.revision(), actor(), "Nope", block, f.steel)
            .unwrap_err();
    assert!(e.message.contains("10.000 mm differs"), "{}", e.message);

    // Two plates joined into a step have varying thickness.
    let step_sketch = l_profile(&mut f.s);
    let step = extrude_profile(&mut f.s, step_sketch, 50.);
    let m = compile(&f.s);
    let analysis = m.analyze_sheet_conversion(step, Some(1.5)).unwrap();
    assert!(!analysis.convertible());
    assert!(
        analysis
            .issues
            .iter()
            .any(|i| i.contains("varying thickness")),
        "{:?}",
        analysis.issues
    );

    // A bent shell's curved faces are reported as bend regions, not developed.
    let shell = revolved_shell(&mut f.s);
    let m = compile(&f.s);
    let analysis = m.analyze_sheet_conversion(shell, Some(1.5)).unwrap();
    assert!(!analysis.convertible());
    assert_eq!(analysis.bend_candidates.len(), 2, "{:?}", analysis.issues);
    assert!(
        analysis.issues.iter().any(|i| i.contains("bend")),
        "{:?}",
        analysis.issues
    );
    let before = f.s.document().clone();
    assert!(
        sheet_metal::prepare_conversion(&m, &f.s, f.s.revision(), actor(), "Shell", shell, f.steel)
            .is_err()
    );
    assert_eq!(f.s.document(), &before);
}

/// An L-shaped section on XZ: a 40 × 1.5 leg and a 1.5 × 20 upstand.
fn l_profile(s: &mut Session) -> (EntityId, sketch_regions::RegionIntent) {
    let (sketch, edits) = Source::create(Datum::Xz, "Step").unwrap();
    insert(s, edits);
    let mut source = Source::read(s.document(), sketch).unwrap().0;
    let points = [
        [300., 0.],
        [340., 0.],
        [340., 1.5],
        [301.5, 1.5],
        [301.5, 20.],
        [300., 20.],
    ]
    .map(|p| source.add_point(p));
    for i in 0..points.len() {
        source.add_line(points[i], points[(i + 1) % points.len()]);
    }
    let regions = sketch_regions::extract(&source);
    assert_eq!(regions.regions.len(), 1);
    let region = regions.regions[0].intent();
    let edits = source.replace(s.document(), sketch).unwrap();
    insert(s, edits);
    (sketch, region)
}
fn extrude_profile(
    s: &mut Session,
    (sketch, region): (EntityId, sketch_regions::RegionIntent),
    distance: f64,
) -> EntityId {
    let request = model::ExtrudeRequest::new(
        sketch,
        vec![region],
        model::ExtrudeParameters {
            extent: model::Extent::OneSided {
                end: model::Termination::Distance {
                    distance: length(distance),
                },
            },
            ..Default::default()
        },
    );
    let body = request.outputs[0].body;
    let p = model::prepare_extrude(s, s.revision(), actor(), &request).unwrap();
    checked(s, p.prepared);
    body
}
/// A 1.5 mm wall revolved through 90° about the sketch's vertical axis.
fn revolved_shell(s: &mut Session) -> EntityId {
    let (sketch, edits) = Source::create(Datum::Xz, "Shell").unwrap();
    insert(s, edits);
    let mut source = Source::read(s.document(), sketch).unwrap().0;
    source.add_rectangle([10., 0.], [11.5, 40.]).unwrap();
    let region = sketch_regions::extract(&source).regions[0].intent();
    let edits = source.replace(s.document(), sketch).unwrap();
    insert(s, edits);
    let request = RevolveRequest::new(
        sketch,
        vec![region],
        RevolveParameters {
            axis_origin: [length(0.), length(0.)],
            axis_angle: angle(90.),
            angle: angle(90.),
        },
    );
    let body = request.outputs[0].body;
    let p = model::prepare_revolve(s, s.revision(), actor(), &request).unwrap();
    checked(s, p.prepared);
    body
}
