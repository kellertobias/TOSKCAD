use multicad_core::{
    constraint_sketch::{CurveKind, source::Source},
    document::{Document, Mutation},
    sketch::Datum,
    sketch_import::{
        self, Format, ImportOptions, SourceUnit, UnitChoice,
        geometry::{P, Segment},
        plan::plan,
    },
};
use std::{
    collections::BTreeSet,
    f64::consts::{FRAC_PI_2, PI},
};

const DXF: &[u8] = include_bytes!("fixtures/sketch_import/plate.dxf");
const SVG: &[u8] = include_bytes!("fixtures/sketch_import/badge.svg");

fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9, "{a} != {b}");
}
fn pnear(a: P, b: P) {
    near(a[0], b[0]);
    near(a[1], b[1]);
}
fn kinds(artwork: &sketch_import::Artwork, layer: &str) -> Vec<&'static str> {
    artwork
        .paths
        .iter()
        .filter(|p| p.layer == layer)
        .flat_map(|p| p.segments.iter().map(Segment::kind))
        .collect()
}
fn empty_source() -> Source {
    Source {
        u: Datum::Xy.frame().u,
        sketch: Default::default(),
    }
}

#[test]
fn dxf_entities_bulges_ocs_units_and_unsupported_content_are_interpreted_explicitly() {
    let art = sketch_import::parse(Format::Dxf, DXF, UnitChoice::FromFile).unwrap();
    assert_eq!(art.layers, ["CUT", "ENGRAVE", "MIRROR", "NOTES"]);
    assert_eq!(
        kinds(&art, "CUT"),
        [
            "line", "line", "arc", "line", "line", "circle", "line", "arc", "line", "arc"
        ]
    );
    // Bulged R20 corner: counterclockwise quarter arc centered at (80, 40).
    let corner = &art.paths[0].segments[2];
    let Segment::Arc {
        center,
        radius,
        sweep,
        ..
    } = corner
    else {
        panic!("{corner:?}")
    };
    pnear(*center, [80., 40.]);
    near(*radius, 20.);
    near(*sweep, FRAC_PI_2);
    pnear(corner.start().unwrap(), [100., 40.]);
    pnear(corner.end().unwrap(), [80., 60.]);
    assert!(art.paths[0].closed);
    // The rational SPLINE keeps degree, normalized knots and weights.
    let spline = &art
        .paths
        .iter()
        .find(|p| p.layer == "ENGRAVE")
        .unwrap()
        .segments[0];
    let Segment::Spline {
        degree,
        poles,
        knots,
        multiplicities,
        weights,
    } = spline
    else {
        panic!()
    };
    assert_eq!((*degree, poles.len()), (3, 5));
    assert_eq!(knots, &[0., 0.5, 1.]);
    assert_eq!(multiplicities, &[4, 1, 4]);
    assert_eq!(weights.as_deref(), Some(&[1., 1., 2., 1., 1.][..]));
    // −Z extrusion mirrors X and reverses the arc direction.
    let mirror: Vec<_> = art.paths.iter().filter(|p| p.layer == "MIRROR").collect();
    let Segment::Arc { center, sweep, .. } = &mirror[0].segments[0] else {
        panic!()
    };
    pnear(*center, [130., 10.]);
    near(*sweep, -FRAC_PI_2);
    pnear(mirror[0].segments[0].start().unwrap(), [120., 10.]);
    pnear(mirror[0].segments[0].end().unwrap(), [130., 20.]);
    // Old-style POLYLINE bulge 1 is a semicircle passing below its chord.
    let Segment::Arc { center, sweep, .. } = &mirror[1].segments[0] else {
        panic!()
    };
    pnear(*center, [130., 40.]);
    near(*sweep, PI);
    let unsupported: BTreeSet<_> = art.unsupported.iter().map(|d| d.kind.as_str()).collect();
    assert_eq!(
        unsupported,
        BTreeSet::from(["dxf.insert", "dxf.paper_space", "dxf.text"])
    );
    assert!(art.notes[0].contains("$INSUNITS 4"));
    // Explicit inches override the header: every coordinate scales by 25.4.
    let inches =
        sketch_import::parse(Format::Dxf, DXF, UnitChoice::Explicit(SourceUnit::Inch)).unwrap();
    pnear(inches.paths[0].segments[1].end().unwrap(), [2540., 1016.]);
}

#[test]
fn unitless_binary_and_malformed_dxf_fail_without_guessing() {
    let unitless =
        String::from_utf8_lossy(DXF).replace("$INSUNITS\r\n70\r\n4", "$INSUNITS\r\n70\r\n0");
    let e =
        sketch_import::parse(Format::Dxf, unitless.as_bytes(), UnitChoice::FromFile).unwrap_err();
    assert!(e.0.contains("choose source units"), "{e}");
    assert!(
        sketch_import::parse(
            Format::Dxf,
            unitless.as_bytes(),
            UnitChoice::Explicit(SourceUnit::Millimeter)
        )
        .is_ok()
    );
    assert!(
        sketch_import::parse(
            Format::Dxf,
            b"AutoCAD Binary DXF\r\n\x1a\0",
            UnitChoice::FromFile
        )
        .is_err()
    );
    assert!(
        sketch_import::parse(
            Format::Dxf,
            b"0\nSECTION\n2\nENTITIES\n0\nLINE\n10\nnan\n",
            UnitChoice::Explicit(SourceUnit::Millimeter)
        )
        .is_err()
    );
    let big = vec![b' '; sketch_import::MAX_FILE_BYTES + 1];
    assert!(
        sketch_import::parse(Format::Dxf, &big, UnitChoice::FromFile)
            .unwrap_err()
            .0
            .contains("limit")
    );
}

#[test]
fn svg_physical_viewbox_nested_transforms_curves_and_y_up_orientation_are_exact() {
    let art = sketch_import::parse(Format::Svg, SVG, UnitChoice::FromFile).unwrap();
    assert_eq!(art.layers, ["Outline", "holes", "island"]);
    near(art.millimeters_per_unit, 0.5);
    // Rounded rectangle: 70 × 30 mm at (5, 5) mm with R5 analytic corners.
    let outline = &art.paths[0];
    assert!(outline.closed);
    assert_eq!(outline.segments.len(), 8);
    pnear(outline.segments[0].start().unwrap(), [10., 35.]);
    let Segment::Arc { radius, sweep, .. } = &outline.segments[1] else {
        panic!()
    };
    near(*radius, 5.);
    near(*sweep, -FRAC_PI_2);
    // User (30, 30) is 15 mm from the top of a 40 mm page: Y up gives 25 mm.
    let Segment::Circle { center, radius } = &art.paths[1].segments[0] else {
        panic!()
    };
    pnear(*center, [15., 25.]);
    near(*radius, 4.);
    // Nested rotate(90): the 12 × 6 user ellipse stands upright as an exact
    // rational spline with semi-axes 3 mm (X) and 6 mm (Y) centered at (60, 20).
    let Segment::Spline {
        degree,
        poles,
        weights,
        ..
    } = &art.paths[2].segments[0]
    else {
        panic!()
    };
    assert_eq!((*degree, poles.len()), (2, 9));
    assert!(weights.is_some());
    pnear(poles[0], [60., 14.]);
    pnear(poles[4], [60., 26.]);
    pnear(poles[2], [57., 20.]);
    // Cubic, quadratic and closing line keep exact control points.
    let kinds: Vec<_> = art.paths[3].segments.iter().map(Segment::kind).collect();
    assert_eq!(kinds, ["spline", "spline"]);
    let Segment::Spline { degree, poles, .. } = &art.paths[3].segments[0] else {
        panic!()
    };
    assert_eq!(*degree, 3);
    pnear(poles[1], [30., 25.]);
    let Segment::Spline { degree, .. } = &art.paths[3].segments[1] else {
        panic!()
    };
    assert_eq!(*degree, 2);
    let unsupported: BTreeSet<_> = art.unsupported.iter().map(|d| d.kind.as_str()).collect();
    assert_eq!(
        unsupported,
        BTreeSet::from([
            "svg.hidden",
            "svg.image",
            "svg.script",
            "svg.style",
            "svg.text",
            "svg.use"
        ])
    );
}

#[test]
fn svg_pixel_policy_explicit_units_and_unsafe_content_are_bounded() {
    let px =
        br#"<svg xmlns="http://www.w3.org/2000/svg"><line x1="0" y1="0" x2="96" y2="0"/></svg>"#;
    let art = sketch_import::parse(Format::Svg, px, UnitChoice::FromFile).unwrap();
    let Segment::Line { a, b } = &art.paths[0].segments[0] else {
        panic!()
    };
    near(b[0] - a[0], 25.4);
    assert!(art.notes.iter().any(|n| n.contains("CSS pixel")));
    let mm = sketch_import::parse(
        Format::Svg,
        px,
        UnitChoice::Explicit(SourceUnit::Millimeter),
    )
    .unwrap();
    let Segment::Line { a, b } = &mm.paths[0].segments[0] else {
        panic!()
    };
    near(b[0] - a[0], 96.);
    let entity = br#"<?xml version="1.0"?><!DOCTYPE svg [<!ENTITY x SYSTEM "file:///etc/passwd">]><svg xmlns="http://www.w3.org/2000/svg"><text>&x;</text></svg>"#;
    assert!(
        sketch_import::parse(Format::Svg, entity, UnitChoice::FromFile)
            .unwrap_err()
            .0
            .contains("entity")
    );
    let skew = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10" width="10mm" height="10mm"><circle cx="5" cy="5" r="2" transform="skewX(30)"/></svg>"#;
    let art = sketch_import::parse(Format::Svg, skew, UnitChoice::FromFile).unwrap();
    assert!(matches!(
        art.paths[0].segments[0],
        Segment::Spline { degree: 2, .. }
    ));
    assert!(sketch_import::parse(Format::Svg, b"<html/>", UnitChoice::FromFile).is_err());
    assert!(
        sketch_import::parse(Format::Svg, b"<svg><path d=\"M0 0 L", UnitChoice::FromFile).is_err()
    );
}

#[test]
fn placement_layers_welding_and_budgets_produce_canonical_editable_curves() {
    let art = sketch_import::parse(Format::Dxf, DXF, UnitChoice::FromFile).unwrap();
    let options = ImportOptions {
        layers: Some(BTreeSet::from(["CUT".to_string()])),
        insertion_mm: [10., 20.],
        rotation_deg: 90.,
        scale: 2.,
        ..Default::default()
    };
    let plan = plan(&art, &options).unwrap();
    let preview = plan.preview(&empty_source());
    assert_eq!(preview.applicable, Ok(()));
    assert_eq!(
        (
            preview.curves.lines,
            preview.curves.arcs,
            preview.curves.circles,
            preview.curves.splines
        ),
        (6, 3, 1, 0)
    );
    // Outer polyline, circle and four separately authored slot entities.
    assert_eq!((preview.closed_loops, preview.open_chains), (3, 0));
    // Rotated 90° about the insertion point and doubled: X ∈ [−110, 10], Y ∈ [20, 220].
    let [min, max] = preview.bounds_mm.unwrap();
    pnear(min, [-110., 20.]);
    pnear(max, [10., 220.]);
    assert!(
        preview
            .layers
            .iter()
            .any(|l| l.name == "ENGRAVE" && !l.selected && l.curves == 1)
    );
    let appended = plan.append_to(&empty_source()).unwrap();
    assert_eq!(appended.curves.len(), 10);
    // Slot endpoints were welded: 4 lines + 2 arcs use 4 shared ends + 2 centers.
    let slot_ends: BTreeSet<_> = appended
        .source
        .sketch
        .curves
        .iter()
        .filter_map(|c| match c.kind {
            CurveKind::Line { start, end } | CurveKind::Arc { start, end, .. } => {
                Some([start, end])
            }
            _ => None,
        })
        .flatten()
        .collect();
    assert_eq!(slot_ends.len(), 5 + 4);
    assert!(appended.source.validate().is_ok());
    // Unknown layers and excessive budgets are explicit errors.
    let missing = ImportOptions {
        layers: Some(BTreeSet::from(["NOPE".to_string()])),
        ..Default::default()
    };
    assert!(plan_err(&art, &missing).contains("not present"));
    let mut full = empty_source();
    for i in 0..250 {
        full.add_point([i as f64, 0.]);
    }
    let preview = plan.preview(&full);
    assert!(preview.applicable.unwrap_err().contains("256 points"));
}

fn plan_err(art: &sketch_import::Artwork, options: &ImportOptions) -> String {
    plan(art, options).unwrap_err().0
}

#[test]
fn provenance_records_validate_link_to_sketches_and_require_a_capability() {
    let mut document = Document::new("Import").unwrap();
    let (sketch, edits) = Source::create(Datum::Xy, "Imported").unwrap();
    for edit in edits {
        let Mutation::Insert(e) = edit else { panic!() };
        document.insert(e).unwrap();
    }
    let record = sketch_import::Provenance {
        format: Format::Svg,
        file_name: "badge.svg".into(),
        sha256: sketch_import::provenance::sha256_hex(SVG),
        byte_length: SVG.len() as u64,
        units: UnitChoice::FromFile,
        millimeters_per_unit: 0.5,
        scale: 1.,
        insertion_mm: [0., 0.],
        rotation_deg: 0.,
        weld_tolerance_mm: 1e-4,
        layers: vec!["Outline".into()],
        curves: vec![],
        unsupported: vec![],
    };
    let entity = record.entity(sketch).unwrap();
    let id = entity.id;
    document.insert(entity).unwrap();
    document.validate().unwrap();
    assert_eq!(
        sketch_import::provenance::for_sketch(&document, sketch)[0].0,
        id
    );
    let project = multicad_core::project_file::ProjectFile::new(document.clone()).unwrap();
    assert!(project.capabilities().contains(sketch_import::CAPABILITY));
    let mut bad = record.clone();
    bad.file_name = "/Users/someone/badge.svg".into();
    assert!(bad.entity(sketch).is_err());
    let mut bad = record;
    bad.sha256 = "zz".into();
    assert!(bad.validate().is_err());
}
