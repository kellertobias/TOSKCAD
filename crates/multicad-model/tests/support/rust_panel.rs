use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::{Curve, CurveKind, Dimension, source::Source},
    document::{Document, Entity, EntityData, EntityId, Mutation},
    model::{
        self, BodyInput, BooleanRequest, Extent, ExtrudeParameters, ExtrudeRequest, Operation,
        PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, Termination, length,
    },
    parameters::BoundExpression,
    sketch::Datum,
    sketch_regions,
};

fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}

pub fn fixture(
    quarter_turn: bool,
    cutter_x: f64,
) -> (Session, EntityId, EntityId, EntityId, EntityId, EntityId) {
    let (sketch, edits) = Source::create(Datum::Xy, "Rounded panel profile").unwrap();
    let mut document = Document::new("Rust backend panel")
        .unwrap()
        .edited(&edits)
        .unwrap();
    let mut profile = Source::read(&document, sketch).unwrap().0;
    let [a, b, c, d, arc_center, hole_center] = [
        [0., 0.],
        [40., 0.],
        [40., 20.],
        [0., 20.],
        [40., 10.],
        [12., 10.],
    ]
    .map(|point| profile.add_point(point));
    profile.add_line(a, b);
    profile.sketch.curves.push(Curve {
        id: EntityId::new(),
        construction: false,
        kind: CurveKind::Arc {
            center: arc_center,
            start: b,
            end: c,
            clockwise: false,
        },
    });
    profile.add_line(c, d);
    profile.add_line(d, a);
    profile.sketch.curves.push(Curve {
        id: EntityId::new(),
        construction: false,
        kind: CurveKind::Circle {
            center: hole_center,
            radius_mm: 3.,
        },
    });
    let region = sketch_regions::extract(&profile)
        .regions
        .into_iter()
        .find(|r| r.holes.len() == 1)
        .unwrap()
        .intent();
    let height = Entity::new(
        "panel_height",
        EntityData::Parameter {
            expression: "8".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    document = document
        .edited(&profile.replace(&document, sketch).unwrap())
        .unwrap()
        .edited(&[Mutation::Insert(height.clone())])
        .unwrap();
    let mut session = Session::new(document).unwrap();
    let distance = Dimension::Expression {
        expression: BoundExpression::bind(session.document(), "=$panel_height", "mm", None)
            .unwrap(),
    };
    let panel = ExtrudeRequest::new(
        sketch,
        vec![region],
        ExtrudeParameters {
            extent: Extent::OneSided {
                end: Termination::Distance { distance },
            },
            ..Default::default()
        },
    );
    let prepared = model::prepare_extrude(&session, session.revision(), actor(), &panel).unwrap();
    session.commit(prepared.prepared).unwrap();
    let mut frame = PrimitiveFrame {
        origin: [length(cutter_x), length(10.), length(-1.)],
        ..PrimitiveFrame::default()
    };
    if quarter_turn {
        frame.u = [0., 1., 0.];
        frame.v = [-1., 0., 0.];
    }
    let cutter = PrimitiveRequest::new(
        "Through-bore tool",
        PrimitiveParameters::Cylinder {
            radius: length(4.),
            height: length(10.),
            frame,
        },
    );
    let prepared =
        model::prepare_primitive(&session, session.revision(), actor(), &cutter).unwrap();
    session.commit(prepared.prepared).unwrap();
    let output = &panel.outputs[0];
    let cut = BooleanRequest::new(
        Operation::Cut,
        BodyInput {
            body: output.body,
            producer: output.feature,
        },
        vec![BodyInput {
            body: cutter.body,
            producer: cutter.feature,
        }],
    );
    let prepared = model::prepare_boolean(&session, session.revision(), actor(), &cut).unwrap();
    session.commit(prepared.prepared).unwrap();
    (
        session,
        height.id,
        output.feature,
        output.body,
        cutter.feature,
        cut.feature,
    )
}
