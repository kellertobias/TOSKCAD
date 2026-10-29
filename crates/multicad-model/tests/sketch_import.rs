//! Native evidence for DXF/SVG sketch import: exact regions, solids, a cut, a
//! constrained edit and save/reopen. Expected values are analytic or come
//! from independent Gauss-Legendre integration of the authored Bézier data.
use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::{Constraint, ConstraintKind, CurveKind, Dimension, source::Source},
    document::{Document, EntityId},
    model::{self, BodyInput, ExtrudeBooleanRequest, ExtrudeRequest, Operation},
    project_file::{self, ProjectFile},
    sketch::Datum,
    sketch_import::{self, Format, ImportOptions, plan::plan, provenance::Provenance},
    sketch_regions::Region,
};
use multicad_kernel::{GeometryPolicy, NativeRegionQueries};
use multicad_model::Compiler;
use multicad_solver::import::{ImportSource, ImportTarget, prepare_import};
use std::{collections::BTreeSet, f64::consts::PI};

const DXF: &[u8] = include_bytes!("../../multicad-core/tests/fixtures/sketch_import/plate.dxf");
const SVG: &[u8] = include_bytes!("../../multicad-core/tests/fixtures/sketch_import/badge.svg");
/// Declared tolerance for native area/volume agreement with analytic values.
const RELATIVE: f64 = 1e-7;

fn actor() -> Actor {
    Actor {
        id: "import-native-test".into(),
        channel: Channel::Ui,
    }
}
fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= RELATIVE * expected.abs().max(1.),
        "{actual} != {expected}"
    );
}

fn import(
    session: &mut Session,
    target: ImportTarget,
    format: Format,
    bytes: &'static [u8],
    options: ImportOptions,
) -> (EntityId, EntityId) {
    let art = sketch_import::parse(format, bytes, options.units).unwrap();
    let plan = plan(&art, &options).unwrap();
    let edit = prepare_import(
        session,
        session.revision(),
        actor(),
        &target,
        None,
        &plan,
        &ImportSource {
            file_name: if format == Format::Dxf {
                "plate.dxf"
            } else {
                "badge.svg"
            },
            bytes,
        },
    )
    .unwrap();
    session.commit(edit.edit.preview.unwrap()).unwrap();
    (edit.sketch, edit.provenance)
}

fn regions(session: &Session, sketch: EntityId) -> Vec<Region> {
    let evaluated = multicad_solver::evaluate_document_sketch(
        session.document(),
        session.revision(),
        sketch,
        None,
    )
    .unwrap();
    let extraction = evaluated
        .regions_with_queries(&NativeRegionQueries::default())
        .unwrap();
    assert!(
        extraction.diagnostics.is_empty(),
        "{:?}",
        extraction.diagnostics
    );
    extraction.regions
}

fn extrude(
    session: &mut Session,
    sketch: EntityId,
    region: &Region,
    cut: Option<BodyInput>,
) -> (EntityId, EntityId) {
    let request = ExtrudeRequest::new(sketch, vec![region.intent()], Default::default());
    let (prepared, result) = match cut {
        None => {
            let prepared =
                model::prepare_extrude(session, session.revision(), actor(), &request).unwrap();
            let out = &prepared.outputs[0];
            let result = (out.body, out.feature);
            (prepared, result)
        }
        Some(target) => {
            let body = target.body;
            let boolean = ExtrudeBooleanRequest::new(request, Operation::Cut, vec![target], false);
            let prepared =
                model::prepare_extrude_boolean(session, session.revision(), actor(), &boolean)
                    .unwrap();
            (prepared, (body, boolean.booleans[0].feature))
        }
    };
    Compiler::default()
        .compile(prepared.prepared.document(), session.revision())
        .unwrap();
    session.commit(prepared.prepared).unwrap();
    result
}

fn archive(document: &Document) -> Vec<u8> {
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(document.clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    bytes
}
fn reopen(bytes: &[u8]) -> Session {
    let document = project_file::read(
        &mut std::io::Cursor::new(bytes),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document;
    Session::new(document).unwrap()
}

/// Area enclosed by a closed chain of Bézier segments (Green's theorem with
/// five-point Gauss-Legendre quadrature, exact for these polynomial degrees).
fn bezier_area(segments: &[Vec<[f64; 2]>]) -> f64 {
    const X: [f64; 5] = [
        0.,
        -0.538_469_310_105_683_1,
        0.538_469_310_105_683_1,
        -0.906_179_845_938_664,
        0.906_179_845_938_664,
    ];
    const W: [f64; 5] = [
        0.568_888_888_888_888_9,
        0.478_628_670_499_366_5,
        0.478_628_670_499_366_5,
        0.236_926_885_056_189_1,
        0.236_926_885_056_189_1,
    ];
    let mut area = 0.;
    for poles in segments {
        let n = poles.len() - 1;
        let binom = |k: usize| (0..k).fold(1., |a, i| a * (n - i) as f64 / (i + 1) as f64);
        for (x, w) in X.iter().zip(W) {
            let t = (x + 1.) / 2.;
            let mut p = [0., 0.];
            let mut d = [0., 0.];
            for (k, pole) in poles.iter().enumerate() {
                let b = binom(k) * t.powi(k as i32) * (1. - t).powi((n - k) as i32);
                let db = binom(k)
                    * (if k > 0 {
                        k as f64 * t.powi(k as i32 - 1) * (1. - t).powi((n - k) as i32)
                    } else {
                        0.
                    } - if k < n {
                        (n - k) as f64 * t.powi(k as i32) * (1. - t).powi((n - k - 1) as i32)
                    } else {
                        0.
                    });
                for i in 0..2 {
                    p[i] += b * pole[i];
                    d[i] += db * pole[i];
                }
            }
            area += w / 2. * 0.5 * (p[0] * d[1] - p[1] * d[0]);
        }
    }
    area.abs()
}

#[test]
fn imported_dxf_regions_extrude_edit_cut_and_reopen_with_native_measurements() {
    let mut session = Session::new(Document::new("Imported plate").unwrap()).unwrap();
    let (plate, record) = import(
        &mut session,
        ImportTarget::New {
            datum: Datum::Xy,
            name: "Plate".into(),
        },
        Format::Dxf,
        DXF,
        ImportOptions {
            layers: Some(BTreeSet::from(["CUT".into()])),
            ..Default::default()
        },
    );
    // 100 × 60 minus the R20 corner remainder, a Ø20 hole and a 15 × 10 slot.
    let outer = 6000. - (400. - PI * 100.);
    let expected = outer - PI * 100. - (150. + PI * 25.);
    let found = regions(&session, plate);
    // Every bounded face is a selectable profile: the plate and both openings.
    assert_eq!(found.len(), 3);
    let main = found
        .iter()
        .find(|r| r.holes.len() == 2)
        .expect("plate with two holes");
    close(main.area_mm2, expected);
    close(main.outer.signed_area_mm2.abs(), outer);
    let mut openings: Vec<_> = found
        .iter()
        .filter(|r| r.holes.is_empty())
        .map(|r| r.area_mm2)
        .collect();
    openings.sort_by(f64::total_cmp);
    close(openings[0], 150. + PI * 25.);
    close(openings[1], PI * 100.);
    let main = main.clone();
    let (body, feature) = extrude(&mut session, plate, &main, None);
    let model = Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    let shape = &model.snapshot().shapes[&feature];
    assert_eq!(shape.solid_count, 1);
    close(shape.volume_mm3, expected * 10.);
    close(shape.bounds.max[0], 100.);
    close(shape.bounds.max[1], 60.);

    // A constrained edit of the imported Ø20 hole regenerates the solid.
    let (mut source, _, _) =
        Source::read_for_evaluation(session.document(), plate, session.revision(), None).unwrap();
    let circle = source
        .sketch
        .curves
        .iter()
        .find(|c| matches!(c.kind, CurveKind::Circle { .. }))
        .unwrap()
        .id;
    source.sketch.constraints.push(Constraint {
        id: EntityId::new(),
        reference: false,
        kind: ConstraintKind::Radius {
            curve: circle,
            dimension: Dimension::Literal {
                value: 12.,
                unit: "mm".into(),
            },
        },
    });
    let edit = multicad_solver::prepare_edit(&session, session.revision(), actor(), plate, source)
        .unwrap();
    session.commit(edit.preview.unwrap()).unwrap();
    let edited = expected - PI * (144. - 100.);
    let model = Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    close(model.snapshot().shapes[&feature].volume_mm3, edited * 10.);

    // An SVG island imported into a second sketch cuts a through hole.
    let (badge, _) = import(
        &mut session,
        ImportTarget::New {
            datum: Datum::Xy,
            name: "Badge island".into(),
        },
        Format::Svg,
        SVG,
        ImportOptions {
            layers: Some(BTreeSet::from(["island".into()])),
            insertion_mm: [70., -15.],
            ..Default::default()
        },
    );
    let island = regions(&session, badge);
    assert_eq!(island.len(), 1);
    close(island[0].area_mm2, PI * 1.5 * 1.5);
    let (_, cut) = extrude(
        &mut session,
        badge,
        &island[0],
        Some(BodyInput {
            body,
            producer: feature,
        }),
    );
    let model = Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    let cut_shape = &model.snapshot().shapes[&cut];
    close(cut_shape.volume_mm3, (edited - PI * 2.25) * 10.);
    let temp = tempfile::tempdir().unwrap();
    let step = temp.path().join("imported.step");
    model.solid(cut).unwrap().write_step(&step).unwrap();
    let read = multicad_kernel::Solid::read_step(&step, GeometryPolicy::default()).unwrap();
    close(read.volume_mm3().unwrap(), cut_shape.volume_mm3);

    // Save/reopen keeps geometry, placement and provenance.
    let reopened = reopen(&archive(session.document()));
    assert_eq!(reopened.document(), session.document());
    let (provenance, linked) = Provenance::read(reopened.document(), record).unwrap();
    assert_eq!(
        (linked, provenance.format, provenance.layers.as_slice()),
        (plate, Format::Dxf, &["CUT".to_string()][..])
    );
    let again = Compiler::default().compile(reopened.document(), 0).unwrap();
    close(
        again.snapshot().shapes[&cut].volume_mm3,
        cut_shape.volume_mm3,
    );
}

#[test]
fn imported_svg_holes_islands_ellipses_and_beziers_have_exact_native_areas() {
    let mut session = Session::new(Document::new("Imported badge").unwrap()).unwrap();
    let (sketch, _) = import(
        &mut session,
        ImportTarget::New {
            datum: Datum::Xz,
            name: "Badge".into(),
        },
        Format::Svg,
        SVG,
        ImportOptions::default(),
    );
    let found = regions(&session, sketch);
    // Outline with three holes, the annulus around the island, the island,
    // the ellipse and the Bézier opening are all bounded faces.
    assert_eq!(found.len(), 5);
    let main = found
        .iter()
        .find(|r| r.holes.len() == 3)
        .expect("outline with three holes");
    let annulus = found.iter().find(|r| r.holes.len() == 1).expect("annulus");
    let mut discs: Vec<_> = found
        .iter()
        .filter(|r| r.holes.is_empty())
        .map(|r| r.area_mm2)
        .collect();
    discs.sort_by(f64::total_cmp);
    // 0.5 mm per user unit: rounded rectangle 70 × 30, R5 corners.
    let outline = 70. * 30. - (4. - PI) * 25.;
    let bezier = bezier_area(&[
        vec![[60., 50.], [60., 30.], [90., 30.], [90., 50.]],
        vec![[90., 50.], [75., 62.], [60., 50.]],
    ]) * 0.25;
    let holes = PI * 16. + PI * 3. * 6. + bezier;
    close(main.area_mm2, outline - holes);
    close(annulus.area_mm2, PI * (16. - 2.25));
    let mut expected_discs = [PI * 2.25, PI * 18., bezier];
    expected_discs.sort_by(f64::total_cmp);
    for (actual, expected) in discs.iter().zip(expected_discs) {
        close(*actual, expected);
    }
    // Hole loops are reported separately from the outer boundary regardless
    // of the authored winding, which the SVG Y flip reverses.
    let hole_sum: f64 = main.holes.iter().map(|h| h.signed_area_mm2.abs()).sum();
    close(hole_sum, holes);
    let (_, feature) = extrude(&mut session, sketch, main, None);
    let model = Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    let shape = &model.snapshot().shapes[&feature];
    close(shape.volume_mm3, (outline - holes) * 10.);
    // The XZ datum receives the sketch: X spans 5..75 mm and Z spans 5..35 mm.
    close(shape.bounds.min[0], 5.);
    close(shape.bounds.max[0], 75.);
    close(shape.bounds.min[2], 5.);
    close(shape.bounds.max[2], 35.);
}

/// A DXF written by the independent, developer-only `dxf` crate in inches.
fn independent_dxf() -> Vec<u8> {
    use dxf::{
        Drawing, LwPolylineVertex, Point,
        entities::{Circle, Entity, EntityType, LwPolyline, Spline},
        enums::{AcadVersion, Units},
    };
    let mut drawing = Drawing::new();
    drawing.header.version = AcadVersion::R2007;
    drawing.header.default_drawing_units = Units::Inches;
    let vertex = |x: f64, y: f64, bulge: f64| LwPolylineVertex {
        x,
        y,
        bulge,
        ..Default::default()
    };
    let mut profile = LwPolyline {
        flags: 1,
        ..Default::default()
    };
    profile.vertices = vec![
        vertex(0., 0., 0.),
        vertex(2., 0., 1.),
        vertex(2., 1., 0.),
        vertex(0., 1., 0.),
    ];
    let mut entity = Entity::new(EntityType::LwPolyline(profile));
    entity.common.layer = "PROFILE".into();
    drawing.add_entity(entity);
    let mut hole = Entity::new(EntityType::Circle(Circle {
        center: Point::new(0.5, 0.5, 0.),
        radius: 0.25,
        ..Default::default()
    }));
    hole.common.layer = "PROFILE".into();
    drawing.add_entity(hole);
    let mut spline = Spline {
        degree_of_curve: 3,
        knot_values: vec![0., 0., 0., 0., 1., 1., 1., 1.],
        ..Default::default()
    };
    spline.control_points = vec![
        Point::new(0., 2., 0.),
        Point::new(1., 3., 0.),
        Point::new(2., 1., 0.),
        Point::new(3., 2., 0.),
    ];
    let mut entity = Entity::new(EntityType::Spline(spline));
    entity.common.layer = "PATH".into();
    drawing.add_entity(entity);
    let mut bytes = Vec::new();
    drawing.save(&mut bytes).unwrap();
    bytes
}

#[test]
fn independently_written_inch_dxf_imports_at_physical_scale() {
    let bytes: &'static [u8] = Box::leak(independent_dxf().into_boxed_slice());
    let art = sketch_import::parse(Format::Dxf, bytes, ImportOptions::default().units).unwrap();
    assert!(art.notes[0].contains("inches"), "{:?}", art.notes);
    assert_eq!(art.layers, ["PROFILE", "PATH"]);
    let mut session = Session::new(Document::new("Independent DXF").unwrap()).unwrap();
    let (sketch, _) = import(
        &mut session,
        ImportTarget::New {
            datum: Datum::Yz,
            name: "Inch profile".into(),
        },
        Format::Dxf,
        bytes,
        ImportOptions {
            layers: Some(BTreeSet::from(["PROFILE".into()])),
            ..Default::default()
        },
    );
    let square_inch = 25.4 * 25.4;
    let found = regions(&session, sketch);
    let main = found
        .iter()
        .find(|r| r.holes.len() == 1)
        .expect("profile with hole");
    close(main.area_mm2, (2. + PI / 8. - PI / 16.) * square_inch);
    let (_, feature) = extrude(&mut session, sketch, main, None);
    let model = Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    let shape = &model.snapshot().shapes[&feature];
    close(
        shape.volume_mm3,
        (2. + PI / 8. - PI / 16.) * square_inch * 10.,
    );
    // The YZ datum maps sketch X to world Y: the semicircle reaches 2.5 in.
    close(shape.bounds.max[1], 2.5 * 25.4);
    // The open spline layer previews as one open chain and does not form a region.
    let path = plan(
        &art,
        &ImportOptions {
            layers: Some(BTreeSet::from(["PATH".into()])),
            ..Default::default()
        },
    )
    .unwrap();
    let empty = Source {
        u: Datum::Yz.frame().u,
        sketch: Default::default(),
    };
    let preview = path.preview(&empty);
    assert_eq!(
        (
            preview.curves.splines,
            preview.closed_loops,
            preview.open_chains
        ),
        (1, 0, 1)
    );
}
