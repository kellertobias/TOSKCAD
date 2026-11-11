use super::common::{actor, apply_parameter, bound};
use multicad_core::{
    commands::{Request, Session},
    constraint_sketch::{Curve, CurveKind, source::Source},
    document::{Document, Entity, EntityData, EntityId, Link, Mutation},
    model::{
        self, AdvancedParameters, AdvancedRequest, BodyInput, Direction, Extent, ExtrudeParameters,
        ExtrudeRequest, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, ShellParameters,
        Termination, length,
    },
    project_file::{self, ProjectFile},
    references::{
        self, Attachment, GeometryIntent, Origin, ReferenceAddress, Resolution, TopologyKey,
        TopologyKind, TopologySnapshot,
    },
    sketch::Datum,
};
use multicad_kernel::{
    Bounds, GeometryErrorKind, GeometryPolicy, MeshOptions, PlaneFrame, RigidTransform, Solid,
    TreatmentFace,
    rust_kernel::{Circle, Rational, cylinder::AxialCylinder},
};
use multicad_model::{Compiler, GeometryBackend, Model};
use std::f64::consts::PI;

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 3e-5, "{actual} != {expected}");
}

fn compile(session: &Session, backend: GeometryBackend) -> Model {
    Compiler::default()
        .compile_with_backend(session.document(), session.revision(), backend, None)
        .unwrap()
}

fn area(solid: &Solid) -> f64 {
    solid
        .faces()
        .unwrap()
        .iter()
        .map(|face| face.area_mm2)
        .sum()
}

fn assert_bounds(actual: Bounds, expected: Bounds) {
    for (actual, expected) in actual
        .min
        .into_iter()
        .chain(actual.max)
        .zip(expected.min.into_iter().chain(expected.max))
    {
        near(actual, expected);
    }
}

fn expected(
    radius: f64,
    height: f64,
    thickness: f64,
    open: [bool; 2],
    frame: PlaneFrame,
) -> (f64, f64, Bounds) {
    let shifted = [
        if open[0] { 0. } else { thickness },
        if open[1] { height } else { height - thickness },
    ];
    let (outer, inner, a, b) = if thickness > 0. {
        (radius, radius - thickness, [0., height], shifted)
    } else {
        (radius - thickness, radius, shifted, [0., height])
    };
    let (outer_height, inner_height) = (a[1] - a[0], b[1] - b[0]);
    let volume = PI * (outer * outer * outer_height - inner * inner * inner_height);
    let openings = open.into_iter().filter(|opened| *opened).count() as f64;
    let area = 2. * PI * (outer * outer_height + inner * inner_height)
        + PI * (openings * (outer * outer - inner * inner)
            + (2. - openings) * (outer * outer + inner * inner));
    let normal = frame.normal();
    let radial = |i: usize| outer * frame.u[i].hypot(frame.v[i]);
    let bounds = Bounds {
        min: std::array::from_fn(|i| {
            frame.origin[i] + (normal[i] * a[0]).min(normal[i] * a[1]) - radial(i)
        }),
        max: std::array::from_fn(|i| {
            frame.origin[i] + (normal[i] * a[0]).max(normal[i] * a[1]) + radial(i)
        }),
    };
    (volume, area, bounds)
}

fn selections(
    solid: &Solid,
    frame: PlaneFrame,
    open: [bool; 2],
    feature: EntityId,
) -> Vec<TreatmentFace> {
    let faces = solid.faces().unwrap();
    open.into_iter()
        .enumerate()
        .filter(|(_, opened)| *opened)
        .map(|(end, _)| {
            let direction = if end == 0 { -1. } else { 1. };
            let index = faces
                .iter()
                .find(|face| {
                    face.plane.is_some_and(|(_, n)| {
                        n.into_iter()
                            .zip(frame.normal())
                            .map(|(a, b)| a * b)
                            .sum::<f64>()
                            * direction
                            > 1. - 1e-10
                    })
                })
                .unwrap()
                .index;
            TreatmentFace {
                key: TopologyKey {
                    kind: TopologyKind::Face,
                    ordinal: index,
                },
                origin: Origin {
                    source: feature,
                    role: format!("opening.{end}"),
                },
            }
        })
        .collect()
}

#[test]
fn rust_cylindrical_shell_signed_cap_openings_keep_analytic_material_mesh_and_step() {
    let policy = GeometryPolicy::default();
    let number = |n| Rational::new(n, 1).unwrap();
    for frame in [
        PlaneFrame::default(),
        PlaneFrame {
            origin: [11., -7., 23.],
            u: [0.8, 0., -0.6],
            v: [0., 1., 0.],
        },
    ] {
        let source = Solid::from_axial_cylinder(
            AxialCylinder::new(
                EntityId::new().uuid_bits(),
                Circle::new([number(0); 2], number(10)).unwrap(),
                number(0),
                number(20),
            )
            .unwrap(),
            PlaneFrame::default(),
            policy,
        )
        .unwrap()
        .transformed(RigidTransform {
            rotation: std::array::from_fn(|i| [frame.u[i], frame.v[i], frame.normal()[i]]),
            translation_mm: frame.origin,
        })
        .unwrap();
        for open in [[true, false], [false, true], [true, true]] {
            for thickness in [2., -2.] {
                let feature = EntityId::new();
                let result = source
                    .shell_named(
                        &selections(&source, frame, open, feature),
                        thickness,
                        feature,
                        policy,
                    )
                    .unwrap();
                let (volume, surface, bounds) = expected(10., 20., thickness, open, frame);
                assert_eq!(result.solid_count().unwrap(), 1);
                near(result.volume_mm3().unwrap(), volume);
                near(area(&result), surface);
                assert_bounds(result.bounds().unwrap(), bounds);
                result
                    .topology_snapshot(feature, 3)
                    .unwrap()
                    .validate()
                    .unwrap();
                let receipt = result
                    .manufacturing_mesh_with_receipt(MeshOptions {
                        linear_deflection_mm: 0.01,
                        ..Default::default()
                    })
                    .unwrap();
                assert!(receipt.edge_pairs > 0 && receipt.vertex_links > 0);
                let inner = if thickness > 0. { 8. } else { 10. };
                let outer = if thickness > 0. { 10. } else { 12. };
                let world = |[x, y, z]: [f64; 3]| {
                    std::array::from_fn(|i| {
                        frame.origin[i] + x * frame.u[i] + y * frame.v[i] + z * frame.normal()[i]
                    })
                };
                assert!(
                    result
                        .rust_revolution_contains(world([(inner + outer) / 2., 0., 10.]))
                        .unwrap()
                );
                assert!(
                    !result
                        .rust_revolution_contains(world([0., 0., 10.]))
                        .unwrap()
                );
                assert!(
                    !result
                        .rust_revolution_contains(world([outer + 1., 0., 10.]))
                        .unwrap()
                );
                for (end, z) in [(0, thickness / 2.), (1, 20. - thickness / 2.)] {
                    if !open[end] {
                        assert!(result.rust_revolution_contains(world([0., 0., z])).unwrap());
                    }
                }
                let folder = tempfile::tempdir().unwrap();
                let path = folder.path().join("cylindrical-shell.step");
                result.write_step(&path).unwrap();
                let reopened = Solid::import_rust_step(
                    &std::fs::read_to_string(&path).unwrap(),
                    EntityId::new(),
                    policy,
                )
                .unwrap();
                near(reopened.volume_mm3().unwrap(), volume);
                near(area(&reopened), surface);
                assert_bounds(reopened.bounds().unwrap(), bounds);
                if cfg!(feature = "occt") {
                    let native_source = Solid::cylinder(frame, 10., 20., policy).unwrap();
                    let native = native_source
                        .shell_named(
                            &selections(&native_source, frame, open, feature),
                            thickness,
                            feature,
                            policy,
                        )
                        .unwrap();
                    near(native.volume_mm3().unwrap(), volume);
                    near(area(&native), surface);
                    assert_bounds(native.bounds().unwrap(), bounds);
                    let read = Solid::read_step(&path, policy).unwrap();
                    near(read.volume_mm3().unwrap(), volume);
                    near(area(&read), surface);
                    assert_eq!(Solid::step_solids(&path).unwrap().len(), 1);
                    assert_bounds(read.bounds().unwrap(), bounds);
                }
            }
        }
    }
}

fn primitive_fixture() -> (Session, PrimitiveRequest) {
    let mut document = Document::new("Editable cylindrical Shell").unwrap();
    for (name, expression) in [("radius", "10"), ("height", "20"), ("wall", "2")] {
        document
            .insert(Entity::new(
                name,
                EntityData::Parameter {
                    expression: expression.into(),
                    unit: "mm".into(),
                    exposed: true,
                    inputs: vec![],
                },
            ))
            .unwrap();
    }
    let mut session = Session::new(document).unwrap();
    let source = PrimitiveRequest::new(
        "Cylinder",
        PrimitiveParameters::Cylinder {
            radius: bound(&session, "radius", "mm"),
            height: bound(&session, "height", "mm"),
            frame: PrimitiveFrame {
                origin: [11., -7., 23.].map(length),
                u: [0.8, 0., -0.6],
                v: [0., 1., 0.],
            },
        },
    );
    let prepared =
        model::prepare_primitive(&session, session.revision(), actor(), &source).unwrap();
    session.commit(prepared.prepared).unwrap();
    (session, source)
}

fn origin(feature: EntityId, role: &str) -> Origin {
    Origin {
        source: feature,
        role: role.into(),
    }
}

fn capture(snapshot: &TopologySnapshot, source: &Origin, kind: TopologyKind) -> GeometryIntent {
    let element = snapshot
        .elements
        .iter()
        .find(|element| element.key.kind == kind && element.origins.contains(source))
        .unwrap();
    GeometryIntent::capture(snapshot, element.key).unwrap()
}

fn anchor(snapshot: &TopologySnapshot, source: &Origin) -> GeometryIntent {
    let mut intent = capture(snapshot, source, TopologyKind::Face);
    intent.attachment = Some(Attachment {
        anchor: source.clone(),
        offset_mm: [0.; 3],
        angle_rad: 0.,
    });
    intent
}

fn datum(feature: EntityId, name: &str, mut intent: GeometryIntent) -> (Entity, ReferenceAddress) {
    let selected = intent.attachment.as_ref().unwrap().anchor.clone();
    intent.origins = std::collections::BTreeSet::from([selected]);
    intent.boundary = None;
    let reference = intent.id;
    let entity = Entity::new(
        name,
        EntityData::Plane {
            origin_mm: [0.; 3],
            normal: [0., 0., 1.],
            anchor: Some(Link {
                target: feature,
                role: "face_attachment".into(),
                geometry: Some(intent),
            }),
        },
    );
    let address = ReferenceAddress {
        owner: entity.id,
        reference,
    };
    (entity, address)
}

fn reopened(session: &Session) -> Session {
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(session.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    Session::new(
        project_file::read(
            &mut bytes.as_slice(),
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap()
        .project
        .document,
    )
    .unwrap()
}

#[test]
fn rust_cylindrical_shell_saved_references_follow_dimensions_signed_edits_and_history() {
    let (mut session, primitive) = primitive_fixture();
    let initial = compile(&session, GeometryBackend::Rust);
    let snapshot = &initial.snapshot().shapes[&primitive.feature].topology;
    let retained_side = capture(
        snapshot,
        &origin(primitive.feature, "side.cylinder"),
        TopologyKind::Face,
    );
    let retained_cap = capture(
        snapshot,
        &origin(primitive.feature, "cap.start"),
        TopologyKind::Face,
    );
    let retained_rim = capture(
        snapshot,
        &origin(primitive.feature, "cap.end"),
        TopologyKind::Edge,
    );
    let request = AdvancedRequest::on_body(
        BodyInput {
            body: primitive.body,
            producer: primitive.feature,
        },
        AdvancedParameters::Shell(ShellParameters {
            thickness: bound(&session, "wall", "mm"),
        }),
        vec![capture(
            snapshot,
            &origin(primitive.feature, "cap.end"),
            TopologyKind::Face,
        )],
    );
    let opening = origin(request.feature, &format!("face/{}", request.faces[0].id));
    let before = session.document().clone();
    let prepared =
        model::prepare_advanced(&session, session.revision(), actor(), &request).unwrap();
    Compiler::default()
        .compile_with_backend(
            prepared.prepared.document(),
            session.revision() + 1,
            GeometryBackend::Rust,
            None,
        )
        .unwrap();
    assert_eq!(session.document(), &before);
    session.commit(prepared.prepared).unwrap();
    let initial = compile(&session, GeometryBackend::Rust);
    let topology = &initial.snapshot().shapes[&request.feature].topology;
    let offset = origin(
        request.feature,
        &format!("offset/{}/cap.start", primitive.feature),
    );
    let opening_intent = anchor(topology, &opening);
    let offset_intent = anchor(topology, &offset);
    let (opening_datum, opening_address) =
        datum(request.feature, "Opening datum", opening_intent.clone());
    let (offset_datum, offset_address) =
        datum(request.feature, "Offset datum", offset_intent.clone());
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Attach cylindrical Shell datums".into(),
            edits: vec![
                Mutation::Insert(opening_datum),
                Mutation::Insert(offset_datum),
            ],
        })
        .unwrap();

    let frame = PlaneFrame {
        origin: [11., -7., 23.],
        u: [0.8, 0., -0.6],
        v: [0., 1., 0.],
    };
    for (radius, height, thickness) in [(10., 20., 2.), (12., 24., 2.), (12., 24., -2.)] {
        apply_parameter(&mut session, "radius", &radius.to_string());
        apply_parameter(&mut session, "height", &height.to_string());
        apply_parameter(&mut session, "wall", &thickness.to_string());
        let saved = reopened(&session);
        for backend in [GeometryBackend::Rust, GeometryBackend::OpenCascade] {
            if backend == GeometryBackend::OpenCascade && !cfg!(feature = "occt") {
                continue;
            }
            let model = compile(&saved, backend);
            let solid = model.solid(primitive.body).unwrap();
            let (volume, surface, bounds) =
                expected(radius, height, thickness, [false, true], frame);
            near(solid.volume_mm3().unwrap(), volume);
            near(area(solid), surface);
            assert_bounds(solid.bounds().unwrap(), bounds);
            assert!(model.snapshot().references.all_resolved());
            for (address, level, outward) in [
                (opening_address, height, 1.),
                (offset_address, thickness, thickness.signum()),
            ] {
                let Resolution::Resolved {
                    frame: Some(actual),
                    ..
                } = model.snapshot().references.get(address).unwrap().resolution
                else {
                    panic!("missing cylindrical Shell datum");
                };
                let delta: [f64; 3] =
                    std::array::from_fn(|i| actual.origin_mm[i] - frame.origin[i]);
                near(
                    delta
                        .into_iter()
                        .zip(frame.normal())
                        .map(|(a, b)| a * b)
                        .sum(),
                    level,
                );
                near(
                    actual
                        .normal()
                        .into_iter()
                        .zip(frame.normal())
                        .map(|(a, b)| a * b)
                        .sum(),
                    outward,
                );
            }
            if backend == GeometryBackend::Rust {
                let topology = &model.snapshot().shapes[&request.feature].topology;
                for intent in [
                    &retained_side,
                    &retained_cap,
                    &retained_rim,
                    &opening_intent,
                    &offset_intent,
                ] {
                    let resolved = references::resolve(intent, topology, saved.revision());
                    assert!(
                        matches!(resolved, Resolution::Resolved { .. }),
                        "{intent:?}: {resolved:?}"
                    );
                }
                let directory = tempfile::tempdir().unwrap();
                let path = directory.path().join("saved-cylinder-shell.step");
                solid.write_step(&path).unwrap();
                near(
                    multicad_model::export::verify_rust_step_shape(
                        &path,
                        solid,
                        &model.snapshot().shapes[&request.feature],
                        1e-7,
                    )
                    .unwrap(),
                    volume,
                );
            }
        }
    }
    let negative = compile(&session, GeometryBackend::Rust)
        .solid(primitive.body)
        .unwrap()
        .volume_mm3()
        .unwrap();
    session.undo(session.revision(), actor()).unwrap();
    let positive = compile(&session, GeometryBackend::Rust)
        .solid(primitive.body)
        .unwrap()
        .volume_mm3()
        .unwrap();
    assert!((negative - positive).abs() > 1.);
    session.redo(session.revision(), actor()).unwrap();
    near(
        compile(&session, GeometryBackend::Rust)
            .solid(primitive.body)
            .unwrap()
            .volume_mm3()
            .unwrap(),
        negative,
    );

    let revision = session.revision();
    let before = session.document().clone();
    let mut edit = AdvancedRequest::existing(session.document(), request.feature).unwrap();
    let AdvancedParameters::Shell(parameters) = &mut edit.parameters else {
        unreachable!();
    };
    parameters.thickness = length(1000.);
    let candidate = model::prepare_advanced_edit(&session, revision, actor(), &edit).unwrap();
    let failure = Compiler::default()
        .compile_with_backend(
            candidate.prepared.document(),
            revision + 1,
            GeometryBackend::Rust,
            None,
        )
        .err()
        .unwrap();
    assert_eq!(failure.entity, Some(request.feature));
    assert!(failure.message.contains("leaves no cavity"));
    assert_eq!(session.document(), &before);
    assert_eq!(session.revision(), revision);
}

#[test]
fn rust_cylindrical_shell_reversed_offset_circle_extrusions_preserve_authored_caps() {
    let (sketch, edits) = Source::create(Datum::Xz, "Reverse circular section").unwrap();
    let document = Document::new("Reverse cylindrical Shell")
        .unwrap()
        .edited(&edits)
        .unwrap();
    let mut source = Source::read(&document, sketch).unwrap().0;
    let center = source.add_point([7., 9.]);
    let circle = EntityId::new();
    source.sketch.curves.push(Curve {
        id: circle,
        construction: false,
        kind: CurveKind::Circle {
            center,
            radius_mm: 10.,
        },
    });
    let region = multicad_core::sketch_regions::extract(&source)
        .regions
        .into_iter()
        .next()
        .unwrap()
        .intent();
    let mut session = Session::new(
        document
            .edited(&source.replace(&document, sketch).unwrap())
            .unwrap(),
    )
    .unwrap();
    let extrusion = ExtrudeRequest::new(
        sketch,
        vec![region],
        ExtrudeParameters {
            direction: Direction {
                reverse: true,
                ..Default::default()
            },
            start_offset: length(2.),
            extent: Extent::OneSided {
                end: Termination::Distance {
                    distance: length(20.),
                },
            },
            ..Default::default()
        },
    );
    let prepared =
        model::prepare_extrude(&session, session.revision(), actor(), &extrusion).unwrap();
    session.commit(prepared.prepared).unwrap();
    let output = extrusion.outputs[0].clone();
    let original = compile(&session, GeometryBackend::Rust);
    let topology = &original.snapshot().shapes[&output.feature].topology;
    let cap_origin = origin(output.feature, "cap.end");
    let authored = topology
        .elements
        .iter()
        .flat_map(|element| &element.frames)
        .find(|anchor| anchor.origin == cap_origin)
        .unwrap()
        .frame;
    let cap = capture(
        topology,
        &origin(output.feature, "cap.end"),
        TopologyKind::Face,
    );
    let retained = topology
        .elements
        .iter()
        .filter(|element| element.key.kind == TopologyKind::Face)
        .map(|element| GeometryIntent::capture(topology, element.key).unwrap())
        .collect::<Vec<_>>();
    let request = AdvancedRequest::on_body(
        BodyInput {
            body: output.body,
            producer: output.feature,
        },
        AdvancedParameters::Shell(ShellParameters {
            thickness: length(2.),
        }),
        vec![cap],
    );
    let prepared =
        model::prepare_advanced(&session, session.revision(), actor(), &request).unwrap();
    session.commit(prepared.prepared).unwrap();
    let session = reopened(&session);
    let rust = compile(&session, GeometryBackend::Rust);
    let solid = rust.solid(output.body).unwrap();
    near(solid.volume_mm3().unwrap(), 848. * PI);
    let topology = &rust.snapshot().shapes[&request.feature].topology;
    let Resolution::Resolved {
        frame: Some(attached),
        ..
    } = references::resolve(&anchor(topology, &cap_origin), topology, session.revision())
    else {
        panic!("missing reversed circular sketch cap anchor");
    };
    for (actual, expected) in attached.origin_mm.into_iter().zip(authored.origin_mm) {
        near(actual, expected);
    }
    for (actual, expected) in attached.u.into_iter().zip(authored.u) {
        near(actual, expected);
    }
    for intent in &retained {
        assert!(matches!(
            references::resolve(intent, topology, session.revision()),
            Resolution::Resolved { .. }
        ));
    }
    assert!(
        topology
            .elements
            .iter()
            .any(|element| element.key.kind == TopologyKind::Face
                && element.origins.iter().any(|origin| origin.source == circle))
    );
    if cfg!(feature = "occt") {
        let native = compile(&session, GeometryBackend::OpenCascade);
        let expected = native.solid(output.body).unwrap();
        near(expected.volume_mm3().unwrap(), solid.volume_mm3().unwrap());
        near(area(expected), area(solid));
        assert_bounds(expected.bounds().unwrap(), solid.bounds().unwrap());
    }
}

#[test]
fn rust_cylindrical_shell_rejects_collapse_invalid_selections_and_policy_changes() {
    let policy = GeometryPolicy::default();
    let number = |n| Rational::new(n, 1).unwrap();
    let source = Solid::from_axial_cylinder(
        AxialCylinder::new(
            EntityId::new().uuid_bits(),
            Circle::new([number(0); 2], number(20)).unwrap(),
            number(0),
            number(4),
        )
        .unwrap(),
        PlaneFrame::default(),
        policy,
    )
    .unwrap();
    let feature = EntityId::new();
    let top = selections(&source, PlaneFrame::default(), [false, true], feature);
    for thickness in [4., 20., 21.] {
        let error = source
            .shell_named(&top, thickness, feature, policy)
            .err()
            .unwrap();
        assert_eq!(error.kind, GeometryErrorKind::InvalidResult);
        assert!(error.message.contains("leaves no cavity"));
    }
    let both = selections(&source, PlaneFrame::default(), [true, true], feature);
    let tube = source.shell_named(&both, 5., feature, policy).unwrap();
    near(tube.volume_mm3().unwrap(), 700. * PI);
    assert_eq!(
        source
            .shell_named(&[], 2., feature, policy)
            .err()
            .unwrap()
            .kind,
        GeometryErrorKind::Unsupported
    );
    let side = TreatmentFace {
        key: TopologyKey {
            kind: TopologyKind::Face,
            ordinal: 2,
        },
        origin: origin(feature, "side-opening"),
    };
    assert_eq!(
        source
            .shell_named(&[side], 2., feature, policy)
            .err()
            .unwrap()
            .kind,
        GeometryErrorKind::Unsupported
    );
    assert_eq!(
        source
            .shell_named(&[top[0].clone(), top[0].clone()], 2., feature, policy)
            .err()
            .unwrap()
            .kind,
        GeometryErrorKind::Input
    );
    let changed_policy = GeometryPolicy {
        linear_tolerance_mm: 1e-6,
        ..policy
    };
    assert_eq!(
        source
            .shell_named(&top, 2., feature, changed_policy)
            .err()
            .unwrap()
            .kind,
        GeometryErrorKind::Unsupported
    );
    let before = source.volume_mm3().unwrap();
    assert_eq!(
        source
            .shell_named(&top, 0., feature, policy)
            .err()
            .unwrap()
            .kind,
        GeometryErrorKind::Input
    );
    near(source.volume_mm3().unwrap(), before);
}
