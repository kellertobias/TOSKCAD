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
