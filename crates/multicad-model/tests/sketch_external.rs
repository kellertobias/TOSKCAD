//! Genuine native B-rep edges drive a canonical constrained sketch and its
//! downstream solid. No tessellation or hand-authored projection substitute.
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::{Axis, ConstraintKind, Curve, CurveKind, Dimension, source::Source},
    document::{Document, Entity, EntityData, EntityId, Mutation},
    model::{self, ExtrudeRequest, PrimitiveParameters, PrimitiveRequest},
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    references::{self, AnalyticGeometry, GeometryIntent, TopologySnapshot},
    sketch::Datum,
    sketch_external::{self, Behavior, CaptureInput, CaptureRequest, Method, OutputIds},
};
use multicad_model::Compiler;
use std::collections::BTreeMap;
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn topologies(
    model: &multicad_model::Model,
) -> BTreeMap<multicad_core::document::EntityId, TopologySnapshot> {
    model
        .snapshot()
        .shapes
        .iter()
        .map(|(id, shape)| (*id, shape.topology.clone()))
        .collect()
}
fn source(session: &Session, sketch: multicad_core::document::EntityId) -> Source {
    let EntityData::Sketch { definition, .. } = &session.document().get(sketch).unwrap().data
    else {
        panic!()
    };
    Source::from_definition(definition).unwrap()
}
#[test]
fn native_projection_rebuilds_dependent_constrained_geometry_and_solid_without_baking_source() {
    let (sketch, edits) = Source::create(Datum::Xy, "Linked cabinet edge").unwrap();
    let mut session = Session::new(
        Document::new("Native adjacent front projection")
            .unwrap()
            .edited(&edits)
            .unwrap(),
    )
    .unwrap();
    let rise = Entity::new(
        "rise",
        EntityData::Parameter {
            expression: "10".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    session
        .apply(Request {
            expected_revision: 0,
            actor: actor(),
            label: "Rise parameter".into(),
            edits: vec![Mutation::Insert(rise)],
        })
        .unwrap();
    let parameters = |width| PrimitiveParameters::Box {
        size: [model::length(width), model::length(20.), model::length(10.)],
        frame: Default::default(),
    };
    let base = PrimitiveRequest::new("Adjacent cabinet", parameters(30.));
    let edit = model::prepare_primitive(&session, session.revision(), actor(), &base).unwrap();
    session.commit(edit.prepared).unwrap();
    let compiler = Compiler::default();
    let native = compiler
        .compile(session.document(), session.revision())
        .unwrap();
    let map = topologies(&native);
    let snapshot = &map[&base.feature];
    let key=snapshot.elements.iter().find(|element|matches!(element.geometry,Some(AnalyticGeometry::Line{a_mm,b_mm}) if a_mm[1].abs()<1e-7&&b_mm[1].abs()<1e-7&&a_mm[2].abs()<1e-7&&b_mm[2].abs()<1e-7&&(a_mm[0]-b_mm[0]).abs()>29.)).expect("Real box bottom edge has exact analytic line geometry").key;
    let request = CaptureRequest::new(
        session.document(),
        session.revision(),
        CaptureInput {
            sketch,
            source: base.feature,
            intent: GeometryIntent::capture(snapshot, key).unwrap(),
            method: Method::Coplanar,
            behavior: Behavior::Derived,
        },
        &map,
    )
    .unwrap();
    let captured = multicad_solver::prepare_external_reference(
        &session,
        session.revision(),
        actor(),
        &request,
        &map,
    )
    .unwrap();
    assert_eq!(session.revision(), 2);
    session.commit(captured.edit.preview.unwrap()).unwrap();
    let mut authored = source(&session, sketch);
    let OutputIds::Line {
        curve: bottom,
        start,
        end,
        ..
    } = request.output
    else {
        panic!()
    };
    let a = authored
        .sketch
        .points
        .iter()
        .find(|point| point.id == start)
        .unwrap()
        .position_mm;
    let b = authored
        .sketch
        .points
        .iter()
        .find(|point| point.id == end)
        .unwrap()
        .position_mm;
    let top_a = authored.add_point([a[0], a[1] + 10.]);
    let top_b = authored.add_point([b[0], b[1] + 10.]);
    let right = authored.add_line(end, top_b);
    let top = authored.add_line(top_b, top_a);
    let left = authored.add_line(top_a, start);
    for kind in [
        ConstraintKind::Vertical { line: right },
        ConstraintKind::Horizontal { line: top },
        ConstraintKind::Vertical { line: left },
        ConstraintKind::Distance {
            a: start,
            b: top_a,
            axis: Axis::Y,
            dimension: Dimension::Expression {
                expression: BoundExpression::bind(session.document(), "=$rise", "mm", None)
                    .unwrap(),
            },
        },
    ] {
        authored.add_constraint(kind);
    }
    let native = compiler
        .compile(session.document(), session.revision())
        .unwrap();
    let map = topologies(&native);
    let report = references::inspect(session.document(), session.revision(), &map);
    let edit = multicad_solver::prepare_canonical_edit(
        &session,
        session.revision(),
        actor(),
        sketch,
        authored,
        Some(&report),
    )
    .unwrap();
    let semantic = edit.semantic.as_ref().unwrap();
    assert!(
        semantic
            .solved()
            .unwrap()
            .constraints
            .iter()
            .all(|constraint| !request.output.locks().contains(&constraint.id))
    );
    let regions = semantic.regions().unwrap();
    assert_eq!(regions.regions.len(), 1);
    assert!(
        regions.regions[0]
            .outer
            .edges
            .iter()
            .any(|edge| edge.authored == bottom)
    );
    let intent = regions.regions[0].intent();
    session.commit(edit.preview.unwrap()).unwrap();
    let extrusion = ExtrudeRequest::new(
        sketch,
        vec![intent],
        model::ExtrudeParameters {
            extent: model::Extent::OneSided {
                end: model::Termination::Distance {
                    distance: model::length(5.),
                },
            },
            ..Default::default()
        },
    );
    let edit = model::prepare_extrude(&session, session.revision(), actor(), &extrusion).unwrap();
    session.commit(edit.prepared).unwrap();
    let before = session.document().clone();
    let canonical = source(&session, sketch);
    let model = compiler.compile(&before, session.revision()).unwrap();
    let output = extrusion.outputs[0].feature;
    assert!((model.snapshot().shapes[&output].volume_mm3 - 1500.).abs() < 1e-6);
    let edit = model::prepare_primitive_edit(
        &session,
        session.revision(),
        actor(),
        base.feature,
        &parameters(40.),
    )
    .unwrap();
    session.commit(edit.prepared).unwrap();
    let changed = session.document().clone();
    let current = compiler.compile(&changed, session.revision()).unwrap();
    assert!((current.snapshot().shapes[&output].volume_mm3 - 2000.).abs() < 1e-6);
    assert_eq!(
        source(&session, sketch),
        canonical,
        "External exact positions and solved dimensions must not replace authored guesses/expressions"
    );
    let evaluated = multicad_solver::evaluate_document_sketch_with_parameters(
        &changed,
        session.revision(),
        sketch,
        Some(&current.snapshot().references),
        session.parameters(),
    )
    .unwrap();
    let solved = evaluated.solved().unwrap();
    assert!(
        solved
            .points
            .iter()
            .any(|point| (point.position_mm[0] - 40.).abs() < 1e-7)
    );
    assert!(evaluated.canonical().has_expressions());
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(changed.clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let opened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(opened.project.document, changed);
    assert!(
        (compiler
            .compile(&opened.project.document, session.revision())
            .unwrap()
            .snapshot()
            .shapes[&output]
            .volume_mm3
            - 2000.)
            .abs()
            < 1e-6
    );
    let revision = session.revision();
    session.undo(revision, actor()).unwrap();
    assert_eq!(session.document(), &before);
    assert!(
        (compiler
            .compile(session.document(), session.revision())
            .unwrap()
            .snapshot()
            .shapes[&output]
            .volume_mm3
            - 1500.)
            .abs()
            < 1e-6
    );
    session.redo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &changed);
    let mut moved = parameters(40.);
    let PrimitiveParameters::Box { frame, .. } = &mut moved else {
        panic!()
    };
    frame.origin[0] = model::length(5.);
    let edit =
        model::prepare_primitive_edit(&session, session.revision(), actor(), base.feature, &moved)
            .unwrap();
    let moved_model = compiler
        .compile(edit.prepared.document(), session.revision())
        .unwrap();
    let shape = &moved_model.snapshot().shapes[&output];
    assert!((shape.bounds.min[0] - 5.).abs() < 1e-6);
    assert!((shape.bounds.max[0] - 45.).abs() < 1e-6);
    assert!((shape.volume_mm3 - 2000.).abs() < 1e-6);
    assert!(
        sketch_external::remove_edits(&changed, sketch, request.reference)
            .unwrap_err()
            .to_string()
            .contains("depends")
    );
    let bad = references::inspect(&changed, session.revision(), &BTreeMap::new());
    assert!(
        multicad_solver::evaluate_document_sketch_with_parameters(
            &changed,
            session.revision(),
            sketch,
            Some(&bad),
            session.parameters()
        )
        .is_err()
    );
}
#[test]
fn native_circle_reference_drives_an_independent_equal_radius_profile() {
    let (sketch, edits) = Source::create(Datum::Xy, "Linked circular reference").unwrap();
    let mut session = Session::new(
        Document::new("Native circle projection")
            .unwrap()
            .edited(&edits)
            .unwrap(),
    )
    .unwrap();
    let parameters = |radius| PrimitiveParameters::Cylinder {
        radius: model::length(radius),
        height: model::length(10.),
        frame: Default::default(),
    };
    let base = PrimitiveRequest::new("Cylinder source", parameters(5.));
    let edit = model::prepare_primitive(&session, 0, actor(), &base).unwrap();
    session.commit(edit.prepared).unwrap();
    let compiler = Compiler::default();
    let native = compiler
        .compile(session.document(), session.revision())
        .unwrap();
    let map = topologies(&native);
    let snapshot = &map[&base.feature];
    let key=snapshot.elements.iter().find(|element|matches!(element.geometry,Some(AnalyticGeometry::CircularArc{frame,sweep_rad,..})if frame.origin_mm[2].abs()<1e-7&&(sweep_rad.abs()-std::f64::consts::TAU).abs()<1e-7)).unwrap().key;
    let request = CaptureRequest::new(
        session.document(),
        session.revision(),
        CaptureInput {
            sketch,
            source: base.feature,
            intent: GeometryIntent::capture(snapshot, key).unwrap(),
            method: Method::Coplanar,
            behavior: Behavior::ReferenceOnly,
        },
        &map,
    )
    .unwrap();
    let edit = multicad_solver::prepare_external_reference(
        &session,
        session.revision(),
        actor(),
        &request,
        &map,
    )
    .unwrap();
    session.commit(edit.edit.preview.unwrap()).unwrap();
    let mut own = source(&session, sketch);
    let center = own.add_point([20., 0.]);
    let circle = EntityId::new();
    own.sketch.curves.push(Curve {
        id: circle,
        construction: false,
        kind: CurveKind::Circle {
            center,
            radius_mm: 2.,
        },
    });
    own.add_constraint(ConstraintKind::Fix { point: center });
    own.add_constraint(ConstraintKind::Equal {
        a: request.output.curve().unwrap(),
        b: circle,
    });
    let native = compiler
        .compile(session.document(), session.revision())
        .unwrap();
    let report = &native.snapshot().references;
    let edit = multicad_solver::prepare_canonical_edit(
        &session,
        session.revision(),
        actor(),
        sketch,
        own,
        Some(report),
    )
    .unwrap();
    let regions = edit.semantic.as_ref().unwrap().regions().unwrap();
    assert_eq!(
        regions.regions.len(),
        1,
        "Reference-only circle does not create a manufacturing profile"
    );
    let region = regions.regions[0].intent();
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
    let edit = model::prepare_extrude(&session, session.revision(), actor(), &extrusion).unwrap();
    session.commit(edit.prepared).unwrap();
    let canonical = source(&session, sketch);
    let output = extrusion.outputs[0].feature;
    assert!(
        (compiler
            .compile(session.document(), session.revision())
            .unwrap()
            .snapshot()
            .shapes[&output]
            .volume_mm3
            - 50. * std::f64::consts::PI)
            .abs()
            < 1e-6
    );
    let edit = model::prepare_primitive_edit(
        &session,
        session.revision(),
        actor(),
        base.feature,
        &parameters(7.),
    )
    .unwrap();
    session.commit(edit.prepared).unwrap();
    let native = compiler
        .compile(session.document(), session.revision())
        .unwrap();
    assert!(
        (native.snapshot().shapes[&output].volume_mm3 - 98. * std::f64::consts::PI).abs() < 1e-6
    );
    assert_eq!(source(&session, sketch), canonical);
}
#[test]
fn a_real_wall_edge_intersection_moves_a_constrained_profile_and_rejects_non_crossing_edits() {
    let (sketch, mut edits) = Source::create(Datum::Xy, "Wall crossing plane").unwrap();
    for mutation in &mut edits {
        if let Mutation::Insert(entity) = mutation
            && let EntityData::Plane { origin_mm, .. } = &mut entity.data
        {
            origin_mm[2] = 5.;
        }
    }
    let mut session = Session::new(
        Document::new("Native finite wall intersection")
            .unwrap()
            .edited(&edits)
            .unwrap(),
    )
    .unwrap();
    let params = |width, height| PrimitiveParameters::Box {
        size: [
            model::length(width),
            model::length(20.),
            model::length(height),
        ],
        frame: Default::default(),
    };
    let base = PrimitiveRequest::new("Wall", params(30., 10.));
    let edit = model::prepare_primitive(&session, 0, actor(), &base).unwrap();
    session.commit(edit.prepared).unwrap();
    let compiler = Compiler::default();
    let native = compiler
        .compile(session.document(), session.revision())
        .unwrap();
    let map = topologies(&native);
    let snapshot = &map[&base.feature];
    let key=snapshot.elements.iter().find(|element|matches!(element.geometry,Some(AnalyticGeometry::Line{a_mm,b_mm})if(a_mm[0]-30.).abs()<1e-7&&(b_mm[0]-30.).abs()<1e-7&&a_mm[1].abs()<1e-7&&b_mm[1].abs()<1e-7&&(a_mm[2]-b_mm[2]).abs()>9.)).unwrap().key;
    let request = CaptureRequest::new(
        session.document(),
        session.revision(),
        CaptureInput {
            sketch,
            source: base.feature,
            intent: GeometryIntent::capture(snapshot, key).unwrap(),
            method: Method::Intersect,
            behavior: Behavior::ReferenceOnly,
        },
        &map,
    )
    .unwrap();
    let edit = multicad_solver::prepare_external_reference(
        &session,
        session.revision(),
        actor(),
        &request,
        &map,
    )
    .unwrap();
    session.commit(edit.edit.preview.unwrap()).unwrap();
    let OutputIds::Point { point, .. } = request.output else {
        panic!()
    };
    let mut own = source(&session, sketch);
    let bottom = own.add_rectangle([30., 0.], [35., 7.]).unwrap();
    let CurveKind::Line { start: corner, .. } = own
        .sketch
        .curves
        .iter()
        .find(|curve| curve.id == bottom)
        .unwrap()
        .kind
    else {
        panic!()
    };
    let side = own.sketch.curves[1].id;
    own.add_constraint(ConstraintKind::Coincident {
        a: point,
        b: corner,
    });
    own.add_constraint(ConstraintKind::Length {
        line: bottom,
        dimension: model::length(5.),
    });
    own.add_constraint(ConstraintKind::Length {
        line: side,
        dimension: model::length(7.),
    });
    let native = compiler
        .compile(session.document(), session.revision())
        .unwrap();
    let edit = multicad_solver::prepare_canonical_edit(
        &session,
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
    let edit = model::prepare_extrude(&session, session.revision(), actor(), &extrusion).unwrap();
    session.commit(edit.prepared).unwrap();
    let canonical = source(&session, sketch);
    let output = extrusion.outputs[0].feature;
    let edit = model::prepare_primitive_edit(
        &session,
        session.revision(),
        actor(),
        base.feature,
        &params(40., 10.),
    )
    .unwrap();
    session.commit(edit.prepared).unwrap();
    let native = compiler
        .compile(session.document(), session.revision())
        .unwrap();
    let shape = &native.snapshot().shapes[&output];
    assert!((shape.volume_mm3 - 70.).abs() < 1e-6);
    assert!((shape.bounds.min[0] - 40.).abs() < 1e-6);
    assert_eq!(source(&session, sketch), canonical);
    let before = session.document().clone();
    let bad = model::prepare_primitive_edit(
        &session,
        session.revision(),
        actor(),
        base.feature,
        &params(40., 2.),
    )
    .unwrap();
    let error = compiler
        .compile(bad.prepared.document(), session.revision())
        .err()
        .unwrap()
        .to_string();
    assert!(
        error.contains("finite edge") || error.contains("does not reach"),
        "{error}"
    );
    assert_eq!(session.document(), &before);
}
