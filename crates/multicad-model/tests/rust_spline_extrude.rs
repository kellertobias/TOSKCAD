use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::{
        Axis, ConstraintKind, Curve, CurveKind, Dimension, SplineBasis, source::Source,
    },
    document::{Document, Entity, EntityData, EntityId, Link, Mutation},
    model::{self, BodyInput, Direction, Extent, ExtrudeParameters, Termination, length},
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    references::{Attachment, GeometryIntent, Origin, ReferenceAddress, Resolution, TopologyKind},
    sketch::Datum,
};
use multicad_kernel::{GeometryPolicy, NativeRegionQueries, Solid};
use multicad_model::{Compiler, GeometryBackend, Model};
use multicad_solver::evaluate_document_sketch;
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}
fn bound(session: &Session, name: &str) -> Dimension {
    Dimension::Expression {
        expression: BoundExpression::bind(session.document(), &format!("=${name}"), "mm", None)
            .unwrap(),
    }
}
fn compile(session: &Session) -> Model {
    Compiler::default()
        .compile_with_backend(
            session.document(),
            session.revision(),
            GeometryBackend::Rust,
            None,
        )
        .unwrap()
}
fn reopen(session: &Session) -> Session {
    let mut bytes = Vec::new();
    project_file::write(
        &ProjectFile::new(session.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let file = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(&file.project.document, session.document());
    Session::new(file.project.document).unwrap()
}
fn parameter(document: &Document, name: &str, value: &str) -> Mutation {
    let mut entity = document
        .entities()
        .find(|e| e.name == name)
        .unwrap()
        .clone();
    let EntityData::Parameter { expression, .. } = &mut entity.data else {
        panic!()
    };
    *expression = value.into();
    Mutation::Replace(entity)
}
#[test]
fn saved_driven_spline_extrusion_updates_datums_occurrences_history_and_real_step() {
    for reverse in [false, true] {
        let (sketch, mut edits) = Source::create(Datum::Xz, "Driven spline").unwrap();
        for (name, value) in [("arch_height", "10"), ("depth", "3")] {
            edits.push(Mutation::Insert(Entity::new(
                name,
                EntityData::Parameter {
                    expression: value.into(),
                    unit: "mm".into(),
                    exposed: true,
                    inputs: vec![],
                },
            )));
        }
        let document = Document::new("Canonical spline extrusion")
            .unwrap()
            .edited(&edits)
            .unwrap();
        let mut source = Source::read(&document, sketch).unwrap().0;
        let [a, b, c] = [[0., 0.], [5., 10.], [10., 0.]].map(|point| source.add_point(point));
        let curve = EntityId::new();
        source.sketch.curves.push(Curve {
            id: curve,
            construction: false,
            kind: CurveKind::BSpline {
                poles: vec![a, b, c],
                basis: SplineBasis::open_uniform(2, 3).unwrap(),
            },
        });
        source.add_line(c, a);
        source.add_constraint(ConstraintKind::Fix { point: a });
        source.add_constraint(ConstraintKind::Fix { point: c });
        source.add_constraint(ConstraintKind::Distance {
            a,
            b,
            axis: Axis::X,
            dimension: Dimension::Literal {
                value: 5.,
                unit: "mm".into(),
            },
        });
        source.add_constraint(ConstraintKind::Distance {
            a,
            b,
            axis: Axis::Y,
            dimension: Dimension::Expression {
                expression: BoundExpression::bind(&document, "=$arch_height", "mm", None).unwrap(),
            },
        });
        let mut session = Session::new(
            document
                .edited(&source.replace(&document, sketch).unwrap())
                .unwrap(),
        )
        .unwrap();
        let canonical = session.document().get(sketch).unwrap().clone();
        let evaluated = evaluate_document_sketch(session.document(), 0, sketch, None).unwrap();
        let queries = NativeRegionQueries::analytic(GeometryPolicy::default()).unwrap();
        let regions = evaluated.regions_with_queries(&queries).unwrap();
        assert_eq!(regions.regions.len(), 1);
        let request = model::ExtrudeRequest::new(
            sketch,
            vec![regions.regions[0].intent()],
            ExtrudeParameters {
                direction: Direction {
                    reverse,
                    ..Default::default()
                },
                extent: Extent::OneSided {
                    end: Termination::Distance {
                        distance: bound(&session, "depth"),
                    },
                },
                ..Default::default()
            },
        );
        let prepared =
            model::prepare_extrude(&session, session.revision(), actor(), &request).unwrap();
        Compiler::default()
            .compile_with_backend(
                prepared.prepared.document(),
                session.revision() + 1,
                GeometryBackend::Rust,
                None,
            )
            .unwrap();
        session.commit(prepared.prepared).unwrap();
        let output = request.outputs[0].clone();
        let original = compile(&session);
        near(
            original.solid(output.body).unwrap().volume_mm3().unwrap(),
            100.,
        );
        let topology = &original.snapshot().shapes[&output.feature].topology;
        let cap = topology
            .elements
            .iter()
            .find(|e| {
                e.key.kind == TopologyKind::Face
                    && e.origins.contains(&Origin {
                        source: output.feature,
                        role: "cap.end".into(),
                    })
            })
            .unwrap();
        let mut intent = GeometryIntent::capture(topology, cap.key).unwrap();
        intent.attachment = Some(Attachment {
            anchor: cap.frames[0].origin.clone(),
            offset_mm: [0.125, -0.25, 0.75],
            angle_rad: 0.2,
        });
        let reference = intent.id;
        let datum = Entity::new(
            "Spline cap datum",
            EntityData::Plane {
                origin_mm: [0.; 3],
                normal: [0., 0., 1.],
                anchor: Some(Link {
                    target: output.feature,
                    role: "face_attachment".into(),
                    geometry: Some(intent),
                }),
            },
        );
        let address = ReferenceAddress {
            owner: datum.id,
            reference,
        };
        session
            .apply(Request {
                expected_revision: session.revision(),
                actor: actor(),
                label: "Attach spline cap datum".into(),
                edits: vec![Mutation::Insert(datum)],
            })
            .unwrap();
        let instance = model::InstanceCopyRequest::new(
            model::InstanceCopySource::Part {
                source: BodyInput {
                    body: output.body,
                    producer: output.feature,
                },
            },
            model::NumericTransform {
                translation: [length(40.), length(0.), length(0.)],
                ..Default::default()
            },
        );
        session
            .commit(
                model::prepare_instance_copies(&session, session.revision(), actor(), &[instance])
                    .unwrap()
                    .prepared,
            )
            .unwrap();
        let initial = session.document().clone();
        let edits = vec![
            parameter(session.document(), "arch_height", "16"),
            parameter(session.document(), "depth", "4"),
        ];
        session
            .apply(Request {
                expected_revision: session.revision(),
                actor: actor(),
                label: "Edit driven spline and depth".into(),
                edits,
            })
            .unwrap();
        let edited = session.document().clone();
        let saved = reopen(&session);
        let model = compile(&saved);
        near(
            model.solid(output.body).unwrap().volume_mm3().unwrap(),
            640. / 3.,
        );
        assert_eq!(saved.document().get(sketch).unwrap(), &canonical);
        assert!(model.snapshot().references.all_resolved());
        assert!(matches!(
            model.snapshot().references.get(address).unwrap().resolution,
            Resolution::Resolved { frame: Some(_), .. }
        ));
        assert!(!model.snapshot().occurrences.is_empty());
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("canonical-spline.step");
        model.solid(output.body).unwrap().write_step(&path).unwrap();
        let imported = Solid::import_rust_step(
            &std::fs::read_to_string(path).unwrap(),
            EntityId::new(),
            Default::default(),
        )
        .unwrap();
        near(imported.volume_mm3().unwrap(), 640. / 3.);
        #[cfg(feature = "occt")]
        {
            let native = Compiler::default()
                .compile_with_backend(
                    saved.document(),
                    saved.revision(),
                    GeometryBackend::OpenCascade,
                    None,
                )
                .unwrap();
            assert!(!native.solid(output.body).unwrap().uses_rust_geometry());
            near(
                native.solid(output.body).unwrap().volume_mm3().unwrap(),
                640. / 3.,
            );
            let Resolution::Resolved { frame: Some(a), .. } =
                model.snapshot().references.get(address).unwrap().resolution
            else {
                panic!()
            };
            let Resolution::Resolved { frame: Some(b), .. } = native
                .snapshot()
                .references
                .get(address)
                .unwrap()
                .resolution
            else {
                panic!()
            };
            for (a, b) in a
                .origin_mm
                .into_iter()
                .chain(a.u)
                .chain(a.v)
                .zip(b.origin_mm.into_iter().chain(b.u).chain(b.v))
            {
                near(a, b);
            }
        }
        session.undo(session.revision(), actor()).unwrap();
        assert_eq!(session.document(), &initial);
        near(
            compile(&session)
                .solid(output.body)
                .unwrap()
                .volume_mm3()
                .unwrap(),
            100.,
        );
        session.redo(session.revision(), actor()).unwrap();
        assert_eq!(session.document(), &edited);
        let before = session.document().clone();
        let revision = session.revision();
        let candidate = session
            .prepare(Request {
                expected_revision: revision,
                actor: actor(),
                label: "Collapse consumed spline".into(),
                edits: vec![parameter(session.document(), "arch_height", "0")],
            })
            .unwrap();
        assert!(
            Compiler::default()
                .compile_with_backend(
                    candidate.document(),
                    revision + 1,
                    GeometryBackend::Rust,
                    None
                )
                .is_err()
        );
        assert_eq!(session.document(), &before);
        assert_eq!(session.revision(), revision);
        assert!(
            Compiler::default()
                .compile_with_backend(
                    session.document(),
                    revision,
                    GeometryBackend::Rust,
                    Some(&|| true)
                )
                .is_err()
        );
        assert_eq!(session.document(), &before);
        near(
            compile(&session)
                .solid(output.body)
                .unwrap()
                .volume_mm3()
                .unwrap(),
            640. / 3.,
        );
    }
}
