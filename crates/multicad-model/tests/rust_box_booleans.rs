use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId},
    exchange::{ExportReply, ExportResult, SetDefinition, prepare_set},
    manufacturing::PlanarEdge,
    model::{
        self, BodyInput, BooleanRequest, Operation, PrimitiveFrame, PrimitiveParameters,
        PrimitiveRequest, length,
    },
    project_file::{self, ProjectFile},
    references::{
        self, AnalyticGeometry, Attachment, GeometryIntent, Origin, Resolution, TopologyKey,
        TopologyKind, TopologySnapshot,
    },
};
use multicad_kernel::{GeometryPolicy, PlaneFrame, Solid};
use multicad_model::{
    Compiler, GeometryBackend, Model,
    export::{ExportWorker, Exporter},
};
use std::{
    collections::BTreeSet,
    path::Path,
    time::{Duration, Instant},
};

#[allow(dead_code)]
#[path = "support/rust_polygon.rs"]
mod rust_polygon;

type Point = [f64; 3];

fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}

fn stock(
    session: &mut Session,
    name: &str,
    size: Point,
    origin: Point,
    axes: [Point; 2],
) -> BodyInput {
    let [u, v] = axes;
    let request = PrimitiveRequest::new(
        name,
        PrimitiveParameters::Box {
            size: size.map(length),
            frame: PrimitiveFrame {
                origin: origin.map(length),
                u,
                v,
            },
        },
    );
    let prepared =
        model::prepare_primitive(session, session.revision(), actor(), &request).unwrap();
    session.commit(prepared.prepared).unwrap();
    BodyInput {
        body: request.body,
        producer: request.feature,
    }
}

fn box_at(session: &mut Session, name: &str, size: Point, origin: Point) -> BodyInput {
    stock(session, name, size, origin, [[1., 0., 0.], [0., 1., 0.]])
}

fn combine(
    session: &mut Session,
    operation: Operation,
    target: BodyInput,
    tool: BodyInput,
) -> BodyInput {
    let request = BooleanRequest::new(operation, target.clone(), vec![tool]);
    let prepared = model::prepare_boolean(session, session.revision(), actor(), &request).unwrap();
    session.commit(prepared.prepared).unwrap();
    BodyInput {
        body: target.body,
        producer: request.feature,
    }
}

fn compile(session: &Session, backend: GeometryBackend) -> Model {
    Compiler::default()
        .compile_with_backend(session.document(), session.revision(), backend, None)
        .unwrap()
}

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
}

fn check_solid(solid: &Solid, volume: f64, bounds: [Point; 2]) {
    assert_eq!(solid.solid_count().unwrap(), 1);
    near(solid.volume_mm3().unwrap(), volume);
    let actual = solid.bounds().unwrap();
    for (actual, expected) in [actual.min, actual.max]
        .into_iter()
        .flatten()
        .zip(bounds.into_iter().flatten())
    {
        near(actual, expected);
    }
}

fn compare_backends(
    session: &Session,
    feature: EntityId,
    volume: f64,
    bounds: [Point; 2],
) -> Model {
    let native = compile(session, GeometryBackend::OpenCascade);
    let rust = compile(session, GeometryBackend::Rust);
    check_solid(native.solid(feature).unwrap(), volume, bounds);
    check_solid(rust.solid(feature).unwrap(), volume, bounds);
    rust
}

#[test]
fn box_literal_roundoff_keeps_the_authored_size_and_can_cut_other_stock() {
    let mut session = Session::new(Document::new("Generated box dimensions").unwrap()).unwrap();
    let stock = box_at(&mut session, "Stock", [10., 10., 30.], [0.; 3]);
    let tool = box_at(
        &mut session,
        "Magnet slot",
        [5.2, 3.3, 27.900000000000002],
        [1., 1., 1.],
    );
    let result = combine(&mut session, Operation::Cut, stock, tool.clone());
    let rebuilt = compare_backends(
        &session,
        result.producer,
        3000. - 5.2 * 3.3 * 27.9,
        [[0.; 3], [10., 10., 30.]],
    );
    let cutter = rebuilt.solid(tool.producer).unwrap();
    near(cutter.bounds().unwrap().max[2], 28.9);
    assert!(
        serde_json::to_string(session.document().get(tool.producer).unwrap())
            .unwrap()
            .contains("27.900000000000002")
    );
}

fn planar_key(topology: &TopologySnapshot, axis: usize, coordinate: f64) -> TopologyKey {
    let mut faces = topology.elements.iter().filter_map(|element| {
        let Some(AnalyticGeometry::Plane { frame }) = element.geometry else {
            return None;
        };
        (frame.normal()[axis] > 1. - 1e-8 && (frame.origin_mm[axis] - coordinate).abs() < 1e-8)
            .then_some(element.key)
    });
    let face = faces.next().expect("expected a merged planar face");
    assert!(faces.next().is_none(), "expected one face on this plane");
    face
}

fn xy_face(z: f64) -> PlaneFrame {
    PlaneFrame {
        origin: [0., 0., z],
        ..Default::default()
    }
}

fn check_contours(model: &Model, feature: EntityId, frame: PlaneFrame, outer_area: f64) {
    let shape = &model.snapshot().shapes[&feature];
    let axis = frame
        .normal()
        .into_iter()
        .position(|coordinate| coordinate == 1.)
        .unwrap();
    let top = planar_key(&shape.topology, axis, frame.origin[axis]);
    let contours = model
        .solid(feature)
        .unwrap()
        .planar_face_contours(top.ordinal, frame)
        .unwrap();
    let [outer, hole] = contours.loops.as_slice() else {
        panic!("expected one exterior and one rectangular through-hole");
    };
    assert!(outer.outer);
    assert!(!hole.outer);
    near(outer.signed_area(), outer_area);
    near(hole.signed_area(), -4.);
    assert!(
        contours
            .loops
            .iter()
            .flat_map(|wire| &wire.edges)
            .all(|edge| matches!(edge, PlanarEdge::Line { .. }))
    );
    let picked_faces: BTreeSet<_> = shape.mesh.face_indices.iter().copied().collect();
    let exact_faces: BTreeSet<_> = shape
        .topology
        .elements
        .iter()
        .filter_map(|element| {
            (element.key.kind == TopologyKind::Face).then_some(element.key.ordinal)
        })
        .collect();
    assert_eq!(picked_faces, exact_faces);
}

fn reopen(session: &Session) -> Session {
    let mut bytes = Vec::new();
    project_file::write(
        &ProjectFile::new(session.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(&reopened.project.document, session.document());
    Session::new(reopened.project.document).unwrap()
}

fn reply(worker: &ExportWorker) -> ExportReply {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(reply) = worker.poll() {
            return reply;
        }
        assert!(
            Instant::now() < deadline,
            "Rust Export worker did not settle"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn export_reopened(session: &mut Session, feature: EntityId, volume: f64, bounds: [Point; 2]) {
    let set = EntityId::new();
    let prepared = prepare_set(
        session,
        session.revision(),
        actor(),
        set,
        "Rectangular stock",
        &SetDefinition::default(),
    )
    .unwrap();
    session.commit(prepared).unwrap();
    let document = session.document();
    let revision = session.revision();
    let worker =
        ExportWorker::with_geometry_backend(Exporter::default(), GeometryBackend::Rust).unwrap();
    worker.preview(document, revision, set, actor()).unwrap();
    let ExportResult::Preview(result) = reply(&worker).result else {
        panic!("expected a preview reply");
    };
    let preview = result.unwrap();
    assert!(preview.can_run(), "{:?}", preview.recipes);
    let [artifact] = preview.artifacts.as_slice() else {
        panic!("expected only the current body, with both tools consumed");
    };
    assert_eq!(artifact.solid, feature);
    near(artifact.volume_mm3, volume);
    let root = tempfile::tempdir().unwrap();
    worker
        .run(document, revision, &preview.key, root.path(), actor())
        .unwrap();
    let ExportResult::Run(result) = reply(&worker).result else {
        panic!("expected a run reply");
    };
    let run = result.unwrap();
    let [file] = run.files.as_slice() else {
        panic!("expected one exact STEP file");
    };
    let path = Path::new(&run.directory).join(&file.artifact.output.filename);
    let imported = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
    check_solid(&imported, volume, bounds);
}

#[test]
fn rust_box_join_cut_and_intersect_keep_exact_stock_and_reopen_with_verified_step() {
    let mut session = Session::new(Document::new("Box Boolean stock").unwrap()).unwrap();
    let base = box_at(&mut session, "Base", [8., 6., 4.], [0.25, -1.5, 2.]);
    let wing = stock(
        &mut session,
        "Cardinal rotated wing",
        [3., 4., 4.],
        [10.25, -0.5, 2.],
        [[0., 1., 0.], [-1., 0., 0.]],
    );
    let punch = box_at(&mut session, "Punch", [2., 2., 6.], [2.25, 0.5, 1.]);
    let original = compile(&session, GeometryBackend::Rust);
    let original_topology = &original.snapshot().shapes[&base.producer].topology;
    let anchor = Origin {
        source: base.producer,
        role: format!("side/{}/primitive.edge.3", base.producer),
    };
    let left = original_topology
        .elements
        .iter()
        .find(|element| element.key.kind == TopologyKind::Face && element.origins.contains(&anchor))
        .unwrap();
    let mut left_intent = GeometryIntent::capture(original_topology, left.key).unwrap();
    left_intent.attachment = Some(Attachment {
        anchor,
        offset_mm: [0.; 3],
        angle_rad: 0.,
    });
    let Resolution::Resolved {
        frame: Some(original_frame),
        ..
    } = references::resolve(&left_intent, original_topology, session.revision())
    else {
        panic!("expected an anchored stock face");
    };
    let [edge_start, edge_end] = [[0.25, -1.5, 2.], [8.25, -1.5, 2.]];
    let original_edge = original_topology
        .elements
        .iter()
        .find(|element| {
            matches!(element.geometry, Some(AnalyticGeometry::Line { a_mm, b_mm })
                if [a_mm, b_mm] == [edge_start, edge_end] || [b_mm, a_mm] == [edge_start, edge_end])
        })
        .unwrap();
    let edge_intent = GeometryIntent::capture(original_topology, original_edge.key).unwrap();
    let original_corner = original_topology
        .elements
        .iter()
        .find(|element| {
            matches!(element.geometry, Some(AnalyticGeometry::Point { position_mm })
                if position_mm == edge_start)
        })
        .unwrap();
    let corner_intent = GeometryIntent::capture(original_topology, original_corner.key).unwrap();

    let joined = combine(&mut session, Operation::Join, base, wing);
    let stock_bounds = [[0.25, -1.5, 2.], [10.25, 4.5, 6.]];
    compare_backends(&session, joined.producer, 216., stock_bounds);
    let cut = combine(&mut session, Operation::Cut, joined, punch);
    let rust = compare_backends(&session, cut.producer, 200., stock_bounds);
    check_contours(&rust, cut.producer, xy_face(6.), 54.);
    let topology = &rust.snapshot().shapes[&cut.producer].topology;
    assert!(matches!(
        references::resolve(&left_intent, topology, session.revision()),
        Resolution::Resolved { frame: Some(frame), .. } if frame == original_frame
    ));
    assert!(matches!(
        references::resolve(&edge_intent, topology, session.revision()),
        Resolution::Resolved { geometry: Some(AnalyticGeometry::Line { a_mm, b_mm }), .. }
            if [a_mm, b_mm] == [edge_start, edge_end] || [b_mm, a_mm] == [edge_start, edge_end]
    ));
    assert!(matches!(
        references::resolve(&corner_intent, topology, session.revision()),
        Resolution::Resolved { geometry: Some(AnalyticGeometry::Point { position_mm }), .. }
            if position_mm == edge_start
    ));
    let pocket = planar_key(topology, 0, 2.25);
    let pocket_intent = GeometryIntent::capture(topology, pocket).unwrap();
    let pocket_corner = [2.25, 0.5, 6.];
    let pocket_corner_key = topology
        .elements
        .iter()
        .find(|element| {
            element.geometry
                == Some(AnalyticGeometry::Point {
                    position_mm: pocket_corner,
                })
        })
        .unwrap()
        .key;
    let corner_intersection = GeometryIntent::capture(topology, pocket_corner_key).unwrap();

    let mut session = reopen(&session);
    let rebuilt = compare_backends(&session, cut.producer, 200., stock_bounds);
    let rebuilt_topology = &rebuilt.snapshot().shapes[&cut.producer].topology;
    assert!(matches!(
        references::resolve(&pocket_intent, rebuilt_topology, session.revision()),
        Resolution::Resolved { key, .. } if key == planar_key(rebuilt_topology, 0, 2.25)
    ));
    assert!(matches!(
        references::resolve(&corner_intersection, rebuilt_topology, session.revision()),
        Resolution::Resolved { geometry: Some(AnalyticGeometry::Point { position_mm }), .. }
            if position_mm == pocket_corner
    ));
    export_reopened(&mut session, cut.producer, 200., stock_bounds);

    let limiter = box_at(&mut session, "Limiter", [6., 7., 2.], [1.25, -2., 3.]);
    let clipped = combine(&mut session, Operation::Intersect, cut, limiter);
    let clipped_bounds = [[1.25, -1.5, 3.], [7.25, 4.5, 5.]];
    let result = compare_backends(&session, clipped.producer, 64., clipped_bounds);
    check_contours(&result, clipped.producer, xy_face(5.), 36.);
}

#[test]
fn rust_boolean_notches_an_xz_sketch_extrusion_and_keeps_its_rectangular_hole() {
    let (mut session, _, feature, _) = rust_polygon::fixture();
    let stock = BodyInput {
        body: model::read_extrude(session.document(), feature)
            .unwrap()
            .output
            .body,
        producer: feature,
    };
    let bounds = [[0., -8., 0.], [12., -2., 10.]];
    compare_backends(&session, stock.producer, 552., bounds);
    let cutter = box_at(
        &mut session,
        "Open side notch",
        [3., 8., 2.],
        [10., -9., 1.],
    );
    let cut = combine(&mut session, Operation::Cut, stock, cutter);
    let rust = compare_backends(&session, cut.producer, 528., bounds);
    check_contours(
        &rust,
        cut.producer,
        PlaneFrame {
            origin: [0., -2., 0.],
            u: [0., 0., 1.],
            v: [1., 0., 0.],
        },
        92.,
    );
}

#[test]
fn rust_box_cut_keeps_an_enclosed_void_inside_one_solid() {
    let mut session = Session::new(Document::new("Enclosed rectangular void").unwrap()).unwrap();
    let base = box_at(&mut session, "Stock", [6.; 3], [0.; 3]);
    let tool = box_at(&mut session, "Void", [2.; 3], [2.; 3]);
    let cut = combine(&mut session, Operation::Cut, base, tool);
    let rust = compare_backends(&session, cut.producer, 208., [[0.; 3], [6.; 3]]);
    let shape = &rust.snapshot().shapes[&cut.producer];
    let pocket_floor = planar_key(&shape.topology, 2, 2.);
    let contours = rust
        .solid(cut.producer)
        .unwrap()
        .planar_face_contours(
            pocket_floor.ordinal,
            PlaneFrame {
                origin: [0., 0., 2.],
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(contours.loops.len(), 1);
    near(contours.area_mm2(), 4.);
    assert!(shape.mesh.face_indices.contains(&pocket_floor.ordinal));
}

#[test]
fn rust_box_void_exports_one_logical_solid_after_saved_pose_and_dimension_edits() {
    use multicad_core::{
        commands::Request,
        constraint_sketch::Dimension,
        document::{Entity, EntityData, Link, Mutation},
        exchange::{OutputSubject, Scope, ScopeRoot},
        parameters::BoundExpression,
    };
    use multicad_model::export::verify_rust_step_shape;
    for posed in [false, true] {
        let mut document = Document::new("Saved enclosed cavity").unwrap();
        let width = Entity::new(
            "width",
            EntityData::Parameter {
                expression: "15".into(),
                unit: "mm".into(),
                exposed: true,
                inputs: vec![],
            },
        );
        let width_id = width.id;
        document.insert(width).unwrap();
        let mut session = Session::new(document).unwrap();
        let outer = PrimitiveRequest::new(
            "Exterior",
            PrimitiveParameters::Box {
                size: [
                    Dimension::Expression {
                        expression: BoundExpression::bind(
                            session.document(),
                            "=$width",
                            "mm",
                            None,
                        )
                        .unwrap(),
                    },
                    length(10.),
                    length(8.),
                ],
                frame: Default::default(),
            },
        );
        session
            .commit(
                model::prepare_primitive(&session, session.revision(), actor(), &outer)
                    .unwrap()
                    .prepared,
            )
            .unwrap();
        let mut base = BodyInput {
            body: outer.body,
            producer: outer.feature,
        };
        let mut inner = box_at(&mut session, "Enclosed cavity", [5., 4., 3.], [2., 3., 1.]);
        if posed {
            for input in [&mut base, &mut inner] {
                let transform = model::TransformRequest::new(
                    input.clone(),
                    model::NumericTransform {
                        translation: [17., -3., 21.].map(length),
                        pivot: [0.; 3].map(length),
                        axis: [1., 2., 3.],
                        angle: model::angle(37.),
                    },
                );
                session
                    .commit(
                        model::prepare_transform(&session, session.revision(), actor(), &transform)
                            .unwrap()
                            .prepared,
                    )
                    .unwrap();
                input.producer = transform.feature;
            }
        }
        let cut = combine(&mut session, Operation::Cut, base, inner.clone());
        let initial = compile(&session, GeometryBackend::Rust);
        let topology = &initial.snapshot().shapes[&cut.producer].topology;
        let captures: Vec<_> = topology
            .elements
            .iter()
            .filter(|element| element.key.kind == TopologyKind::Face)
            .map(|element| GeometryIntent::capture(topology, element.key).unwrap())
            .collect();
        assert_eq!(captures.len(), 12);
        let datums: Vec<_> = topology
            .elements
            .iter()
            .filter(|element| element.key.kind == TopologyKind::Face)
            .map(|element| {
                let mut intent = GeometryIntent::capture(topology, element.key).unwrap();
                let anchor = element
                    .frames
                    .first()
                    .expect("authored cavity face frame")
                    .origin
                    .clone();
                intent.origins = BTreeSet::from([anchor.clone()]);
                intent.boundary = None;
                intent.attachment = Some(Attachment {
                    anchor,
                    offset_mm: [0.; 3],
                    angle_rad: 0.,
                });
                Mutation::Insert(Entity::new(
                    "Cavity face datum",
                    EntityData::Plane {
                        origin_mm: [0.; 3],
                        normal: [0., 0., 1.],
                        anchor: Some(Link {
                            target: cut.producer,
                            role: "face_attachment".into(),
                            geometry: Some(intent),
                        }),
                    },
                ))
            })
            .collect();
        session
            .apply(Request {
                expected_revision: session.revision(),
                actor: actor(),
                label: "Attach cavity boundary datums".into(),
                edits: datums,
            })
            .unwrap();
        let set = EntityId::new();
        session
            .commit(
                prepare_set(
                    &session,
                    session.revision(),
                    actor(),
                    set,
                    "Cavity STEP",
                    &SetDefinition {
                        scope: Scope {
                            root: ScopeRoot::Explicit {
                                entities: vec![cut.body],
                            },
                            tags: None,
                        },
                        ..Default::default()
                    },
                )
                .unwrap(),
            )
            .unwrap();
        session = reopen(&session);
        for width in [15., 20.] {
            if width == 20. {
                let mut parameter = session.document().get(width_id).unwrap().clone();
                let EntityData::Parameter { expression, .. } = &mut parameter.data else {
                    unreachable!()
                };
                *expression = "20".into();
                session
                    .apply(Request {
                        expected_revision: session.revision(),
                        actor: actor(),
                        label: "Resize cavity exterior".into(),
                        edits: vec![Mutation::Replace(parameter)],
                    })
                    .unwrap();
            }
            let model = compile(&session, GeometryBackend::Rust);
            let shape = &model.snapshot().shapes[&cut.producer];
            let solid = model.solid(cut.body).unwrap();
            let expected = width * 80. - 60.;
            near(solid.volume_mm3().unwrap(), expected);
            assert_eq!(solid.solid_count().unwrap(), 1);
            assert!(model.snapshot().references.all_resolved());
            for capture in &captures {
                assert!(matches!(
                    references::resolve(capture, &shape.topology, session.revision()),
                    Resolution::Resolved { .. }
                ));
            }
            near(
                solid
                    .rust_occupied_overlap_volume(
                        model.solid(inner.producer).unwrap(),
                        Default::default(),
                    )
                    .unwrap(),
                0.,
            );
            let before = session.document().clone();
            let revision = session.revision();
            let worker =
                ExportWorker::with_geometry_backend(Exporter::default(), GeometryBackend::Rust)
                    .unwrap();
            worker
                .preview(session.document(), revision, set, actor())
                .unwrap();
            let ExportResult::Preview(preview) = reply(&worker).result else {
                panic!("expected cavity preview")
            };
            let preview = preview.unwrap();
            assert!(preview.can_run(), "{preview:?}");
            let [artifact] = preview.artifacts.as_slice() else {
                panic!("one logical cavity Part")
            };
            assert_eq!(artifact.solid_count, 1);
            let temporary = tempfile::tempdir().unwrap();
            worker
                .run(
                    session.document(),
                    revision,
                    &preview.key,
                    temporary.path(),
                    actor(),
                )
                .unwrap();
            let ExportResult::Run(run) = reply(&worker).result else {
                panic!("expected cavity publication")
            };
            let run = run.unwrap();
            let [file] = run.files.as_slice() else {
                panic!("one cavity STEP artifact")
            };
            assert_eq!(file.artifact.solid_count, 1);
            assert_eq!(file.artifact.producer, cut.producer);
            let OutputSubject::Part(subject) = &file.artifact.output.subject else {
                panic!("expected cavity Part attribution")
            };
            assert_eq!(subject.part, cut.body);
            let path = Path::new(&run.directory).join(&file.artifact.output.filename);
            let source = std::fs::read_to_string(&path).unwrap();
            assert_eq!(source.matches("BREP_WITH_VOIDS(").count(), 1);
            assert_eq!(source.matches("CLOSED_SHELL(").count(), 3);
            near(
                verify_rust_step_shape(&path, solid, shape, 1e-7).unwrap(),
                expected,
            );
            let imported =
                Solid::import_rust_step(&source, EntityId::new(), Default::default()).unwrap();
            near(imported.volume_mm3().unwrap(), expected);
            assert_eq!(imported.solid_count().unwrap(), 1);
            near(
                imported
                    .rust_occupied_overlap_volume(
                        model.solid(inner.producer).unwrap(),
                        Default::default(),
                    )
                    .unwrap(),
                0.,
            );
            imported
                .manufacturing_mesh_with_receipt(Default::default())
                .unwrap();
            if multicad_kernel::OCCT_LINKED {
                let native = Solid::read_step(&path, Default::default()).unwrap();
                near(native.volume_mm3().unwrap(), expected);
                assert_eq!(native.solid_count().unwrap(), 1);
                assert!(
                    native
                        .boolean(
                            &Solid::read_step(
                                &{
                                    let probe = temporary.path().join("probe.step");
                                    model
                                        .solid(inner.producer)
                                        .unwrap()
                                        .write_step(&probe)
                                        .unwrap();
                                    probe
                                },
                                Default::default()
                            )
                            .unwrap(),
                            multicad_kernel::BooleanOperation::Intersect,
                            Default::default()
                        )
                        .unwrap()
                        .is_none()
                );
            }
            assert_eq!(session.document(), &before);
            assert_eq!(session.revision(), revision);
        }
        session.undo(session.revision(), actor()).unwrap();
        near(
            compile(&session, GeometryBackend::Rust)
                .solid(cut.body)
                .unwrap()
                .volume_mm3()
                .unwrap(),
            1140.,
        );
        session.redo(session.revision(), actor()).unwrap();
        near(
            compile(&session, GeometryBackend::Rust)
                .solid(cut.body)
                .unwrap()
                .volume_mm3()
                .unwrap(),
            1540.,
        );
    }
}

#[test]
fn rust_box_boolean_refuses_contact_only_intersection_and_disconnected_join_before_commit() {
    for (operation, origin, message) in [
        (Operation::Intersect, [2., 0., 0.], "removed all material"),
        (Operation::Join, [3., 0., 0.], "disconnected fragments"),
        (Operation::Cut, [0.; 3], "removed all material"),
    ] {
        let mut session = Session::new(Document::new("Boolean candidate").unwrap()).unwrap();
        let base = box_at(&mut session, "Stock", [2.; 3], [0.; 3]);
        let tool = box_at(&mut session, "Tool", [2.; 3], origin);
        let before = session.document().clone();
        let revision = session.revision();
        let request = BooleanRequest::new(operation, base.clone(), vec![tool.clone()]);
        let prepared = model::prepare_boolean(&session, revision, actor(), &request).unwrap();
        let failure = Compiler::default()
            .compile_with_backend(
                prepared.prepared.document(),
                revision + 1,
                GeometryBackend::Rust,
                None,
            )
            .err()
            .expect("Boolean candidate must fail");
        assert_eq!(failure.entity, Some(request.feature));
        assert!(failure.message.contains(message), "{failure}");
        assert_eq!(session.revision(), revision);
        assert_eq!(session.document(), &before);
        let unchanged = compile(&session, GeometryBackend::Rust);
        assert_eq!(unchanged.snapshot().bodies[&base.body], base.producer);
        assert_eq!(unchanged.snapshot().bodies[&tool.body], tool.producer);
    }
}
