use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Mutation},
    model::{
        self, BodyInput, HelicalTubeCutParameters, HelicalTubeCutRequest, PrimitiveParameters,
        PrimitiveRequest, angle, length,
    },
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    references::{self, GeometryIntent, Resolution, TopologyKind},
};
use multicad_kernel::rust_kernel::prism::Location;
use multicad_model::{Compiler, GeometryBackend, Model};
use std::f64::consts::{FRAC_PI_2, TAU};

fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}

fn compile(document: &Document, revision: u64) -> Model {
    Compiler::default()
        .compile_with_backend(document, revision, GeometryBackend::Rust, None)
        .unwrap()
}

fn fixture(stock: PrimitiveParameters) -> (Session, BodyInput, EntityId) {
    let mut document = Document::new("Helical seam cut").unwrap();
    let parameter = Entity::new(
        "cut_start",
        EntityData::Parameter {
            expression: "16.2".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let parameter_id = parameter.id;
    document.insert(parameter).unwrap();
    let mut session = Session::new(document).unwrap();
    let primitive = PrimitiveRequest::new("Stock", stock);
    let prepared =
        model::prepare_primitive(&session, session.revision(), actor(), &primitive).unwrap();
    session.commit(prepared.prepared).unwrap();
    (
        session,
        BodyInput {
            body: primitive.body,
            producer: primitive.feature,
        },
        parameter_id,
    )
}

fn parameters(session: &Session) -> HelicalTubeCutParameters {
    HelicalTubeCutParameters {
        radius: length(0.6),
        pitch: length(2.),
        path_height: length(15.),
        start_height: Dimension::Expression {
            expression: BoundExpression::bind(session.document(), "=$cut_start", "mm", None)
                .unwrap(),
        },
        start_angle: angle(0.),
    }
}

fn cylinder() -> PrimitiveParameters {
    PrimitiveParameters::Cylinder {
        radius: length(8.),
        height: length(30.),
        frame: Default::default(),
    }
}

fn verify_cut(
    model: &Model,
    body: EntityId,
    producer: EntityId,
    start: f64,
    phase: f64,
    expected_volume: f64,
) {
    let snapshot = model.snapshot();
    assert_eq!(snapshot.bodies[&body], producer);
    let shape = &snapshot.shapes[&producer];
    assert_eq!(shape.solid_count, 1);
    assert_eq!(shape.bounds.min, [-8., -8., 0.]);
    assert_eq!(shape.bounds.max, [8., 8., 30.]);
    assert!((shape.volume_mm3 - expected_volume).abs() < 1e-8);
    let solid = model.solid(producer).unwrap();
    for theta in [TAU - 0.01, TAU + 0.01, (30. - start) * TAU / 2. - 0.01] {
        let (sine, cosine) = (theta + phase).sin_cos();
        let height = start + 2. * theta / TAU;
        for (radius, expected) in [
            (7.7, Location::Exterior),
            (7.4, Location::Boundary),
            (7.2, Location::Interior),
        ] {
            assert_eq!(
                solid
                    .rust_helical_tube_material([radius * cosine, radius * sine, height])
                    .unwrap(),
                expected
            );
        }
    }
}

#[test]
fn canonical_helical_cut_saves_edits_and_undoes_with_stable_floor_provenance() {
    let (mut session, stock, start_parameter) = fixture(cylinder());
    let request = HelicalTubeCutRequest::new(stock.clone(), parameters(&session));
    let before = session.document().clone();
    let draft =
        model::prepare_helical_tube_cut(&session, session.revision(), actor(), &request).unwrap();
    let preview = compile(draft.prepared.document(), session.revision() + 1);
    verify_cut(
        &preview,
        stock.body,
        request.feature,
        16.2,
        0.,
        5841.816902505,
    );
    assert_eq!(session.document(), &before);
    assert!(
        draft
            .outputs
            .iter()
            .any(|output| { output.body == stock.body && output.feature == request.feature })
    );
    session.commit(draft.prepared).unwrap();
    let stale = HelicalTubeCutRequest::new(stock.clone(), request.parameters.clone());
    assert!(
        model::prepare_helical_tube_cut(&session, session.revision(), actor(), &stale).is_err()
    );

    let saved = model::read_helical_tube_cut(session.document(), request.feature).unwrap();
    assert_eq!(saved.input, stock);
    assert_eq!(saved.parameters, request.parameters);
    assert!(
        model::required_capabilities(session.document())
            .contains(model::HELICAL_TUBE_CUT_CAPABILITY)
    );
    let preview_topology = &preview.snapshot().shapes[&request.feature].topology;
    let floor = preview_topology
        .elements
        .iter()
        .find(|element| {
            element.key.kind == TopologyKind::Face
                && element.origins.iter().any(|origin| {
                    origin.source == request.feature && origin.role == "helical_tube.start"
                })
        })
        .unwrap();
    let floor = GeometryIntent::capture(preview_topology, floor.key).unwrap();

    let mut bytes = Vec::new();
    project_file::write(
        &ProjectFile::new(session.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let mut supported = project_file::supported_capabilities();
    assert!(supported.remove(model::HELICAL_TUBE_CUT_CAPABILITY));
    assert!(
        project_file::read(
            &mut std::io::Cursor::new(&bytes),
            Default::default(),
            &supported
        )
        .is_err()
    );
    let opened = project_file::read(
        &mut std::io::Cursor::new(bytes),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    session = Session::new(opened.project.document).unwrap();
    let reopened = compile(session.document(), session.revision());
    verify_cut(
        &reopened,
        stock.body,
        request.feature,
        16.2,
        0.,
        5841.816902505,
    );

    let mut parameter = session.document().get(start_parameter).unwrap().clone();
    let EntityData::Parameter { expression, .. } = &mut parameter.data else {
        panic!()
    };
    *expression = "16.6".into();
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Move helical cut start".into(),
            edits: vec![Mutation::Replace(parameter)],
        })
        .unwrap();
    let mut changed = saved.parameters;
    changed.start_angle = angle(90.);
    let edit = model::prepare_helical_tube_cut_edit(
        &session,
        session.revision(),
        actor(),
        request.feature,
        &changed,
    )
    .unwrap();
    let model = compile(edit.prepared.document(), session.revision() + 1);
    verify_cut(
        &model,
        stock.body,
        request.feature,
        16.6,
        FRAC_PI_2,
        5847.325337067,
    );
    session.commit(edit.prepared).unwrap();
    let topology = &model.snapshot().shapes[&request.feature].topology;
    let Resolution::Resolved { key, .. } =
        references::resolve(&floor, topology, session.revision())
    else {
        panic!("saved floor origin was lost")
    };
    let floor = topology
        .elements
        .iter()
        .find(|element| element.key == key)
        .unwrap();
    assert!((floor.frames[0].frame.origin_mm[2] - 16.6).abs() < 1e-9);

    session.undo(session.revision(), actor()).unwrap();
    let undone = compile(session.document(), session.revision());
    verify_cut(
        &undone,
        stock.body,
        request.feature,
        16.6,
        0.,
        5847.325337067,
    );
    session.redo(session.revision(), actor()).unwrap();
    let redone = compile(session.document(), session.revision());
    verify_cut(
        &redone,
        stock.body,
        request.feature,
        16.6,
        FRAC_PI_2,
        5847.325337067,
    );
}

#[test]
fn helical_cut_rejects_bad_source_or_unproven_geometry_before_publication() {
    for (stock, too_short, diagnostic) in [
        (cylinder(), true, "exit its top cap"),
        (
            PrimitiveParameters::Box {
                size: [length(16.), length(16.), length(30.)],
                frame: Default::default(),
            },
            false,
            "requires an exact axial cylinder",
        ),
    ] {
        let (session, input, _) = fixture(stock);
        let before = session.document().clone();
        let revision = session.revision();
        let mut parameters = parameters(&session);
        if too_short {
            parameters.path_height = length(12.);
        }
        let request = HelicalTubeCutRequest::new(input, parameters);
        assert!(
            model::prepare_helical_tube_cut(&session, revision + 1, actor(), &request).is_err()
        );
        let prepared =
            model::prepare_helical_tube_cut(&session, revision, actor(), &request).unwrap();
        let result = Compiler::default().compile_with_backend(
            prepared.prepared.document(),
            revision + 1,
            GeometryBackend::Rust,
            None,
        );
        let failure = result.err().expect("unproved cut must refuse");
        assert_eq!(failure.entity, Some(request.feature));
        assert!(failure.to_string().contains(diagnostic), "{failure}");
        assert_eq!(session.document(), &before);
        assert_eq!(session.revision(), revision);
        assert!(session.document().get(request.feature).is_none());

        let mut invalid = request;
        invalid.parameters.pitch = angle(2.);
        assert!(model::prepare_helical_tube_cut(&session, revision, actor(), &invalid).is_err());
    }
}

#[test]
fn a_circular_sketch_extrusion_is_supported_stock_with_its_authored_wall_origin() {
    use multicad_core::{
        constraint_sketch::{Curve, CurveKind, source::Source},
        sketch::Datum,
    };
    let (sketch, edits) = Source::create(Datum::Xy, "Circular stock profile").unwrap();
    let document = Document::new("Helical groove in extruded stock")
        .unwrap()
        .edited(&edits)
        .unwrap();
    let mut source = Source::read(&document, sketch).unwrap().0;
    let center = source.add_point([0.; 2]);
    let circle = EntityId::new();
    source.sketch.curves.push(Curve {
        id: circle,
        construction: false,
        kind: CurveKind::Circle {
            center,
            radius_mm: 8.,
        },
    });
    let region = multicad_core::sketch_regions::extract(&source).regions[0].intent();
    let document = document
        .edited(&source.replace(&document, sketch).unwrap())
        .unwrap();
    let mut session = Session::new(document).unwrap();
    let extrusion = model::ExtrudeRequest::new(
        sketch,
        vec![region],
        model::ExtrudeParameters {
            extent: model::Extent::OneSided {
                end: model::Termination::Distance {
                    distance: length(30.),
                },
            },
            ..Default::default()
        },
    );
    let stock = &extrusion.outputs[0];
    let prepared =
        model::prepare_extrude(&session, session.revision(), actor(), &extrusion).unwrap();
    session.commit(prepared.prepared).unwrap();
    let request = HelicalTubeCutRequest::new(
        BodyInput {
            body: stock.body,
            producer: stock.feature,
        },
        HelicalTubeCutParameters {
            radius: length(0.6),
            pitch: length(2.),
            path_height: length(15.),
            start_height: length(16.2),
            start_angle: angle(0.),
        },
    );
    let prepared =
        model::prepare_helical_tube_cut(&session, session.revision(), actor(), &request).unwrap();
    let model = compile(prepared.prepared.document(), session.revision() + 1);
    verify_cut(
        &model,
        stock.body,
        request.feature,
        16.2,
        0.,
        5841.816902505,
    );
    let topology = &model.snapshot().shapes[&request.feature].topology;
    assert!(topology.elements.iter().any(|element| {
        element.key.kind == TopologyKind::Face
            && element.origins.iter().any(|origin| origin.source == circle)
    }));
}
