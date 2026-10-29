//! A native project whose two consumer sketches each intersect a wall edge.
//! Lowering both walls below the sketch planes breaks both references.
#![allow(dead_code)]
use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::{ConstraintKind, CurveKind, source::Source},
    document::{Document, EntityData, EntityId, Mutation},
    model::{self, ExtrudeRequest, PrimitiveParameters, PrimitiveRequest},
    references::{AnalyticGeometry, GeometryIntent, TopologyKey},
    sketch::Datum,
    sketch_external::{Behavior, CaptureInput, CaptureRequest, Method, OutputIds},
};
use multicad_model::Compiler;

pub fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
pub fn wall(width: f64, height: f64) -> PrimitiveParameters {
    PrimitiveParameters::Box {
        size: [
            model::length(width),
            model::length(20.),
            model::length(height),
        ],
        frame: Default::default(),
    }
}
/// The vertical edge at (x, 0) of a 20 mm deep wall.
pub fn edge(
    compiler: &Compiler,
    document: &Document,
    revision: u64,
    feature: EntityId,
    x: f64,
) -> TopologyKey {
    let native = compiler
        .compile_sources(document, revision, &[feature])
        .unwrap();
    let topology = &native.snapshot().shapes[&feature].topology;
    topology.elements.iter().find(|e| matches!(e.geometry, Some(AnalyticGeometry::Line { a_mm: a, b_mm: b }) if (a[0] - x).abs() < 1e-7 && (b[0] - x).abs() < 1e-7 && a[1].abs() < 1e-7 && b[1].abs() < 1e-7 && (a[2] - b[2]).abs() > 9.)).unwrap().key
}
pub struct Consumer {
    pub sketch: EntityId,
    pub reference: EntityId,
    pub coincident: EntityId,
    pub part: EntityId,
}
/// A Z5 sketch that intersects the wall edge at `x` and drives a 5 × 7 mm
/// profile through a coincident constraint, extruded 2 mm.
pub fn consumer(
    session: &mut Session,
    compiler: &Compiler,
    source: EntityId,
    x: f64,
    name: &str,
) -> Consumer {
    let (sketch, mut edits) = Source::create(Datum::Xy, name).unwrap();
    for edit in &mut edits {
        if let Mutation::Insert(entity) = edit
            && let EntityData::Plane { origin_mm, .. } = &mut entity.data
        {
            origin_mm[2] = 5.;
        }
    }
    session
        .apply(multicad_core::commands::Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Create sketch".into(),
            edits,
        })
        .unwrap();
    let native = compiler
        .compile(session.document(), session.revision())
        .unwrap();
    let map = native
        .snapshot()
        .shapes
        .iter()
        .map(|(id, s)| (*id, s.topology.clone()))
        .collect();
    let topology = &native.snapshot().shapes[&source].topology;
    let key = edge(compiler, session.document(), session.revision(), source, x);
    let capture = CaptureRequest::new(
        session.document(),
        session.revision(),
        CaptureInput {
            sketch,
            source,
            intent: GeometryIntent::capture(topology, key).unwrap(),
            method: Method::Intersect,
            behavior: Behavior::ReferenceOnly,
        },
        &map,
    )
    .unwrap();
    let reference = multicad_solver::prepare_external_reference(
        session,
        session.revision(),
        actor(),
        &capture,
        &map,
    )
    .unwrap();
    session.commit(reference.edit.preview.unwrap()).unwrap();
    let EntityData::Sketch { definition, .. } = &session.document().get(sketch).unwrap().data
    else {
        panic!()
    };
    let mut own = Source::from_definition(definition).unwrap();
    let OutputIds::Point { point, .. } = capture.output else {
        panic!()
    };
    let line = own.add_rectangle([x, 0.], [x + 5., 7.]).unwrap();
    let CurveKind::Line { start, .. } = own
        .sketch
        .curves
        .iter()
        .find(|c| c.id == line)
        .unwrap()
        .kind
    else {
        panic!()
    };
    let side = own.sketch.curves[1].id;
    let coincident = own.add_constraint(ConstraintKind::Coincident { a: point, b: start });
    for kind in [
        ConstraintKind::Length {
            line,
            dimension: model::length(5.),
        },
        ConstraintKind::Length {
            line: side,
            dimension: model::length(7.),
        },
    ] {
        own.add_constraint(kind);
    }
    let native = compiler
        .compile(session.document(), session.revision())
        .unwrap();
    let edit = multicad_solver::prepare_canonical_edit(
        session,
        session.revision(),
        actor(),
        sketch,
        own,
        Some(&native.snapshot().references),
    )
    .unwrap();
    let region = edit.semantic.as_ref().unwrap().regions().unwrap().regions[0].intent();
    session.commit(edit.preview.unwrap()).unwrap();
    let extrusion = ExtrudeRequest::new(
        sketch,
        vec![region],
        model::ExtrudeParameters {
            extent: model::Extent::OneSided {
                end: model::Termination::Distance {
                    distance: model::length(2.),
                },
            },
            ..Default::default()
        },
    );
    let edit = model::prepare_extrude(session, session.revision(), actor(), &extrusion).unwrap();
    session.commit(edit.prepared).unwrap();
    Consumer {
        sketch,
        reference: capture.reference,
        coincident,
        part: extrusion.outputs[0].feature,
    }
}
pub fn primitive(session: &mut Session, request: &PrimitiveRequest) {
    let edit = model::prepare_primitive(session, session.revision(), actor(), request).unwrap();
    session.commit(edit.prepared).unwrap();
}
pub fn shrink(session: &mut Session, feature: EntityId, width: f64) {
    let edit = model::prepare_primitive_edit(
        session,
        session.revision(),
        actor(),
        feature,
        &wall(width, 2.),
    )
    .unwrap();
    session.commit(edit.prepared).unwrap();
}
pub struct Fixture {
    pub session: Session,
    pub a: Consumer,
    pub b: Consumer,
    pub replacement_a: PrimitiveRequest,
    pub replacement_b: PrimitiveRequest,
}
/// Two walls feed two independent consumer sketches; both walls are then
/// lowered below the Z5 sketch planes, so both references break.
pub fn broken(with_replacement_b: bool) -> Fixture {
    let compiler = Compiler::default();
    let mut session = Session::new(Document::new("Two broken references").unwrap()).unwrap();
    let original_a = PrimitiveRequest::new("Original wall A", wall(30., 10.));
    let original_b = PrimitiveRequest::new("Original wall B", wall(60., 10.));
    let replacement_a = PrimitiveRequest::new("Replacement wall A", wall(40., 10.));
    let replacement_b = PrimitiveRequest::new("Replacement wall B", wall(50., 10.));
    primitive(&mut session, &original_a);
    primitive(&mut session, &original_b);
    primitive(&mut session, &replacement_a);
    if with_replacement_b {
        primitive(&mut session, &replacement_b);
    }
    let a = consumer(&mut session, &compiler, original_a.feature, 30., "Sketch A");
    let b = consumer(&mut session, &compiler, original_b.feature, 60., "Sketch B");
    assert!(
        compiler
            .compile(session.document(), session.revision())
            .is_ok()
    );
    shrink(&mut session, original_a.feature, 30.);
    shrink(&mut session, original_b.feature, 60.);
    Fixture {
        session,
        a,
        b,
        replacement_a,
        replacement_b,
    }
}
