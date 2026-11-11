use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::{Curve, CurveKind, Dimension, source::Source},
    document::{Document, Entity, EntityData, EntityId, Link, Mutation},
    model::{
        self, AdvancedParameters, AdvancedRequest, BodyInput, Direction, Extent, ExtrudeParameters,
        ExtrudeRequest, MirrorParameters, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest,
        Termination, length,
    },
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    references::{self, Attachment, GeometryIntent, Origin, Resolution, TopologyKind},
    sketch::Datum,
};
use multicad_kernel::{GeometryPolicy, Solid};
use multicad_model::{Compiler, GeometryBackend};
use std::{f64::consts::PI, io::Cursor};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-7, "{a} != {b}");
}
fn bind(s: &Session, name: &str) -> Dimension {
    Dimension::Expression {
        expression: BoundExpression::bind(s.document(), &format!("=${name}"), "mm", None).unwrap(),
    }
}
fn set(s: &mut Session, name: &str, value: &str) {
    let mut entity = s
        .document()
        .entities()
        .find(|e| e.name == name)
        .unwrap()
        .clone();
    let EntityData::Parameter { expression, .. } = &mut entity.data else {
        panic!()
    };
    if expression == value {
        return;
    }
    *expression = value.into();
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: format!("Edit {name}"),
        edits: vec![Mutation::Replace(entity)],
    })
    .unwrap();
}
fn reopen(s: &Session) -> Session {
    let mut bytes = Vec::new();
    project_file::write(
        &ProjectFile::new(s.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let read = project_file::read(
        &mut Cursor::new(bytes),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(read.project.document, *s.document());
    Session::new(read.project.document).unwrap()
}

#[test]
fn saved_circular_mirrors_keep_cap_datums_after_two_reflections_and_parameter_edits() {
    let compiler = Compiler {
        omit_display: true,
        ..Default::default()
    };
    for reverse in [None, Some(false), Some(true)] {
        let (sketch, edits) = Source::create(Datum::Xz, "Circular sketch").unwrap();
        let mut document = Document::new("Reflected circular caps")
            .unwrap()
            .edited(&edits)
            .unwrap();
        for (name, expression) in [("height", "3"), ("mirror_z", "1")] {
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
        let mut source = Source::read(&document, sketch).unwrap().0;
        let center = source.add_point([7., 9.]);
        source.sketch.curves.push(Curve {
            id: EntityId::new(),
            construction: false,
            kind: CurveKind::Circle {
                center,
                radius_mm: 5.,
            },
        });
        let region = multicad_core::sketch_regions::extract(&source)
            .regions
            .into_iter()
            .find(|r| r.holes.is_empty())
            .unwrap()
            .intent();
        document = document
            .edited(&source.replace(&document, sketch).unwrap())
            .unwrap();
        let mut session = Session::new(document).unwrap();
        let extrude = ExtrudeRequest::new(
            sketch,
            vec![region],
            ExtrudeParameters {
                direction: Direction {
                    reverse: reverse.unwrap_or(false),
                    ..Default::default()
                },
                extent: Extent::OneSided {
                    end: Termination::Distance {
                        distance: bind(&session, "height"),
                    },
                },
                ..Default::default()
            },
        );
        let part = if reverse.is_some() {
            let prepared =
                model::prepare_extrude(&session, session.revision(), actor(), &extrude).unwrap();
            session.commit(prepared.prepared).unwrap();
            BodyInput {
                body: extrude.outputs[0].body,
                producer: extrude.outputs[0].feature,
            }
        } else {
            let primitive = PrimitiveRequest::new(
                "Circular stock",
                PrimitiveParameters::Cylinder {
                    radius: length(5.),
                    height: bind(&session, "height"),
                    frame: PrimitiveFrame {
                        origin: [length(7.), length(0.), length(9.)],
                        u: [1., 0., 0.],
                        v: [0., 0., 1.],
                    },
                },
            );
            let prepared =
                model::prepare_primitive(&session, session.revision(), actor(), &primitive)
                    .unwrap();
            session.commit(prepared.prepared).unwrap();
            BodyInput {
                body: primitive.body,
                producer: primitive.feature,
            }
        };
        let build = |session: &Session, backend| {
            compiler
                .compile_with_backend(session.document(), session.revision(), backend, None)
                .unwrap()
        };
        let initial = build(&session, GeometryBackend::Rust);
        let before = &initial.snapshot().shapes[&part.producer].topology;
        let captures = before
            .elements
            .iter()
            .map(|element| {
                let mut intent = GeometryIntent::capture(before, element.key).unwrap();
                intent.boundary = Some(element.boundary.clone());
                intent
            })
            .collect::<Vec<_>>();
        let plane = PrimitiveFrame {
            origin: [length(3.), length(-2.), bind(&session, "mirror_z")],
            u: [0.8, 0., -0.6],
            v: [0., 1., 0.],
        };
        let first = AdvancedRequest::on_body(
            BodyInput {
                body: part.body,
                producer: part.producer,
            },
            AdvancedParameters::Mirror(MirrorParameters {
                plane: plane.clone(),
            }),
            vec![],
        );
        let base = session.document().clone();
        let prepared =
            model::prepare_advanced(&session, session.revision(), actor(), &first).unwrap();
        session.commit(prepared.prepared).unwrap();
        let authored = session.document().clone();
        session.undo(session.revision(), actor()).unwrap();
        assert_eq!(session.document(), &base);
        session.redo(session.revision(), actor()).unwrap();
        assert_eq!(session.document(), &authored);
        let second = AdvancedRequest::on_body(
            BodyInput {
                body: part.body,
                producer: first.feature,
            },
            AdvancedParameters::Mirror(MirrorParameters { plane }),
            vec![],
        );
        let prepared =
            model::prepare_advanced(&session, session.revision(), actor(), &second).unwrap();
        session.commit(prepared.prepared).unwrap();
        let mut datums = Vec::new();
        for cap in 0..2 {
            let mut intent = captures
                .iter()
                .find(|intent| {
                    intent.kind == TopologyKind::Face
                        && intent.origins.contains(&Origin {
                            source: part.producer,
                            role: if cap == 0 { "cap.start" } else { "cap.end" }.into(),
                        })
                })
                .unwrap()
                .clone();
            intent.attachment = Some(Attachment {
                anchor: Origin {
                    source: part.producer,
                    role: if cap == 0 { "cap.start" } else { "cap.end" }.into(),
                },
                offset_mm: [0.25, -0.5, 0.75],
                angle_rad: 0.3,
            });
            let datum = Entity::new(
                format!("Restored cap {cap}"),
                EntityData::Plane {
                    origin_mm: [0.; 3],
                    normal: [0., 0., 1.],
                    anchor: Some(Link {
                        target: second.feature,
                        role: "face_attachment".into(),
                        geometry: Some(intent.clone()),
                    }),
                },
            );
            datums.push((datum.id, intent));
            session
                .apply(Request {
                    expected_revision: session.revision(),
                    actor: actor(),
                    label: "Attach restored cap datum".into(),
                    edits: vec![Mutation::Insert(datum)],
                })
                .unwrap();
        }
        for (index, (offset, height)) in
            [("1", "3"), ("4", "3"), ("4", "5")].into_iter().enumerate()
        {
            let previous = session.document().clone();
            set(&mut session, "mirror_z", offset);
            set(&mut session, "height", height);
            let opened = reopen(&session);
            let rust = build(&opened, GeometryBackend::Rust);
            let snapshot = rust.snapshot();
            let original = &snapshot.shapes[&part.producer];
            let one = &snapshot.shapes[&first.feature];
            let restored = &snapshot.shapes[&second.feature];
            let expected = 25. * PI * height.parse::<f64>().unwrap();
            for shape in [one, restored] {
                near(shape.volume_mm3, expected);
                assert_eq!(shape.solid_count, 1);
                assert_eq!(shape.topology.elements.len(), 5);
                for intent in &captures {
                    assert!(matches!(
                        references::resolve(intent, &shape.topology, opened.revision()),
                        Resolution::Resolved { .. }
                    ));
                }
            }
            for (id, intent) in &datums {
                assert!(opened.document().get(*id).is_some());
                let Resolution::Resolved {
                    frame: Some(expected),
                    ..
                } = references::resolve(intent, &original.topology, opened.revision())
                else {
                    panic!("source frame")
                };
                let Resolution::Resolved {
                    frame: Some(actual),
                    ..
                } = references::resolve(intent, &restored.topology, opened.revision())
                else {
                    panic!("restored frame")
                };
                for (a, b) in actual
                    .origin_mm
                    .into_iter()
                    .chain(actual.u)
                    .chain(actual.v)
                    .zip(
                        expected
                            .origin_mm
                            .into_iter()
                            .chain(expected.u)
                            .chain(expected.v),
                    )
                {
                    near(a, b);
                }
            }
            for (a, b) in restored
                .bounds
                .min
                .into_iter()
                .chain(restored.bounds.max)
                .zip(original.bounds.min.into_iter().chain(original.bounds.max))
            {
                near(a, b);
            }
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("saved-circular-mirror.step");
            rust.solid(first.feature)
                .unwrap()
                .write_step(&path)
                .unwrap();
            let read = Solid::import_rust_step(
                &std::fs::read_to_string(&path).unwrap(),
                EntityId::new(),
                GeometryPolicy::default(),
            )
            .unwrap();
            near(read.volume_mm3().unwrap(), expected);
            #[cfg(feature = "occt")]
            {
                let native = build(&opened, GeometryBackend::OpenCascade);
                for feature in [first.feature, second.feature] {
                    let native = &native.snapshot().shapes[&feature];
                    let actual = &snapshot.shapes[&feature];
                    near(native.volume_mm3, expected);
                    for (a, b) in native
                        .bounds
                        .min
                        .into_iter()
                        .chain(native.bounds.max)
                        .zip(actual.bounds.min.into_iter().chain(actual.bounds.max))
                    {
                        near(a, b);
                    }
                }
                let native = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
                near(native.volume_mm3().unwrap(), expected);
            }
            if index > 0 {
                let changed = session.document().clone();
                session.undo(session.revision(), actor()).unwrap();
                assert_eq!(session.document(), &previous);
                session.redo(session.revision(), actor()).unwrap();
                assert_eq!(session.document(), &changed);
                near(
                    build(&session, GeometryBackend::Rust)
                        .solid(second.feature)
                        .unwrap()
                        .volume_mm3()
                        .unwrap(),
                    expected,
                );
            }
        }
    }
}
