use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::{Curve, CurveKind, Dimension, source::Source},
    document::{Document, Entity, EntityData, EntityId, Mutation},
    model::{self, Extent, ExtrudeParameters, ExtrudeRequest, Termination, length},
    parameters::BoundExpression,
    sketch::Datum,
    sketch_regions,
};

pub fn fixture() -> (Session, EntityId, EntityId, EntityId) {
    fixture_with_height("6")
}

pub fn fixture_with_height(expression: &str) -> (Session, EntityId, EntityId, EntityId) {
    build_fixture(expression, false, Datum::Xz, 2.)
}

pub fn fixture_with_circle() -> (Session, EntityId, EntityId, EntityId) {
    build_fixture("6", true, Datum::Xz, 2.)
}

pub fn build_fixture(
    expression: &str,
    round_hole: bool,
    datum: Datum,
    start_offset: f64,
) -> (Session, EntityId, EntityId, EntityId) {
    let (sketch, edits) = Source::create(datum, "Concave profile").unwrap();
    let document = Document::new("Straight sketch extrusion")
        .unwrap()
        .edited(&edits)
        .unwrap();
    let mut source = Source::read(&document, sketch).unwrap().0;
    let mut side = EntityId::nil();
    for ring in [
        &[
            [0., 0.],
            [12., 0.],
            [12., 6.],
            [6., 6.],
            [6., 10.],
            [0., 10.],
        ][..],
        &[[2., 2.], [4., 2.], [4., 4.], [2., 4.]][..],
    ] {
        let points: Vec<_> = ring.iter().map(|point| source.add_point(*point)).collect();
        for (&a, &b) in points.iter().zip(points.iter().cycle().skip(1)) {
            side = source.add_line(a, b);
        }
    }
    if round_hole {
        let center = source.add_point([9., 3.]);
        side = EntityId::new();
        source.sketch.curves.push(Curve {
            id: side,
            construction: false,
            kind: CurveKind::Circle {
                center,
                radius_mm: 1.25,
            },
        });
    }
    let region = sketch_regions::extract(&source)
        .regions
        .into_iter()
        .find(|region| region.holes.len() == 1 + usize::from(round_hole))
        .unwrap()
        .intent();
    let height = Entity::new(
        "height",
        EntityData::Parameter {
            expression: expression.into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let document = document
        .edited(&source.replace(&document, sketch).unwrap())
        .unwrap()
        .edited(&[Mutation::Insert(height.clone())])
        .unwrap();
    let mut session = Session::new(document).unwrap();
    let distance = Dimension::Expression {
        expression: BoundExpression::bind(session.document(), "=$height", "mm", None).unwrap(),
    };
    let extrude = ExtrudeRequest::new(
        sketch,
        vec![region],
        ExtrudeParameters {
            start_offset: length(start_offset),
            extent: Extent::OneSided {
                end: Termination::Distance { distance },
            },
            ..Default::default()
        },
    );
    let prepared = model::prepare_extrude(
        &session,
        session.revision(),
        Actor::local(Channel::Mcp),
        &extrude,
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
    (session, height.id, extrude.outputs[0].feature, side)
}
