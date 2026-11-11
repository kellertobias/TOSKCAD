//! Assembled Rust bevel pairs publish one verified multi-solid STEP.
use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::Dimension,
    document::{Document, EntityId},
    exchange::{
        ExportReply, ExportResult, OutputSubject, RunReceipt, Scope, ScopeRoot, SetDefinition,
        prepare_set,
    },
    model::{
        self, BevelDimensions, BevelToothSystem, ComponentPlacementRequest, GearFamily,
        GearPairRequest, GearParameters, GearRequest, NumericTransform, OccurrencePath, angle,
        length,
    },
};
use multicad_kernel::Solid;
use multicad_model::{
    Compiler, GeometryBackend,
    export::{ExportWorker, Exporter},
};
use std::{
    path::Path,
    time::{Duration, Instant},
};

fn actor() -> Actor {
    Actor::local(Channel::Ui)
}

fn near(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{actual} != {expected}"
    );
}

fn bounds_match(actual: [[f64; 3]; 2], expected: [[f64; 3]; 2], tolerance: f64) {
    for (actual, expected) in actual
        .into_iter()
        .flatten()
        .zip(expected.into_iter().flatten())
    {
        near(actual, expected, tolerance);
    }
}

fn member(teeth: u32, mate: u32, bore: f64) -> GearParameters {
    let count = |value: u32| Dimension::Literal {
        value: f64::from(value),
        unit: "count".into(),
    };
    GearParameters {
        family: GearFamily::Bevel,
        teeth: count(teeth),
        face_width: length(6.),
        bore_diameter: length(bore),
        bevel: Some(BevelDimensions {
            tooth_system: BevelToothSystem::SphericalInvoluteStraight,
            mate_teeth: count(mate),
            shaft_angle: angle(90.),
        }),
        ..Default::default()
    }
}

fn reply(worker: &ExportWorker, request: EntityId) -> ExportReply {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(reply) = worker.poll() {
            assert_eq!(reply.id, request);
            return reply;
        }
        assert!(Instant::now() < deadline, "Bevel STEP worker did not reply");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn publish(session: &Session, set: EntityId, directory: &Path) -> RunReceipt {
    let worker =
        ExportWorker::with_geometry_backend(Exporter::default(), GeometryBackend::Rust).unwrap();
    let request = worker
        .preview(session.document(), session.revision(), set, actor())
        .unwrap();
    let ExportResult::Preview(preview) = reply(&worker, request).result else {
        panic!("Expected STEP preview");
    };
    let preview = preview.unwrap();
    assert!(preview.can_run(), "{preview:?}");
    let request = worker
        .run(
            session.document(),
            session.revision(),
            &preview.key,
            directory,
            actor(),
        )
        .unwrap();
    let ExportResult::Run(run) = reply(&worker, request).result else {
        panic!("Expected verified STEP publication");
    };
    run.unwrap()
}

#[cfg(feature = "occt")]
fn native_readback(path: &Path, expected: &Solid) {
    let native = Solid::read_step(path, Default::default()).unwrap();
    assert_eq!(
        native.solid_count().unwrap(),
        expected.solid_count().unwrap()
    );
    let volume = expected.volume_mm3().unwrap();
    near(native.volume_mm3().unwrap(), volume, 1e-7 * volume);
    let bounds = native.bounds().unwrap();
    let expected = expected.bounds().unwrap();
    bounds_match([bounds.min, bounds.max], [expected.min, expected.max], 2e-3);
}

#[test]
fn placed_bevel_pair_assemblies_export_one_verified_two_solid_step() {
    let mut session = Session::new(Document::new("Bevel pair assembly").unwrap()).unwrap();
    let request = GearPairRequest::new(
        "Bevel pair",
        [(24, 32, 8.), (32, 24, 0.)]
            .map(|(z, mate, bore)| GearRequest::new("Bevel", member(z, mate, bore))),
    );
    let draft = model::prepare_gear_pair(&session, 0, actor(), &request).unwrap();
    session.commit(draft.prepared).unwrap();
    let root = ComponentPlacementRequest::new(
        request.outputs.assembly,
        NumericTransform {
            translation: [-12., 8., 4.].map(length),
            axis: [0.6, 0., 0.8],
            angle: angle(35.),
            ..Default::default()
        },
    );
    let prepared =
        model::prepare_place_component(&session, session.revision(), actor(), &root).unwrap();
    session.commit(prepared.prepared).unwrap();
    let compiler = Compiler {
        omit_display: true,
        ..Default::default()
    };
    let model = compiler
        .compile_with_backend(
            session.document(),
            session.revision(),
            GeometryBackend::Rust,
            None,
        )
        .unwrap();
    let aggregate = model.solid(root.instance).unwrap();
    assert!(aggregate.uses_rust_geometry());
    assert_eq!(aggregate.solid_count().unwrap(), 2);
    assert!(aggregate.supports_step_export());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("bevel-pair.step");
    aggregate.write_step(&path).unwrap();
    let source = std::fs::read_to_string(&path).unwrap();
    assert_eq!(source.matches("MANIFOLD_SOLID_BREP").count(), 2);
    let tolerance = compiler.policy.linear_tolerance_mm;
    let (count, volume, bounds) = aggregate
        .verify_twisted_step_compound(&source, tolerance)
        .expect("compound bevel verification")
        .unwrap();
    assert_eq!(count, 2);
    let expected = aggregate.volume_mm3().unwrap();
    near(volume, expected, 1e-8 * expected);
    let world = aggregate.bounds().unwrap();
    bounds_match(bounds, [world.min, world.max], 1e-9);
    #[cfg(feature = "occt")]
    native_readback(&path, aggregate);
    println!("{{\"assembled_bevel_pair_volume_mm3\":{expected},\"rust_readback_mm3\":{volume}}}");

    // A placed member exported alone cannot pass as the pair, and a
    // narrower authored pair cannot certify the original file.
    let leaf = model
        .leaf_solid(&OccurrencePath {
            root: root.instance,
            members: vec![request.outputs.instances[0]],
        })
        .unwrap();
    let single = directory.path().join("member.step");
    leaf.write_step(&single).unwrap();
    let single_source = std::fs::read_to_string(&single).unwrap();
    assert!(
        aggregate
            .verify_twisted_step_compound(&single_source, tolerance)
            .unwrap()
            .is_err()
    );
    let mut parameters = request.parameters.clone();
    parameters.drive_angle = angle(0.);
    let narrower = [(24, 32, 8.), (32, 24, 0.)].map(|(z, mate, bore)| {
        let mut gear = member(z, mate, bore);
        gear.face_width = length(5.);
        gear
    });
    let edited = model::prepare_gear_pair_geometry_edit(
        &session,
        session.revision(),
        actor(),
        request.pair,
        &parameters,
        &narrower,
    )
    .unwrap();
    let other = compiler
        .compile_with_backend(
            edited.prepared.document(),
            session.revision() + 1,
            GeometryBackend::Rust,
            None,
        )
        .unwrap();
    assert!(
        other
            .solid(root.instance)
            .unwrap()
            .verify_twisted_step_compound(&source, tolerance)
            .unwrap()
            .is_err()
    );

    // The export worker publishes each placed member as its own verified Part.
    let set = EntityId::new();
    let prepared = prepare_set(
        &session,
        session.revision(),
        actor(),
        set,
        "Bevel STEP",
        &SetDefinition {
            scope: Scope {
                root: ScopeRoot::Explicit {
                    entities: vec![root.instance],
                },
                tags: None,
            },
            ..Default::default()
        },
    )
    .unwrap();
    session.commit(prepared).unwrap();
    let run = publish(&session, set, directory.path());
    assert_eq!(run.files.len(), 2);
    for (index, instance) in request.outputs.instances.into_iter().enumerate() {
        let leaf = model
            .leaf_solid(&OccurrencePath {
                root: root.instance,
                members: vec![instance],
            })
            .unwrap();
        let file = run
            .files
            .iter()
            .find(|file| {
                let OutputSubject::Part(subject) = &file.artifact.output.subject else {
                    return false;
                };
                subject.occurrence_path == vec![root.instance, instance]
            })
            .unwrap_or_else(|| panic!("member {index} artifact"));
        assert_eq!(file.artifact.solid_count, 1);
        near(
            file.artifact.volume_mm3,
            leaf.volume_mm3().unwrap(),
            1e-8 * file.artifact.volume_mm3,
        );
        #[cfg(feature = "occt")]
        native_readback(
            &Path::new(&run.directory).join(&file.artifact.output.filename),
            leaf,
        );
    }
}
