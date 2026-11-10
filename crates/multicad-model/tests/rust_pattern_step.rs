use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::Dimension,
    document::{Document, EntityId},
    exchange::{ExportReply, ExportResult, SetDefinition, prepare_set},
    model::{
        self, AdvancedParameters, AdvancedRequest, BodyInput, CircularPatternParameters,
        PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, angle, imports::prepare_step_import,
        length,
    },
    project_file::{self, ProjectFile},
};
use multicad_kernel::{Solid, rust_kernel::step_read};
use multicad_model::{
    Compiler, GeometryBackend,
    export::{ExportWorker, Exporter},
    imports::read_step_file_with_backend,
};
use std::{
    path::Path,
    time::{Duration, Instant},
};

fn reply(worker: &ExportWorker, id: EntityId) -> ExportReply {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(reply) = worker.poll() {
            assert_eq!(reply.id, id);
            return reply;
        }
        assert!(Instant::now() < deadline, "STEP worker did not reply");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
}

fn check_bounds(actual: [[f64; 3]; 2], expected: [[f64; 3]; 2]) {
    for (actual, expected) in actual
        .into_iter()
        .flatten()
        .zip(expected.into_iter().flatten())
    {
        near(actual, expected);
    }
}

#[test]
fn rust_step_publishes_and_reopens_partially_coalesced_circular_pattern() {
    let actor = Actor::local(Channel::Ui);
    let mut session =
        Session::new(Document::new("Partially coalesced circular pattern").unwrap()).unwrap();
    let stock = PrimitiveRequest::new(
        "Offset box",
        PrimitiveParameters::Box {
            size: [6., 10., 4.].map(length),
            frame: PrimitiveFrame {
                origin: [3., -5., 0.].map(length),
                ..Default::default()
            },
        },
    );
    let prepared =
        model::prepare_primitive(&session, session.revision(), actor.clone(), &stock).unwrap();
    session.commit(prepared.prepared).unwrap();
    let pattern = AdvancedRequest::on_body(
        BodyInput {
            body: stock.body,
            producer: stock.feature,
        },
        AdvancedParameters::CircularPattern(CircularPatternParameters {
            count: Dimension::Literal {
                value: 3.,
                unit: "count".into(),
            },
            step_angle: angle(135.),
            merge: true,
            ..Default::default()
        }),
        vec![],
    );
    let prepared =
        model::prepare_advanced(&session, session.revision(), actor.clone(), &pattern).unwrap();
    session.commit(prepared.prepared).unwrap();
    let set = EntityId::new();
    let prepared = prepare_set(
        &session,
        session.revision(),
        actor.clone(),
        set,
        "Merged pattern STEP",
        &SetDefinition::default(),
    )
    .unwrap();
    session.commit(prepared).unwrap();

    // The first and last members share a 2 × 2 × 4 overlap. The 135° member
    // has X < 3 and Y > -3 everywhere, so it misses both of the others.
    let extent = 7. * std::f64::consts::SQRT_2;
    let expected_bounds = [[-extent, -9., 0.], [9., extent, 4.]];
    let check = |solid: &Solid| {
        assert_eq!(solid.solid_count().unwrap(), 2);
        near(solid.volume_mm3().unwrap(), 704.);
        let bounds = solid.bounds().unwrap();
        check_bounds([bounds.min, bounds.max], expected_bounds);
    };
    let compiler = Compiler {
        omit_display: true,
        ..Default::default()
    };
    let compile = |session: &Session, backend| {
        compiler
            .compile_with_backend(session.document(), session.revision(), backend, None)
            .unwrap()
    };
    let rust = compile(&session, GeometryBackend::Rust);
    let solid = rust.solid(pattern.feature).unwrap();
    assert!(solid.uses_rust_geometry());
    check(solid);

    let worker =
        ExportWorker::with_geometry_backend(Exporter::default(), GeometryBackend::Rust).unwrap();
    let document = session.document();
    let revision = session.revision();
    let preview = worker
        .preview(document, revision, set, actor.clone())
        .unwrap();
    let ExportResult::Preview(preview) = reply(&worker, preview).result else {
        panic!("expected STEP preview");
    };
    let preview = preview.unwrap();
    assert!(preview.can_run(), "{preview:?}");
    let directory = tempfile::tempdir().unwrap();
    let run = worker
        .run(
            document,
            revision,
            &preview.key,
            directory.path(),
            actor.clone(),
        )
        .unwrap();
    let ExportResult::Run(run) = reply(&worker, run).result else {
        panic!("expected verified STEP publication");
    };
    let run = run.unwrap();
    let [file] = run.files.as_slice() else {
        panic!("expected one STEP artifact for the patterned Part");
    };
    assert_eq!(file.artifact.producer, pattern.feature);
    assert_eq!(file.artifact.solid_count, 2);
    near(file.artifact.volume_mm3, 704.);
    let path = Path::new(&run.directory).join(&file.artifact.output.filename);
    let mut decoded = step_read::read_analytic_compound_with_helical_cuts(
        &std::fs::read_to_string(&path).unwrap(),
        compiler.policy.linear_tolerance_mm,
    )
    .unwrap();
    assert_eq!(decoded.len(), 2);
    assert!(
        decoded
            .iter()
            .all(|(member, profile)| member.solid_count == 1 && profile.is_none())
    );
    let mut volumes: Vec<_> = decoded
        .iter()
        .map(|(member, _)| member.volume_mm3)
        .collect();
    volumes.sort_by(f64::total_cmp);
    near(volumes[0], 240.);
    near(volumes[1], 464.);
    let topology = &rust.snapshot().shapes[&pattern.feature].topology;
    let tolerance = compiler.policy.linear_tolerance_mm * 2.;
    assert!(solid.step_readback_matches(topology, &decoded, tolerance));
    decoded.swap(0, 1);
    assert!(!solid.step_readback_matches(topology, &decoded, tolerance));
    decoded.swap(0, 1);
    decoded[0].0.edges_mm.pop().unwrap();
    assert!(!solid.step_readback_matches(topology, &decoded, tolerance));

    #[cfg(feature = "occt")]
    {
        let native = compile(&session, GeometryBackend::OpenCascade);
        check(native.solid(pattern.feature).unwrap());
        check(&Solid::read_step(&path, compiler.policy).unwrap());
    }

    let input = read_step_file_with_backend(&path, GeometryBackend::Rust).unwrap();
    let mut imported = Session::new(Document::new("Reopened pattern STEP").unwrap()).unwrap();
    let prepared = prepare_step_import(&imported, imported.revision(), actor, &input).unwrap();
    assert_eq!(prepared.model.outputs.len(), 1);
    let body = prepared.model.outputs[0].body;
    imported.commit(prepared.model.prepared).unwrap();
    let mut bytes = Vec::new();
    project_file::write(
        &ProjectFile::new(imported.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    std::fs::remove_file(&path).unwrap();
    let opened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    let imported = Session::new(opened.project.document).unwrap();
    let model = compile(&imported, GeometryBackend::Rust);
    check(model.solid(body).unwrap());
}
