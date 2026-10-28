use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::{Dimension, source::Source},
    document::Document,
    model::{
        self, Direction, Extent, ExtrudeParameters, ExtrudeRequest, Termination, angle, length,
    },
    sketch::Datum,
    sketch_regions,
};
use multicad_model::Compiler;
use std::f64::consts::FRAC_PI_2;

fn build(elevation: Dimension, reverse: bool) -> multicad_model::ShapeSnapshot {
    let (sketch, edits) = Source::create(Datum::Xy, "Integer-coordinate rectangle").unwrap();
    let mut document = Document::new("Exact perpendicular direction")
        .unwrap()
        .edited(&edits)
        .unwrap();
    let mut source = Source::read(&document, sketch).unwrap().0;
    source.add_rectangle([10., 10.], [30., 30.]).unwrap();
    let region = sketch_regions::extract(&source).regions[0].intent();
    document = document
        .edited(&source.replace(&document, sketch).unwrap())
        .unwrap();
    let mut session = Session::new(document).unwrap();
    let request = ExtrudeRequest::new(
        sketch,
        vec![region],
        ExtrudeParameters {
            direction: Direction {
                elevation,
                reverse,
                ..Default::default()
            },
            extent: Extent::OneSided {
                end: Termination::Distance {
                    distance: length(100.),
                },
            },
            ..Default::default()
        },
    );
    let prepared =
        model::prepare_extrude(&session, 0, Actor::local(Channel::Mcp), &request).unwrap();
    session.commit(prepared.prepared).unwrap();
    let canonical = session.document().clone();
    let model = Compiler::default()
        .compile(&canonical, session.revision())
        .unwrap();
    assert_eq!(session.document(), &canonical);
    (*model.snapshot().shapes[&request.outputs[0].feature]).clone()
}
#[test]
fn native_perpendicular_prism_keeps_exact_integer_in_plane_bounds() {
    for elevation in [
        angle(90.),
        Dimension::Literal {
            value: FRAC_PI_2,
            unit: "rad".into(),
        },
    ] {
        for reverse in [false, true] {
            let shape = build(elevation.clone(), reverse);
            assert_eq!(
                shape.bounds.min,
                [10., 10., if reverse { -100. } else { 0. }]
            );
            assert_eq!(
                shape.bounds.max,
                [30., 30., if reverse { 0. } else { 100. }]
            );
            assert!((shape.volume_mm3 - 40000.).abs() < 1e-7);
            assert_eq!(shape.solid_count, 1);
        }
    }
}
#[test]
fn native_near_perpendicular_prism_keeps_its_authored_tilt() {
    let shape = build(angle(89.9999), false);
    assert!(shape.bounds.max[0] > 30.0001);
    let expected = 30. + 100. * 89.9999_f64.to_radians().cos();
    assert!((shape.bounds.max[0] - expected).abs() < 1e-10);
    assert_eq!(shape.bounds.max[1], 30.);
}
