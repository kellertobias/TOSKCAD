use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Mutation},
    exchange::{self, ExportReply, ExportResult, SetDefinition},
    model::*,
    parameters::BoundExpression,
    project_file::{self, FileLimits, ProjectFile},
};
use multicad_kernel::{Bounds, GeometryPolicy, Solid};
use multicad_model::{
    Compiler, GeometryBackend, Model,
    export::{ExportWorker, Exporter},
};
use std::{
    io::Cursor,
    path::Path,
    time::{Duration, Instant},
};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn fixture() -> (Session, PrimitiveRequest, EntityId, EntityId, EntityId) {
    let mut d = Document::new("Exact parametric scale").unwrap();
    let p = Entity::new(
        "factor",
        EntityData::Parameter {
            expression: "2".into(),
            unit: "ratio".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let factor = p.id;
    d.insert(p).unwrap();
    let mut s = Session::new(d).unwrap();
    let part = PrimitiveRequest::new(
        "Stock",
        PrimitiveParameters::Box {
            size: [length(2.), length(3.), length(4.)],
            frame: Default::default(),
        },
    );
    s.commit(
        prepare_primitive(&s, s.revision(), actor(), &part)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let a = InstanceCopyRequest::new(
        InstanceCopySource::Part {
            source: BodyInput {
                body: part.body,
                producer: part.feature,
            },
        },
        NumericTransform {
            translation: [length(100.), length(50.), length(0.)],
            angle: angle(90.),
            ..Default::default()
        },
    );
    let b = InstanceCopyRequest::new(
        a.source.clone(),
        NumericTransform {
            translation: [length(200.), length(0.), length(0.)],
            ..Default::default()
        },
    );
    s.commit(
        prepare_instance_copies(&s, s.revision(), actor(), &[a.clone(), b.clone()])
            .unwrap()
            .prepared,
    )
    .unwrap();
    (s, part, factor, a.instance, b.instance)
}
fn compile(s: &Session) -> Model {
    Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap()
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}
fn bounds(a: Bounds, min: [f64; 3], max: [f64; 3]) {
    for (a, b) in a
        .min
        .into_iter()
        .chain(a.max)
        .zip(min.into_iter().chain(max))
    {
        near(a, b);
    }
}
fn reopen(s: &Session) -> Session {
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(s.document().clone()).unwrap(),
        &mut bytes,
        FileLimits::default(),
    )
    .unwrap();
    let read = project_file::read(
        &mut Cursor::new(bytes),
        FileLimits::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(read.project.document, *s.document());
    Session::new(read.project.document).unwrap()
}
fn export_reply(worker: &ExportWorker) -> ExportReply {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(reply) = worker.poll() {
            return reply;
        }
        assert!(
            Instant::now() < deadline,
            "Scale STEP worker did not finish"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn bound(s: &Session) -> ScaleParameters {
    let expression = BoundExpression::bind(s.document(), "=$factor", "ratio", None).unwrap();
    ScaleParameters {
        factors: std::array::from_fn(|_| Dimension::Expression {
            expression: expression.clone(),
        }),
        pivot: [length(1.), length(2.), length(3.)],
        ..Default::default()
    }
}
#[test]
fn uniform_definition_scale_rebuilds_bound_factors_and_dependent_occurrences_after_save_reopen() {
    let (mut s, part, factor, instance, other) = fixture();
    let original = s.document().clone();
    let request = ScaleBatchRequest::new(
        s.document(),
        vec![ScaleTarget::Part {
            input: BodyInput {
                body: part.body,
                producer: part.feature,
            },
        }],
        bound(&s),
        false,
    )
    .unwrap();
    let result = prepare_scale_batch(&s, s.revision(), actor(), &request).unwrap();
    let feature = result.outputs[0].feature;
    let candidate = Compiler::default()
        .compile(result.prepared.document(), s.revision() + 1)
        .unwrap();
    let shape = &candidate.snapshot().shapes[&feature];
    bounds(shape.bounds, [-1., -2., -3.], [3., 4., 5.]);
    near(shape.volume_mm3, 192.);
    assert_eq!(shape.solid_count, 1);
    for id in [instance, other] {
        near(candidate.solid(id).unwrap().volume_mm3().unwrap(), 192.);
    }
    assert_eq!(s.document(), &original);
    s.commit(result.prepared).unwrap();
    let reopened = reopen(&s);
    let native = compile(&reopened);
    bounds(
        native.snapshot().shapes[&feature].bounds,
        [-1., -2., -3.],
        [3., 4., 5.],
    );
    assert_eq!(
        read_scale(reopened.document(), feature).unwrap().parameters,
        request.parameters
    );
    let mut p = s.document().get(factor).unwrap().clone();
    let EntityData::Parameter { expression, .. } = &mut p.data else {
        panic!()
    };
    *expression = "3".into();
    p.name = "renamed_factor".into();
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Change bound factor".into(),
        edits: vec![Mutation::Replace(p)],
    })
    .unwrap();
    let native = compile(&s);
    bounds(
        native.snapshot().shapes[&feature].bounds,
        [-2., -4., -6.],
        [4., 5., 6.],
    );
    near(native.snapshot().shapes[&feature].volume_mm3, 648.);
    assert_eq!(s.document().get(part.feature), original.get(part.feature));
    assert!(
        read_scale(s.document(), feature)
            .unwrap()
            .parameters
            .factors
            .iter()
            .all(|d| d.expression().unwrap().bindings[0].target == factor)
    );
}
#[test]
fn nonuniform_instance_scale_changes_only_owned_geometry_and_has_one_world_pose_and_no_phantom_exports()
 {
    let (mut s, part, _, instance, other) = fixture();
    let original = s.document().clone();
    let revision = s.revision();
    let initial = compile(&s);
    let original_pose = initial.snapshot().occurrences[&instance].pose;
    let request = ScaleBatchRequest::new(
        s.document(),
        vec![ScaleTarget::Instance { instance }],
        ScaleParameters {
            factors: [ratio(2.), ratio(3.), ratio(4.)],
            pivot: [length(100.), length(50.), length(0.)],
            ..Default::default()
        },
        false,
    )
    .unwrap();
    let result = prepare_scale_batch(&s, revision, actor(), &request).unwrap();
    let native = Compiler::default()
        .compile(result.prepared.document(), revision + 1)
        .unwrap();
    let occurrence = &native.snapshot().occurrences[&instance];
    let branch = occurrence.component;
    assert_ne!(branch, part.body);
    assert_eq!(occurrence.pose, original_pose);
    bounds(occurrence.bounds, [94., 50., 0.], [100., 56., 16.]);
    near(native.solid(instance).unwrap().volume_mm3().unwrap(), 576.);
    bounds(
        native.snapshot().occurrences[&other].bounds,
        [200., 0., 0.],
        [202., 3., 4.],
    );
    near(native.solid(other).unwrap().volume_mm3().unwrap(), 24.);
    near(native.solid(part.body).unwrap().volume_mm3().unwrap(), 24.);
    assert_eq!(native.snapshot().bodies.len(), 1);
    assert!(!native.snapshot().bodies.contains_key(&branch));
    assert!(native.snapshot().body_sources.contains_key(&branch));
    assert_eq!(native.snapshot().occurrences.len(), 2);
    for id in [part.body, part.feature, other] {
        assert_eq!(result.prepared.document().get(id), original.get(id));
    }
    s.commit(result.prepared).unwrap();
    let after = s.document().clone();
    let opened = reopen(&s);
    let rebuilt = compile(&opened);
    bounds(
        rebuilt.snapshot().occurrences[&instance].bounds,
        [94., 50., 0.],
        [100., 56., 16.],
    );
    assert_eq!(
        rebuilt.snapshot().occurrences[&instance].producer,
        occurrence.producer
    );
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &original);
    bounds(
        compile(&s).snapshot().occurrences[&instance].bounds,
        [97., 50., 0.],
        [100., 52., 4.],
    );
    s.redo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &after);
    let set = EntityId::new();
    let prepared = exchange::prepare_set(
        &s,
        s.revision(),
        actor(),
        set,
        "Whole project",
        &SetDefinition::default(),
    )
    .unwrap();
    s.commit(prepared).unwrap();
    let backend = GeometryBackend::default();
    let worker = ExportWorker::with_geometry_backend(Exporter::default(), backend).unwrap();
    worker
        .preview(s.document(), s.revision(), set, actor())
        .unwrap();
    let ExportResult::Preview(preview) = export_reply(&worker).result else {
        panic!("expected STEP preview");
    };
    let preview = preview.unwrap();
    assert!(preview.can_run());
    assert_eq!(preview.artifacts.len(), 3);
    let root = tempfile::tempdir().unwrap();
    worker
        .run(
            s.document(),
            s.revision(),
            &preview.key,
            root.path(),
            actor(),
        )
        .unwrap();
    let ExportResult::Run(receipt) = export_reply(&worker).result else {
        panic!("expected STEP publication");
    };
    let receipt = receipt.unwrap();
    assert_eq!(receipt.files.len(), 3);
    let mut volumes = vec![];
    for file in &receipt.files {
        let path = Path::new(&receipt.directory).join(&file.artifact.output.filename);
        let solid = if backend == GeometryBackend::Rust {
            let text = std::fs::read_to_string(path).unwrap();
            Solid::import_rust_step(&text, EntityId::new(), GeometryPolicy::default()).unwrap()
        } else {
            Solid::read_step(&path, GeometryPolicy::default()).unwrap()
        };
        assert_eq!(solid.solid_count().unwrap(), 1);
        volumes.push(solid.volume_mm3().unwrap());
    }
    volumes.sort_by(f64::total_cmp);
    for (a, b) in volumes.into_iter().zip([24., 24., 576.]) {
        near(a, b);
    }
}
#[test]
fn copying_an_already_scaled_occurrence_keeps_source_and_copies_once_then_upstream_definition_edits_propagate()
 {
    let (mut s, part, _, instance, other) = fixture();
    let first = ScaleBatchRequest::new(
        s.document(),
        vec![ScaleTarget::Instance { instance }],
        ScaleParameters {
            factors: [ratio(2.), ratio(2.), ratio(2.)],
            pivot: [length(100.), length(50.), length(0.)],
            ..Default::default()
        },
        false,
    )
    .unwrap();
    let stage = prepare_scale_batch(&s, s.revision(), actor(), &first).unwrap();
    s.commit(stage.prepared).unwrap();
    let original = s.document().clone();
    let current = compile(&s);
    let original_bound = current.snapshot().occurrences[&instance].bounds;
    let copy = ScaleBatchRequest::new(
        s.document(),
        vec![ScaleTarget::Instance { instance }],
        first.parameters.clone(),
        true,
    )
    .unwrap();
    let a = prepare_scale_batch(&s, s.revision(), actor(), &copy).unwrap();
    let b = prepare_scale_batch(&s, s.revision(), actor(), &copy).unwrap();
    assert_eq!(a.prepared.document(), b.prepared.document());
    let new_instance = a.instances[0];
    let native = Compiler::default()
        .compile(a.prepared.document(), s.revision() + 1)
        .unwrap();
    assert_eq!(native.snapshot().bodies.len(), 1);
    assert_eq!(native.snapshot().occurrences.len(), 3);
    near(
        native.solid(new_instance).unwrap().volume_mm3().unwrap(),
        1536.,
    );
    assert_eq!(
        native.snapshot().occurrences[&instance].bounds,
        original_bound
    );
    for id in [part.body, part.feature, instance, other] {
        assert_eq!(a.prepared.document().get(id), original.get(id));
    }
    s.commit(a.prepared).unwrap();
    let native = compile(&reopen(&s));
    near(
        native.solid(new_instance).unwrap().volume_mm3().unwrap(),
        1536.,
    );
    let part_scale = ScaleBatchRequest::new(
        s.document(),
        vec![ScaleTarget::Part {
            input: BodyInput {
                body: part.body,
                producer: part.feature,
            },
        }],
        ScaleParameters {
            factors: [2.; 3].map(ratio),
            ..Default::default()
        },
        false,
    )
    .unwrap();
    let stage = prepare_scale_batch(&s, s.revision(), actor(), &part_scale).unwrap();
    s.commit(stage.prepared).unwrap();
    let rebuilt = compile(&s);
    near(
        rebuilt.solid(part.body).unwrap().volume_mm3().unwrap(),
        192.,
    );
    near(
        rebuilt.solid(instance).unwrap().volume_mm3().unwrap(),
        1536.,
    );
    near(
        rebuilt.solid(new_instance).unwrap().volume_mm3().unwrap(),
        12288.,
    );
    near(rebuilt.solid(other).unwrap().volume_mm3().unwrap(), 192.);
}
#[test]
fn failed_native_scale_and_missing_bound_parameter_do_not_substitute_geometry_or_modify_source() {
    let (mut s, part, factor, instance, _) = fixture();
    let original = s.document().clone();
    let tiny = ScaleBatchRequest::new(
        s.document(),
        vec![ScaleTarget::Instance { instance }],
        ScaleParameters {
            factors: [1e-12; 3].map(ratio),
            ..Default::default()
        },
        false,
    )
    .unwrap();
    let stage = prepare_scale_batch(&s, s.revision(), actor(), &tiny).unwrap();
    assert!(
        Compiler::default()
            .compile(stage.prepared.document(), s.revision() + 1)
            .is_err()
    );
    assert_eq!(s.document(), &original);
    let request = ScaleBatchRequest::new(
        s.document(),
        vec![ScaleTarget::Part {
            input: BodyInput {
                body: part.body,
                producer: part.feature,
            },
        }],
        bound(&s),
        false,
    )
    .unwrap();
    let stage = prepare_scale_batch(&s, s.revision(), actor(), &request).unwrap();
    s.commit(stage.prepared).unwrap();
    let missing = s.document().edited(&[Mutation::Remove(factor)]).unwrap();
    assert!(
        Compiler::default()
            .compile(&missing, s.revision() + 1)
            .is_err()
    );
}

#[test]
fn an_existing_face_aligned_occurrence_retains_its_rigid_driver_chain_when_scaled_and_copied() {
    let (mut s, part, _, instance, other) = fixture();
    let before = compile(&s);
    let topology = &before.snapshot().shapes[&part.feature].topology;
    let face = topology
        .elements
        .iter()
        .find(|e| {
            e.key.kind == multicad_core::references::TopologyKind::Face
                && e.origins.iter().any(|o| o.role == "cap.start")
        })
        .unwrap();
    let mut intent =
        multicad_core::references::GeometryIntent::capture(topology, face.key).unwrap();
    intent.attachment = Some(multicad_core::references::Attachment {
        anchor: multicad_core::references::Origin {
            source: part.feature,
            role: "cap.start".into(),
        },
        offset_mm: [0.; 3],
        angle_rad: 0.,
    });
    let alignment = InstanceAlignmentRequest::new(
        instance,
        AlignmentParameters {
            source: AlignmentAnchor::Geometry {
                reference: intent.id,
            },
            target: AlignmentAnchor::Frame {
                frame: multicad_core::references::ReferenceFrame {
                    origin_mm: [100., 20., 10.],
                    u: [1., 0., 0.],
                    v: [0., 1., 0.],
                },
            },
            ..Default::default()
        },
        vec![AlignmentInput {
            source: part.feature,
            intent,
        }],
    );
    s.commit(
        prepare_instance_alignments(&s, s.revision(), actor(), std::slice::from_ref(&alignment))
            .unwrap()
            .prepared,
    )
    .unwrap();
    let aligned = compile(&s);
    let initial = &aligned.snapshot().occurrences[&instance];
    let old_pose = initial.pose;
    let old_bounds = initial.bounds;
    let center = std::array::from_fn(|i| (old_bounds.min[i] + old_bounds.max[i]) * 0.5);
    let request = ScaleBatchRequest::new(
        s.document(),
        vec![ScaleTarget::Instance { instance }],
        ScaleParameters {
            factors: [ratio(2.), ratio(3.), ratio(4.)],
            pivot: center.map(length),
            ..Default::default()
        },
        false,
    )
    .unwrap();
    let stage = prepare_scale_batch(&s, s.revision(), actor(), &request).unwrap();
    s.commit(stage.prepared).unwrap();
    let scaled = compile(&s);
    assert_eq!(scaled.snapshot().occurrences[&instance].pose, old_pose);
    let expected =
        |p: [f64; 3]| std::array::from_fn(|i| center[i] + [2., 3., 4.][i] * (p[i] - center[i]));
    bounds(
        scaled.snapshot().occurrences[&instance].bounds,
        expected(old_bounds.min),
        expected(old_bounds.max),
    );
    near(scaled.solid(instance).unwrap().volume_mm3().unwrap(), 576.);
    assert!(scaled.snapshot().references.all_resolved());
    assert_eq!(
        scaled.snapshot().occurrences[&other].bounds,
        aligned.snapshot().occurrences[&other].bounds
    );
    let copy = ScaleBatchRequest::new(
        s.document(),
        vec![ScaleTarget::Instance { instance }],
        ScaleParameters::default(),
        true,
    )
    .unwrap();
    let stage = prepare_scale_batch(&s, s.revision(), actor(), &copy).unwrap();
    let copied = stage.instances[0];
    s.commit(stage.prepared).unwrap();
    let rebuilt = compile(&reopen(&s));
    assert_eq!(rebuilt.snapshot().bodies.len(), 1);
    assert!(rebuilt.snapshot().references.all_resolved());
    near(rebuilt.solid(copied).unwrap().volume_mm3().unwrap(), 576.);
    bounds(
        rebuilt.snapshot().occurrences[&copied].bounds,
        expected(old_bounds.min),
        expected(old_bounds.max),
    );
}

#[test]
fn saved_rotated_axis_scale_preserves_every_authored_boundary_intent() {
    use multicad_core::references::{
        self, Attachment, GeometryIntent, Origin, Resolution, TopologyKind,
    };

    let (mut session, part, _, _, _) = fixture();
    let compiler = Compiler {
        omit_display: true,
        ..Default::default()
    };
    let before = compiler
        .compile_with_backend(
            session.document(),
            session.revision(),
            GeometryBackend::Rust,
            None,
        )
        .unwrap();
    let original = &before.snapshot().shapes[&part.feature].topology;
    let mut intents = original
        .elements
        .iter()
        .map(|element| GeometryIntent::capture(original, element.key).unwrap())
        .collect::<Vec<_>>();
    let cap = Origin {
        source: part.feature,
        role: "cap.end".into(),
    };
    let cap_index = intents
        .iter()
        .position(|intent| intent.kind == TopologyKind::Face && intent.origins.contains(&cap))
        .unwrap();
    intents[cap_index].attachment = Some(Attachment {
        anchor: cap,
        offset_mm: [0.; 3],
        angle_rad: 0.,
    });
    let c = 0.5_f64.sqrt();
    let parameters = ScaleParameters {
        factors: [2., 3., 0.5].map(ratio),
        pivot: [3., 4., 5.].map(length),
        axes: [[c, c, 0.], [-c, c, 0.], [0., 0., 1.]],
    };
    let request = ScaleBatchRequest::new(
        session.document(),
        vec![ScaleTarget::Part {
            input: BodyInput {
                body: part.body,
                producer: part.feature,
            },
        }],
        parameters.clone(),
        false,
    )
    .unwrap();
    let prepared = prepare_scale_batch(&session, session.revision(), actor(), &request).unwrap();
    let feature = prepared.outputs[0].feature;
    session.commit(prepared.prepared).unwrap();
    let reopened = reopen(&session);
    assert_eq!(
        read_scale(reopened.document(), feature).unwrap().parameters,
        parameters
    );
    let rust = compiler
        .compile_with_backend(reopened.document(), 0, GeometryBackend::Rust, None)
        .unwrap();
    let shape = &rust.snapshot().shapes[&feature];
    assert!(rust.solid(feature).unwrap().uses_rust_geometry());
    bounds(shape.bounds, [-4., -5.5, 2.5], [2.5, 3., 4.5]);
    near(shape.volume_mm3, 72.);
    assert_eq!(shape.topology.elements.len(), original.elements.len());
    for intent in &intents {
        assert!(matches!(
            references::resolve(intent, &shape.topology, 0),
            Resolution::Resolved { .. }
        ));
    }
    let Resolution::Resolved {
        frame: Some(cap), ..
    } = references::resolve(&intents[cap_index], &shape.topology, 0)
    else {
        panic!("scaled cap lost its saved frame")
    };
    bounds(
        Bounds {
            min: cap.origin_mm,
            max: cap.origin_mm,
        },
        [-2.5, -4.5, 4.5],
        [-2.5, -4.5, 4.5],
    );
    #[cfg(feature = "occt")]
    {
        let native = compiler
            .compile_with_backend(reopened.document(), 0, GeometryBackend::OpenCascade, None)
            .unwrap();
        let shape = &native.snapshot().shapes[&feature];
        bounds(shape.bounds, [-4., -5.5, 2.5], [2.5, 3., 4.5]);
        near(shape.volume_mm3, 72.);
        let Resolution::Resolved {
            frame: Some(native_cap),
            ..
        } = references::resolve(&intents[cap_index], &shape.topology, 0)
        else {
            panic!("native cap lost its saved frame")
        };
        for (a, b) in cap.origin_mm.into_iter().chain(cap.u).chain(cap.v).zip(
            native_cap
                .origin_mm
                .into_iter()
                .chain(native_cap.u)
                .chain(native_cap.v),
        ) {
            near(a, b);
        }
    }
}

#[test]
fn rust_scale_refuses_curved_stock_without_changing_the_source() {
    let mut session = Session::new(Document::new("Curved Scale refusal").unwrap()).unwrap();
    let cylinder = PrimitiveRequest::new(
        "Round stock",
        PrimitiveParameters::Cylinder {
            radius: length(5.),
            height: length(10.),
            frame: Default::default(),
        },
    );
    let prepared = prepare_primitive(&session, 0, actor(), &cylinder).unwrap();
    session.commit(prepared.prepared).unwrap();
    let original = session.document().clone();
    let request = ScaleBatchRequest::new(
        session.document(),
        vec![ScaleTarget::Part {
            input: BodyInput {
                body: cylinder.body,
                producer: cylinder.feature,
            },
        }],
        ScaleParameters {
            factors: [2., 1., 3.].map(ratio),
            ..Default::default()
        },
        false,
    )
    .unwrap();
    let prepared = prepare_scale_batch(&session, session.revision(), actor(), &request).unwrap();
    let error = Compiler::default()
        .compile_with_backend(
            prepared.prepared.document(),
            session.revision() + 1,
            GeometryBackend::Rust,
            None,
        )
        .err()
        .expect("unsupported geometry must fail preview");
    assert_eq!(error.entity, Some(prepared.outputs[0].feature));
    assert!(
        error.message.contains("straight planar"),
        "{}",
        error.message
    );
    assert_eq!(session.document(), &original);
}

#[test]
fn saved_axis_aligned_scale_keeps_exact_rectangular_and_round_cut_providers() {
    use multicad_core::{
        constraint_sketch::{Curve, CurveKind, source::Source},
        references::{self, Attachment, GeometryIntent, Origin, Resolution},
        sketch::Datum,
        sketch_regions,
    };
    use std::f64::consts::PI;

    let compiler = Compiler {
        omit_display: true,
        ..Default::default()
    };
    for (factors, round, pivot_z) in [
        ([2., 2., 2.], true, -1.),
        ([2., 2., 2.], true, 0.),
        ([2., 3., 2.], false, -1.),
    ] {
        let (mut session, part, _, instance, _) = fixture();
        let request = ScaleBatchRequest::new(
            session.document(),
            vec![ScaleTarget::Part {
                input: BodyInput {
                    body: part.body,
                    producer: part.feature,
                },
            }],
            ScaleParameters {
                factors: factors.map(ratio),
                pivot: [1., 1., pivot_z].map(length),
                ..Default::default()
            },
            false,
        )
        .unwrap();
        let scale = prepare_scale_batch(&session, session.revision(), actor(), &request).unwrap();
        let target = scale.outputs[0].clone();
        session.commit(scale.prepared).unwrap();
        let mut session = reopen(&session);
        let (sketch, edits) = Source::create(Datum::Xy, "Through cut").unwrap();
        session
            .apply(Request {
                expected_revision: session.revision(),
                actor: actor(),
                label: "Create cut sketch".into(),
                edits,
            })
            .unwrap();
        let mut source = Source::read(session.document(), sketch).unwrap().0;
        let removed = if round {
            let center = source.add_point([1., 1.]);
            source.sketch.curves.push(Curve {
                id: EntityId::new(),
                construction: false,
                kind: CurveKind::Circle {
                    center,
                    radius_mm: 0.5,
                },
            });
            2. * PI
        } else {
            source.add_rectangle([0.5, 0.5], [1.5, 1.5]).unwrap();
            8.
        };
        let region = sketch_regions::extract(&source).regions[0].intent();
        session
            .apply(Request {
                expected_revision: session.revision(),
                actor: actor(),
                label: "Edit cut sketch".into(),
                edits: source.replace(session.document(), sketch).unwrap(),
            })
            .unwrap();
        let compile_rust = |session: &Session| {
            compiler
                .compile_with_backend(
                    session.document(),
                    session.revision(),
                    GeometryBackend::Rust,
                    None,
                )
                .unwrap()
        };
        let scaled = compile_rust(&session);
        let topology = &scaled.snapshot().shapes[&target.feature].topology;
        let cap = Origin {
            source: part.feature,
            role: "cap.end".into(),
        };
        let element = topology
            .elements
            .iter()
            .find(|element| element.frames.iter().any(|frame| frame.origin == cap))
            .unwrap();
        let mut anchor = GeometryIntent::capture(topology, element.key).unwrap();
        anchor.attachment = Some(Attachment {
            anchor: cap,
            offset_mm: [0.; 3],
            angle_rad: 0.,
        });
        let Resolution::Resolved {
            frame: Some(expected_frame),
            ..
        } = references::resolve(&anchor, topology, session.revision())
        else {
            panic!("Scale lost its cap frame")
        };
        let cut = ExtrudeBooleanRequest::new(
            ExtrudeRequest::new(
                sketch,
                vec![region],
                ExtrudeParameters {
                    extent: Extent::OneSided {
                        end: Termination::Distance {
                            distance: length(10.),
                        },
                    },
                    ..Default::default()
                },
            ),
            Operation::Cut,
            vec![BodyInput {
                body: target.body,
                producer: target.feature,
            }],
            false,
        );
        let cut_id = cut.booleans[0].feature;
        let before_cut = session.document().clone();
        let draft = prepare_extrude_boolean(&session, session.revision(), actor(), &cut).unwrap();
        session.commit(draft.prepared).unwrap();
        let opened = reopen(&session);
        let rust = compile_rust(&opened);
        let shape = &rust.snapshot().shapes[&cut_id];
        let expected = 24. * factors.into_iter().product::<f64>() - removed;
        let expected_bounds = Bounds {
            min: [1. - factors[0], 1. - factors[1], -pivot_z],
            max: [1. + factors[0], 1. + 2. * factors[1], 8. - pivot_z],
        };
        bounds(shape.bounds, expected_bounds.min, expected_bounds.max);
        near(shape.volume_mm3, expected);
        assert_eq!(shape.solid_count, 1);
        near(
            rust.solid(instance).unwrap().volume_mm3().unwrap(),
            expected,
        );
        let Resolution::Resolved {
            frame: Some(actual_frame),
            ..
        } = references::resolve(&anchor, &shape.topology, 0)
        else {
            panic!("Cut lost the named Scale cap")
        };
        assert_eq!(actual_frame, expected_frame);
        if round {
            assert!(shape.topology.elements.iter().any(|element| matches!(
                element.geometry,
                Some(references::AnalyticGeometry::CircularArc { radius_mm: 0.5, .. })
            )));
        }
        #[cfg(feature = "occt")]
        {
            let native = compiler
                .compile_with_backend(opened.document(), 0, GeometryBackend::OpenCascade, None)
                .unwrap();
            let reference = &native.snapshot().shapes[&cut_id];
            bounds(reference.bounds, shape.bounds.min, shape.bounds.max);
            near(reference.volume_mm3, shape.volume_mm3);
            assert_eq!(reference.solid_count, shape.solid_count);
        }
        session.undo(session.revision(), actor()).unwrap();
        assert_eq!(session.document(), &before_cut);
        assert_eq!(
            compile_rust(&session).snapshot().shapes[&target.feature]
                .topology
                .elements,
            topology.elements
        );
    }
}

#[test]
fn rust_compound_scale_rebuilds_saved_pattern_parts_and_owned_occurrences_atomically() {
    use multicad_core::{
        document::Link,
        references::{
            self, AnalyticGeometry, Attachment, GeometryIntent, Origin, Resolution, TopologyKind,
        },
    };
    let compiler = Compiler {
        omit_display: true,
        ..Default::default()
    };
    for occurrence in [false, true] {
        let (mut session, part, factor, instance, other) = fixture();
        let pattern = AdvancedRequest::on_body(
            BodyInput {
                body: part.body,
                producer: part.feature,
            },
            AdvancedParameters::LinearPattern(LinearPatternParameters {
                count: Dimension::Literal {
                    value: 3.,
                    unit: "count".into(),
                },
                step: [10., 0., 0.].map(length),
                merge: false,
            }),
            vec![],
        );
        let prepared = prepare_advanced(&session, session.revision(), actor(), &pattern).unwrap();
        session.commit(prepared.prepared).unwrap();
        let compile_rust = |session: &Session| {
            compiler
                .compile_with_backend(
                    session.document(),
                    session.revision(),
                    GeometryBackend::Rust,
                    None,
                )
                .unwrap()
        };
        let before = compile_rust(&session);
        let original = if occurrence {
            &before.snapshot().occurrences[&instance].topology
        } else {
            &before.snapshot().shapes[&pattern.feature].topology
        };
        assert_eq!(original.elements.len(), 78);
        let mut intents = original
            .elements
            .iter()
            .map(|element| {
                let mut intent = GeometryIntent::capture(original, element.key).unwrap();
                intent.boundary = Some(element.boundary.clone());
                intent
            })
            .collect::<Vec<_>>();
        let cap = Origin {
            source: part.feature,
            role: "cap.end".into(),
        };
        let cap_index = intents
            .iter()
            .position(|intent| {
                intent.kind == TopologyKind::Face
                    && intent.origins.contains(&cap)
                    && intent.origins.contains(&Origin {
                        source: pattern.feature,
                        role: "member/1".into(),
                    })
            })
            .unwrap();
        intents[cap_index].attachment = Some(Attachment {
            anchor: cap,
            offset_mm: [0.; 3],
            angle_rad: 0.,
        });
        let c = 0.5_f64.sqrt();
        let parameters = ScaleParameters {
            factors: [
                Dimension::Expression {
                    expression: BoundExpression::bind(
                        session.document(),
                        "=$factor",
                        "ratio",
                        None,
                    )
                    .unwrap(),
                },
                ratio(3.),
                ratio(0.5),
            ],
            pivot: [3., 4., 5.].map(length),
            axes: [[c, c, 0.], [-c, c, 0.], [0., 0., 1.]],
        };
        let target = if occurrence {
            ScaleTarget::Instance { instance }
        } else {
            ScaleTarget::Part {
                input: BodyInput {
                    body: part.body,
                    producer: pattern.feature,
                },
            }
        };
        let request =
            ScaleBatchRequest::new(session.document(), vec![target], parameters, false).unwrap();
        let old_document = session.document().clone();
        let prepared =
            prepare_scale_batch(&session, session.revision(), actor(), &request).unwrap();
        let output = if occurrence {
            pattern.feature
        } else {
            prepared.outputs[0].feature
        };
        session.commit(prepared.prepared).unwrap();
        let scaled_document = session.document().clone();
        session.undo(session.revision(), actor()).unwrap();
        assert_eq!(session.document(), &old_document);
        near(
            compile_rust(&session)
                .solid(instance)
                .unwrap()
                .volume_mm3()
                .unwrap(),
            72.,
        );
        session.redo(session.revision(), actor()).unwrap();
        assert_eq!(session.document(), &scaled_document);
        let datum = Entity::new(
            "Scaled middle member cap",
            EntityData::Plane {
                origin_mm: [0.; 3],
                normal: [0., 0., 1.],
                anchor: Some(Link {
                    target: if occurrence { instance } else { output },
                    role: "face_attachment".into(),
                    geometry: Some(intents[cap_index].clone()),
                }),
            },
        );
        let datum_id = datum.id;
        session
            .apply(Request {
                expected_revision: session.revision(),
                actor: actor(),
                label: "Save scaled face reference".into(),
                edits: vec![Mutation::Insert(datum)],
            })
            .unwrap();
        for (factor_value, width, expected_volume) in
            [(2., 2., 216.), (3., 2., 324.), (3., 4., 648.)]
        {
            let mut entity = session.document().get(factor).unwrap().clone();
            let EntityData::Parameter { expression, .. } = &mut entity.data else {
                panic!()
            };
            *expression = factor_value.to_string();
            session
                .apply(Request {
                    expected_revision: session.revision(),
                    actor: actor(),
                    label: "Edit bound compound scale factor".into(),
                    edits: vec![Mutation::Replace(entity)],
                })
                .unwrap();
            let mut parameters = part.parameters.clone();
            let PrimitiveParameters::Box { size, .. } = &mut parameters else {
                panic!()
            };
            size[0] = length(width);
            let prepared = prepare_primitive_edit(
                &session,
                session.revision(),
                actor(),
                part.feature,
                &parameters,
            )
            .unwrap();
            session.commit(prepared.prepared).unwrap();
            let opened = reopen(&session);
            let rust = compile_rust(&opened);
            let topology = if occurrence {
                &rust.snapshot().occurrences[&instance].topology
            } else {
                &rust.snapshot().shapes[&output].topology
            };
            let solid = rust
                .solid(if occurrence { instance } else { output })
                .unwrap();
            near(solid.volume_mm3().unwrap(), expected_volume);
            assert_eq!(solid.solid_count().unwrap(), 3);
            assert_eq!(topology.elements.len(), 78);
            for intent in &intents {
                assert!(
                    matches!(
                        references::resolve(intent, topology, opened.revision()),
                        Resolution::Resolved { .. }
                    ),
                    "lost saved member intent: {intent:?}"
                );
            }
            assert!(opened.document().get(datum_id).is_some());
            let expected_unscaled = 3. * width * 3. * 4.;
            near(
                rust.solid(other).unwrap().volume_mm3().unwrap(),
                if occurrence {
                    expected_unscaled
                } else {
                    expected_volume
                },
            );
            if occurrence {
                assert_eq!(
                    rust.snapshot().occurrences[&instance].pose,
                    before.snapshot().occurrences[&instance].pose
                );
                near(
                    rust.solid(part.body).unwrap().volume_mm3().unwrap(),
                    expected_unscaled,
                );
            }
            // Independently transform the eight corners of all three input boxes.
            let matrix = [
                [(factor_value + 3.) / 2., (factor_value - 3.) / 2., 0.],
                [(factor_value - 3.) / 2., (factor_value + 3.) / 2., 0.],
                [0., 0., 0.5],
            ];
            let pivot = [3., 4., 5.];
            let mut min = [f64::INFINITY; 3];
            let mut max = [f64::NEG_INFINITY; 3];
            for member in 0..3 {
                for corner in 0..8 {
                    let local = [
                        member as f64 * 10. + if corner & 1 != 0 { width } else { 0. },
                        if corner & 2 != 0 { 3. } else { 0. },
                        if corner & 4 != 0 { 4. } else { 0. },
                    ];
                    let point = if occurrence {
                        [100. - local[1], 50. + local[0], local[2]]
                    } else {
                        local
                    };
                    for i in 0..3 {
                        let value = pivot[i]
                            + (0..3)
                                .map(|j| matrix[i][j] * (point[j] - pivot[j]))
                                .sum::<f64>();
                        min[i] = min[i].min(value);
                        max[i] = max[i].max(value);
                    }
                }
            }
            bounds(solid.bounds().unwrap(), min, max);
            assert!(
                topology
                    .elements
                    .iter()
                    .filter(|e| e.key.kind == TopologyKind::Face)
                    .all(|e| matches!(e.geometry, Some(AnalyticGeometry::Plane { .. })))
            );
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("compound-scale.step");
            solid.write_step(&path).unwrap();
            let imported = Solid::import_rust_step(
                &std::fs::read_to_string(&path).unwrap(),
                EntityId::new(),
                GeometryPolicy::default(),
            )
            .unwrap();
            near(imported.volume_mm3().unwrap(), expected_volume);
            assert_eq!(imported.solid_count().unwrap(), 3);
            bounds(imported.bounds().unwrap(), min, max);
            #[cfg(feature = "occt")]
            {
                let native = compiler
                    .compile_with_backend(
                        opened.document(),
                        opened.revision(),
                        GeometryBackend::OpenCascade,
                        None,
                    )
                    .unwrap();
                let native = native
                    .solid(if occurrence { instance } else { output })
                    .unwrap();
                near(native.volume_mm3().unwrap(), expected_volume);
                bounds(native.bounds().unwrap(), min, max);
                let readback = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
                near(readback.volume_mm3().unwrap(), expected_volume);
                assert_eq!(readback.solid_count().unwrap(), 3);
            }
        }
        let subject = if occurrence { instance } else { part.body };
        let set = EntityId::new();
        let prepared = exchange::prepare_set(
            &session,
            session.revision(),
            actor(),
            set,
            "Scaled compound",
            &SetDefinition {
                scope: exchange::Scope {
                    root: exchange::ScopeRoot::Explicit {
                        entities: vec![subject],
                    },
                    tags: None,
                },
                ..Default::default()
            },
        )
        .unwrap();
        session.commit(prepared).unwrap();
        let worker =
            ExportWorker::with_geometry_backend(Exporter::default(), GeometryBackend::Rust)
                .unwrap();
        worker
            .preview(session.document(), session.revision(), set, actor())
            .unwrap();
        let ExportResult::Preview(preview) = export_reply(&worker).result else {
            panic!("Expected compound export preview")
        };
        let preview = preview.unwrap();
        assert!(preview.can_run(), "{preview:?}");
        let directory = tempfile::tempdir().unwrap();
        worker
            .run(
                session.document(),
                session.revision(),
                &preview.key,
                directory.path(),
                actor(),
            )
            .unwrap();
        let ExportResult::Run(receipt) = export_reply(&worker).result else {
            panic!("Expected compound export receipt")
        };
        let receipt = receipt.unwrap();
        assert_eq!(receipt.files.len(), 1);
        let file = &receipt.files[0];
        let exchange::OutputSubject::Part(subject) = &file.artifact.output.subject else {
            panic!("Expected exact Part attribution")
        };
        if occurrence {
            assert_eq!(subject.occurrence_path, vec![instance]);
        } else {
            assert_eq!(subject.part, part.body);
            assert!(subject.occurrence_path.is_empty());
        }
        let path = Path::new(&receipt.directory).join(&file.artifact.output.filename);
        let imported = Solid::import_rust_step(
            &std::fs::read_to_string(path).unwrap(),
            EntityId::new(),
            GeometryPolicy::default(),
        )
        .unwrap();
        near(imported.volume_mm3().unwrap(), 648.);
        assert_eq!(imported.solid_count().unwrap(), 3);
    }
}

#[test]
fn indexed_planar_scale_rebuilds_saved_cut_part_occurrences_datums_and_parameter_edits() {
    use multicad_core::references::{
        self, Attachment, GeometryIntent, ReferenceAddress, Resolution, TopologyKind,
    };
    let (mut session, part, factor, instance, other) = fixture();
    let tool = PrimitiveRequest::new(
        "Angled pocket tool",
        PrimitiveParameters::Box {
            size: [0.5, 1., 1.].map(length),
            frame: PrimitiveFrame {
                origin: [0.5, 0.5, 3.5].map(length),
                u: [0.8, 0., -0.6],
                v: [0., 1., 0.],
            },
        },
    );
    session
        .commit(
            prepare_primitive(&session, session.revision(), actor(), &tool)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let cut = BooleanRequest::new(
        Operation::Cut,
        BodyInput {
            body: part.body,
            producer: part.feature,
        },
        vec![BodyInput {
            body: tool.body,
            producer: tool.feature,
        }],
    );
    session
        .commit(
            prepare_boolean(&session, session.revision(), actor(), &cut)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let compiler = Compiler::default();
    let build = |s: &Session, backend| {
        compiler
            .compile_with_backend(s.document(), s.revision(), backend, None)
            .unwrap()
    };
    let before = build(&session, GeometryBackend::Rust);
    let original = &before.snapshot().shapes[&cut.feature].topology;
    let intents: Vec<_> = original
        .elements
        .iter()
        .map(|e| GeometryIntent::capture(original, e.key).unwrap())
        .collect();
    let occurrence_intents: Vec<_> = [instance, other]
        .map(|id| {
            let topology = &before.snapshot().occurrences[&id].topology;
            (
                id,
                topology
                    .elements
                    .iter()
                    .map(|e| GeometryIntent::capture(topology, e.key).unwrap())
                    .collect::<Vec<_>>(),
            )
        })
        .into();
    let request = ScaleBatchRequest::new(
        session.document(),
        vec![ScaleTarget::Part {
            input: BodyInput {
                body: part.body,
                producer: cut.feature,
            },
        }],
        bound(&session),
        false,
    )
    .unwrap();
    let prepared = prepare_scale_batch(&session, session.revision(), actor(), &request).unwrap();
    let feature = prepared.outputs[0].feature;
    session.commit(prepared.prepared).unwrap();
    let scaled = build(&session, GeometryBackend::Rust);
    let topology = &scaled.snapshot().shapes[&feature].topology;
    let face = topology
        .elements
        .iter()
        .find(|e| e.key.kind == TopologyKind::Face && !e.frames.is_empty())
        .unwrap();
    let mut intent = GeometryIntent::capture(topology, face.key).unwrap();
    intent.attachment = Some(Attachment {
        anchor: face.frames[0].origin.clone(),
        offset_mm: [0.; 3],
        angle_rad: 0.,
    });
    let reference = intent.id;
    let datum = Entity::new(
        "Scaled pocket datum",
        EntityData::Plane {
            origin_mm: [0.; 3],
            normal: [0., 0., 1.],
            anchor: Some(multicad_core::document::Link {
                target: feature,
                role: "face_attachment".into(),
                geometry: Some(intent),
            }),
        },
    );
    let owner = datum.id;
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Attach scaled pocket datum".into(),
            edits: vec![Mutation::Insert(datum)],
        })
        .unwrap();
    for value in ["2", "3"] {
        let mut parameter = session.document().get(factor).unwrap().clone();
        let EntityData::Parameter { expression, .. } = &mut parameter.data else {
            panic!()
        };
        *expression = value.into();
        session
            .apply(Request {
                expected_revision: session.revision(),
                actor: actor(),
                label: "Edit scale factor".into(),
                edits: vec![Mutation::Replace(parameter)],
            })
            .unwrap();
        let opened = reopen(&session);
        let rust = build(&opened, GeometryBackend::Rust);
        let scale: f64 = value.parse().unwrap();
        let volume = before.solid(part.body).unwrap().volume_mm3().unwrap() * scale.powi(3);
        for id in [part.body, instance, other] {
            near(rust.solid(id).unwrap().volume_mm3().unwrap(), volume);
        }
        let topology = &rust.snapshot().shapes[&feature].topology;
        for intent in &intents {
            assert!(matches!(
                references::resolve(intent, topology, opened.revision()),
                Resolution::Resolved { .. }
            ));
        }
        for (id, intents) in &occurrence_intents {
            let topology = &rust.snapshot().occurrences[id].topology;
            for intent in intents {
                assert!(matches!(
                    references::resolve(intent, topology, opened.revision()),
                    Resolution::Resolved { .. }
                ));
            }
        }
        assert!(matches!(
            rust.snapshot()
                .references
                .get(ReferenceAddress { owner, reference })
                .unwrap()
                .resolution,
            Resolution::Resolved { frame: Some(_), .. }
        ));
        rust.solid(part.body)
            .unwrap()
            .manufacturing_mesh_with_receipt(Default::default())
            .unwrap();
        #[cfg(feature = "occt")]
        {
            let native = build(&opened, GeometryBackend::OpenCascade);
            near(
                native.solid(part.body).unwrap().volume_mm3().unwrap(),
                volume,
            );
            let expected = native.solid(part.body).unwrap().bounds().unwrap();
            bounds(
                rust.solid(part.body).unwrap().bounds().unwrap(),
                expected.min,
                expected.max,
            );
        }
    }
    let final_document = session.document().clone();
    session.undo(session.revision(), actor()).unwrap();
    session.redo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &final_document);
    let rebuilt = build(&session, GeometryBackend::Rust);
    near(
        rebuilt.solid(part.body).unwrap().volume_mm3().unwrap(),
        before.solid(part.body).unwrap().volume_mm3().unwrap() * 27.,
    );
}
