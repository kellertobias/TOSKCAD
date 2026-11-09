use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::{Curve, CurveKind, Dimension, source::Source},
    document::{Document, Entity, EntityData, EntityId, Mutation},
    model::{
        self, BodyInput, BooleanRequest, Extent, ExtrudeParameters, ExtrudeRequest, Operation,
        PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, Termination, length,
    },
    parameters::BoundExpression,
    project_file::{self, FileLimits, ProjectFile},
    references::{Origin, TopologyKind},
    sketch::{self, CreationPlane, Datum, PlaneOptions},
    sketch_regions,
};
use multicad_kernel::rust_kernel::{Rational, parse_length_mm, prism::Location};
use multicad_model::{Compiler, GeometryBackend, Model};
use std::{f64::consts::PI, io::Cursor};

fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-7, "{actual} != {expected}");
}

fn compiler() -> Compiler {
    Compiler {
        omit_display: true,
        ..Compiler::default()
    }
}

fn compile(session: &Session, backend: GeometryBackend) -> Model {
    compiler()
        .compile_with_backend(session.document(), session.revision(), backend, None)
        .unwrap()
}

fn reopen(session: &Session) -> Session {
    let project = ProjectFile::new(session.document().clone()).unwrap();
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, FileLimits::default()).unwrap();
    let result = project_file::read(
        &mut Cursor::new(bytes),
        FileLimits::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(&result.project.document, session.document());
    Session::new(result.project.document).unwrap()
}

fn fixture(
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

#[test]
fn saved_rounded_panel_rebuilds_and_moves_a_downstream_face_attachment() {
    for quarter_turn in [false, true] {
        let (mut session, height, extrusion, body, cutter, cut) = fixture(quarter_turn, 28.);
        let first = compile(&session, GeometryBackend::Rust);
        let native = compile(&session, GeometryBackend::OpenCascade);
        let exact = first.solid(cut).unwrap().rounded_prism().unwrap();
        assert_eq!(exact.dimensions()[2].ratio(), (8, 1));
        let rust_cutter = first.solid(cutter).unwrap();
        let native_cutter = native.solid(cutter).unwrap();
        assert!(rust_cutter.axial_cylinder().is_some());
        near(
            rust_cutter.volume_mm3().unwrap(),
            native_cutter.volume_mm3().unwrap(),
        );
        let mut rust_faces = rust_cutter.faces().unwrap();
        let mut native_faces = native_cutter.faces().unwrap();
        let by_height = |a: &multicad_kernel::Face, b: &multicad_kernel::Face| {
            a.center_mm[2].total_cmp(&b.center_mm[2])
        };
        rust_faces.sort_by(by_height);
        native_faces.sort_by(by_height);
        assert_eq!(rust_faces.len(), native_faces.len());
        for (actual, expected) in rust_faces.into_iter().zip(native_faces) {
            near(actual.area_mm2, expected.area_mm2);
            for (a, b) in actual.center_mm.into_iter().zip(expected.center_mm) {
                near(a, b);
            }
        }
        let cylinder_cap = |model: &Model| {
            model.snapshot().shapes[&cutter]
                .topology
                .elements
                .iter()
                .flat_map(|element| &element.frames)
                .find(|anchor| anchor.origin.source == cutter && anchor.origin.role == "cap.end")
                .unwrap()
                .frame
        };
        assert_eq!(cylinder_cap(&first), cylinder_cap(&native));
        assert_eq!(first.snapshot().bodies.get(&body), Some(&cut));
        assert_eq!(first.snapshot().bodies.len(), 1);
        near(
            first.solid(cut).unwrap().volume_mm3().unwrap(),
            native.solid(cut).unwrap().volume_mm3().unwrap(),
        );
        let topology = &first.snapshot().shapes[&cut].topology;
        let cap = Origin {
            source: extrusion,
            role: "cap.end".into(),
        };
        let key = topology
            .elements
            .iter()
            .find(|element| {
                element.key.kind == TopologyKind::Face && element.origins.contains(&cap)
            })
            .unwrap()
            .key;
        let attachment = sketch::prepare_new(
            &session,
            session.revision(),
            actor(),
            "Cap sketch",
            CreationPlane::Face {
                snapshot: topology,
                key,
                anchor: Some(&cap),
            },
            PlaneOptions::default(),
        )
        .unwrap();
        near(attachment.frame.origin[2], 8.);
        let plane = attachment.plane;
        session.commit(attachment.prepared).unwrap();
        session = reopen(&session);
        let mut edit = session.document().get(height).unwrap().clone();
        let EntityData::Parameter { expression, .. } = &mut edit.data else {
            unreachable!()
        };
        *expression = "=10 mm / 3".into();
        session
            .apply(Request {
                expected_revision: session.revision(),
                actor: actor(),
                label: "Fractional thickness".into(),
                edits: vec![Mutation::Replace(edit)],
            })
            .unwrap();
        session = reopen(&session);
        let second = compile(&session, GeometryBackend::Rust);
        let native = compile(&session, GeometryBackend::OpenCascade);
        let solid = second.solid(cut).unwrap();
        let exact = solid.rounded_prism().unwrap();
        assert_eq!(exact.dimensions()[2], parse_length_mm("10/3 mm").unwrap());
        near(solid.volume_mm3().unwrap(), (800. + 25. * PI) * 10. / 3.);
        near(
            solid.volume_mm3().unwrap(),
            native.solid(cut).unwrap().volume_mm3().unwrap(),
        );
        let probe = [28, 10, 2].map(|value| Rational::new(value, 1).unwrap());
        assert_eq!(exact.classify(probe), Ok(Location::Exterior));
        let attached = sketch::inspect_plane(
            session.document(),
            plane,
            session.revision(),
            Some(&second.snapshot().references),
        )
        .unwrap();
        near(attached.origin[2], 10. / 3.);
    }
}

#[test]
fn a_tangent_bore_is_rejected_without_native_fallback() {
    let (session, _, _, _, _, cut) = fixture(false, 19.);
    let result = compiler()
        .compile_with_backend(
            session.document(),
            session.revision(),
            GeometryBackend::Rust,
            None,
        )
        .err()
        .unwrap();
    assert_eq!(result.entity, Some(cut));
    assert!(result.message.contains("HoleContact"), "{}", result.message);
    let cancel = || true;
    let cancelled = compiler()
        .compile_with_backend(
            session.document(),
            session.revision(),
            GeometryBackend::Rust,
            Some(&cancel),
        )
        .err()
        .unwrap();
    assert!(multicad_model::is_superseded(&cancelled));
}

#[test]
fn unsupported_documents_and_display_requests_return_explicit_errors() {
    let (session, _, _, _, _, _) = fixture(false, 28.);
    let display = Compiler::default()
        .compile_with_backend(
            session.document(),
            session.revision(),
            GeometryBackend::Rust,
            None,
        )
        .err()
        .unwrap();
    assert!(display.message.contains("display tessellation"));
    let mut session = Session::new(Document::new("Unsupported Rust primitive").unwrap()).unwrap();
    let stock = PrimitiveRequest::new(
        "Box",
        PrimitiveParameters::Box {
            size: [length(1.), length(2.), length(3.)],
            frame: PrimitiveFrame::default(),
        },
    );
    let prepared = model::prepare_primitive(&session, session.revision(), actor(), &stock).unwrap();
    session.commit(prepared.prepared).unwrap();
    let native = compile(&session, GeometryBackend::OpenCascade);
    near(
        native.solid(stock.feature).unwrap().volume_mm3().unwrap(),
        6.,
    );
    let rust = compiler()
        .compile_with_backend(
            session.document(),
            session.revision(),
            GeometryBackend::Rust,
            None,
        )
        .err()
        .unwrap();
    assert_eq!(rust.entity, Some(stock.feature));
    assert!(rust.message.contains("only axial cylinder"));
}
