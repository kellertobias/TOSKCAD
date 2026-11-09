use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::{Dimension, source::Source},
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
    let (sketch, edits) = Source::create(Datum::Xz, "Concave profile").unwrap();
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
    let region = sketch_regions::extract(&source)
        .regions
        .into_iter()
        .find(|region| region.holes.len() == 1)
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
            start_offset: length(2.),
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
